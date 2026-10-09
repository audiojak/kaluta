import Foundation

/// The tips shown over the Inbox, one at a time and in this order, each
/// until the user acts on it or puts it away (oagc-z87).
enum Tip: String, CaseIterable {
    /// Clean Up, for a large Inbox (spec §14.12); first, since it shows
    /// only when the Inbox is large.
    case cleanUp
    case categories
    case importantOnly
    case agent

    /// What the list's state says about each tip.
    struct Context {
        var inInbox: Bool
        var searching: Bool
        /// Gmail sorts this account's Inbox into categories.
        var categoriesAvailable: Bool
        var categoriesShown: Bool
        var importantOnly: Bool
        var agentShown: Bool
        /// The account has Gmail's Important marks (an agent mailbox does not).
        var importantAvailable = true
        /// Conversations in the Inbox (each has a message at least).
        var inboxCount = 0
        /// The account can be cleaned up (an imported mailbox cannot).
        var cleanUpAvailable = true
    }

    /// The Inbox size over which Clean Up is suggested: more than 1,000
    /// conversations, so more than 1,000 messages.
    static let cleanUpThreshold = 1_000

    /// The tip to show now, if any.
    static func next(dismissed: Set<String>, context c: Context) -> Tip? {
        guard c.inInbox, !c.searching else { return nil }
        return allCases.first { tip in
            guard !dismissed.contains(tip.rawValue) else { return false }
            switch tip {
            case .cleanUp: return c.cleanUpAvailable && c.inboxCount > cleanUpThreshold
            case .categories: return c.categoriesAvailable && c.categoriesShown
            case .importantOnly: return c.importantAvailable && !c.importantOnly
            case .agent: return !c.agentShown
            }
        }
    }

    var systemImage: String {
        switch self {
        case .cleanUp: "tray.full"
        case .categories: "rectangle.3.group"
        case .importantOnly: "chevron.right.2"
        case .agent: "sparkles"
        }
    }

    var title: String {
        switch self {
        case .cleanUp: "Clean Up"
        case .categories: "Categories"
        case .importantOnly: "Important Only"
        case .agent: "Ask the Agent"
        }
    }

    var text: String {
        switch self {
        case .cleanUp: "See your Inbox grouped by sender, list or size, and archive thousands of messages at once."
        case .categories: "Your Inbox is sorted into Primary, Promotions, Social and Updates, as in Gmail."
        case .importantOnly: "See only the mail Gmail marks Important, and leave the rest for later."
        case .agent: "Have the agent summarise a thread, draft a reply or file mail into labels."
        }
    }

    var action: String {
        switch self {
        case .categories: "Keep"
        case .cleanUp: "Open Clean Up"
        case .importantOnly, .agent: "Try"
        }
    }

    var actionHelp: String {
        switch self {
        case .cleanUp: "Open the Clean Up window for this account"
        case .categories: "Keep the category tabs above the Inbox"
        case .importantOnly: "Show only Important mail in the Inbox"
        case .agent: "Open the agent column (⌥⌘I)"
        }
    }

    var dismiss: String {
        switch self {
        case .categories: "Turn Off"
        case .cleanUp, .importantOnly, .agent: "Not Now"
        }
    }
}
