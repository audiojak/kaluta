import Foundation

/// One example prompt (spec §14.6b). Text ending in "…" needs the user's
/// words, so choosing it fills the field instead of sending.
struct AgentSuggestion: Hashable, Identifiable {
    let text: String
    var id: String { text }
    var fillsOnly: Bool { text.hasSuffix("…") }
    /// What goes in the prompt field when it only fills: the text without
    /// the ellipsis, ready for the user's words.
    var fillText: String { fillsOnly ? String(text.dropLast()).trimmingCharacters(in: .whitespaces) + " " : text }
}

/// What the user is looking at, which decides the suggestions.
struct SuggestionContext: Equatable {
    var selectedCount = 0
    var mailboxID: String?
    var unreadInMailbox = 0
    var searchQuery: String?
    /// The one selected thread has a file attached.
    var hasAttachment = false
    /// False in an imported mailbox: it cannot draft (spec §7.8).
    var canDraft = true
}

/// Suggestions are made here from state alone: no model call, no network
/// (spec §14.6b). Every one maps to tools the agent has (§10.2); sending is
/// never suggested, only drafting, since a send needs approval (§10.4).
enum AgentSuggestions {
    static let chipLimit = 4

    struct Group: Identifiable, Equatable {
        let title: String
        let symbol: String
        let examples: [AgentSuggestion]
        var id: String { title }
    }

    /// The agent column's empty state: what an agent can do, in groups.
    static func groups(canDraft: Bool) -> [Group] {
        var groups = [
            Group(title: "Find and summarise", symbol: "magnifyingglass", examples: [
                "What needs a reply today?",
                "Summarise the thread with …",
                "Find the latest invoice from …",
                "What did I agree to last week?",
            ].map(AgentSuggestion.init)),
        ]
        if canDraft {
            groups.append(Group(title: "Draft for you", symbol: "square.and.pencil", examples: [
                "Draft a reply that …",
                "Draft a polite follow-up to …",
                "Draft an email to … about …",
            ].map(AgentSuggestion.init)))
        }
        groups.append(Group(title: "Tidy up", symbol: "tray.full", examples: [
            "Archive newsletters older than a week",
            "Label this month’s receipts as Receipts",
            "Mark everything from notifications as read",
        ].map(AgentSuggestion.init)))
        return groups
    }

    /// Up to four chips for this context: prompts the user sent before
    /// that fit it (most recent first), then examples for the context. The
    /// most relevant example stays first; the rest rotate daily so the
    /// list does not look static.
    static func chips(for context: SuggestionContext, recent: [String] = [], day: Int = 0) -> [AgentSuggestion] {
        let examples = contextual(context)
        let fitting = recent
            .filter { fits($0, context) && allowed($0, context) }
            .prefix(2)
            .map(AgentSuggestion.init)
        var out: [AgentSuggestion] = []
        for suggestion in fitting where !out.contains(suggestion) { out.append(suggestion) }
        let rotated = examples.isEmpty ? [] : [examples[0]] + rotate(Array(examples.dropFirst()), by: day)
        for suggestion in rotated where out.count < chipLimit && !out.contains(suggestion) {
            out.append(suggestion)
        }
        return Array(out.prefix(chipLimit))
    }

    /// Examples for the context, most relevant first.
    static func contextual(_ c: SuggestionContext) -> [AgentSuggestion] {
        var texts: [String]
        if c.selectedCount > 1 {
            texts = ["Which of these need a reply?", "Summarise these threads", "Archive these (reversible)",
                     "Label these …"]
        } else if c.selectedCount == 1 {
            texts = ["Summarise this thread", "What is being asked of me here?"]
            if c.hasAttachment { texts.insert("What does the attachment say?", at: 1) }
            if c.canDraft { texts.append("Draft a reply that …") }
            texts.append("Add the label …")
        } else if let query = c.searchQuery, !query.isEmpty {
            texts = ["Summarise these results", "Find the one that mentions …", "Which of these need a reply?"]
        } else if c.unreadInMailbox > 0 {
            texts = ["What's new since yesterday?", "Which unread messages need a reply?", "Summarise my unread mail",
                     "What needs a reply today?"]
        } else {
            texts = ["What needs a reply today?", "Find the latest invoice from …", "What did I agree to last week?"]
            if c.canDraft { texts.append("Draft an email to … about …") }
        }
        return texts.map(AgentSuggestion.init)
    }

    /// Whether a prompt sent before suits what is on screen now: one about
    /// "this" thread needs one selected, "these" several, and a general one
    /// needs none.
    static func fits(_ prompt: String, _ c: SuggestionContext) -> Bool {
        let words = Set(prompt.lowercased().split { !$0.isLetter }.map(String.init))
        if words.contains("these") || words.contains("results") { return c.selectedCount > 1 || c.searchQuery != nil }
        if words.contains("this") || words.contains("here") { return c.selectedCount == 1 }
        return c.selectedCount == 0
    }

    /// Honesty: nothing that would send, and no drafting where it cannot.
    static func allowed(_ prompt: String, _ c: SuggestionContext) -> Bool {
        let lower = prompt.lowercased()
        if lower.hasPrefix("send") || lower.contains(" send ") { return false }
        if !c.canDraft, lower.contains("draft") || lower.contains("reply to") || lower.contains("forward") {
            return false
        }
        return true
    }

    private static func rotate(_ items: [AgentSuggestion], by day: Int) -> [AgentSuggestion] {
        guard !items.isEmpty else { return items }
        let shift = ((day % items.count) + items.count) % items.count
        return Array(items[shift...] + items[..<shift])
    }

    /// Today's rotation seed.
    static func today(_ date: Date = Date()) -> Int {
        Calendar.current.ordinality(of: .day, in: .era, for: date) ?? 0
    }
}

/// The last prompts the user sent, per account (spec §14.6b Learning).
struct RecentPrompts {
    static let limit = 20
    let defaults: UserDefaults

    static func key(_ accountID: String) -> String { "agentRecentPrompts.\(accountID)" }

    func prompts(for accountID: String) -> [String] {
        defaults.stringArray(forKey: Self.key(accountID)) ?? []
    }

    func record(_ prompt: String, for accountID: String) {
        let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        var list = prompts(for: accountID).filter { $0.caseInsensitiveCompare(trimmed) != .orderedSame }
        list.insert(trimmed, at: 0)
        defaults.set(Array(list.prefix(Self.limit)), forKey: Self.key(accountID))
    }

    func clear(_ accountIDs: [String]) {
        for id in accountIDs { defaults.removeObject(forKey: Self.key(id)) }
    }
}
