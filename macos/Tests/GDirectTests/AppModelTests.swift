import Foundation
import Testing
@testable import GDirect

@MainActor
struct AppModelTests {
    @Test func activeTransfersBlockEveryCredentialMutation() async {
        let gate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { _, output, _, _ in
            await gate.wait()
            return URL(fileURLWithPath: output)
        }
        let model = AppModel(service: service, defaults: makeDefaults())
        _ = model.downloadPublicLink(link: "first", name: "first", destination: URL(fileURLWithPath: "/tmp/first"))
        _ = model.downloadPublicLink(link: "second", name: "second", destination: URL(fileURLWithPath: "/tmp/second"))
        await model.login(clientSecret: nil, credentials: nil)
        await model.reauth()
        await model.logout()
        #expect(model.errorMessage?.contains("transfers") == true)
        #expect(await service.events().allSatisfy { $0.hasPrefix("download:") })
        await gate.open()
        await model.transfers.waitForIdle()
        await model.logout()
        #expect(await service.events().contains("logout"))
    }

    @Test func credentialGateRejectsEnqueueAndRetryUntilReleased() async {
        let service = FakeService()
        let model = AppModel(service: service, defaults: makeDefaults())
        #expect(model.transfers.beginCredentialChange())
        let id = model.upload(source: URL(fileURLWithPath: "/tmp/blocked"))
        model.transfers.retry(id: id)
        #expect(model.transfers.item(id: id)?.state == .failed)
        #expect(model.transfers.item(id: id)?.errorMessage?.isEmpty == false)
        #expect(await service.events().isEmpty)
        model.transfers.endCredentialChange()
        model.transfers.retry(id: id)
        await model.transfers.waitForIdle()
        #expect(model.transfers.item(id: id)?.state == .succeeded)
    }

    @Test func accountChangeResetsHistoryAndRejectsOldList() async {
        let gate = AsyncGate()
        let service = FakeService()
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.openFolder(id: "old", name: "Old account")
        await service.setListHandler { folder in
            if folder == "old" { await gate.wait() }
            return [makeFile(id: folder, name: folder)]
        }
        let old = Task { await model.refresh() }
        await eventually { await service.listCallCount() == 2 }
        await model.reauth()
        #expect(model.currentFolderID == "root")
        #expect(!model.canGoBack && !model.canGoForward)
        #expect(model.files.map(\.id) == ["root"])
        await gate.open()
        await old.value
        #expect(model.files.map(\.id) == ["root"])
        await model.openFolder(id: "child", name: "Child")
        await model.logout()
        #expect(model.currentFolderID == "root")
        #expect(!model.canGoBack && !model.canGoForward)
        #expect(model.files.isEmpty && !model.isAuthenticated)
    }

    @Test func startupListingRecognizesExistingCredentials() async {
        let model = AppModel(service: FakeService(), defaults: makeDefaults())
        #expect(!model.isAuthenticated)
        await model.loadRoot()
        #expect(model.isAuthenticated)
    }

    @Test func folderNavigationSupportsBackAndForward() async throws {
        let service = FakeService()
        await service.setListResponses([
            "root": [makeFile(id: "a", name: "A")],
            "b": [makeFile(id: "b", name: "B")],
            "c": [makeFile(id: "c", name: "C")]
        ])
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.openFolder(id: "root", name: "My Drive")
        await model.openFolder(id: "b", name: "Folder B")
        await model.openFolder(id: "c", name: "Folder C")
        await model.goBack()

        #expect(model.currentFolderID == "b")
        #expect(model.files.map(\.id) == ["b"])
        #expect(model.canGoForward)

        await model.goForward()

        #expect(model.currentFolderID == "c")
        #expect(model.files.map(\.id) == ["c"])
        #expect(model.canGoForward == false)
    }

