import Combine
import Foundation
import UniformTypeIdentifiers

struct AppSettings: Codable, Sendable {
    var downloadDirectoryPath: String
    var connections: Int
    var customIP: String
    var sni: String
    var noSNI: Bool
    var caCertPath: String
    var mountPath: String
    var readOnly: Bool

    static var standard: AppSettings {
        let downloads = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("Downloads")
        return AppSettings(
            downloadDirectoryPath: downloads.path,
            connections: 8,
            customIP: "",
            sni: "",
            noSNI: false,
            caCertPath: "",
            mountPath: URL(fileURLWithPath: NSHomeDirectory()).appendingPathComponent("GDirect").path,
            readOnly: false
        )
    }

    mutating func normalizeNetworkSettings() {
        if noSNI { sni = "" }
    }

    var transferSettings: TransferSettings {
        TransferSettings(
            ip: customIP.nonEmpty,
            sni: noSNI ? nil : sni.nonEmpty,
            noSNI: noSNI,
            caCert: caCertPath.nonEmpty,
            connections: min(16, max(1, connections))
        )
    }
}

private struct FolderLocation {
    let id: String
    let name: String
}

@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var files: [DriveFile] = []
    @Published private(set) var currentFolderID = "root"
    @Published private(set) var currentFolderName = "My Drive"
    @Published private(set) var isLoading = false
    @Published private(set) var isAccountBusy = false
    @Published private(set) var isMaintenanceBusy = false
    @Published private(set) var isAuthenticated = false
    @Published private(set) var isMounted = false
    @Published var errorMessage: String?
    @Published var scanOutput = ""
    @Published var scanTargets = ""
    @Published var settings: AppSettings {
        didSet { saveSettings() }
    }

    private(set) var transfers: TransferQueue!

    private let service: GDirectServing
    private let defaults: UserDefaults
    private let settingsKey = "nonsecret-app-settings-v1"
    private var history = [FolderLocation(id: "root", name: "My Drive")]
    private var historyIndex = 0
    private var listGeneration = 0
    private var activeListTask: Task<[DriveFile], Error>?
    private var reservedDestinations: Set<String> = []

    init(service: GDirectServing, defaults: UserDefaults = .standard) {
        self.service = service
        self.defaults = defaults
        if let data = defaults.data(forKey: settingsKey),
           let saved = try? JSONDecoder().decode(AppSettings.self, from: data) {
            var normalized = saved
            normalized.normalizeNetworkSettings()
            settings = normalized
        } else {
            settings = .standard
        }
        transfers = TransferQueue(
            service: service,
            settings: { [weak self] in self?.settings.transferSettings ?? TransferSettings() },
            uploadSucceeded: { [weak self] parent in
                await self?.uploadSucceeded(in: parent)
            }
        )
    }

    var canGoBack: Bool { historyIndex > 0 }
    var canGoForward: Bool { historyIndex + 1 < history.count }
    var downloadDirectory: URL { URL(fileURLWithPath: settings.downloadDirectoryPath, isDirectory: true) }

    func loadRoot() async {
        await openFolder(id: "root", name: "My Drive")
    }

    func openFolder(id: String, name: String) async {
        guard !isAccountBusy else { return }
        if id == currentFolderID {
            await refresh()
            return
        }
        history.removeSubrange((historyIndex + 1)...)
        history.append(FolderLocation(id: id, name: name))
        historyIndex = history.count - 1
        await navigate(to: FolderLocation(id: id, name: name), clearFiles: true)
    }

    func goBack() async {
        guard !isAccountBusy else { return }
        guard canGoBack else { return }
        historyIndex -= 1
        await navigate(to: history[historyIndex], clearFiles: true)
    }

    func goForward() async {
        guard !isAccountBusy else { return }
        guard canGoForward else { return }
        historyIndex += 1
        await navigate(to: history[historyIndex], clearFiles: true)
    }

    func refresh() async {
        await navigate(
            to: FolderLocation(id: currentFolderID, name: currentFolderName),
            clearFiles: false
        )
    }

    func exportMIME(for file: DriveFile) -> String? {
        guard file.isExportable else { return nil }
        return "application/pdf"
    }

    func destination(for file: DriveFile, in directory: URL, exportMIME: String? = nil) throws -> URL {
        let export = exportMIME ?? self.exportMIME(for: file)
        var name = safeFilename(file.name)
        if let export, let pathExtension = UTType(mimeType: export)?.preferredFilenameExtension {
            let currentExtension = (name as NSString).pathExtension
            if currentExtension.caseInsensitiveCompare(pathExtension) != .orderedSame {
                name = (name as NSString).deletingPathExtension
                name += ".\(pathExtension)"
            }
        }
        var candidate = directory.appendingPathComponent(name, isDirectory: false)
        var suffix = 2
        while FileManager.default.fileExists(atPath: candidate.path) || reservedDestinations.contains(candidate.path) {
            let base = (name as NSString).deletingPathExtension
            let ext = (name as NSString).pathExtension
            let nextName = ext.isEmpty ? "\(base) \(suffix)" : "\(base) \(suffix).\(ext)"
            candidate = directory.appendingPathComponent(nextName, isDirectory: false)
            suffix += 1
        }
        reservedDestinations.insert(candidate.path)
        return candidate
    }

    @discardableResult
    func download(_ file: DriveFile, exportMIME: String? = nil) -> UUID? {
        guard file.isDownloadable else {
            errorMessage = "The gdirect CLI cannot download this item (\(file.mimeType))."
            return nil
        }
        do {
            try FileManager.default.createDirectory(at: downloadDirectory, withIntermediateDirectories: true)
            let destination = try destination(for: file, in: downloadDirectory, exportMIME: exportMIME)
            return transfers.enqueueDownload(file: file, destination: destination, exportMIME: exportMIME ?? self.exportMIME(for: file))
        } catch {
            errorMessage = error.localizedDescription
            return nil
        }
    }

    @discardableResult
    func downloadPublicLink(link: String, name: String, destination: URL) -> UUID {
        transfers.enqueuePublicDownload(link: link, name: name, destination: destination)
    }

    @discardableResult
    func upload(source: URL) -> UUID {
        transfers.enqueueUpload(
            source: source,
            parent: currentFolderID,
            name: source.lastPathComponent
        )
    }

    func login(clientSecret: String?, credentials: String?) async {
        await accountOperation {
            _ = try await service.login(clientSecret: clientSecret, credentials: credentials)
            isAuthenticated = true
            await resetAccountBrowser(load: true)
        }
    }

    func reauth() async {
        await accountOperation {
            _ = try await service.reauth()
            isAuthenticated = true
            await resetAccountBrowser(load: true)
        }
    }

    func logout() async {
        await accountOperation {
            _ = try await service.logout()
            isAuthenticated = false
            await resetAccountBrowser(load: false)
        }
    }

    func scan() async {
        await maintenanceOperation {
            _ = try await service.scan(ips: scanTargets.nonEmpty)
        }
    }

    func listEndpoints() async {
        await maintenanceOperation {
            let result = try await service.scanList()
            scanOutput = result.stdout
        }
    }

    func mount() async {
        await maintenanceOperation {
            _ = try await service.mount(
                path: settings.mountPath,
                readOnly: settings.readOnly,
                connections: min(16, max(1, settings.connections))
            )
            isMounted = true
        }
    }

    func unmount() async {
        await maintenanceOperation {
            _ = try await service.unmount()
            isMounted = false
        }
    }

    private func resetAccountBrowser(load: Bool) async {
        invalidateListing()
        let root = FolderLocation(id: "root", name: "My Drive")
        history = [root]
        historyIndex = 0
        currentFolderID = root.id
        currentFolderName = root.name
        files = []
        errorMessage = nil
        if load { await navigate(to: root, clearFiles: true, duringAccountChange: true) }
    }

    private func invalidateListing() {
        listGeneration += 1
        activeListTask?.cancel()
        activeListTask = nil
        isLoading = false
    }

    private func navigate(to location: FolderLocation, clearFiles: Bool, duringAccountChange: Bool = false) async {
        guard !isAccountBusy || duringAccountChange else { return }
        listGeneration += 1
        let generation = listGeneration
        activeListTask?.cancel()
        currentFolderID = location.id
        currentFolderName = location.name
        if clearFiles { files = [] }
        errorMessage = nil
        isLoading = true
        let request = Task { [service] in
            try await service.list(folder: location.id, ip: settings.transferSettings.ip)
        }
        activeListTask = request
        do {
            let result = try await request.value
            guard generation == listGeneration else { return }
            isAuthenticated = true
            files = result.sorted { lhs, rhs in
                let lhsFolder = lhs.mimeType == "application/vnd.google-apps.folder"
                let rhsFolder = rhs.mimeType == "application/vnd.google-apps.folder"
                if lhsFolder != rhsFolder { return lhsFolder }
                return lhs.name.localizedStandardCompare(rhs.name) == .orderedAscending
            }
        } catch {
            guard generation == listGeneration else { return }
            errorMessage = error.localizedDescription
        }
        if generation == listGeneration {
            isLoading = false
            activeListTask = nil
        }
    }

    private func accountOperation(_ operation: () async throws -> Void) async {
        guard !isAccountBusy else { return }
        guard transfers.beginCredentialChange() else {
            errorMessage = "Wait for all queued and running transfers to finish before changing accounts."
            return
        }
        isAccountBusy = true
        invalidateListing()
        errorMessage = nil
        defer {
            transfers.endCredentialChange()
            isAccountBusy = false
        }
        do {
            try await operation()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func maintenanceOperation(_ operation: () async throws -> Void) async {
        guard !isMaintenanceBusy else { return }
        isMaintenanceBusy = true
        errorMessage = nil
        defer { isMaintenanceBusy = false }
        do {
            try await operation()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func uploadSucceeded(in folder: String) async {
        guard folder == currentFolderID else { return }
        await refresh()
    }

    private func safeFilename(_ input: String) -> String {
        var output = ""
        for scalar in input.precomposedStringWithCanonicalMapping.unicodeScalars {
            if scalar == "/" {
                output.append("-")
            } else if CharacterSet.controlCharacters.contains(scalar) {
                output.append(" ")
            } else {
                output.unicodeScalars.append(scalar)
            }
        }
        let trimmed = output.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty || trimmed == "." || trimmed == ".." ? "Untitled" : trimmed
    }

    private func saveSettings() {
        guard let data = try? JSONEncoder().encode(settings) else { return }
        defaults.set(data, forKey: settingsKey)
    }
}

private extension String {
    var nonEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
