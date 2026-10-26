import Foundation
import Testing
@testable import GDirect

@MainActor
struct TransferQueueTests {
    @Test func downloadsRunSequentiallyInEnqueueOrder() async throws {
        let firstGate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { file, output, _, _ in
            if file == "first" {
                await firstGate.wait()
            }
            return URL(fileURLWithPath: output)
        }
        let queue = TransferQueue(service: service)

        let first = queue.enqueueDownload(
            file: makeQueueFile(id: "first", name: "First"),
            destination: URL(fileURLWithPath: "/tmp/first")
        )
        let second = queue.enqueueDownload(
            file: makeQueueFile(id: "second", name: "Second"),
            destination: URL(fileURLWithPath: "/tmp/second")
        )
        await eventually { await service.events() == ["download:first"] }

        #expect(queue.item(id: first)?.state == .running)
        #expect(queue.item(id: second)?.state == .queued)
        await firstGate.open()
        await queue.waitForIdle()

        #expect(await service.events() == ["download:first", "download:second"])
        #expect(queue.item(id: first)?.state == .succeeded)
        #expect(queue.item(id: second)?.state == .succeeded)
    }

    @Test func cancellingRunningTransferStartsNextAndDoesNotDeleteDestination() async throws {
        let service = FakeService()
        await service.setDownloadHandler { file, output, _, _ in
            if file == "cancel" {
                try await Task.sleep(for: .seconds(30))
            }
            return URL(fileURLWithPath: output)
        }
        let queue = TransferQueue(service: service)
        let cancelled = queue.enqueueDownload(
            file: makeQueueFile(id: "cancel", name: "Cancel"),
            destination: URL(fileURLWithPath: "/tmp/cancel")
        )
        let next = queue.enqueueDownload(
            file: makeQueueFile(id: "next", name: "Next"),
            destination: URL(fileURLWithPath: "/tmp/next")
        )
        await eventually { queue.item(id: cancelled)?.state == .running }

        queue.cancel(id: cancelled)
        await queue.waitForIdle()

        #expect(queue.item(id: cancelled)?.state == .cancelled)
        #expect(queue.item(id: next)?.state == .succeeded)
        #expect(await service.events() == ["download:cancel", "download:next"])
    }

    @Test func cancellingQueuedTransferNeverStartsIt() async throws {
        let firstGate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { file, output, _, _ in
            if file == "blocking" { await firstGate.wait() }
            return URL(fileURLWithPath: output)
        }
        let queue = TransferQueue(service: service)
        _ = queue.enqueueDownload(
            file: makeQueueFile(id: "blocking", name: "Blocking"),
            destination: URL(fileURLWithPath: "/tmp/blocking")
        )
        let queued = queue.enqueueDownload(
            file: makeQueueFile(id: "queued", name: "Queued"),
            destination: URL(fileURLWithPath: "/tmp/queued")
        )
        queue.cancel(id: queued)
        await firstGate.open()
        await queue.waitForIdle()

        #expect(queue.item(id: queued)?.state == .cancelled)
        #expect(await service.events() == ["download:blocking"])
    }

    @Test func failedTransferCanBeRetriedWithSameIdentity() async throws {
        let service = FakeService()
        await service.setDownloadError(TestError.failed)
        let queue = TransferQueue(service: service)
        let id = queue.enqueueDownload(
            file: makeQueueFile(id: "retry", name: "Retry"),
            destination: URL(fileURLWithPath: "/tmp/retry")
        )
        await queue.waitForIdle()
        #expect(queue.item(id: id)?.state == .failed)

        await service.setDownloadError(nil)
        queue.retry(id: id)
        #expect(queue.item(id: id)?.state == .queued)
        await queue.waitForIdle()

        #expect(queue.item(id: id)?.state == .succeeded)
        #expect(await service.events() == ["download:retry", "download:retry"])
    }

    @Test func lateSuccessAfterCancellationCannotOverwriteCancelledState() async throws {
        let completionGate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { _, output, _, _ in
            await completionGate.wait()
            return URL(fileURLWithPath: output)
        }
        let queue = TransferQueue(service: service)
        let id = queue.enqueueDownload(
            file: makeQueueFile(id: "late", name: "Late"),
            destination: URL(fileURLWithPath: "/tmp/late")
        )
        await eventually { queue.item(id: id)?.state == .running }

        queue.cancel(id: id)
        #expect(queue.item(id: id)?.state == .cancelled)
        await completionGate.open()
        await queue.waitForIdle()

        #expect(queue.item(id: id)?.state == .cancelled)
    }

    @Test func publicLinkDownloadUsesPredeterminedDestination() async throws {
        let service = FakeService()
        let queue = TransferQueue(service: service)
        let destination = URL(fileURLWithPath: "/tmp/public result.pdf")

        let id = queue.enqueuePublicDownload(
            link: "https://drive.google.com/file/d/public/view",
            name: "Public result",
            destination: destination
        )
        await queue.waitForIdle()

        #expect(queue.item(id: id)?.state == .succeeded)
        #expect(queue.item(id: id)?.destination == destination)
        #expect(await service.events().contains("download:https://drive.google.com/file/d/public/view"))
    }
}

private func makeQueueFile(id: String, name: String) -> DriveFile {
    DriveFile(
        id: id,
        name: name,
        mimeType: "application/octet-stream",
        size: nil,
        modifiedTime: nil,
        md5Checksum: nil,
        resourceKey: nil
    )
}

enum TestError: Error, LocalizedError {
    case failed

    var errorDescription: String? { "Test operation failed" }
}

