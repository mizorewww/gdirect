import Combine
import Foundation

enum TransferState: String, Sendable {
    case queued
    case running
    case succeeded
    case failed
    case cancelled
}

enum TransferOperation: Sendable {
    case download(file: DriveFile, destination: URL, exportMIME: String?)
    case publicDownload(link: String, destination: URL)
    case upload(source: URL, parent: String, name: String?)
}

struct TransferItem: Identifiable, Sendable {
    let id: UUID
    let name: String
    let operation: TransferOperation
    var state: TransferState
    var destination: URL?
    var errorMessage: String?

    var isActive: Bool { state == .queued || state == .running }
}

@MainActor
final class TransferQueue: ObservableObject {
    @Published private(set) var items: [TransferItem] = []

    private let service: GDirectServing
    private let settings: @MainActor () -> TransferSettings
    private let uploadSucceeded: @MainActor (String) async -> Void
    private var activeTask: Task<Void, Never>?
    private var activeID: UUID?
    private var changingCredentials = false

    // MainActor makes the idle check and gate acquisition atomic with enqueue/retry.
    func beginCredentialChange() -> Bool {
        guard !changingCredentials, activeTask == nil, !items.contains(where: \.isActive) else { return false }
        changingCredentials = true
        return true
    }

    func endCredentialChange() { changingCredentials = false }


    init(
        service: GDirectServing,
        settings: @escaping @MainActor () -> TransferSettings = { TransferSettings() },
        uploadSucceeded: @escaping @MainActor (String) async -> Void = { _ in }
    ) {
        self.service = service
        self.settings = settings
        self.uploadSucceeded = uploadSucceeded
    }

    @discardableResult
    func enqueueDownload(file: DriveFile, destination: URL, exportMIME: String? = nil) -> UUID {
        enqueue(
            name: file.name,
            operation: .download(file: file, destination: destination, exportMIME: exportMIME),
            destination: destination
        )
    }

    @discardableResult
    func enqueuePublicDownload(link: String, name: String, destination: URL) -> UUID {
        enqueue(
            name: name,
            operation: .publicDownload(link: link, destination: destination),
            destination: destination
        )
    }

    @discardableResult
    func enqueueUpload(source: URL, parent: String, name: String? = nil) -> UUID {
        enqueue(
            name: source.lastPathComponent,
            operation: .upload(source: source, parent: parent, name: name),
            destination: nil
        )
    }

    func item(id: UUID) -> TransferItem? {
        items.first { $0.id == id }
    }

    func cancel(id: UUID) {
        guard let index = items.firstIndex(where: { $0.id == id }) else { return }
        switch items[index].state {
        case .queued:
            items[index].state = .cancelled
        case .running:
            items[index].state = .cancelled
            if activeID == id {
                activeTask?.cancel()
            }
        case .succeeded, .failed, .cancelled:
            break
        }
    }

    func retry(id: UUID) {
        guard !changingCredentials, let index = items.firstIndex(where: { $0.id == id }),
              items[index].state == .failed || items[index].state == .cancelled else { return }
        items[index].state = .queued
        items[index].errorMessage = nil
        scheduleDrain()
    }

    func waitForIdle() async {
        while activeTask != nil || items.contains(where: { $0.state == .queued }) {
            if let activeTask {
                await activeTask.value
            } else {
                await Task.yield()
            }
        }
    }

    @discardableResult
    private func enqueue(name: String, operation: TransferOperation, destination: URL?) -> UUID {
        let id = UUID()
        items.append(TransferItem(
            id: id,
            name: name,
            operation: operation,
            state: changingCredentials ? .failed : .queued,
            destination: destination,
            errorMessage: changingCredentials ? "Account change in progress. Retry after it finishes." : nil
        ))
        scheduleDrain()
        return id
    }

    private func scheduleDrain() {
        guard activeTask == nil, items.contains(where: { $0.state == .queued }) else { return }
        activeTask = Task { [weak self] in
            await self?.drainNext()
        }
    }

    private func drainNext() async {
        guard let index = items.firstIndex(where: { $0.state == .queued }) else {
            activeTask = nil
            activeID = nil
            return
        }
        let id = items[index].id
        let operation = items[index].operation
        activeID = id
        items[index].state = .running

        do {
            switch operation {
            case let .download(file, destination, exportMIME):
                let result = try await service.download(
                    file: file.id,
                    output: destination.path,
                    exportMIME: exportMIME,
                    settings: settings()
                )
                updateRunning(id: id) { $0.destination = result; $0.state = .succeeded }
            case let .publicDownload(link, destination):
                let result = try await service.download(
                    file: link,
                    output: destination.path,
                    exportMIME: nil,
                    settings: settings()
                )
                updateRunning(id: id) { $0.destination = result; $0.state = .succeeded }
            case let .upload(source, parent, name):
                _ = try await service.upload(
                    file: source.path,
                    parent: parent,
                    name: name,
                    ip: settings().ip
                )
                if updateRunning(id: id, { $0.state = .succeeded }) {
                    await uploadSucceeded(parent)
                }
            }
        } catch {
            updateRunning(id: id) {
                if isCancellation(error) {
                    $0.state = .cancelled
                } else {
                    $0.state = .failed
                    $0.errorMessage = error.localizedDescription
                }
            }
        }

        activeID = nil
        activeTask = nil
        scheduleDrain()
    }

    @discardableResult
    private func updateRunning(id: UUID, _ update: (inout TransferItem) -> Void) -> Bool {
        guard let index = items.firstIndex(where: { $0.id == id }),
              items[index].state == .running else { return false }
        update(&items[index])
        return true
    }

    private func isCancellation(_ error: Error) -> Bool {
        if error is CancellationError { return true }
        if case CLIError.cancelled = error { return true }
        return false
    }
}
