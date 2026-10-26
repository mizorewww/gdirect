import SwiftUI

@MainActor
private final class FileBrowserViewState: ObservableObject {
    @Published var downloadFile: DriveFile?
    @Published var selection: DriveFile.ID?
    @Published var exportFile: DriveFile?
    @Published var showingPublicDownload = false
}

struct FileBrowserView: View {
    @ObservedObject var model: AppModel
    @StateObject private var viewState = FileBrowserViewState()

    private var selectedFile: DriveFile? {
        model.files.first { $0.id == viewState.selection }
    }

    var body: some View {
        VStack(spacing: 0) {
            browserToolbar
            Divider()
            ZStack {
                Table(model.files, selection: $viewState.selection) {
                    TableColumn("Name") { file in
                        Label {
                            Text(file.name)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        } icon: {
                            Image(systemName: isFolder(file) ? "folder" : icon(for: file))
                                .foregroundStyle(isFolder(file) ? Color.accentColor : Color.secondary)
                        }
                        .onTapGesture(count: 2) { open(file) }
                    }
                    .width(min: 240, ideal: 360)
                    TableColumn("Type") { file in
                        Text(typeDescription(for: file))
                            .foregroundStyle(.secondary)
                    }
                    .width(min: 90, ideal: 120, max: 170)
                    TableColumn("Size") { file in
                        Text(sizeDescription(for: file))
                            .foregroundStyle(.secondary)
                            .monospacedDigit()
                    }
                    .width(min: 64, ideal: 80, max: 110)
                    TableColumn("Modified") { file in
                        Text(dateDescription(for: file))
                            .foregroundStyle(.secondary)
                    }
                    .width(min: 120, ideal: 145, max: 180)
                }
                .tableStyle(.inset)
                .contextMenu(forSelectionType: DriveFile.ID.self) { ids in
                    if let id = ids.first, let file = model.files.first(where: { $0.id == id }) {
                        if isFolder(file) {
                            Button("Open") { open(file) }
                        } else {
                            Button("Download") { viewState.downloadFile = file }
                            if model.exportMIME(for: file) != nil {
                                Button("Export With Custom Format…") { viewState.exportFile = file }
                            }
                        }
                    }
                }

                if model.isLoading {
                    ProgressView("Loading folder…")
                        .padding(20)
                        .background(.regularMaterial, in: .rect(cornerRadius: 8))
                } else if model.files.isEmpty {
                    ContentUnavailableView(
                        "This folder is empty",
                        systemImage: "folder",
                        description: Text("Upload a file or refresh the folder.")
                    )
                }
            }
        }
        .navigationTitle(model.currentFolderName)
        .alert(
            viewState.downloadFile?.isDownloadable == true ? "Confirm Download" : "Download Unavailable",
            isPresented: Binding(
                get: { viewState.downloadFile != nil },
                set: { if !$0 { viewState.downloadFile = nil } }
            ),
            presenting: viewState.downloadFile
        ) { file in
            if file.isDownloadable {
                Button("Download") { model.download(file) }
                Button("Cancel", role: .cancel) { }
            } else {
                Button("OK", role: .cancel) { }
            }
        } message: { file in
            if file.isDownloadable {
                Text(file.isExportable
                    ? "Export \(file.name) as PDF to \(model.settings.downloadDirectoryPath)?"
                    : "Download \(file.name) to \(model.settings.downloadDirectoryPath)?")
            } else {
                Text("The gdirect CLI cannot download this Google Drive item (\(file.mimeType)). It has no supported downloadable content.")
            }
        }
        .sheet(item: $viewState.exportFile) { file in
            ExportView(file: file, model: model)
        }
        .sheet(isPresented: $viewState.showingPublicDownload) {
            PublicDownloadView(model: model)
        }
    }

