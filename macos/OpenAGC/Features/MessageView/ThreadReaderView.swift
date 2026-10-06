import SwiftUI

/// The reader pane: subject, a remote-images banner when needed, and the
/// thread rendered in one locked-down web view. In the main window it shows
/// the selection; a thread window passes its own store and thread.
struct ThreadReaderView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    /// A window of its own: its store and thread (nil: the selection).
    var store: ReaderStore?
    var threadID: String?

    private var reader: ReaderStore { store ?? model.reader }
    private var shownID: String? { store == nil ? model.selectedThreadID : threadID }

    var body: some View {
        let reader = reader
        VStack(spacing: 0) {
            if let detail = reader.detail {
                header(detail)
                if !reader.attachments.isEmpty {
                    AttachmentStrip(attachments: reader.attachments)
                }
                if reader.hasRemoteImages && !reader.allowsRemoteImages {
                    remoteImagesBanner
                }
                MessageWebView(
                    html: EmailDocument.thread(reader.documentMessages, isDark: colorScheme == .dark),
                    allowRemoteImages: reader.allowsRemoteImages,
                    // The main window's reader is a stop in its Tab loop.
                    onCreated: store == nil ? { [focus = model.focus] in focus.register($0, as: .reader) } : nil,
                    inlineImages: reader.inlineImages)
            } else {
                Color.clear
            }
        }
        .task(id: shownID) {
            await reader.show(threadID: shownID)
            // Looking at it marks it read (spec §14.4).
            if let id = shownID, let detail = reader.detail, detail.thread.id == id {
                model.threadShown(id, hasUnread: detail.messages.contains { !$0.isRead }, inMainWindow: store == nil)
            }
        }
    }

    private func header(_ detail: ThreadDetail) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(detail.thread.subject.isEmpty ? "(no subject)" : detail.thread.subject)
                .font(TypeRole.title)
                .textSelection(.enabled)
                .lineLimit(2)
            Spacer()
            if detail.messages.contains(where: \.isDraft) {
                Button("Edit Draft", systemImage: "pencil") { model.editDraft(threadID: detail.thread.id) }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.small)
                    .hoverHelp("Open this draft to edit and send it (Return or double-click in Drafts)")
            }
            if detail.messages.count > 1 {
                Text("\(detail.messages.count) messages")
                    .font(TypeRole.meta)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, Space.xxl)
        .padding(.top, Space.xl)
        .padding(.bottom, Space.s)
    }

    private var remoteImagesBanner: some View {
        Banner("Remote images are hidden to protect your privacy.", systemImage: "photo.badge.exclamationmark",
               intent: .neutral, inset: Space.xxl) {
            Button("Load Images") { reader.loadRemoteImagesForThread() }
                .hoverHelp("Show remote images in this conversation only (⇧⌘I)")
            Button("Always from Sender") { reader.alwaysLoadRemoteImagesFromSenders() }
                .hoverHelp("Always show remote images from these senders (change in Settings › Privacy)")
        }
    }
}
