import AppKit

/// What a snippet puts into a message (spec §14.5 amendment 2026-11-18).
enum SnippetText {
    /// The one placeholder: the first recipient's first name.
    static let firstName = "{first name}"

    /// The name to put for `{first name}`: the first word of the first
    /// recipient's name, else their address before the @, else nothing.
    static func firstName(of recipient: AddressInfo?) -> String {
        guard let recipient else { return "" }
        if let word = recipient.name?.split(separator: " ").first, !word.isEmpty { return String(word) }
        return String(recipient.email.split(separator: "@").first ?? "")
    }

    /// A snippet's HTML with its placeholder filled in.
    static func filled(_ html: String, recipient: AddressInfo?) -> String {
        html.replacingOccurrences(of: firstName, with: ComposerHTML.escape(firstName(of: recipient)))
    }

    /// Plain text as the editor's HTML: a line a div.
    static func html(fromPlain text: String) -> String {
        text.replacingOccurrences(of: "\r\n", with: "\n").split(separator: "\n", omittingEmptySubsequences: false)
            .map { $0.isEmpty ? "<div><br></div>" : "<div>\(ComposerHTML.escape(String($0)))</div>" }
            .joined()
    }

    /// A name for text saved as a snippet: its first line, shortened.
    static func name(for text: String) -> String {
        let line = text.split(whereSeparator: \.isNewline).first.map(String.init)?
            .trimmingCharacters(in: .whitespaces) ?? ""
        guard !line.isEmpty else { return "Snippet" }
        return line.count > 40 ? String(line.prefix(39)) + "…" : line
    }

    /// The snippets whose names match what is typed after `/`, best first.
    static func matching(_ snippets: [SnippetInfo], _ query: String) -> [SnippetInfo] {
        let query = query.trimmingCharacters(in: .whitespaces)
        guard !query.isEmpty else { return snippets }
        return snippets.enumerated()
            .compactMap { i, s in PaletteModel.rank(s.name, query).map { (rank: $0, index: i, snippet: s) } }
            .sorted { ($0.rank, $0.index) < ($1.rank, $1.index) }
            .map(\.snippet)
    }
}
