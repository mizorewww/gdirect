import Darwin
import Foundation
import Testing
@testable import GDirect

@Suite(.serialized)
struct CLIRunnerTests {
    @Test func outputIsBoundedWhilePipesContinueToDrain() async throws {
        let fixture = try makeFixture(mode: "large", outputLimit: 4096)
        defer { fixture.remove() }
        let result = try await withTimeout { try await fixture.runner.run(.logout) }
        #expect(result.status == 0)
        #expect(result.stdout.utf8.count == 4096)
        #expect(result.stderr.utf8.count == 4096)
    }

    @Test func oversizedListHasExplicitErrorRatherThanTruncatedJSON() async throws {
        let fixture = try makeFixture(mode: "list", listOutputLimit: 32)
        defer { fixture.remove() }
        do {
            _ = try await fixture.service.list()
            Issue.record("Expected explicit oversized list failure")
        } catch CLIError.listOutputTooLarge {
        }
    }

    @Test func listUsesItsOwnLimitToPreserveCompleteJSON() async throws {
        let fixture = try makeFixture(mode: "list", outputLimit: 8, listOutputLimit: 1024)
        defer { fixture.remove() }
        #expect(try await fixture.service.list().count == 1)
    }

    @Test func separatesStdoutAndStderr() async throws {
        let fixture = try makeFixture(mode: "success")
        defer { fixture.remove() }

        let result = try await withTimeout {
            try await fixture.runner.run(.logout)
        }

        #expect(result.status == 0)
        #expect(result.stdout == "stdout text\n")
        #expect(result.stderr == "stderr text\n")
    }

    @Test func drainsBothPipesWhenEitherExceedsPipeCapacity() async throws {
        let fixture = try makeFixture(mode: "large")
        defer { fixture.remove() }

        let result = try await withTimeout {
            try await fixture.runner.run(.logout)
        }

        #expect(result.status == 0)
        #expect(result.stdout.count == 262_144)
        #expect(result.stderr.count == 262_144)
        #expect(result.stdout.allSatisfy { $0 == "o" })
        #expect(result.stderr.allSatisfy { $0 == "e" })
    }

    @Test func preservesNonzeroExitAndBothOutputStreams() async throws {
        let fixture = try makeFixture(mode: "exit")
        defer { fixture.remove() }

        let result = try await withTimeout {
            try await fixture.runner.run(.logout)
        }

        #expect(result.status == 23)
        #expect(result.stdout == "partial output\n")
        #expect(result.stderr == "failure detail\n")
    }

    @Test func emitsOutputBeforeTheProcessExits() async throws {
        let marker = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString)
        let fixture = try makeFixture(mode: "stream", extraEnvironment: ["FAKE_MARKER": marker.path])
        defer { fixture.remove() }

        let result = try await withTimeout {
            try await fixture.runner.run(.logout) { stream, data in
                if stream == .stdout, String(decoding: data, as: UTF8.self).contains("ready") {
                    try? Data().write(to: marker)
                }
            }
        }

        #expect(result.stdout == "ready\nfinished\n")
        #expect(result.stderr == "stream error\n")
    }

    @Test func cancellationTerminatesAndReapsTheProcess() async throws {
        for _ in 0..<30 {
            let fixture = try makeFixture(mode: "cancel")
            let task = Task { try await fixture.runner.run(.logout) }
            let pid = try await waitForPID(fixture)

            task.cancel()
            task.cancel()

            do {
                _ = try await withTimeout { try await task.value }
                Issue.record("Expected cancellation to throw")
            } catch CLIError.cancelled(_) {
            }

            #expect(FileManager.default.fileExists(atPath: fixture.termFile.path))
            #expect(kill(pid, 0) == -1)
            #expect(errno == ESRCH)
            fixture.remove()
        }
    }

    @Test func cancellationEscalatesToSIGKILLAndReapsAnUncooperativeProcess() async throws {
        let fixture = try makeFixture(mode: "ignore-term", terminationGrace: .milliseconds(100))
        defer { fixture.remove() }
        let task = Task { try await fixture.runner.run(.logout) }
        let pid = try await waitForPID(fixture)
        let start = ContinuousClock.now

        task.cancel()

        do {
            _ = try await withTimeout { try await task.value }
            Issue.record("Expected cancellation to throw")
        } catch CLIError.cancelled(_) {
        }

        #expect(start.duration(to: .now) < .seconds(3))
        #expect(kill(pid, 0) == -1)
        #expect(errno == ESRCH)
    }

    @Test func repeatedCancellationAndExitRacesCompleteExactlyOnce() async throws {
        for _ in 0..<30 {
            let fixture = try makeFixture(mode: "race", terminationGrace: .milliseconds(20))
            let task = Task { try await fixture.runner.run(.logout) }
            try await waitForFile(at: fixture.pidFile)
            task.cancel()
            task.cancel()

            do {
                let result = try await withTimeout { try await task.value }
                #expect(result.status == 0 || result.status == 42)
            } catch CLIError.cancelled(_) {
            }
            fixture.remove()
        }
    }

    private func waitForPID(_ fixture: Fixture) async throws -> pid_t {
        try await withTimeout {
            while true {
                if let contents = try? String(contentsOf: fixture.pidFile, encoding: .utf8),
                   let pid = pid_t(contents.trimmingCharacters(in: .whitespacesAndNewlines)) {
                    return pid
                }
                try Task.checkCancellation()
                try await Task.sleep(for: .milliseconds(10))
            }
        }
    }

    private func waitForFile(at url: URL) async throws {
        try await withTimeout {
            while !FileManager.default.fileExists(atPath: url.path) {
                try Task.checkCancellation()
                try await Task.sleep(for: .milliseconds(10))
            }
        }
    }
}

