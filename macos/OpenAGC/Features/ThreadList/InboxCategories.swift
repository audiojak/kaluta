import Foundation

/// Gmail's categories as Inbox tabs (spec §14.3 amendment 2026-09-28,
/// categories): which tabs show, which one is active, and how each one
/// reads. The core counts; this decides.
enum InboxCategories {
    /// Primary: `CATEGORY_PERSONAL`, or Inbox mail with no other category.
    static let primary = "CATEGORY_PERSONAL"

    static func title(_ id: String) -> String {
        switch id {
        case primary: "Primary"
        case "CATEGORY_PROMOTIONS": "Promotions"
        case "CATEGORY_SOCIAL": "Social"
        case "CATEGORY_UPDATES": "Updates"
        case "CATEGORY_FORUMS": "Forums"
        default: id.replacingOccurrences(of: "CATEGORY_", with: "").capitalized
        }
    }

    static func symbol(_ id: String) -> String {
        switch id {
        case primary: "person"
        case "CATEGORY_PROMOTIONS": "megaphone"
        case "CATEGORY_SOCIAL": "bubble.left.and.bubble.right"
        case "CATEGORY_UPDATES": "info.bubble"
        case "CATEGORY_FORUMS": "person.3"
        default: "tray"
        }
    }

    /// Whether the account uses categories at all: some Inbox mail sits in
    /// a category other than Primary.
    static func inUse(_ all: [InboxCategory]) -> Bool {
        all.contains { $0.id != primary && $0.totalCount > 0 }
    }

    /// The tabs to show: Primary, then every other category with mail;
    /// none when no other category has mail (one tab is no choice).
    static func visible(_ all: [InboxCategory]) -> [InboxCategory] {
        guard inUse(all) else { return [] }
        return all.filter { $0.id == primary || $0.totalCount > 0 }
    }

    /// The tab the list narrows to: the chosen one while it has mail,
    /// otherwise Primary; nil when there are no tabs.
    static func active(chosen: String?, visible: [InboxCategory]) -> String? {
        guard !visible.isEmpty else { return nil }
        if let chosen, visible.contains(where: { $0.id == chosen }) { return chosen }
        return primary
    }

    /// The tab a thread belongs in, from its labels: its first category
    /// other than Primary in tab order, else Primary. Matches the core's
    /// counting.
    static func category(of labelIDs: [String], in order: [String]) -> String {
        order.first { $0 != primary && labelIDs.contains($0) } ?? primary
    }
}
