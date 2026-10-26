import Foundation

protocol GDirectServing: Sendable {
    @discardableResult
    func login(clientSecret: String?, credentials: String?, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func reauth(googleOnly: Bool) async throws -> CLIResult
    func accounts(googleOnly: Bool) async throws -> [DriveAccount]
    @discardableResult
    func switchAccount(_ account: String, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func logout(account: String?, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func logoutAll(googleOnly: Bool) async throws -> CLIResult
    func list(folder: String, ip: String?, sni: String?, googleOnly: Bool) async throws -> [DriveFile]
    func download(
        file: String,
        output: String,
        exportMIME: String?,
        settings: TransferSettings
    ) async throws -> URL
    @discardableResult
    func upload(file: String, parent: String, name: String?, ip: String?, sni: String?, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func scan(ips: String?, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func scanList(googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func mount(path: String, readOnly: Bool, connections: Int, googleOnly: Bool) async throws -> CLIResult
    @discardableResult
    func unmount(googleOnly: Bool) async throws -> CLIResult
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
    func login(clientSecret: String? = nil, credentials: String? = nil, googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.login(clientSecret: clientSecret, credentials: credentials), googleOnly: googleOnly)
    }

    @discardableResult
    func reauth(googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.reauth, googleOnly: googleOnly)
    }

    func accounts(googleOnly: Bool = false) async throws -> [DriveAccount] {
        let result = try await execute(.accounts, googleOnly: googleOnly)
        return result.stdout.split(whereSeparator: \.isNewline).compactMap { line in
            let isDefault = line.hasPrefix("* ")
            guard isDefault || line.hasPrefix("  ") else { return nil }
            let email = line.dropFirst(2).trimmingCharacters(in: .whitespacesAndNewlines)
            return email.isEmpty ? nil : DriveAccount(email: email, isDefault: isDefault)
        }
    }

    @discardableResult
    func switchAccount(_ account: String, googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.switchAccount(account), googleOnly: googleOnly)
    }

    @discardableResult
    func logout(account: String? = nil, googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.logout(account: account), googleOnly: googleOnly)
    }

    @discardableResult
    func logoutAll(googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.logoutAll, googleOnly: googleOnly)
    }

    func list(folder: String = "root", ip: String? = nil, sni: String? = nil, googleOnly: Bool = false) async throws -> [DriveFile] {
        let result = try await execute(.list(folder: folder, ip: ip, sni: sni), googleOnly: googleOnly)
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
        ), googleOnly: settings.googleOnly)
        return URL(fileURLWithPath: output)
    }

    @discardableResult
    func upload(
        file: String,
        parent: String = "root",
        name: String? = nil,
        ip: String? = nil,
        sni: String? = nil,
        googleOnly: Bool = false
    ) async throws -> CLIResult {
        try await execute(.upload(file: file, parent: parent, name: name, ip: ip, sni: sni), googleOnly: googleOnly)
    }

    @discardableResult
    func scan(ips: String? = nil, googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.scan(ips: ips), googleOnly: googleOnly)
    }

    @discardableResult
    func scanList(googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.scanList, googleOnly: googleOnly)
    }

    @discardableResult
    func mount(path: String, readOnly: Bool = false, connections: Int = 8, googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.mount(path: path, readOnly: readOnly, connections: connections), googleOnly: googleOnly)
    }

    @discardableResult
    func unmount(googleOnly: Bool = false) async throws -> CLIResult {
        try await execute(.unmount, googleOnly: googleOnly)
    }

    private func execute(_ command: CLICommand, googleOnly: Bool) async throws -> CLIResult {
        do {
            let result = try await runner.run(command, googleOnly: googleOnly)
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
