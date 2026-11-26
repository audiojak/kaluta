import SwiftUI

/// Completing a tag in the search field (spec §8): typing `tag:` opens a
/// menu of the account's tags, narrowed as the user types. Tags that some
/// contact has come first, each with how many, under Relationship and
/// Role; the rest follow. Built locally from the tag list: no agent.
enum TagCompletion {
    struct Suggestion: Identifiable, Equatable {
        let tag: ContactTagInfo
        /// The whole search text once this tag is chosen.
        let query: String
        var id: String { tag.id }
        /// "Customer, 14 contacts", as VoiceOver reads the row.
        var spoken: String {
            tag.contacts == 0 ? "\(tag.name), no contacts" : "\(tag.name), \(tag.contacts) contact\(tag.contacts == 1 ? "" : "s")"
        }
    }

    struct Group: Identifiable, Equatable {
        let title: String
        let suggestions: [Suggestion]
        var id: String { title }
    }

    /// What is being typed after `tag:` at the end of the search text:
    /// the text before it (kept as it is) and the letters typed so far.
    /// `nil` when the text does not end in a tag being typed.
    static func partial(in text: String) -> (before: String, typed: String)? {
        // The last token: from the last space that is not inside quotes.
        var start = text.startIndex
        var quoted = false
        for i in text.indices {
            let c = text[i]
            if c == "\"" { quoted.toggle() }
            if c.isWhitespace || c == "(", !quoted { start = text.index(after: i) }
        }
        var token = text[start...]
        var before = String(text[..<start])
        if token.hasPrefix("-") {
            before += "-"
            token = token.dropFirst()
        }
        guard token.lowercased().hasPrefix("tag:") else { return nil }
        var typed = String(token.dropFirst(4))
        if typed.hasPrefix("\"") { typed.removeFirst() }
        // A closed quote is a finished tag, not one being typed.
        if typed.contains("\"") { return nil }
        return (before, typed)
    }

    /// A tag as the search field takes it: quoted when it has a space.
    static func term(_ name: String) -> String {
        name.contains(where: \.isWhitespace) ? "tag:\"\(name)\"" : "tag:\(name)"
    }

    /// The menu for the search text: tags in use first, by set, then the rest.
    static func groups(for text: String, tags: [ContactTagInfo]) -> [Group] {
        guard let (before, typed) = partial(in: text) else { return [] }
        let matching = tags.filter { ContactsStore.matches($0.name, typed: typed) }
        // Typed in full already: nothing left to complete.
        if matching.count == 1, let only = matching.first, only.name.lowercased() == typed.lowercased().replacingOccurrences(of: "-", with: " ") {
            return []
        }
        let make = { (tag: ContactTagInfo) in Suggestion(tag: tag, query: before + term(tag.name) + " ") }
        var out: [Group] = []
        for set in [ContactTagSet.relationship, .role] {
            let used = matching.filter { $0.set == set && $0.contacts > 0 }.sorted { $0.contacts > $1.contacts }
            if !used.isEmpty { out.append(Group(title: set.title, suggestions: used.map(make))) }
        }
        let unused = matching.filter { $0.contacts == 0 }
        if !unused.isEmpty { out.append(Group(title: "On no contact yet", suggestions: Array(unused.prefix(Self.unusedShown)).map(make))) }
        return out
    }

    /// Tags no contact has, shown after those in use: enough to find one
    /// by typing, not the whole list at once.
    static let unusedShown = 12
}

/// The menu under the search field while a tag is being typed.
struct TagSuggestions: View {
    let groups: [TagCompletion.Group]

    var body: some View {
        ForEach(groups) { group in
            Section(group.title) {
                ForEach(group.suggestions) { s in
                    HStack {
                        Label(s.tag.name, systemImage: "tag")
                        Spacer()
                        if s.tag.contacts > 0 {
                            Text("\(s.tag.contacts)").foregroundStyle(.secondary)
                        }
                    }
                    .foregroundStyle(s.tag.contacts > 0 ? .primary : .secondary)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(s.spoken)
                    .searchCompletion(s.query)
                }
            }
        }
    }
}
