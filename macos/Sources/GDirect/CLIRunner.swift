import Darwin
import Foundation

struct CLIResult: Sendable {
    let status: Int32
    let stdout: String
    let stderr: String
}

enum CLIError: LocalizedError, Sendable {
    case launchFailed(String)
    case nonZeroExit(CLIResult)
    case cancelled(CLIResult)
    case listOutputTooLarge
    case invalidListJSON

    var errorDescription: String? {
        switch self {
        case let .launchFailed(message):
            return "Unable to launch gdirect: \(message)"
        case let .nonZeroExit(result):
            let output = result.stderr.isEmpty ? result.stdout : result.stderr
            let detail = LogRedactor.redact(output).trimmingCharacters(in: .whitespacesAndNewlines)
            return detail.isEmpty
                ? "gdirect exited with status \(result.status)"
                : "gdirect exited with status \(result.status): \(detail)"
        case .cancelled:
            return "gdirect operation was cancelled"
        case .listOutputTooLarge:
            return "File list exceeds the supported output limit. Open a smaller folder."
        case .invalidListJSON:
            return "gdirect returned an invalid file list"
        }
    }
}

enum CLIOutputStream: Sendable {
    case stdout
    case stderr
}

enum LogRedactor {
    static func redact(_ input: String) -> String {
        let bytes = Array(input.utf8)
        let marker = Array("?<redacted>".utf8)
        var output: [UInt8] = []
        output.reserveCapacity(bytes.count)
        var index = 0
        while index < bytes.count {
            if isASCIILetter(bytes[index]) {
                var schemeEnd = index + 1
                while schemeEnd < bytes.count, isSchemeCharacter(bytes[schemeEnd]) {
                    schemeEnd += 1
                }
                if schemeEnd + 2 < bytes.count,
                   bytes[schemeEnd] == 58,
                   bytes[schemeEnd + 1] == 47,
                   bytes[schemeEnd + 2] == 47 {
                    var urlEnd = schemeEnd + 3
                    while urlEnd < bytes.count, !isURLBoundary(bytes[urlEnd]) {
                        urlEnd += 1
                    }
                    output.append(contentsOf: bytes[index..<urlEnd])
                    if urlEnd < bytes.count, bytes[urlEnd] == 63 {
                        output.append(contentsOf: marker)
                        urlEnd += 1
                        while urlEnd < bytes.count,
                              !isASCIIWhitespace(bytes[urlEnd]),
                              bytes[urlEnd] != 35 {
                            urlEnd += 1
                        }
                    }
                    index = urlEnd
                    continue
                }
                output.append(contentsOf: bytes[index..<schemeEnd])
                index = schemeEnd
                continue
            }
            output.append(bytes[index])
            index += 1
        }
        return String(decoding: output, as: UTF8.self)
    }

    private static func isSchemeCharacter(_ byte: UInt8) -> Bool {
        isASCIILetter(byte) || (48...57).contains(byte) || byte == 43 || byte == 45 || byte == 46
    }

    private static func isASCIILetter(_ byte: UInt8) -> Bool {
        (65...90).contains(byte) || (97...122).contains(byte)
    }

    private static func isURLBoundary(_ byte: UInt8) -> Bool {
        byte == 63 || byte == 35 || isASCIIWhitespace(byte)
    }

    private static func isASCIIWhitespace(_ byte: UInt8) -> Bool {
        byte == 32 || (9...13).contains(byte)
    }
}

final class CLIRunner: @unchecked Sendable {
    private let locator: CLILocator
    private let terminationGrace: Duration
    private let outputLimit: Int
    private let listOutputLimit: Int

    init(
        locator: CLILocator = CLILocator(),
        terminationGrace: Duration = .milliseconds(500),
        outputLimit: Int = 1_048_576,
        listOutputLimit: Int = 16_777_216
    ) {
        self.outputLimit = max(1, outputLimit)
        self.listOutputLimit = max(1, listOutputLimit)
        self.locator = locator
        self.terminationGrace = terminationGrace
    }

    func run(
        _ command: CLICommand,
        onOutput: (@Sendable (CLIOutputStream, Data) -> Void)? = nil
    ) async throws -> CLIResult {
        let execution = try CLIExecution(
            executable: locator.executableURL(),
            arguments: command.arguments,
            environment: locator.processEnvironment(),
            terminationGrace: terminationGrace,
            onOutput: onOutput,
            outputLimit: outputLimit,
            listOutputLimit: command.arguments.first == "ls" ? listOutputLimit : nil
        )
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                execution.start(continuation: continuation)
            }
        } onCancel: {
            execution.cancel()
        }
    }
}

private final class CLIExecution: @unchecked Sendable {
    fileprivate typealias Continuation = CheckedContinuation<CLIResult, any Error>
    fileprivate typealias Completion = (Continuation, Result<CLIResult, any Error>)

    private let lock = NSLock()
    private let process = Process()
    private let stdoutPipe = Pipe()
    private let stderrPipe = Pipe()
    private let terminationGrace: Duration
    private let onOutput: (@Sendable (CLIOutputStream, Data) -> Void)?
    private var continuation: Continuation?
    private let outputLimit: Int
    private let listOutputLimit: Int?
    private var oversizedList = false
    private var stdout = Data()
    private var stderr = Data()
    private var stdoutEnded = false
    private var stderrEnded = false
    private var terminationStatus: Int32?
    private var launched = false
    private var cancelled = false
    private var finished = false
    private var escalationScheduled = false

