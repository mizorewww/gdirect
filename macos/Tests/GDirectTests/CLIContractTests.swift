import Foundation
import Testing
@testable import GDirect

struct DriveFileTests {
    @Test func decodesEveryCurrentField() throws {
        let data = Data(#"{"id":"abc123","name":"report.pdf","mimeType":"application/pdf","size":"42","modifiedTime":"2026-10-25T12:34:56.000Z","md5Checksum":"0123456789abcdef","resourceKey":"resource-value"}"#.utf8)

        let file = try JSONDecoder().decode(DriveFile.self, from: data)

        #expect(file.id == "abc123")
        #expect(file.name == "report.pdf")
        #expect(file.mimeType == "application/pdf")
        #expect(file.size == "42")
        #expect(file.modifiedTime == "2026-10-25T12:34:56.000Z")
        #expect(file.md5Checksum == "0123456789abcdef")
        #expect(file.resourceKey == "resource-value")
    }

    @Test func missingOptionalFieldsDecodeAsNil() throws {
        let data = Data(#"{"id":"abc123","name":"folder","mimeType":"application/vnd.google-apps.folder"}"#.utf8)

        let file = try JSONDecoder().decode(DriveFile.self, from: data)

        #expect(file.size == nil)
        #expect(file.modifiedTime == nil)
        #expect(file.md5Checksum == nil)
        #expect(file.resourceKey == nil)
    }

    @Test func unknownFieldsAreIgnored() throws {
        let data = Data(#"{"id":"abc123","name":"future","mimeType":"application/pdf","futureField":{"nested":true}}"#.utf8)

        let file = try JSONDecoder().decode(DriveFile.self, from: data)

        #expect(file.name == "future")
    }

    @Test func unicodeNameIsPreserved() throws {
        let data = Data(#"{"id":"abc123","name":"资料 📄 café","mimeType":"application/pdf"}"#.utf8)

        let file = try JSONDecoder().decode(DriveFile.self, from: data)

        #expect(file.name == "资料 📄 café")
    }

    @Test func malformedJSONIsRejected() {
        let data = Data(#"{"id":"abc123","name":}"#.utf8)

        #expect(throws: (any Error).self) {
            try JSONDecoder().decode(DriveFile.self, from: data)
        }
    }
}

struct CLICommandTests {
    @Test func accountCommandsMatchClap() {
        #expect(CLICommand.login(clientSecret: "/tmp/client secret.json").arguments ==
                ["login", "--client-secret=/tmp/client secret.json"])
        #expect(CLICommand.login(credentials: "-credentials.json").arguments ==
                ["login", "--credentials=-credentials.json"])
        #expect(CLICommand.reauth.arguments == ["login", "--reauth"])
        #expect(CLICommand.logout.arguments == ["logout"])
    }

    @Test func listUsesJSONAndDelimitsOptionLikeFolder() {
        #expect(CLICommand.list(folder: "-folder id", ip: "203.0.113.8").arguments ==
                ["ls", "--json", "--ip", "203.0.113.8", "--", "-folder id"])
    }

    @Test func downloadKeepsEveryValueInItsOwnSafeArgument() {
        let settings = TransferSettings(
            ip: "203.0.113.8",
            sni: "-custom.sni",
            caCert: "-CA bundle.pem",
            connections: 16
        )
        let command = CLICommand.download(
            file: "-file id",
            output: "/tmp/output with spaces.pdf",
            exportMIME: "application/pdf",
            settings: settings
        )

        #expect(command.arguments == [
            "download",
            "--output=/tmp/output with spaces.pdf",
            "--export=application/pdf",
            "--ip", "203.0.113.8",
            "--sni=-custom.sni",
            "--ca-cert=-CA bundle.pem",
            "--connections", "16",
            "--", "-file id"
        ])
    }

    @Test func downloadCanDisableSNI() {
        let command = CLICommand.download(
            file: "file-id",
            output: "output.pdf",
            settings: TransferSettings(noSNI: true, connections: 1)
        )

        #expect(command.arguments == [
            "download", "--output=output.pdf", "--no-sni", "--connections", "1", "--", "file-id"
        ])
    }

    @Test func everySupportedConnectionCountIsPassedWithoutClamping() {
        for connections in 1...16 {
            let command = CLICommand.download(
                file: "file-id",
                output: "output.pdf",
                settings: TransferSettings(connections: connections)
            )

            #expect(command.arguments == [
                "download", "--output=output.pdf", "--connections", "\(connections)", "--", "file-id"
            ])
        }
    }

    @Test func uploadDelimitsPathAndProtectsOptionValues() {
        let command = CLICommand.upload(
            file: "-local file.txt",
            parent: "parent-id",
            name: "-Drive name.txt",
            ip: "203.0.113.9"
        )

        #expect(command.arguments == [
            "upload", "--parent", "parent-id", "--name=-Drive name.txt",
            "--ip", "203.0.113.9", "--", "-local file.txt"
        ])
    }

    @Test func scanCommandsMatchClap() {
        #expect(CLICommand.scan(ips: "-IP list.txt").arguments ==
                ["scan", "--ips=-IP list.txt"])
        #expect(CLICommand.scanList.arguments == ["scan", "--list"])
    }

    @Test func mountAndUnmountMatchClap() {
        #expect(CLICommand.mount(path: "-Mount Point", readOnly: true, connections: 16).arguments ==
                ["mount", "--read-only", "--connections", "16", "--", "-Mount Point"])
        #expect(CLICommand.unmount.arguments == ["unmount"])
    }
}