    @Test func staleListSuccessCannotReplaceNewerFolder() async throws {
        let staleGate = AsyncGate()
        let service = FakeService()
        await service.setListHandler { folder in
            if folder == "root" {
                await staleGate.wait()
                return [makeFile(id: "stale", name: "Stale")]
            }
            return [makeFile(id: "current", name: "Current")]
        }
        let model = AppModel(service: service, defaults: makeDefaults())

        let staleRequest = Task { await model.openFolder(id: "root", name: "My Drive") }
        await eventually { await service.listCallCount() == 1 }
        await model.openFolder(id: "new", name: "New folder")
        await staleGate.open()
        await staleRequest.value

        #expect(model.currentFolderID == "new")
        #expect(model.files.map(\.id) == ["current"])
        #expect(model.errorMessage == nil)
    }

    @Test func failedRefreshPreservesCurrentFiles() async throws {
        let service = FakeService()
        await service.setListResponses(["root": [makeFile(id: "kept", name: "Kept")]])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.openFolder(id: "root", name: "My Drive")
        await service.setListError(TestError.failed)

        await model.refresh()

        #expect(model.files.map(\.id) == ["kept"])
        #expect(model.errorMessage?.isEmpty == false)
        #expect(model.isLoading == false)
    }

    @Test func workspaceDownloadDefaultsToPDFAndCustomExportIsPreserved() async throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let service = FakeService()
        let model = AppModel(service: service, defaults: makeDefaults())
        let document = makeFile(
            id: "doc",
            name: "Plan",
            mimeType: "application/vnd.google-apps.document"
        )

        let pdf = try model.destination(for: document, in: directory)
        let custom = try model.destination(
            for: document,
            in: directory,
            exportMIME: "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        )

        #expect(pdf.lastPathComponent == "Plan.pdf")
        #expect(custom.lastPathComponent == "Plan.docx")
        #expect(model.exportMIME(for: document) == "application/pdf")
    }

    @Test func destinationNamingHandlesUnicodeSlashesControlsAndOptionLikeNames() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let model = AppModel(service: FakeService(), defaults: makeDefaults())
        let file = makeFile(id: "odd", name: "资料/草案\n--output=其他.txt")

        let destination = try model.destination(for: file, in: directory)

