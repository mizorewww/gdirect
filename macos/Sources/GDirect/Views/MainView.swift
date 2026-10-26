import AppKit
import SwiftUI
import UniformTypeIdentifiers

private enum MainSection: String, CaseIterable, Identifiable {
    case files = "Files"
    case network = "Network & Mount"
    case settings = "Settings"

    var id: String { rawValue }
    var symbol: String {
        switch self {
        case .files: "folder"
        case .network: "network"
        case .settings: "slider.horizontal.3"
        }
    }
}

@MainActor
private final class MainViewState: ObservableObject {
    @Published var section: MainSection? = .files
    @Published var showingAccount = false
}

struct MainView: View {
    @ObservedObject var model: AppModel
    @StateObject private var viewState = MainViewState()

    var body: some View {
        NavigationSplitView {
            List(MainSection.allCases, selection: $viewState.section) { item in
                Label(item.rawValue, systemImage: item.symbol)
                    .tag(item)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 180, max: 210)
        } detail: {
            VStack(spacing: 0) {
                header
                if let message = model.errorMessage {
                    errorBanner(message)
                }
                Group {
                    switch viewState.section ?? .files {
                    case .files:
                        FileBrowserView(model: model)
                    case .network:
                        MaintenanceView(model: model)
                    case .settings:
                        SettingsView(model: model)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                TransferQueueView(queue: model.transfers)
            }
        }
        .sheet(isPresented: $viewState.showingAccount) {
            AccountView(model: model)
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Circle()
                .fill(model.isAuthenticated ? Color.green : Color.secondary.opacity(0.45))
                .frame(width: 7, height: 7)
            Text(model.isAuthenticated ? "Google Drive connected" : "Google Drive")
                .font(.headline)
            Spacer()
            Button("Account…") { viewState.showingAccount = true }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }

    private func errorBanner(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            Text(message)
                .font(.callout)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button("Dismiss") { model.errorMessage = nil }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(Color.orange.opacity(0.09))
    }
}

@MainActor
enum AppKitPanels {
    static func file(
        title: String,
        prompt: String = "Choose",
        allowedTypes: [UTType]? = nil
    ) -> URL? {
        let panel = NSOpenPanel()
        panel.title = title
        panel.prompt = prompt
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.allowedContentTypes = allowedTypes ?? []
        return panel.runModal() == .OK ? panel.url : nil
    }

    static func directory(title: String, prompt: String = "Choose") -> URL? {
        let panel = NSOpenPanel()
        panel.title = title
        panel.prompt = prompt
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        return panel.runModal() == .OK ? panel.url : nil
    }

    static func save(title: String, name: String, directory: URL? = nil) -> URL? {
        let panel = NSSavePanel()
        panel.title = title
        panel.nameFieldStringValue = name
        panel.directoryURL = directory
        panel.canCreateDirectories = true
        return panel.runModal() == .OK ? panel.url : nil
    }
}
