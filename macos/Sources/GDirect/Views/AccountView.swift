import SwiftUI
import UniformTypeIdentifiers

@MainActor
private final class AccountViewState: ObservableObject {
    @Published var method: SignInMethod = .desktop
    @Published var clientSecretPath = ""
    @Published var credentialsPath = ""
    @Published var selectedAccount = ""
}

struct AccountView: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var viewState = AccountViewState()

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Google Drive Account")
                .font(.title2)
                .fontWeight(.semibold)
            Text("Sign in with a desktop OAuth client, or import an existing credentials file. Credentials remain managed by gdirect.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if !model.accounts.isEmpty {
                GroupBox("Signed-in accounts") {
                    VStack(alignment: .leading, spacing: 10) {
                        Picker("Account", selection: $viewState.selectedAccount) {
                            ForEach(model.accounts) { account in
                                Text(account.email + (account.isDefault ? " (default)" : ""))
                                    .tag(account.email)
                            }
                        }
                        HStack {
                            Button("Switch") {
                                run { await model.switchAccount(viewState.selectedAccount) }
                            }
                            .disabled(model.isAccountBusy || viewState.selectedAccount.isEmpty ||
                                      model.accounts.first(where: { $0.email == viewState.selectedAccount })?.isDefault == true)
                            Button("Log Out All") { run { await model.logoutAll() } }
                                .disabled(model.isAccountBusy)
                        }
                    }
                    .padding(6)
                }
            }

            Picker("Sign-in method", selection: $viewState.method) {
                ForEach(SignInMethod.allCases) { method in
                    Text(method.rawValue).tag(method)
                }
            }
            .pickerStyle(.segmented)

            if viewState.method == .desktop {
                GroupBox("Desktop OAuth client") {
                    HStack {
                        TextField("Client secret JSON (optional)", text: $viewState.clientSecretPath)
                            .textFieldStyle(.roundedBorder)
                        Button("Browse…") {
                            if let url = AppKitPanels.file(title: "Choose Client Secret", allowedTypes: [.json]) {
                                viewState.clientSecretPath = url.path
                            }
                        }
                    }
                    .padding(6)
                }

            } else {
                GroupBox("Existing credentials") {
                    HStack {
                        TextField("Credentials JSON (optional)", text: $viewState.credentialsPath)
                            .textFieldStyle(.roundedBorder)
                        Button("Browse…") {
                            if let url = AppKitPanels.file(title: "Import Credentials", allowedTypes: [.json]) {
                                viewState.credentialsPath = url.path
                            }
                        }
                    }
                    .padding(6)
                }

            }

            if let message = model.errorMessage {
                Text(message)
                    .foregroundStyle(.red)
                    .font(.callout)
                    .textSelection(.enabled)
            }

            HStack {
                Button("Reauthorize…") { run { await model.reauth() } }
                    .disabled(model.isAccountBusy)
                Button("Log Out") { run { await model.logout() } }
                    .disabled(model.isAccountBusy)
                Spacer()
                Button("Close") { dismiss() }
                Button("Sign In") {
                    run {
                        let sources = viewState.method.sources(
                            clientSecret: viewState.clientSecretPath,
                            credentials: viewState.credentialsPath
                        )
                        await model.login(clientSecret: sources.clientSecret, credentials: sources.credentials)
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(model.isAccountBusy)
            }
            if model.isAccountBusy {
                ProgressView()
                    .controlSize(.small)
            }
        }
        .padding(22)
        .frame(width: 540)
        .task {
            await model.refreshAccounts()
            viewState.selectedAccount = model.accounts.first(where: \.isDefault)?.email
                ?? model.accounts.first?.email ?? ""
        }
    }

    private func run(_ operation: @escaping @MainActor () async -> Void) {
        Task {
            await operation()
            if model.errorMessage == nil { dismiss() }
        }
    }
}
