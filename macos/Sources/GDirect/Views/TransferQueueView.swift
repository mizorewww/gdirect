import AppKit
import SwiftUI

@MainActor
private final class TransferQueueViewState: ObservableObject {
    @Published var expanded = true
}

struct TransferQueueView: View {
    @ObservedObject var queue: TransferQueue
    @StateObject private var viewState = TransferQueueViewState()

    private var activeCount: Int { queue.items.filter(\.isActive).count }

    var body: some View {
        VStack(spacing: 0) {
            Button { withAnimation(.easeOut(duration: 0.15)) { viewState.expanded.toggle() } } label: {
                HStack(spacing: 8) {
                    Image(systemName: viewState.expanded ? "chevron.down" : "chevron.right")
                        .font(.caption.weight(.semibold))
                    Text("Transfers")
                        .font(.headline)
                    if activeCount > 0 {
                        Text("\(activeCount) active")
                            .foregroundStyle(.secondary)
                            .font(.callout)
                        ProgressView()
                            .controlSize(.small)
                    }
                    Spacer()
                    Text("\(queue.items.count) this session")
                        .foregroundStyle(.secondary)
                        .font(.caption)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 14)
            .padding(.vertical, 9)

            if viewState.expanded {
                Divider()
                if queue.items.isEmpty {
                    Text("Downloads and uploads will appear here.")
                        .foregroundStyle(.secondary)
                        .font(.callout)
                        .frame(maxWidth: .infinity, minHeight: 52)
                } else {
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(queue.items.reversed()) { item in
                                TransferRow(item: item, queue: queue)
                                Divider().padding(.leading, 36)
                            }
                        }
                    }
                    .frame(height: 148)
                }
            }
        }
        .background(.regularMaterial)
        .overlay(alignment: .top) { Divider() }
    }
}

private struct TransferRow: View {
    let item: TransferItem
    let queue: TransferQueue

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .foregroundStyle(color)
                .frame(width: 16)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.name)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let message = item.errorMessage {
                    Text(message)
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(1)
                        .textSelection(.enabled)
                        .help(message)
                } else if let destination = item.destination, item.state == .succeeded {
                    Text(destination.path)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(item.state.rawValue.capitalized)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 74, alignment: .trailing)
            if item.isActive {
                Button("Cancel") { queue.cancel(id: item.id) }
                    .controlSize(.small)
            } else if item.state == .failed || item.state == .cancelled {
                Button("Retry") { queue.retry(id: item.id) }
                    .controlSize(.small)
            }
            if item.state == .succeeded, let destination = item.destination {
                Button {
                    NSWorkspace.shared.activateFileViewerSelecting([destination])
                } label: {
                    Image(systemName: "finder")
                }
                .buttonStyle(.borderless)
                .help("Show in Finder")
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    private var symbol: String {
        switch item.state {
        case .queued: "clock"
        case .running: "arrow.down.circle"
        case .succeeded: "checkmark.circle.fill"
        case .failed: "exclamationmark.circle.fill"
        case .cancelled: "xmark.circle"
        }
    }

    private var color: Color {
        switch item.state {
        case .succeeded: .green
        case .failed: .red
        case .running: .accentColor
        case .queued, .cancelled: .secondary
        }
    }
}