struct CLILocatorTests {
    @Test func bundledExecutableWinsOverPATH() throws {
        let root = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try makeAppBundle(at: root, includeCLI: true)
        let pathDirectory = root.appendingPathComponent("user-path")
        try FileManager.default.createDirectory(at: pathDirectory, withIntermediateDirectories: true)
        try Data("wrong-cli".utf8).write(to: pathDirectory.appendingPathComponent("gdirect"))
        let bundle = try #require(Bundle(path: app.path))
        let locator = CLILocator(bundle: bundle, environment: ["PATH": pathDirectory.path])

        let executable = try locator.executableURL()

        #expect(executable.standardizedFileURL ==
                app.appendingPathComponent("Contents/MacOS/gdirect").standardizedFileURL)
    }

    @Test func missingBundledExecutableThrowsInsteadOfUsingPATH() throws {
        let root = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try makeAppBundle(at: root, includeCLI: false)
        let pathDirectory = root.appendingPathComponent("user-path")
        try FileManager.default.createDirectory(at: pathDirectory, withIntermediateDirectories: true)
        let pathCLI = pathDirectory.appendingPathComponent("gdirect")
        try Data("#!/bin/sh\n".utf8).write(to: pathCLI)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: pathCLI.path)
        let bundle = try #require(Bundle(path: app.path))
        let locator = CLILocator(bundle: bundle, environment: ["PATH": pathDirectory.path])
        let bundledCLI = app.appendingPathComponent("Contents/MacOS/gdirect")

        #expect(FileManager.default.fileExists(atPath: bundledCLI.path) == false)
        do {
            _ = try locator.executableURL()
            Issue.record("Expected a missing bundled executable to throw")
        } catch {}
    }

    @Test func processEnvironmentPrependsHomebrewAndPreservesExistingPATH() {
        let locator = CLILocator(environment: [
            "PATH": "/custom/bin:/opt/homebrew/bin",
            "HOME": "/Users/example"
        ])

        let environment = locator.processEnvironment()

        #expect(environment["PATH"] == "/opt/homebrew/bin:/usr/local/bin:/custom/bin")
        #expect(environment["HOME"] == "/Users/example")
    }

    @Test func processEnvironmentSuppliesPATHWhenInheritedPATHIsMissing() {
        let locator = CLILocator(environment: ["HOME": "/Users/example"])

        #expect(locator.processEnvironment()["PATH"] == "/opt/homebrew/bin:/usr/local/bin")
    }

    private func temporaryDirectory() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    }

    private func makeAppBundle(at root: URL, includeCLI: Bool) throws -> URL {
        let app = root.appendingPathComponent("GDirect.app", isDirectory: true)
        let executableDirectory = app.appendingPathComponent("Contents/MacOS", isDirectory: true)
        try FileManager.default.createDirectory(at: executableDirectory, withIntermediateDirectories: true)
        let plist = """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0"><dict>
        <key>CFBundleExecutable</key><string>GDirect</string>
        <key>CFBundleIdentifier</key><string>dev.gdirect.tests</string>
        <key>CFBundlePackageType</key><string>APPL</string>
        </dict></plist>
        """
        try Data(plist.utf8).write(to: app.appendingPathComponent("Contents/Info.plist"))
        if includeCLI {
            let cli = executableDirectory.appendingPathComponent("gdirect")
            try Data("#!/bin/sh\n".utf8).write(to: cli)
            try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: cli.path)
        }
        return app
    }
}
