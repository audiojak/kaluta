import Foundation

/// The tips shown over a list, one at a time and in this order, each
/// until the user acts on it or puts it away (oagc-z87): three over the
/// Inbox, and one each explaining the Writing Guide and Facts pages
/// (spec §14.9, §14.11), so those pages carry no permanent narration.
enum Tip: String, CaseIterable {
    case categories
    case importantOnly
    case agent
    case guide
    case facts

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
        var inGuide = false
        var inFacts = false
    }

    /// The tip to show now, if any.
    static func next(dismissed: Set<String>, context c: Context) -> Tip? {
        guard !c.searching else { return nil }
        return allCases.first { tip in
            guard !dismissed.contains(tip.rawValue) else { return false }
            switch tip {
            case .categories: return c.inInbox && c.categoriesAvailable && c.categoriesShown
            case .importantOnly: return c.inInbox && c.importantAvailable && !c.importantOnly
            case .agent: return c.inInbox && !c.agentShown
            case .guide: return c.inGuide
            case .facts: return c.inFacts
            }
        }
    }

    var systemImage: String {
        switch self {
        case .categories: "rectangle.3.group"
        case .importantOnly: "chevron.right.2"
        case .agent: "sparkles"
        case .guide: "text.book.closed"
        case .facts: "person.text.rectangle"
        }
    }

    var title: String {
        switch self {
        case .categories: "Categories"
        case .importantOnly: "Important Only"
        case .agent: "Ask the Agent"
        case .guide: "Your Writing Guide"
        case .facts: "Your Facts"
        }
    }

    var text: String {
        switch self {
        case .categories: "Your Inbox is sorted into Primary, Promotions, Social and Updates, as in Gmail."
        case .importantOnly: "See only the mail Gmail marks Important, and leave the rest for later."
        case .agent: "Have the agent summarise a thread, draft a reply or file mail into labels."
        case .guide:
            "How you write, as rules AI drafts follow. Learning reads your sent mail, and each day's review compares AI drafts with what you sent. What they propose waits at the top until you decide; every decision can be undone."
        case .facts:
            "Things AI drafts may use about you: your role, time zone, calendar link, the people you mention. Each day's review finds new ones in the mail you send; they wait at the top until you decide."
        }
    }

    var action: String {
        switch self {
        case .categories: "Keep"
        case .importantOnly, .agent: "Try"
        case .guide, .facts: "Learning Settings…"
        }
    }

    var actionHelp: String {
        switch self {
        case .categories: "Keep the category tabs above the Inbox"
        case .importantOnly: "Show only Important mail in the Inbox"
        case .agent: "Open the agent column (⌥⌘I)"
        case .guide: "The daily review, and how long AI drafts are kept"
        case .facts: "Where facts are learned from, and the daily review"
        }
    }

    var dismiss: String {
        switch self {
        case .categories: "Turn Off"
        case .importantOnly, .agent: "Not Now"
        case .guide, .facts: "Got It"
        }
    }
}
