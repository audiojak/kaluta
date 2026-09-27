import SwiftUI

/// Sidebar footer: what sync is doing, in one line.
struct SyncStatusView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            switch model.syncDisplay {
            case let .syncing(pending):
                ProgressView().controlSize(.mini)
                Text(SyncStatusView.syncingText(pending: pending, transport: model.backfillTransport))
                    .help(model.backfillTransport == "imap"
                        ? "Downloading message bodies over IMAP (Settings › Accounts)" : "")
            case .offline:
                Image(systemName: "wifi.slash")
                Text("Offline")
            case .error:
                Image(systemName: "exclamationmark.triangle")
                Text("Sync paused")
            case .idle:
                if model.isArchive {
                    Image(systemName: "archivebox")
                    Text("Imported mailbox · cannot send")
                } else if model.needsReauthentication {
                    Image(systemName: "exclamationmark.triangle")
                    Text("Not syncing — sign in again")
                } else if let email = model.accountEmail, case .open = model.accountState, model.core?.currentAccountID != AppModel.demoAccountID {
                    Text(email).lineLimit(1).truncationMode(.middle)
                } else if case .open = model.accountState {
                    Text("Demo mailbox")
                }
            }
            Spacer(minLength: 0)
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    /// "Syncing over IMAP — 7,258 left".
    static func syncingText(pending: UInt32, transport: String?) -> String {
        let how = transport == "imap" ? "Syncing over IMAP" : "Syncing"
        return pending > 0 ? "\(how) — \(pending.formatted()) left" : "\(how)…"
    }

    /// Sync state as a phrase for the window subtitle, or nil when there is
    /// nothing to say (idle and healthy).
    @MainActor
    static func subtitle(for model: AppModel) -> String? {
        switch model.syncDisplay {
        case let .syncing(pending): return syncingText(pending: pending, transport: model.backfillTransport)
        case .offline: return "Offline"
        case .error: return "Sync paused"
        case .idle:
            if model.isArchive { return "Imported mailbox · cannot send" }
            if model.needsReauthentication { return "Not syncing — sign in again" }
            if model.openAccountID == AppModel.demoAccountID { return "Demo mailbox" }
            return nil
        }
    }
}
