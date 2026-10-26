import AppKit
import SwiftUI

@MainActor
final class GDirectAppDelegate: NSObject, NSApplicationDelegate {
    var model: AppModel?

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let model else { return .terminateNow }
        return model.applicationShouldTerminate() ? .terminateNow : .terminateCancel
    }
}

@main
struct GDirectApp: App {
    @NSApplicationDelegateAdaptor(GDirectAppDelegate.self) private var appDelegate
    @StateObject private var model = AppModel(service: GDirectService())

    var body: some Scene {
        WindowGroup("GDirect") {
            MainView(model: model)
                .frame(minWidth: 860, minHeight: 560)
                .task {
                    appDelegate.model = model
                    await model.loadRoot()
                }
        }
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(after: .newItem) {
                Button("Refresh Folder") {
                    Task { await model.refresh() }
                }
                .keyboardShortcut("r")
            }
        }
    }
}
