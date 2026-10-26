import SwiftUI

struct SettingsView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            Section("Transfers") {
                LabeledContent("Download folder") {
                    HStack {
                        Text(model.settings.downloadDirectoryPath)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: 300, alignment: .trailing)
                        Button("Choose…") {
                            if let url = AppKitPanels.directory(title: "Default Download Folder") {
                                model.settings.downloadDirectoryPath = url.path
                            }
                        }
                    }
                }
                Stepper(value: $model.settings.connections, in: 1...16) {
                    LabeledContent("Connections", value: "\(model.settings.connections)")
                }
            }

            Section("Network") {
                Toggle("Google endpoints only", isOn: $model.settings.googleOnly)
                Text("Passes --google-only to every gdirect operation.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                TextField("Custom IP (optional)", text: $model.settings.customIP, prompt: Text("Automatic"))
                TextField("SNI (optional)", text: Binding(
                    get: { model.settings.sni },
                    set: { value in
                        model.settings.sni = value
                        if !value.isEmpty { model.settings.noSNI = false }
                    }
                ), prompt: Text("Default"))
                Toggle("Disable SNI", isOn: Binding(
                    get: { model.settings.noSNI },
                    set: { value in
                        model.settings.noSNI = value
                        if value { model.settings.sni = "" }
                    }
                ))
                LabeledContent("Custom CA") {
                    HStack {
                        Text(model.settings.caCertPath.isEmpty ? "CLI embedded Google GTS roots" : model.settings.caCertPath)
                            .foregroundStyle(model.settings.caCertPath.isEmpty ? .secondary : .primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: 300, alignment: .trailing)
                        Button("Choose…") {
                            if let url = AppKitPanels.file(title: "Custom CA Certificate") {
                                model.settings.caCertPath = url.path
                            }
                        }
                        if !model.settings.caCertPath.isEmpty {
                            Button("Clear") { model.settings.caCertPath = "" }
                        }
                    }
                }
            }

            Section("Mount") {
                LabeledContent("Mount folder") {
                    HStack {
                        Text(model.settings.mountPath)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: 300, alignment: .trailing)
                        Button("Choose…") {
                            if let url = AppKitPanels.directory(title: "Mount Folder") {
                                model.settings.mountPath = url.path
                            }
                        }
                    }
                }
                Toggle("Read-only mount", isOn: $model.settings.readOnly)
            }

            Text("Only these non-sensitive preferences are saved on this Mac. Credentials and CLI diagnostics are not stored by the app.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
        .frame(maxWidth: 720)
        .padding()
    }
}
