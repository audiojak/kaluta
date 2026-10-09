import Foundation
import Observation

/// Sidebar data: system mailboxes then labels, with unread counts.
@MainActor
@Observable
final class MailboxStore {
    private(set) var mailboxes: [MailboxInfo] = []
    /// Label colors by label id, for the sidebar's tags.
    private(set) var labelColors: [String: String] = [:]
    private let core: CoreClient?

    init(core: CoreClient?) {
        self.core = core
    }

    /// Inbox, Starred, Important, Sent, Drafts, Archive, Spam, Trash (Gmail's order).
    var systemMailboxes: [MailboxInfo] { mailboxes.filter { $0.kind != .label } }
    /// Mail's Favorites: the mailboxes used most, at the top of the sidebar.
    static let favoriteKinds: [MailboxKind] = [.inbox, .starred, .sent]
    var favorites: [MailboxInfo] {
        Self.favoriteKinds.compactMap { kind in mailboxes.first { $0.kind == kind } }
    }
    /// The account's other mailboxes, in Mail's order, above its labels.
    var accountMailboxes: [MailboxInfo] {
        let order: [MailboxKind] = [.important, .drafts, .spam, .trash, .archive]
        return order.flatMap { kind in mailboxes.filter { $0.kind == kind } }
    }
    var labels: [MailboxInfo] { mailboxes.filter { $0.kind == .label } }

    func reload() async {
        guard let core, let fresh = try? await core.mailboxes() else { return }
        if fresh != mailboxes { mailboxes = fresh }
        let colors = ((try? await core.labels()) ?? []).reduce(into: [String: String]()) { acc, l in
            if let bg = l.backgroundColor { acc[l.id] = bg }
        }
        if colors != labelColors { labelColors = colors }
    }
}