actor AsyncGate {
    private var isOpen = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        if isOpen { return }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func open() {
        isOpen = true
        let current = waiters
        waiters.removeAll()
        current.forEach { $0.resume() }
    }
}

actor FakeService: GDirectServing {
    typealias ListHandler = @Sendable (String) async throws -> [DriveFile]
    typealias DownloadHandler = @Sendable (String, String, String?, TransferSettings) async throws -> URL
    typealias UploadHandler = @Sendable () async throws -> CLIResult

    private var recordedEvents: [String] = []
    private var recordedGoogleOnly: [Bool] = []
    private var listResponses: [String: [DriveFile]] = [:]
    private var listHandler: ListHandler?
    private var listError: Error?
    private var downloadHandler: DownloadHandler?
    private var downloadError: Error?
    private var uploadHandler: UploadHandler?
    private var uploadError: Error?
    private var accountError: Error?
    private var accountList: [DriveAccount] = []
    private var maintenanceError: Error?
    private var scanListResult = CLIResult(status: 0, stdout: "", stderr: "")

    func events() -> [String] { recordedEvents }
    func googleOnlyValues() -> [Bool] { recordedGoogleOnly }
    func clearEvents() { recordedEvents.removeAll() }
    func listCallCount() -> Int { recordedEvents.filter { $0.hasPrefix("list:") }.count }
    func setListResponses(_ responses: [String: [DriveFile]]) { listResponses = responses }
    func setListHandler(_ handler: @escaping ListHandler) { listHandler = handler }
    func setListError(_ error: Error?) { listError = error }
    func setDownloadHandler(_ handler: @escaping DownloadHandler) { downloadHandler = handler }
    func setDownloadError(_ error: Error?) { downloadError = error }
    func setUploadHandler(_ handler: @escaping UploadHandler) { uploadHandler = handler }
    func setUploadError(_ error: Error?) { uploadError = error }
    func setAccountError(_ error: Error?) { accountError = error }
    func setAccounts(_ accounts: [DriveAccount]) { accountList = accounts }
    func setMaintenanceError(_ error: Error?) { maintenanceError = error }
    func setScanListResult(_ result: CLIResult) { scanListResult = result }

    func login(clientSecret: String?, credentials: String?, googleOnly: Bool) async throws -> CLIResult {
        if let accountError { throw accountError }
        record("login", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func reauth(googleOnly: Bool) async throws -> CLIResult {
        if let accountError { throw accountError }
        record("reauth", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func accounts(googleOnly: Bool) async throws -> [DriveAccount] {
        if let accountError { throw accountError }
        record("accounts", googleOnly: googleOnly)
        return accountList
    }

    func switchAccount(_ account: String, googleOnly: Bool) async throws -> CLIResult {
        if let accountError { throw accountError }
        record("switch:\(account)", googleOnly: googleOnly)
        accountList = accountList.map {
            DriveAccount(email: $0.email, isDefault: $0.email == account)
        }
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func logout(account: String?, googleOnly: Bool) async throws -> CLIResult {
        if let accountError { throw accountError }
        record("logout", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func logoutAll(googleOnly: Bool) async throws -> CLIResult {
        if let accountError { throw accountError }
        record("logoutAll", googleOnly: googleOnly)
        accountList = []
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func list(folder: String, ip: String?, sni: String?, googleOnly: Bool) async throws -> [DriveFile] {
        record("list:\(folder)", googleOnly: googleOnly)
        if let listError { throw listError }
        if let listHandler { return try await listHandler(folder) }
        return listResponses[folder] ?? []
    }

    func download(
        file: String,
        output: String,
        exportMIME: String?,
        settings: TransferSettings
    ) async throws -> URL {
        record("download:\(file)", googleOnly: settings.googleOnly)
        if let downloadError { throw downloadError }
        if let downloadHandler {
            return try await downloadHandler(file, output, exportMIME, settings)
        }
        return URL(fileURLWithPath: output)
    }

    func upload(file: String, parent: String, name: String?, ip: String?, sni: String?, googleOnly: Bool) async throws -> CLIResult {
        record("upload:\(file):\(parent)", googleOnly: googleOnly)
        if let uploadError { throw uploadError }
        if let uploadHandler { return try await uploadHandler() }
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func scan(ips: String?, googleOnly: Bool) async throws -> CLIResult {
        if let maintenanceError { throw maintenanceError }
        record("scan", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func scanList(googleOnly: Bool) async throws -> CLIResult {
        if let maintenanceError { throw maintenanceError }
        record("scanList", googleOnly: googleOnly)
        return scanListResult
    }

    func mount(path: String, readOnly: Bool, connections: Int, googleOnly: Bool) async throws -> CLIResult {
        if let maintenanceError { throw maintenanceError }
        record("mount:\(path)", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    func unmount(googleOnly: Bool) async throws -> CLIResult {
        if let maintenanceError { throw maintenanceError }
        record("unmount", googleOnly: googleOnly)
        return CLIResult(status: 0, stdout: "", stderr: "")
    }

    private func record(_ event: String, googleOnly: Bool) {
        recordedEvents.append(event)
        recordedGoogleOnly.append(googleOnly)
    }
}

@MainActor
func eventually(
    timeout: UInt64 = 2_000_000_000,
    condition: @MainActor () async -> Bool
) async {
    let start = DispatchTime.now().uptimeNanoseconds
    while await !condition() {
        if DispatchTime.now().uptimeNanoseconds - start > timeout {
            Issue.record("Timed out waiting for asynchronous condition")
            return
        }
        try? await Task.sleep(for: .milliseconds(5))
    }
}