    private var browserToolbar: some View {
        HStack(spacing: 8) {
            Button { Task { await model.goBack() } } label: {
                Image(systemName: "chevron.left")
            }
            .disabled(!model.canGoBack)
            .help("Back")
            Button { Task { await model.goForward() } } label: {
                Image(systemName: "chevron.right")
            }
            .disabled(!model.canGoForward)
            .help("Forward")
            Text(model.currentFolderName)
                .font(.headline)
                .lineLimit(1)
                .padding(.leading, 4)
            Spacer()
            if let file = selectedFile, !isFolder(file) {
                Button("Download") { viewState.downloadFile = file }
                if model.exportMIME(for: file) != nil {
                    Button("Export…") { viewState.exportFile = file }
                }
            } else if let file = selectedFile {
                Button("Open") { open(file) }
            }
            Button {
                viewState.showingPublicDownload = true
            } label: {
                Label("Public Link", systemImage: "link")
            }
            Button {
                if let url = AppKitPanels.file(title: "Upload to \(model.currentFolderName)", prompt: "Upload") {
                    model.upload(source: url)
                }
            } label: {
                Label("Upload", systemImage: "arrow.up.doc")
            }
            Button { Task { await model.refresh() } } label: {
                Image(systemName: "arrow.clockwise")
            }
            .disabled(model.isLoading)
            .help("Refresh")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func open(_ file: DriveFile) {
        guard isFolder(file) else { return }
        Task { await model.openFolder(id: file.id, name: file.name) }
    }

    private func isFolder(_ file: DriveFile) -> Bool {
        file.mimeType == "application/vnd.google-apps.folder"
    }

    private func icon(for file: DriveFile) -> String {
        if file.mimeType.hasPrefix("image/") { return "photo" }
        if file.mimeType == "application/pdf" { return "doc.richtext" }
        if file.mimeType.hasPrefix("video/") { return "film" }
        if file.mimeType.hasPrefix("audio/") { return "waveform" }
        return "doc"
    }

    private func typeDescription(for file: DriveFile) -> String {
        if isFolder(file) { return "Folder" }
        if file.mimeType.hasPrefix("application/vnd.google-apps.") { return "Google document" }
        return (file.name as NSString).pathExtension.uppercased().nonEmptyDescription ?? "File"
    }

    private func sizeDescription(for file: DriveFile) -> String {
        guard !isFolder(file), let size = file.size, let bytes = Int64(size) else { return "—" }
        return ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private func dateDescription(for file: DriveFile) -> String {
        guard let value = file.modifiedTime else { return "—" }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard let date = formatter.date(from: value) ?? ISO8601DateFormatter().date(from: value) else {
            return value
        }
        return date.formatted(date: .abbreviated, time: .shortened)
    }
}

@MainActor
private final class ExportViewState: ObservableObject {
    @Published var mimeType = "application/pdf"
}

private struct ExportView: View {
    let file: DriveFile
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var viewState = ExportViewState()

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Export \(file.name)")
                .font(.title3)
                .fontWeight(.semibold)
            Text("Google Workspace files default to PDF. Enter another MIME type supported by Google Drive to use a different format.")
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("Export MIME type", text: $viewState.mimeType)
                .textFieldStyle(.roundedBorder)
                .font(.system(.body, design: .monospaced))
            Text("The exported file will be saved in \(model.settings.downloadDirectoryPath)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Export") {
                    model.download(file, exportMIME: viewState.mimeType)
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
                .disabled(viewState.mimeType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 500)
        .onAppear { viewState.mimeType = model.exportMIME(for: file) ?? "application/pdf" }
    }
}

@MainActor
private final class PublicDownloadViewState: ObservableObject {
    @Published var link = ""
    @Published var name = "Public download"
    @Published var destination: URL?
}

private struct PublicDownloadView: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @StateObject private var viewState = PublicDownloadViewState()

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Download a Public Link")
                .font(.title3)
                .fontWeight(.semibold)
            TextField("Google Drive public URL", text: $viewState.link)
                .textFieldStyle(.roundedBorder)
            TextField("Display name", text: $viewState.name)
                .textFieldStyle(.roundedBorder)
            LabeledContent("Save to") {
                HStack {
                    Text(viewState.destination?.path ?? "Choose a destination")
                        .foregroundStyle(viewState.destination == nil ? .secondary : .primary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Button("Choose…") {
                        viewState.destination = AppKitPanels.save(
                            title: "Download Public File",
                            name: viewState.name,
                            directory: model.downloadDirectory
                        )
                    }
                }
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Download") {
                    if let destination = viewState.destination {
                        model.downloadPublicLink(link: viewState.link, name: viewState.name, destination: destination)
                        dismiss()
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(viewState.link.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || viewState.destination == nil)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}

private extension String {
    var nonEmptyDescription: String? { isEmpty ? nil : self }
}