@Suite(.serialized)
struct GDirectServiceTests {
    @Test func listDecodesJSONAndReturnsDriveFiles() async throws {
        let fixture = try makeFixture(mode: "list")
        defer { fixture.remove() }

        let files = try await fixture.service.list(folder: "folder-id")

        #expect(files.count == 1)
        #expect(files[0].id == "abc123")
        #expect(files[0].name == "资料.pdf")
        #expect(files[0].mimeType == "application/pdf")
        #expect(files[0].size == "42")
    }

    @Test func malformedListJSONFailsInsteadOfReturningAnEmptyList() async throws {
        let fixture = try makeFixture(mode: "invalid-list")
        defer { fixture.remove() }

        do {
            _ = try await fixture.service.list()
            Issue.record("Expected malformed JSON to throw")
        } catch CLIError.invalidListJSON {
        }
    }

    @Test func failedListDoesNotDecodeItsStdout() async throws {
        let fixture = try makeFixture(mode: "list-fail")
        defer { fixture.remove() }

        do {
            _ = try await fixture.service.list()
            Issue.record("Expected a nonzero listing exit to throw")
        } catch let CLIError.nonZeroExit(result) {
            #expect(result.status == 9)
        }
    }

    @Test func nonListOperationsUseExitStatusRatherThanJSON() async throws {
        let fixture = try makeFixture(mode: "list")
        defer { fixture.remove() }
        let service = fixture.service

        _ = try await service.login(clientSecret: "client secret.json")
        _ = try await service.reauth()
        _ = try await service.logout()
        _ = try await service.upload(file: "/tmp/local file.txt", parent: "parent", name: "drive.txt")
        _ = try await service.scan(ips: "ips.txt")
        let scanList = try await service.scanList()
        _ = try await service.mount(path: "/tmp/mount point", readOnly: true, connections: 4)
        _ = try await service.unmount()

        #expect(scanList.status == 0)
        #expect(scanList.stdout == "not JSON and not a listing\n")
    }

    @Test func downloadReturnsThePredeterminedDestination() async throws {
        let fixture = try makeFixture(mode: "list")
        defer { fixture.remove() }
        let output = "/tmp/known destination.pdf"

        let destination = try await fixture.service.download(
            file: "file-id",
            output: output,
            exportMIME: "application/pdf",
            settings: TransferSettings(connections: 3)
        )

        #expect(destination.path == output)
    }

    @Test func nonzeroOperationsExposeRedactedFailureDiagnostics() async throws {
        let fixture = try makeFixture(mode: "exit")
        defer { fixture.remove() }

        do {
            _ = try await fixture.service.logout()
            Issue.record("Expected a nonzero exit to throw")
        } catch let CLIError.nonZeroExit(result) {
            #expect(result.status == 23)
            #expect(result.stderr == "failure detail\n")
        }
    }

    @Test func serviceErrorsCapCapturedOutput() async throws {
        let fixture = try makeFixture(mode: "large-exit", logCapacity: 512)
        defer { fixture.remove() }

        do {
            _ = try await fixture.service.logout()
            Issue.record("Expected a nonzero exit to throw")
        } catch let CLIError.nonZeroExit(result) {
            #expect(result.stderr.utf8.count == 512)
            #expect(result.stderr.allSatisfy { $0 == "e" })
        }
    }

