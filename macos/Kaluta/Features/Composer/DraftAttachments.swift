import os
import SwiftUI
import UniformTypeIdentifiers

/// The files a draft attaches, copied into the app's data directory so
/// the draft keeps them even if the originals move. The composer and a
/// workflow's reply (spec §14.13) attach the same way: a file chooser,
/// or files dropped on the email.
struct DraftAttachments {
    let directory: URL
    private static let logger = Logger(subsystem: "org.kaluta.Kaluta", category: "composer")

    /// `directory` nil: beside the app's data, as every draft's copies are.
    init(directory: URL? = nil) {
        self.directory = directory
            ?? ((try? CoreClient.defaultDataDirectory()) ?? FileManager.default.temporaryDirectory)
                .appending(path: "DraftAttachments", directoryHint: .isDirectory)
    }

    /// Copies of these files, each in its own folder, as attachments.
    func copy(_ urls: [URL]) -> [DraftAttachmentInfo] {
        urls.compactMap { url in
            do {
                let folder = directory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
                try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                let copy = folder.appending(path: url.lastPathComponent)
                try FileManager.default.copyItem(at: url, to: copy)
                let size = (try? copy.resourceValues(forKeys: [.fileSizeKey]).fileSize).map(UInt64.init) ?? 0
                let type = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
                return DraftAttachmentInfo(path: copy.path, filename: url.lastPathComponent, mimeType: type, size: size)
            } catch {
                Self.logger.error("attach failed: \(error.localizedDescription, privacy: .private)")
                return nil
            }
        }
    }

    /// Deletes the copy; only ever our own copies.
    func remove(_ attachment: DraftAttachmentInfo) {
        let url = URL(filePath: attachment.path)
        guard url.path.hasPrefix(directory.path) else { return }
        try? FileManager.default.removeItem(at: url.deletingLastPathComponent())
    }
}

extension View {
    /// Files come in two ways: the chooser (`choosing` true opens it) and
    /// a drop onto the view. Each file reaches `attach` once.
    func attachingFiles(choosing: Binding<Bool>, attach: @escaping ([URL]) -> Void) -> some View {
        fileImporter(isPresented: choosing, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case let .success(urls) = result else { return }
            let scoped = urls.map { ($0, $0.startAccessingSecurityScopedResource()) }
            attach(urls)
            for (url, started) in scoped where started { url.stopAccessingSecurityScopedResource() }
        }
        .dropDestination(for: URL.self) { urls, _ in
            let files = urls.filter(\.isFileURL)
            attach(files)
            return !files.isEmpty
        }
    }
}

/// A draft's attachments as chips in a row: name, size, and Remove.
struct DraftAttachmentChips: View {
    let attachments: [DraftAttachmentInfo]
    let remove: (DraftAttachmentInfo) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: Space.m) {
                ForEach(attachments, id: \.path) { attachment in
                    HStack(spacing: Space.s) {
                        Image(systemName: "doc")
                        Text(attachment.filename).lineLimit(1)
                        Text(ByteCountFormatter.string(fromByteCount: Int64(attachment.size), countStyle: .file))
                            .foregroundStyle(.secondary)
                        Button("Remove", systemImage: "xmark.circle.fill") { remove(attachment) }
                            .hoverHelp("Remove \(attachment.filename) from the message")
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                    }
                    .font(TypeRole.meta)
                    .padding(.horizontal, Space.m)
                    .padding(.vertical, Space.xs)
                    .background(.quaternary, in: .capsule)
                }
            }
        }
    }
}
