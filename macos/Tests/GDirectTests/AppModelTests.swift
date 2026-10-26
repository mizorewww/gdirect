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
        await model.switchAccount("other@example.com")
        await model.logout()
        await model.logoutAll()
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
        #expect(await service.events() == ["login", "accounts", "list:root"])

        await service.clearEvents()
        await service.setAccountError(TestError.failed)
        await model.reauth()
        #expect(model.isAuthenticated)
        #expect(await service.events().isEmpty)

        await service.setAccountError(nil)
        await service.clearEvents()
        await model.reauth()
        #expect(model.isAuthenticated)
        #expect(await service.events() == ["reauth", "accounts", "list:root"])
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

    @Test func logoutWithRemainingAccountStaysConnectedAndLoadsItsRoot() async {
        let service = FakeService()
        let remaining = DriveAccount(email: "remaining@example.com", isDefault: true)
        await service.setAccounts([remaining])
        await service.setListResponses(["root": [makeFile(id: "remaining", name: "Remaining")]])
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.logout()

        #expect(model.isAuthenticated)
        #expect(model.accounts == [remaining])
        #expect(model.currentFolderID == "root")
        #expect(model.files.map(\.id) == ["remaining"])
        #expect(await service.events() == ["logout", "accounts", "list:root"])
    }

    @Test func switchAccountRefreshesAccountsAndRoot() async {
        let service = FakeService()
        let accounts = [
            DriveAccount(email: "first@example.com", isDefault: true),
            DriveAccount(email: "other@example.com", isDefault: false)
        ]
        await service.setAccounts(accounts)
        await service.setListResponses(["root": [makeFile(id: "other", name: "Other")]])
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.refreshAccounts()
        await model.switchAccount("other@example.com")

        #expect(model.isAuthenticated)
        #expect(model.accounts.first(where: \.isDefault)?.email == "other@example.com")
        #expect(model.files.map(\.id) == ["other"])
        #expect(await service.events() == [
            "accounts", "switch:other@example.com", "accounts", "list:root"
        ])
    }

    @Test func logoutAllClearsAuthenticationAndAccountsWithoutLoadingRoot() async {
        let service = FakeService()
        await service.setAccounts([DriveAccount(email: "remaining@example.com", isDefault: true)])
        let model = AppModel(service: service, defaults: makeDefaults())

        await model.logoutAll()

        #expect(!model.isAuthenticated)
        #expect(model.accounts.isEmpty)
        #expect(model.files.isEmpty)
        #expect(await service.events() == ["logoutAll"])
    }

    @Test(arguments: AccountMutation.allCases)
    func accountMutationsInvalidateLateListCompletion(mutation: AccountMutation) async {
        let listGate = AsyncGate()
        let service = FakeService()
        await service.setAccounts([
            DriveAccount(email: "first@example.com", isDefault: true),
            DriveAccount(email: "other@example.com", isDefault: false)
        ])
        await service.setListResponses(["old": [makeFile(id: "old", name: "Old")]])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.openFolder(id: "old", name: "Old")
        await service.setListHandler { folder in
            if folder == "old" { await listGate.wait() }
            return [makeFile(id: folder, name: folder)]
        }
        let stale = Task { await model.refresh() }
        await eventually { await service.listCallCount() == 2 }

        switch mutation {
        case .switchAccount: await model.switchAccount("other@example.com")
        case .logout:
            await service.setAccounts([])
            await model.logout()
        case .logoutAll: await model.logoutAll()
        }

        #expect(model.currentFolderID == "root")
        #expect(model.files.map(\.id) == (mutation == .switchAccount ? ["root"] : []))
        #expect(model.isAuthenticated == (mutation == .switchAccount))
        #expect(!model.isLoading)
        #expect(model.errorMessage == nil)
        await listGate.open()
        await stale.value
        #expect(model.files.map(\.id) == (mutation == .switchAccount ? ["root"] : []))
        #expect(model.errorMessage == nil)
    }

    @Test(arguments: AccountMutation.allCases)
    func staleAccountRefreshSuccessCannotOverwriteMutation(mutation: AccountMutation) async {
        await assertStaleAccountRefresh(mutation: mutation, error: nil)
    }

    @Test(arguments: AccountMutation.allCases)
    func staleAccountRefreshFailureCannotOverwriteMutation(mutation: AccountMutation) async {
        await assertStaleAccountRefresh(mutation: mutation, error: TestError.failed)
    }

    private func assertStaleAccountRefresh(mutation: AccountMutation, error: Error?) async {
        let first = DriveAccount(email: "first@example.com", isDefault: true)
        let other = DriveAccount(email: "other@example.com", isDefault: false)
        let remaining = DriveAccount(email: "remaining@example.com", isDefault: true)
        let service = GatedAccountsService()
        await service.setAccounts([first, other])
        await service.setListResponses(["root": [makeFile(id: "root", name: "Root")]])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.refreshAccounts()
        await service.gateNextAccounts(error: error)
        let stale = Task { await model.refreshAccounts() }
        await eventually { await service.accountsCallCount() == 2 }

        let expectedAccounts: [DriveAccount]
        switch mutation {
        case .switchAccount:
            await model.switchAccount(other.email)
            expectedAccounts = [
                DriveAccount(email: first.email, isDefault: false),
                DriveAccount(email: other.email, isDefault: true)
            ]
        case .logout:
            await service.setAccounts([remaining])
            await model.logout()
            expectedAccounts = [remaining]
        case .logoutAll:
            await model.logoutAll()
            expectedAccounts = []
        }
        await service.openAccounts()
        await stale.value

        #expect(model.accounts == expectedAccounts)
        #expect(model.currentFolderID == "root")
        #expect(model.files.map(\.id) == (mutation == .logoutAll ? [] : ["root"]))
        #expect(model.isAuthenticated == (mutation != .logoutAll))
        #expect(!model.isLoading)
        #expect(model.errorMessage == nil)
    }

    @Test(arguments: AccountRefreshMutation.allCases)
    func successfulMutationWithFailedAccountRefreshClearsOldState(
        mutation: AccountRefreshMutation
    ) async {
        let service = GatedAccountsService()
        await service.setAccounts([DriveAccount(email: "old@example.com", isDefault: true)])
        await service.setListResponses([
            "old": [makeFile(id: "old", name: "Old")],
            "root": [makeFile(id: "root", name: "Root")]
        ])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.refreshAccounts()
        await model.openFolder(id: "old", name: "Old")
        await service.failNextAccounts()
        await service.clearEvents()

        switch mutation {
        case .login: await model.login(clientSecret: nil, credentials: nil)
        case .reauth: await model.reauth()
        case .switchAccount: await model.switchAccount("other@example.com")
        case .logout: await model.logout()
        }

        #expect(model.accounts.isEmpty)
        #expect(model.currentFolderID == "root")
        #expect(!model.canGoBack && !model.canGoForward)
        #expect(model.files.isEmpty)
        #expect(!model.isAuthenticated)
        #expect(!model.isLoading)
        #expect(model.errorMessage == TestError.failed.localizedDescription)
        #expect(await service.events().contains("list:root") == false)
    }

    @Test func googleOnlyPersistenceAndEveryModelOperationReachTheService() async {
        let defaults = makeDefaults()
        let service = FakeService()
        await service.setAccounts([DriveAccount(email: "first@example.com", isDefault: true)])
        let model = AppModel(service: service, defaults: defaults)
        model.settings.googleOnly = true
        let restored = AppModel(service: service, defaults: defaults)
        #expect(restored.settings.googleOnly)

        await model.login(clientSecret: nil, credentials: nil)
        await model.reauth()
        await model.refreshAccounts()
        await model.switchAccount("first@example.com")
        await model.logout()
        await model.logoutAll()
        await model.refresh()
        await model.scan()
        await model.listEndpoints()
        await model.mount()
        await model.unmount()
        _ = model.downloadPublicLink(
            link: "https://drive.google.com/file/d/public/view",
            name: "Public",
            destination: URL(fileURLWithPath: "/tmp/public")
        )
        _ = model.upload(source: URL(fileURLWithPath: "/tmp/upload"))
        await model.transfers.waitForIdle()

        let values = await service.googleOnlyValues()
        #expect(values.count == 22)
        #expect(values.allSatisfy { $0 })
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

    @Test func idleModelAllowsTermination() {
        let model = AppModel(service: FakeService(), defaults: makeDefaults())

        #expect(model.applicationShouldTerminate())
        #expect(model.errorMessage == nil)
    }

    @Test func listingBlocksTermination() async {
        let gate = AsyncGate()
        let service = GatedAccountsService()
        await service.setListHandler { _ in
            await gate.wait()
            return []
        }
        let model = AppModel(service: service, defaults: makeDefaults())
        let listing = Task { await model.loadRoot() }
        await eventually { await service.listCallCount() == 1 }

        assertTerminationBlocked(model)

        await gate.open()
        await listing.value
    }

    @Test func accountMutationBlocksTermination() async {
        let service = GatedAccountsService()
        await service.gateNextAccounts(error: nil)
        let model = AppModel(service: service, defaults: makeDefaults())
        let mutation = Task { await model.reauth() }
        await eventually { await service.accountsCallCount() == 1 }

        assertTerminationBlocked(model)

        await service.openAccounts()
        await mutation.value
    }

    @Test(arguments: [false, true])
    func standaloneAccountRefreshBlocksTerminationUntilCompletion(refreshFails: Bool) async {
        let service = GatedAccountsService()
        await service.gateNextAccounts(error: refreshFails ? TestError.failed : nil)
        let model = AppModel(service: service, defaults: makeDefaults())
        let refresh = Task { await model.refreshAccounts() }
        await eventually { await service.accountsCallCount() == 1 }

        assertTerminationBlocked(model)

        await service.openAccounts()
        await refresh.value
        #expect(model.applicationShouldTerminate())
        #expect(model.errorMessage == (refreshFails ? TestError.failed.localizedDescription : nil))
    }

    @Test func maintenanceBlocksTermination() async {
        let service = GatedAccountsService()
        await service.gateMaintenance()
        let model = AppModel(service: service, defaults: makeDefaults())
        let maintenance = Task { await model.scan() }
        await eventually { model.isMaintenanceBusy }

        assertTerminationBlocked(model)

        await service.openMaintenance()
        await maintenance.value
    }

    @Test func queuedTransferBlocksTermination() async {
        let model = AppModel(service: FakeService(), defaults: makeDefaults())
        let id = model.downloadPublicLink(
            link: "public",
            name: "Queued",
            destination: URL(fileURLWithPath: "/tmp/queued")
        )
        #expect(model.transfers.item(id: id)?.state == .queued)

        assertTerminationBlocked(model)

        await model.transfers.waitForIdle()
    }

    @Test func runningTransferBlocksTermination() async {
        let gate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { _, output, _, _ in
            await gate.wait()
            return URL(fileURLWithPath: output)
        }
        let model = AppModel(service: service, defaults: makeDefaults())
        let id = model.downloadPublicLink(
            link: "public",
            name: "Running",
            destination: URL(fileURLWithPath: "/tmp/running")
        )
        await eventually { model.transfers.item(id: id)?.state == .running }

        assertTerminationBlocked(model)

        await gate.open()
        await model.transfers.waitForIdle()
    }

    @Test func cancellingTransferBlocksTermination() async {
        let gate = AsyncGate()
        let service = FakeService()
        await service.setDownloadHandler { _, output, _, _ in
            await gate.wait()
            return URL(fileURLWithPath: output)
        }
        let model = AppModel(service: service, defaults: makeDefaults())
        let id = model.downloadPublicLink(
            link: "public",
            name: "Cancelling",
            destination: URL(fileURLWithPath: "/tmp/cancelling")
        )
        await eventually { model.transfers.item(id: id)?.state == .running }
        model.transfers.cancel(id: id)
        #expect(model.transfers.item(id: id)?.state == .cancelled)

        assertTerminationBlocked(model)

        await gate.open()
        await model.transfers.waitForIdle()
    }

    private func assertTerminationBlocked(_ model: AppModel) {
        #expect(!model.applicationShouldTerminate())
        #expect(model.errorMessage?.contains("before quitting") == true)
    }

    @Test func accountChangesResetSessionMountState() async {
        let service = FakeService()
        await service.setAccounts([DriveAccount(email: "first@example.com", isDefault: true)])
        let model = AppModel(service: service, defaults: makeDefaults())
        await model.mount()
        #expect(model.isMounted)

        await model.switchAccount("first@example.com")
        #expect(!model.isMounted)

        await model.mount()
        #expect(model.isMounted)
        await model.logoutAll()
        #expect(!model.isMounted)
        #expect(await service.events().filter { $0.hasPrefix("mount:") }.count == 2)
    }
}