    @Test func diagnosticsRedactBeforeTrimmingLongQueries() async throws {
        let fixture = try makeFixture(mode: "query-only", logCapacity: 64)
        defer { fixture.remove() }

        _ = try await fixture.service.logout()
        let diagnostics = fixture.service.diagnostics

        #expect(diagnostics.utf8.count <= 64)
        #expect(diagnostics.contains("?<redacted>"))
        #expect(diagnostics.contains("ssss") == false)
    }

    @Test func diagnosticsAreMemoryCappedAndRedactAllURLQueries() async throws {
        let fixture = try makeFixture(mode: "logs", logCapacity: 512)
        defer { fixture.remove() }

        _ = try await fixture.service.logout()
        let diagnostics = fixture.service.diagnostics

        #expect(diagnostics.utf8.count <= 512)
        #expect(diagnostics.contains("stderr-end"))
        #expect(diagnostics.contains("secret-client") == false)
        #expect(diagnostics.contains("secret-state") == false)
        #expect(diagnostics.contains("secret-session") == false)
        #expect(diagnostics.contains("?<redacted>"))
    }
}

struct LogRedactorTests {
    @Test func redactsQueriesWithoutChangingOrdinaryText() {
        let input = "Open https://accounts.google.com/o/oauth2/auth?client_id=abc&state=xyz then continue"

        #expect(LogRedactor.redact(input) ==
                "Open https://accounts.google.com/o/oauth2/auth?<redacted> then continue")
        #expect(LogRedactor.redact("https://example.com/path and plain text") ==
                "https://example.com/path and plain text")
    }

    @Test func redactsMultipleURLsAndPreservesFragments() {
        let input = "https://one.test/path?a=1#fragment https://two.test/path?b=2"

        #expect(LogRedactor.redact(input) ==
                "https://one.test/path?<redacted>#fragment https://two.test/path?<redacted>")
    }
}

private struct Fixture {
    let root: URL
    let runner: CLIRunner
    let service: GDirectService
    let pidFile: URL
    let termFile: URL

    func remove() {
        try? FileManager.default.removeItem(at: root)
    }
}

private func makeFixture(
    mode: String,
    logCapacity: Int = 16_384,
    terminationGrace: Duration = .milliseconds(200),
    outputLimit: Int = 1_048_576,
    listOutputLimit: Int = 16_777_216,
    extraEnvironment: [String: String] = [:]
) throws -> Fixture {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    let executableDirectory = root.appendingPathComponent("GDirect.app/Contents/MacOS", isDirectory: true)
    try FileManager.default.createDirectory(at: executableDirectory, withIntermediateDirectories: true)
    let plist = """
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0"><dict>
    <key>CFBundleExecutable</key><string>GDirect</string>
    <key>CFBundleIdentifier</key><string>dev.gdirect.runner-tests</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    </dict></plist>
    """
    try Data(plist.utf8).write(to: root.appendingPathComponent("GDirect.app/Contents/Info.plist"))
    let fixtureURL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .appendingPathComponent("Fixtures/fake-gdirect.sh")
    let executable = executableDirectory.appendingPathComponent("gdirect")
    try FileManager.default.copyItem(at: fixtureURL, to: executable)
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: executable.path)
    let pidFile = root.appendingPathComponent("pid")
    let termFile = root.appendingPathComponent("terminated")
    var environment = ProcessInfo.processInfo.environment
    environment["FAKE_GDIRECT_MODE"] = mode
    environment["FAKE_PID_FILE"] = pidFile.path
    environment["FAKE_TERM_FILE"] = termFile.path
    environment.merge(extraEnvironment) { _, new in new }
    let bundle = try #require(Bundle(path: root.appendingPathComponent("GDirect.app").path))
    let locator = CLILocator(bundle: bundle, environment: environment)
    let runner = CLIRunner(locator: locator, terminationGrace: terminationGrace, outputLimit: outputLimit, listOutputLimit: listOutputLimit)
    return Fixture(
        root: root,
        runner: runner,
        service: GDirectService(runner: runner, logCapacity: logCapacity),
        pidFile: pidFile,
        termFile: termFile
    )
}

private struct TestTimeout: Error {}

private func withTimeout<T: Sendable>(
    seconds: UInt64 = 5,
    operation: @escaping @Sendable () async throws -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask {
            try await operation()
        }
        group.addTask {
            try await Task.sleep(nanoseconds: seconds * 1_000_000_000)
            throw TestTimeout()
        }
        let result = try await group.next()!
        group.cancelAll()
        return result
    }
}
