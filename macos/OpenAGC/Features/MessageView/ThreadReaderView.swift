import SwiftUI

/// The reader pane: subject, a remote-images banner when needed, and the
/// thread rendered in one locked-down web view.
struct ThreadReaderView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let reader = model.reader
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
                    inlineImages: reader.inlineImages)
            } else {
                Color.clear
            }
        }
        .task(id: model.selectedThreadID) { await reader.show(threadID: model.selectedThreadID) }
    }

    private func header(_ detail: ThreadDetail) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(detail.thread.subject.isEmpty ? "(no subject)" : detail.thread.subject)
                .font(TypeRole.title)
                .textSelection(.enabled)
                .lineLimit(2)
            Spacer()
            if detail.messages.count > 1 {
                Text("\(detail.messages.count) messages")
                    .font(.callout)
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
            Button("Load Images") { model.reader.loadRemoteImagesForThread() }
                .help("Show remote images in this conversation only (⇧⌘I)")
            Button("Always from Sender") { model.reader.alwaysLoadRemoteImagesFromSenders() }
                .help("Always show remote images from these senders (change in Settings › Privacy)")
        }
    }
}
