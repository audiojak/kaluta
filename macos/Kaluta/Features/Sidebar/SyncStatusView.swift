import SwiftUI

/// The sidebar's footer, as in Mail: while mail downloads, a thin
/// progress bar over "Downloading Messages" and what is left; otherwise
/// nothing, unless sync is offline or paused.
struct SyncStatusView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let lines = Self.footer(for: model) {
            VStack(spacing: Space.xs) {
                if case .syncing = model.syncDisplay {
                    ProgressView(value: model.syncProgress)
                        .progressViewStyle(.linear)
                        .controlSize(.mini)
                        .frame(maxWidth: 150)
                        .padding(.bottom, Space.hair)
                }
                Text(lines.title)
                    .font(TypeRole.caption.weight(.medium))
                if let detail = lines.detail {
                    Text(detail)
                        .font(TypeRole.fine)
                        .foregroundStyle(.secondary)
                }
            }
            .lineLimit(1)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
            .accessibilityElement(children: .combine)
            .hoverHelp(model.transportNote.map { "Using the Gmail API because \($0.prefix(1).lowercased() + $0.dropFirst()). Kaluta tries IMAP again shortly; Window › Sync Debugger shows more." }
                ?? (model.backfillTransport == "imap" ? "Downloading over IMAP (Window › Sync Debugger)" : "Sync status"))
        }
    }

    /// The footer's two lines, or nil when there is nothing to say.
    @MainActor
    static func footer(for model: AppModel) -> (title: String, detail: String?)? {
        footer(model.syncDisplay, transport: model.backfillTransport, note: model.transportNote,
               needsSignIn: model.needsReauthentication)
    }

    /// `note`: why the Gmail API is doing IMAP's job, if it is.
    static func footer(_ display: AppModel.SyncDisplay, transport: String?, note: String? = nil,
                       needsSignIn: Bool) -> (title: String, detail: String?)? {
        switch display {
        case let .syncing(pending, headers):
            let how = note != nil ? "Downloading over the Gmail API"
                : transport == "imap" ? "Downloading over IMAP" : "Downloading Messages"
            var detail: String? = headers > 0 ? "headers for \(headers.formatted()) messages left"
                : pending > 0 ? "\(pending.formatted()) left" : nil
            if let note { detail = [note, detail].compactMap { $0 }.joined(separator: " · ") }
            return (how, detail)
        case .offline:
            return ("Offline", "Changes are sent when you reconnect")
        case let .error(message):
            // The provider's words, so the pause can be understood and
            // reported (an HTTP status and what was wrong).
            return ("Sync Paused", message.map { "Trying again shortly · \($0)" } ?? "Trying again shortly")
        case .idle:
            if needsSignIn { return ("Not Syncing", "Sign in again in Settings › Accounts") }
            return note.map { ("Using the Gmail API", $0) }
        }
    }
}
