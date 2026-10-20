import SwiftUI

/// The reader pane: subject, a remote-images banner when needed, and the
/// thread rendered in one locked-down web view. In the main window it shows
/// the selection; a thread window passes its own store and thread.
struct ThreadReaderView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme
    /// An attachment clicked on its message's card, in Quick Look.
    @State private var previewURL: URL?
    /// Another conversation has been loading a moment: the one still on
    /// screen is dimmed, not passed off as the selection.
    @State private var showsLoading = false
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
                    // By the email each came with (each card lists its own too).
                    AttachmentStrip(groups: reader.attachmentGroups)
                }
                if reader.hasRemoteImages && !reader.allowsRemoteImages {
                    remoteImagesBanner
                }
                MessageWebView(
                    html: EmailDocument.thread(reader.documentMessages, isDark: colorScheme == .dark,
                                               actions: !model.isArchive),
                    allowRemoteImages: reader.allowsRemoteImages,
                    // The main window's reader is a stop in its Tab loop.
                    onCreated: store == nil ? { [focus = model.focus] in focus.register($0, as: .reader) } : nil,
                    inlineImages: reader.inlineImages,
                    onAttachment: { id in
                        Task { previewURL = try? await model.core?.attachmentFile(id).url }
                    },
                    // Any message answered, not only the latest; in the task
                    // list the reply answers the task, as the toolbar's does.
                    onMessageAction: { action, id in
                        let task = store == nil ? model.answeringTaskID : nil
                        switch action {
                        case .reply: model.compose(.reply(messageID: id, all: false, task: task))
                        case .replyAll: model.compose(.reply(messageID: id, all: true, task: task))
                        case .forward: model.compose(.forward(messageID: id, task: task))
                        }
                    },
                    // A click makes a message the one ⌘R, ⇧⌘R and ⇧⌘F answer.
                    currentMessageID: reader.currentMessageID,
                    onCurrentMessage: { id in reader.currentMessageID = id })
                    .quickLookPreview($previewURL)
            } else {
                Color.clear
            }
        }
        .opacity(stale && showsLoading ? 0.35 : 1)
        .overlay {
            if stale && showsLoading { ProgressView().controlSize(.small) }
        }
        .task(id: shownID) {
            // Quick loads swap without a flash; a slow one dims the last.
            showsLoading = false
            let pending = Task {
                try? await Task.sleep(for: .milliseconds(200))
                if !Task.isCancelled { showsLoading = true }
            }
            defer { pending.cancel() }
            await reader.show(threadID: shownID)
            // Looking at it marks it read (spec §14.4).
            if let id = shownID, let detail = reader.detail, detail.thread.id == id {
                model.threadShown(id, hasUnread: detail.messages.contains { !$0.isRead }, inMainWindow: store == nil)
            }
        }
    }

    /// The reader still holds another conversation than the one selected.
    private var stale: Bool { shownID != nil && reader.detail?.thread.id != shownID }

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
            Button { reader.loadRemoteImagesForThread() } label: {
                HStack(spacing: Space.s) {
                    Text("Load Images")
                    Text("⇧⌘I").foregroundStyle(.secondary)
                }
            }
            .hoverHelp("Show remote images in this conversation only (⇧⌘I)")
            Button("Always from Sender") { reader.alwaysLoadRemoteImagesFromSenders() }
                .hoverHelp("Always show remote images from these senders (change in Settings › Privacy)")
        }
    }
}
