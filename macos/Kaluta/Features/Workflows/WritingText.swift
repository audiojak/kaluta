import SwiftUI

/// The questions in an email and the starters drawn from them, for *What
/// do you want to say?* (spec §14.13). Pure.
enum WritingText {
    /// The sentences that ask something, in order: each ends in "?".
    static func questions(in text: String) -> [String] {
        var found: [String] = []
        text.enumerateSubstrings(in: text.startIndex..., options: .bySentences) { sentence, _, _, _ in
            guard let sentence else { return }
            let s = sentence.trimmingCharacters(in: .whitespacesAndNewlines)
            // A greeting line before the question is not part of it.
            let line = s.split(separator: "\n").last.map(String.init) ?? s
            if line.hasSuffix("?"), line.count > 3, !found.contains(line) { found.append(line) }
        }
        return found
    }

    /// Short ways to begin an answer: a choice the question offers ("Tuesday,
    /// or Thursday?" gives "Tuesday" and "Thursday"), else yes and no for
    /// a question that takes them. At most four, each once.
    static func starters(for questions: [String]) -> [String] {
        var out: [String] = []
        func add(_ s: String) {
            let s = s.trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
            guard !s.isEmpty, s.count <= 40, !out.contains(where: { $0.caseInsensitiveCompare(s) == .orderedSame }) else { return }
            out.append(s)
        }
        for q in questions {
            let body = q.trimmingCharacters(in: CharacterSet(charactersIn: "?").union(.whitespaces))
            if let choices = choices(in: body) {
                choices.forEach(add)
            } else if yesNo(body) {
                add("Yes")
                add("No")
            }
        }
        return Array(out.prefix(4))
    }

    /// The email with its questions marked (a tint behind each).
    static func highlighted(_ text: String, questions: [String], tint: Color) -> AttributedString {
        var out = AttributedString(text)
        for q in questions {
            guard let range = out.range(of: q) else { continue }
            out[range].backgroundColor = tint
        }
        return out
    }

    private static let yesNoStarts = ["can", "could", "would", "will", "do", "does", "did", "are", "is", "should",
                                      "shall", "may", "have", "has", "any chance", "ok", "okay", "happy to"]

    private static func yesNo(_ q: String) -> Bool {
        let lower = q.lowercased()
        let words = lower.split(whereSeparator: { !$0.isLetter && $0 != "'" })
        // "Are we go for Tuesday": the question's own first word, after a
        // leading "So," or "And".
        let start = words.drop { ["so", "and", "but", "also", "then"].contains(String($0)) }
        guard let first = start.first else { return false }
        return yesNoStarts.contains { $0.contains(" ") ? lower.contains($0) : String(first) == $0 }
    }

    /// "…Tuesday, or do you want to push to Thursday" → the two options,
    /// each the last few words before and after " or ".
    private static func choices(in q: String) -> [String]? {
        let parts = q.components(separatedBy: " or ")
        guard parts.count == 2 else { return nil }
        let before = parts[0].split(separator: " ").suffix(3).joined(separator: " ")
        let after = parts[1].split(separator: " ").suffix(3).joined(separator: " ")
        let clean = { (s: String) in
            s.trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
        }
        let a = clean(lastPhrase(before)), b = clean(lastPhrase(after))
        guard !a.isEmpty, !b.isEmpty else { return nil }
        return [a, b]
    }

    /// From "go for Tuesday" keep "Tuesday"-like words: after "for", "to",
    /// "on" or "at" when there is one.
    private static func lastPhrase(_ s: String) -> String {
        let words = s.split(separator: " ").map(String.init)
        if let i = words.lastIndex(where: { ["for", "to", "on", "at", "with"].contains($0.lowercased()) }), i < words.count - 1 {
            return words[(i + 1)...].joined(separator: " ")
        }
        return s
    }
}