enum AccountMutation: CaseIterable {
    case switchAccount
    case logout
    case logoutAll
}

enum AccountRefreshMutation: CaseIterable {
    case login
    case reauth
    case switchAccount
    case logout
}

private actor GatedAccountsService: GDirectServing {
    private let base = FakeService()
    private let gate = AsyncGate()
    private let maintenanceGate = AsyncGate()
    private var shouldGateAccounts = false
    private var shouldGateMaintenance = false
    private var nextAccountsError: Error?
    private var refreshCallCount = 0

    func gateNextAccounts(error: Error?) {
        shouldGateAccounts = true
        nextAccountsError = error
    }

    func failNextAccounts() {
        nextAccountsError = TestError.failed
    }

    func openAccounts() async {
        await gate.open()
    }

    func gateMaintenance() {
        shouldGateMaintenance = true
    }

    func openMaintenance() async {
        await maintenanceGate.open()
    }

    func accountsCallCount() -> Int { refreshCallCount }
    func setAccounts(_ accounts: [DriveAccount]) async { await base.setAccounts(accounts) }
    func setListResponses(_ responses: [String: [DriveFile]]) async { await base.setListResponses(responses) }
    func setListHandler(_ handler: @escaping FakeService.ListHandler) async { await base.setListHandler(handler) }
    func listCallCount() async -> Int { await base.listCallCount() }
    func events() async -> [String] { await base.events() }
    func clearEvents() async { await base.clearEvents() }

    func accounts(googleOnly: Bool) async throws -> [DriveAccount] {
        refreshCallCount += 1
        let result = try await base.accounts(googleOnly: googleOnly)
        let shouldWait = shouldGateAccounts
        let error = nextAccountsError
        shouldGateAccounts = false
        nextAccountsError = nil
        if shouldWait { await gate.wait() }
        if let error { throw error }
        return result
    }

    func login(clientSecret: String?, credentials: String?, googleOnly: Bool) async throws -> CLIResult {
        try await base.login(clientSecret: clientSecret, credentials: credentials, googleOnly: googleOnly)
    }

    func reauth(googleOnly: Bool) async throws -> CLIResult {
        try await base.reauth(googleOnly: googleOnly)
    }

    func switchAccount(_ account: String, googleOnly: Bool) async throws -> CLIResult {
        try await base.switchAccount(account, googleOnly: googleOnly)
    }

    func logout(account: String?, googleOnly: Bool) async throws -> CLIResult {
        try await base.logout(account: account, googleOnly: googleOnly)
    }

    func logoutAll(googleOnly: Bool) async throws -> CLIResult {
        try await base.logoutAll(googleOnly: googleOnly)
    }

    func list(folder: String, ip: String?, sni: String?, googleOnly: Bool) async throws -> [DriveFile] {
        try await base.list(folder: folder, ip: ip, sni: sni, googleOnly: googleOnly)
    }

    func download(file: String, output: String, exportMIME: String?, settings: TransferSettings) async throws -> URL {
        try await base.download(file: file, output: output, exportMIME: exportMIME, settings: settings)
    }

    func upload(file: String, parent: String, name: String?, ip: String?, sni: String?, googleOnly: Bool) async throws -> CLIResult {
        try await base.upload(file: file, parent: parent, name: name, ip: ip, sni: sni, googleOnly: googleOnly)
    }

    func scan(ips: String?, googleOnly: Bool) async throws -> CLIResult {
        if shouldGateMaintenance {
            shouldGateMaintenance = false
            await maintenanceGate.wait()
        }
        return try await base.scan(ips: ips, googleOnly: googleOnly)
    }

    func scanList(googleOnly: Bool) async throws -> CLIResult {
        try await base.scanList(googleOnly: googleOnly)
    }

    func mount(path: String, readOnly: Bool, connections: Int, googleOnly: Bool) async throws -> CLIResult {
        try await base.mount(path: path, readOnly: readOnly, connections: connections, googleOnly: googleOnly)
    }

    func unmount(googleOnly: Bool) async throws -> CLIResult {
        try await base.unmount(googleOnly: googleOnly)
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
