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
                    .font(.caption.weight(.medium))
                if let detail = lines.detail {
                    Text(detail)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .lineLimit(1)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
            .accessibilityElement(children: .combine)
            .hoverHelp(model.backfillTransport == "imap" ? "Downloading over IMAP (Settings › Accounts)" : "")
        }
    }

    /// The footer's two lines, or nil when there is nothing to say.
    @MainActor
    static func footer(for model: AppModel) -> (title: String, detail: String?)? {
        footer(model.syncDisplay, transport: model.backfillTransport, needsSignIn: model.needsReauthentication)
    }

    static func footer(_ display: AppModel.SyncDisplay, transport: String?,
                       needsSignIn: Bool) -> (title: String, detail: String?)? {
        switch display {
        case let .syncing(pending, headers):
            let how = transport == "imap" ? "Downloading over IMAP" : "Downloading Messages"
            if headers > 0 { return (how, "headers for \(headers.formatted()) messages left") }
            return (how, pending > 0 ? "\(pending.formatted()) left" : nil)
        case .offline:
            return ("Offline", "Changes are sent when you reconnect")
        case .error:
            return ("Sync Paused", "Trying again shortly")
        case .idle:
            return needsSignIn ? ("Not Syncing", "Sign in again in Settings › Accounts") : nil
        }
    }
}
