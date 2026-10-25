import SwiftUI

@main
struct GDirectApp: App {
    @StateObject private var model = AppModel(service: GDirectService())

    var body: some Scene {
        WindowGroup("GDirect") {
            MainView(model: model)
                .frame(minWidth: 860, minHeight: 560)
                .task { await model.loadRoot() }
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
