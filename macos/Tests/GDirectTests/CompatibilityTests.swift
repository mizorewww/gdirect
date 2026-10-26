import Foundation
import Testing
@testable import GDirect

struct CompatibilityTests {
    @Test func signInSelectsOnlyOneSourceAndAllowsSavedClient() {
        let desktop = SignInMethod.desktop.sources(clientSecret: "client.json", credentials: "credentials.json")
        #expect(desktop.clientSecret == "client.json")
        #expect(desktop.credentials == nil)
        let imported = SignInMethod.credentials.sources(clientSecret: "client.json", credentials: "credentials.json")
        #expect(imported.clientSecret == nil)
        #expect(imported.credentials == "credentials.json")
        let saved = SignInMethod.desktop.sources(clientSecret: "  ", credentials: "credentials.json")
        #expect(saved.clientSecret == nil && saved.credentials == nil)
    }

    @Test func conflictingNetworkSettingsNeverEmitBothFlags() {
        var settings = AppSettings.standard
        settings.sni = "example.com"
        settings.noSNI = true
        settings.normalizeNetworkSettings()
        #expect(settings.sni.isEmpty)
        #expect(settings.transferSettings.sni == nil)
        let args = CLICommand.download(file: "id", output: "/tmp/file", settings: TransferSettings(sni: "example.com", noSNI: true)).arguments
        #expect(args.contains("--no-sni"))
        #expect(!args.contains(where: { $0.hasPrefix("--sni=") }))
    }

    @Test func downloadClassificationMatchesCLI() {
        for kind in ["document", "spreadsheet", "presentation", "drawing"] {
            let file = makeFile(id: kind, mimeType: "application/vnd.google-apps.\(kind)")
            #expect(file.isDownloadable)
            #expect(file.isExportable)
        }
        for kind in ["folder", "shortcut", "form", "site", "unknown"] {
            let file = makeFile(id: kind, mimeType: "application/vnd.google-apps.\(kind)")
            #expect(!file.isDownloadable)
            #expect(!file.isExportable)
        }
        #expect(makeFile(id: "binary", mimeType: "application/zip").isDownloadable)
        #expect(!makeFile(id: "binary", mimeType: "application/zip").isExportable)
    }

    @Test @MainActor func unsupportedDownloadDoesNotEnqueue() {
        let model = AppModel(service: FakeService(), defaults: makeDefaults())
        for kind in ["shortcut", "form", "folder"] {
            let file = makeFile(id: kind, mimeType: "application/vnd.google-apps.\(kind)")
            #expect(model.download(file) == nil)
            #expect(model.exportMIME(for: file) == nil)
        }
        #expect(model.transfers.items.isEmpty)
    }
}

private func makeDefaults() -> UserDefaults {
    UserDefaults(suiteName: "GDirectCompatibility.\(UUID().uuidString)")!
}

private func makeFile(id: String, mimeType: String) -> DriveFile {
    DriveFile(id: id, name: id, mimeType: mimeType, size: nil, modifiedTime: nil, md5Checksum: nil, resourceKey: nil)
}

@MainActor
struct SavedCompatibilityTests {
    @Test func settingsWithoutGoogleOnlyDecodeWithGoogleAccessEnabled() throws {
        let legacy = Data(#"{"downloadDirectoryPath":"/tmp/Downloads","connections":8,"customIP":"","sni":"","noSNI":false,"caCertPath":"","mountPath":"/tmp/GDirect","readOnly":false}"#.utf8)

        let settings = try JSONDecoder().decode(AppSettings.self, from: legacy)

        #expect(settings.googleOnly == false)
        #expect(settings.downloadDirectoryPath == "/tmp/Downloads")
    }

    @Test func legacyContradictorySettingsNormalizeOnLoad() throws {
        let defaults = makeDefaults()
        var saved = AppSettings.standard
        saved.sni = "legacy.example"
        saved.noSNI = true
        defaults.set(try JSONEncoder().encode(saved), forKey: "nonsecret-app-settings-v1")
        let model = AppModel(service: FakeService(), defaults: defaults)
        #expect(model.settings.noSNI)
        #expect(model.settings.sni.isEmpty)
        #expect(model.settings.transferSettings.sni == nil)
    }
}
