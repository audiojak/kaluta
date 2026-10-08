import Foundation

/// The tips shown over the Inbox, one at a time and in this order, each
/// until the user acts on it or puts it away (oagc-z87).
enum Tip: String, CaseIterable {
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
    }

    /// The tip to show now, if any.
    static func next(dismissed: Set<String>, context c: Context) -> Tip? {
        guard c.inInbox, !c.searching else { return nil }
        return allCases.first { tip in
            guard !dismissed.contains(tip.rawValue) else { return false }
            switch tip {
            case .categories: return c.categoriesAvailable && c.categoriesShown
            case .importantOnly: return c.importantAvailable && !c.importantOnly
            case .agent: return !c.agentShown
            }
        }
    }

    var systemImage: String {
        switch self {
        case .categories: "rectangle.3.group"
        case .importantOnly: "chevron.right.2"
        case .agent: "sparkles"
        }
    }

    var title: String {
        switch self {
        case .categories: "Categories"
        case .importantOnly: "Important Only"
        case .agent: "Ask the Agent"
        }
    }

    var text: String {
        switch self {
        case .categories: "Your Inbox is sorted into Primary, Promotions, Social and Updates, as in Gmail."
        case .importantOnly: "See only the mail Gmail marks Important, and leave the rest for later."
        case .agent: "Have the agent summarise a thread, draft a reply or file mail into labels."
        }
    }

    var action: String {
        switch self {
        case .categories: "Keep"
        case .importantOnly, .agent: "Try"
        }
    }

    var actionHelp: String {
        switch self {
        case .categories: "Keep the category tabs above the Inbox"
        case .importantOnly: "Show only Important mail in the Inbox"
        case .agent: "Open the agent column (⌥⌘I)"
        }
    }

    var dismiss: String {
        switch self {
        case .categories: "Turn Off"
        case .importantOnly, .agent: "Not Now"
        }
    }
}
