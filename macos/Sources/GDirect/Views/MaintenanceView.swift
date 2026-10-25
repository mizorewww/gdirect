import AppKit
import SwiftUI

struct MaintenanceView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(alignment: .top, spacing: 28) {
                Form {
                    Section("Endpoint Scan") {
                        TextField("IP file path (optional)", text: $model.scanTargets)
                        Button("Choose IP File…") {
                            let panel = NSOpenPanel()
                            panel.canChooseDirectories = false
                            panel.allowsMultipleSelection = false
                            if panel.runModal() == .OK, let url = panel.url {
                                model.scanTargets = url.path
                            }
                        }
                        HStack {
                            Button("Rescan") { run { await model.scan() } }
                            Button("List Endpoints") { run { await model.listEndpoints() } }
                        }
                        .disabled(model.isMaintenanceBusy)
                    }
                }
                .formStyle(.grouped)

                Form {
                    Section("Drive Mount") {
                        LabeledContent("Location") {
                            Text(model.settings.mountPath)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        LabeledContent("Mode", value: model.settings.readOnly ? "Read only" : "Read/write")
                        HStack {
                            Button("Mount") { run { await model.mount() } }
                                .disabled(model.isMaintenanceBusy || model.isMounted)
                            Button("Unmount") { run { await model.unmount() } }
                                .disabled(model.isMaintenanceBusy)
                        }
                        Text("Requires rclone 1.75.2 or newer. Installation is not performed by GDirect.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                .formStyle(.grouped)
            }

            if model.isMaintenanceBusy {
                ProgressView()
                    .controlSize(.small)
            }

            GroupBox("Endpoints") {
                ScrollView {
                    Text(model.scanOutput.isEmpty ? "No endpoint results yet." : model.scanOutput)
                        .foregroundStyle(model.scanOutput.isEmpty ? .secondary : .primary)
                        .font(.system(.callout, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                }
                .frame(minHeight: 130)
            }
        }
        .padding(20)
    }

    private func run(_ operation: @escaping @MainActor () async -> Void) {
        Task { await operation() }
    }
}
