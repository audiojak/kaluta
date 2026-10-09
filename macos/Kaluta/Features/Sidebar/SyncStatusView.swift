import SwiftUI

/// The sidebar's footer, as in Mail: while mail downloads, a thin
/// progress bar over "Downloading Messages" and what is left; while a
/// round for new mail takes a while, "Checking for New Mail"; while the
/// provider has asked sync to wait, how long for; otherwise nothing,
/// unless sync is offline or paused.
struct SyncStatusView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let until = model.syncPausedUntil, until > .now {
            // Counts down the provider's pause, a second at a time.
            TimelineView(.periodic(from: .now, by: 1)) { context in
                footer(pausedFor: until.timeIntervalSince(context.date))
            }
        } else {
            footer(pausedFor: nil)
        }
    }

    @ViewBuilder
    private func footer(pausedFor: TimeInterval?) -> some View {
        if let lines = Self.footer(for: model, pausedFor: pausedFor) {
            VStack(spacing: Space.xs) {
                switch model.syncDisplay {
                case .syncing:
                    ProgressView(value: model.syncProgress)
                        .progressViewStyle(.linear)
                        .controlSize(.mini)
                        .frame(maxWidth: 150)
                        .padding(.bottom, Space.hair)
                case .checking:
                    // How much is new is not known until the round ends.
                    ProgressView()
                        .progressViewStyle(.linear)
                        .controlSize(.mini)
                        .frame(maxWidth: 150)
                        .padding(.bottom, Space.hair)
                default:
                    EmptyView()
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
            .hoverHelp(hoverText(paused: pausedFor != nil))
        }
    }

    private func hoverText(paused: Bool) -> String {
        if paused {
            return "\(Self.sentence(model.pausingService)) limits how fast mail can be fetched. Kaluta waits as asked, then carries on more slowly; nothing is lost."
        }
        return model.transportNote.map { "Using the Gmail API because \($0.prefix(1).lowercased() + $0.dropFirst()). Kaluta tries IMAP again shortly; Window › Sync Debugger shows more." }
            ?? (model.backfillTransport == "imap" ? "Downloading over IMAP (Window › Sync Debugger)" : "Sync status")
    }

    /// The footer's two lines, or nil when there is nothing to say.
    @MainActor
    static func footer(for model: AppModel, pausedFor: TimeInterval? = nil) -> (title: String, detail: String?)? {
        footer(model.syncDisplay, transport: model.backfillTransport, note: model.transportNote,
               needsSignIn: model.needsReauthentication, pausedFor: pausedFor, service: model.pausingService)
    }

    /// `note`: why the Gmail API is doing IMAP's job, if it is.
    /// `pausedFor`: how much longer the provider asked sync to wait.
    static func footer(_ display: AppModel.SyncDisplay, transport: String?, note: String? = nil,
                       needsSignIn: Bool, pausedFor: TimeInterval? = nil,
                       service: String = "Gmail") -> (title: String, detail: String?)? {
        let pause = pausedFor.flatMap(pauseLine(_:)).map { "\(sentence(service)) asked to wait · resuming in \($0)" }
        switch display {
        case let .syncing(pending, headers):
            let how = note != nil ? "Downloading over the Gmail API"
                : transport == "imap" ? "Downloading over IMAP" : "Downloading Messages"
            var detail: String? = headers > 0 ? "headers for \(headers.formatted()) messages left"
                : pending > 0 ? "\(pending.formatted()) left" : nil
            if let note { detail = [note, detail].compactMap { $0 }.joined(separator: " · ") }
            return (how, pause ?? detail)
        case .checking:
            return ("Checking for New Mail", pause)
        case .offline:
            return ("Offline", "Changes are sent when you reconnect")
        case let .error(message):
            // The provider's words, so the pause can be understood and
            // reported (an HTTP status and what was wrong).
            return ("Sync Paused", message.map { "Trying again shortly · \($0)" } ?? "Trying again shortly")
        case .idle:
            if needsSignIn { return ("Not Syncing", "Sign in again in Settings › Accounts") }
            if let pause { return ("Waiting for \(service)", pause) }
            return note.map { ("Using the Gmail API", $0) }
        }
    }

    /// "the mail service" → "The mail service".
    static func sentence(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    /// "0:42", "1:05"; nil once the pause is over.
    static func pauseLine(_ seconds: TimeInterval) -> String? {
        let left = Int(seconds.rounded(.up))
        guard left > 0 else { return nil }
        return "\(left / 60):" + String(format: "%02d", left % 60)
    }
}