    init(
        executable: URL,
        arguments: [String],
        environment: [String: String],
        terminationGrace: Duration,
        onOutput: (@Sendable (CLIOutputStream, Data) -> Void)?,
        outputLimit: Int,
        listOutputLimit: Int?
    ) {
        self.terminationGrace = terminationGrace
        self.outputLimit = outputLimit
        self.listOutputLimit = listOutputLimit
        self.onOutput = onOutput
        process.executableURL = executable
        process.arguments = arguments
        process.environment = environment
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe
        process.terminationHandler = { [weak self] process in
            self?.processTerminated(status: process.terminationStatus)
        }
        stdoutPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            self?.read(handle: handle, stream: .stdout)
        }
        stderrPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            self?.read(handle: handle, stream: .stderr)
        }
    }

    func start(continuation: Continuation) {
        lock.lock()
        self.continuation = continuation
        if cancelled {
            let completion = finishLocked(status: -1)
            lock.unlock()
            resume(completion)
            return
        }
        do {
            try process.run()
            launched = true
            let shouldTerminate = cancelled
            lock.unlock()
            if shouldTerminate {
                terminateAndScheduleEscalation()
            }
        } catch {
            launched = false
            let completion = finishLocked(error: .launchFailed(error.localizedDescription))
            lock.unlock()
            resume(completion)
        }
    }

    func cancel() {
        lock.lock()
        if finished {
            lock.unlock()
            return
        }
        cancelled = true
        let shouldTerminate = launched && terminationStatus == nil
        lock.unlock()
        if shouldTerminate {
            terminateAndScheduleEscalation()
        }
    }

    private func read(handle: FileHandle, stream: CLIOutputStream) {
        // FileHandle.read(upToCount:) can wait to fill its buffer, delaying OAuth
        // prompts until exit. A single POSIX read returns currently available bytes.
        var buffer = [UInt8](repeating: 0, count: 65_536)
        var count: Int
        repeat {
            count = Darwin.read(handle.fileDescriptor, &buffer, buffer.count)
        } while count < 0 && errno == EINTR
        if count < 0 { return }
        let data = Data(buffer.prefix(count))
        var completion: Completion?
        lock.lock()
        if data.isEmpty {
            switch stream {
            case .stdout:
                stdoutEnded = true
            case .stderr:
                stderrEnded = true
            }
            completion = finishIfReadyLocked()
        } else {
            switch stream {
            case .stdout:
                let limit = listOutputLimit ?? outputLimit
                if listOutputLimit != nil, data.count > limit - stdout.count { oversizedList = true }
                stdout.append(data.prefix(max(0, limit - stdout.count)))
            case .stderr:
                stderr.append(data.prefix(max(0, outputLimit - stderr.count)))
            }
        }
        lock.unlock()
        if !data.isEmpty {
            onOutput?(stream, data)
        }
        resume(completion)
    }

    private func processTerminated(status: Int32) {
        lock.lock()
        terminationStatus = status
        let completion = finishIfReadyLocked()
        lock.unlock()
        resume(completion)
    }

    private func terminateAndScheduleEscalation() {
        if process.isRunning {
            process.terminate()
        }
        lock.lock()
        if escalationScheduled || finished {
            lock.unlock()
            return
        }
        escalationScheduled = true
        lock.unlock()
        Task.detached { [self] in
            try? await Task.sleep(for: terminationGrace)
            let shouldKill = lock.withLock { !finished && terminationStatus == nil }
            if shouldKill, process.isRunning {
                Darwin.kill(process.processIdentifier, SIGKILL)
            }
        }
    }

    private func finishIfReadyLocked() -> Completion? {
        guard let terminationStatus, stdoutEnded, stderrEnded else {
            return nil
        }
        return finishLocked(status: terminationStatus)
    }

    private func finishLocked(status: Int32) -> Completion? {
        guard !finished, let continuation else {
            return nil
        }
        finished = true
        let result = CLIResult(
            status: status,
            stdout: String(decoding: stdout, as: UTF8.self),
            stderr: String(decoding: stderr, as: UTF8.self)
        )
        if cancelled { return (continuation, .failure(CLIError.cancelled(result))) }
        if oversizedList { return (continuation, .failure(CLIError.listOutputTooLarge)) }
        return (continuation, .success(result))
    }

    private func finishLocked(error: CLIError) -> Completion? {
        guard !finished, let continuation else {
            return nil
        }
        finished = true
        return (continuation, .failure(error))
    }

    private func resume(_ completion: Completion?) {
        guard let (continuation, result) = completion else {
            return
        }
        process.terminationHandler = nil
        stdoutPipe.fileHandleForReading.readabilityHandler = nil
        stderrPipe.fileHandleForReading.readabilityHandler = nil
        try? stdoutPipe.fileHandleForReading.close()
        try? stderrPipe.fileHandleForReading.close()
        continuation.resume(with: result)
    }
}
