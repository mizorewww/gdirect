import Foundation

protocol GDirectServing: Sendable {
    @discardableResult
    func login(clientSecret: String?, credentials: String?) async throws -> CLIResult
    @discardableResult
    func reauth() async throws -> CLIResult
    @discardableResult
    func logout() async throws -> CLIResult
    func list(folder: String, ip: String?) async throws -> [DriveFile]
    func download(
        file: String,
        output: String,
        exportMIME: String?,
        settings: TransferSettings
    ) async throws -> URL
    @discardableResult
    func upload(file: String, parent: String, name: String?, ip: String?) async throws -> CLIResult
    @discardableResult
    func scan(ips: String?) async throws -> CLIResult
    @discardableResult
    func scanList() async throws -> CLIResult
    @discardableResult
    func mount(path: String, readOnly: Bool, connections: Int) async throws -> CLIResult
    @discardableResult
    func unmount() async throws -> CLIResult
}

final class GDirectService: GDirectServing, @unchecked Sendable {
    private let runner: CLIRunner
    private let log: DiagnosticLog

    var diagnostics: String {
        log.contents
    }

    init(runner: CLIRunner = CLIRunner(), logCapacity: Int = 65_536) {
        self.runner = runner
        log = DiagnosticLog(capacity: logCapacity)
    }

    @discardableResult
    func login(clientSecret: String? = nil, credentials: String? = nil) async throws -> CLIResult {
        try await execute(.login(clientSecret: clientSecret, credentials: credentials))
    }

    @discardableResult
    func reauth() async throws -> CLIResult {
        try await execute(.reauth)
    }

    @discardableResult
    func logout() async throws -> CLIResult {
        try await execute(.logout)
    }

    func list(folder: String = "root", ip: String? = nil) async throws -> [DriveFile] {
        let result = try await execute(.list(folder: folder, ip: ip))
        do {
            return try JSONDecoder().decode([DriveFile].self, from: Data(result.stdout.utf8))
        } catch {
            throw CLIError.invalidListJSON
        }
    }

    func download(
        file: String,
        output: String,
        exportMIME: String? = nil,
        settings: TransferSettings = TransferSettings()
    ) async throws -> URL {
        _ = try await execute(.download(
            file: file,
            output: output,
            exportMIME: exportMIME,
            settings: settings
        ))
        return URL(fileURLWithPath: output)
    }

    @discardableResult
    func upload(
        file: String,
        parent: String = "root",
        name: String? = nil,
        ip: String? = nil
    ) async throws -> CLIResult {
        try await execute(.upload(file: file, parent: parent, name: name, ip: ip))
    }

    @discardableResult
    func scan(ips: String? = nil) async throws -> CLIResult {
        try await execute(.scan(ips: ips))
    }

    @discardableResult
    func scanList() async throws -> CLIResult {
        try await execute(.scanList)
    }

    @discardableResult
    func mount(path: String, readOnly: Bool = false, connections: Int = 8) async throws -> CLIResult {
        try await execute(.mount(path: path, readOnly: readOnly, connections: connections))
    }

    @discardableResult
    func unmount() async throws -> CLIResult {
        try await execute(.unmount)
    }

    private func execute(_ command: CLICommand) async throws -> CLIResult {
        do {
            let result = try await runner.run(command)
            log.append(result)
            guard result.status == 0 else {
                throw CLIError.nonZeroExit(log.capped(result))
            }
            return result
        } catch let CLIError.cancelled(result) {
            log.append(result)
            throw CLIError.cancelled(log.capped(result))
        }
    }
}

private final class DiagnosticLog: @unchecked Sendable {
    private let lock = NSLock()
    private let capacity: Int
    private var data = Data()

    init(capacity: Int) {
        self.capacity = max(0, capacity)
    }

    var contents: String {
        lock.lock()
        let snapshot = data
        lock.unlock()
        return String(decoding: snapshot, as: UTF8.self)
    }

    func append(_ result: CLIResult) {
        guard capacity > 0 else {
            return
        }
        let entry = "\n[stdout] \(capped(result.stdout))\n[stderr] \(capped(result.stderr))"
        lock.lock()
        data.append(Data(entry.utf8))
        if data.count > capacity {
            data = Data(data.suffix(capacity))
        }
        lock.unlock()
    }

    func capped(_ result: CLIResult) -> CLIResult {
        CLIResult(status: result.status, stdout: capped(result.stdout), stderr: capped(result.stderr))
    }

    private func capped(_ input: String) -> String {
        let redacted = LogRedactor.redact(input)
        guard capacity > 0 else {
            return ""
        }
        guard redacted.utf8.count > capacity else {
            return redacted
        }
        return String(decoding: redacted.utf8.suffix(capacity), as: UTF8.self)
    }
}