        #expect(destination.deletingLastPathComponent().standardizedFileURL == directory.standardizedFileURL)
        #expect(destination.lastPathComponent == "资料-草案 --output=其他.txt")
        #expect(destination.lastPathComponent.contains("/") == false)
        #expect(destination.lastPathComponent.contains("\n") == false)
    }

    @Test func destinationNamingAvoidsExistingFilesWithoutOverwrite() throws {
        let directory = try makeTemporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data().write(to: directory.appendingPathComponent("报告.pdf"))
        try Data().write(to: directory.appendingPathComponent("报告 2.pdf"))
        let model = AppModel(service: FakeService(), defaults: makeDefaults())

        let destination = try model.destination(for: makeFile(id: "report", name: "报告.pdf"), in: directory)

        #expect(destination.lastPathComponent == "报告 3.pdf")
    }

    @Test func uploadRefreshesOnlyAfterSuccessfulCompletion() async throws {
        let uploadGate = AsyncGate()
        let service = FakeService()
        await service.setListResponses(["root": [makeFile(id: "new", name: "Uploaded")]])
        await service.setUploadHandler {
            await uploadGate.wait()
            return CLIResult(status: 0, stdout: "", stderr: "")
        }
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.openFolder(id: "root", name: "My Drive")
        await service.clearEvents()
        let source = URL(fileURLWithPath: "/tmp/local.txt")

        _ = model.upload(source: source)
        await eventually { await service.events() == ["upload:/tmp/local.txt:root"] }
        #expect(await service.listCallCount() == 0)
        await uploadGate.open()
        await model.transfers.waitForIdle()

        #expect(await service.events() == ["upload:/tmp/local.txt:root", "list:root"])
        #expect(model.files.map(\.id) == ["new"])
    }

    @Test func failedUploadDoesNotRefresh() async throws {
        let service = FakeService()
        await service.setUploadError(TestError.failed)
        let model = AppModel(service: service, defaults: makeDefaults())

        _ = model.upload(source: URL(fileURLWithPath: "/tmp/fail.txt"))
        await model.transfers.waitForIdle()

        #expect(await service.events() == ["upload:/tmp/fail.txt:root"])
        #expect(model.transfers.items.first?.state == .failed)
    }

    @Test func successfulLoginAndReauthenticationRefreshDriveAndSetConnectedState() async throws {
        let service = FakeService()
        await service.setListResponses(["root": [makeFile(id: "ready", name: "Ready")]])
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.login(clientSecret: nil, credentials: nil)

        #expect(model.isAuthenticated)
        #expect(model.files.map(\.id) == ["ready"])
        #expect(await service.events() == ["login", "list:root"])

        await service.clearEvents()
        await service.setAccountError(TestError.failed)
        await model.reauth()
        #expect(model.isAuthenticated)
        #expect(await service.events().isEmpty)

        await service.setAccountError(nil)
        await service.clearEvents()
        await model.reauth()
        #expect(model.isAuthenticated)
        #expect(await service.events() == ["reauth", "list:root"])
    }

    @Test func accountSuccessAndFailureUpdateVisibleState() async throws {
        let service = FakeService()
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.login(clientSecret: "/tmp/client.json", credentials: nil)
        #expect(model.isAuthenticated)

        await service.setAccountError(TestError.failed)
        await model.reauth()
        #expect(model.isAuthenticated)
        #expect(model.errorMessage?.isEmpty == false)

        await service.setAccountError(nil)
        await model.logout()
        #expect(model.isAuthenticated == false)
    }

    @Test(arguments: [false, true])
    func successfulLogoutInvalidatesLateListCompletion(lateRequestFails: Bool) async throws {
        let listGate = AsyncGate()
        let service = FakeService()
        await service.setListResponses(["root": [makeFile(id: "kept", name: "Kept")]])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.openFolder(id: "root", name: "My Drive")
        await service.setListHandler { _ in
            await listGate.wait()
            if lateRequestFails { throw TestError.failed }
            return [makeFile(id: "stale", name: "Stale")]
        }

        let refresh = Task { await model.refresh() }
        await eventually { await service.listCallCount() == 2 }
        await model.logout()

        #expect(model.isAuthenticated == false)
        #expect(model.files.isEmpty)
        #expect(model.isLoading == false)
        #expect(model.errorMessage == nil)

        await listGate.open()
        await refresh.value

        #expect(model.files.isEmpty)
        #expect(model.isLoading == false)
        #expect(model.errorMessage == nil)
    }

    @Test func maintenanceOperationsExposeOutputAndErrors() async throws {
        let service = FakeService()
        await service.setScanListResult(CLIResult(status: 0, stdout: "203.0.113.8\n", stderr: ""))
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.scan()
        await model.listEndpoints()
        await model.mount()
        await model.unmount()

        #expect(model.scanOutput == "203.0.113.8\n")
        #expect(model.isMounted == false)

        await service.setMaintenanceError(TestError.failed)
        await model.mount()
        #expect(model.errorMessage?.isEmpty == false)
    }
}

@MainActor
private func makeDefaults() -> UserDefaults {
    let name = "GDirectTests.\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: name)!
    defaults.removePersistentDomain(forName: name)
    return defaults
}

private func makeFile(
    id: String,
    name: String,
    mimeType: String = "application/octet-stream"
) -> DriveFile {
    DriveFile(
        id: id,
        name: name,
        mimeType: mimeType,
        size: nil,
        modifiedTime: nil,
        md5Checksum: nil,
        resourceKey: nil
    )
}

private func makeTemporaryDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("gdirect-task3-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}
