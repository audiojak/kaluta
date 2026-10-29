import SwiftUI

/// The questions in an email and the starters drawn from them, for *What
/// do you want to say?* (spec §14.13). Pure.
enum WritingText {
    /// The sentences that ask something, in order: each ends in "?".
    /// The email asks to meet, talk or find a time (spec §14.9).
    static func asksToMeet(_ text: String) -> Bool {
        let t = text.lowercased()
        return ["meet", "talk", "a call", "find a time", "catch up", "schedule", "coffee", "your availability",
                "when are you free", "when works"].contains { t.contains($0) }
    }

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
            } else if isYesNo(body) {
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

    /// A question that takes yes or no ("Can you do Thursday?").
    static func isYesNo(_ q: String) -> Bool {
        let lower = q.lowercased()
        let words = lower.split(whereSeparator: { !$0.isLetter && $0 != "'" })
        // "Are we go for Tuesday": the question's own first word, after a
        // leading "So," or "And".
        let start = words.drop { ["so", "and", "but", "also", "then"].contains(String($0)) }
        guard let first = start.first else { return false }
        return yesNoStarts.contains { $0.contains(" ") ? lower.contains($0) : String(first) == $0 }
    }

    /// "Are we go for Tuesday, or do you want to push to Thursday" → "Go for
    /// Tuesday", "Push to Thursday": each side of " or " as the thing to do,
    /// the asking words ("do you want to", "should I") left out. A side too
    /// long to be a starter keeps its last words after "for", "to", "on"…
    private static func choices(in q: String) -> [String]? {
        let parts = q.components(separatedBy: " or ")
        guard parts.count == 2 else { return nil }
        // The first option starts after the last comma or sentence break.
        let first = parts[0].components(separatedBy: CharacterSet(charactersIn: ",;:."))
            .last { !$0.trimmingCharacters(in: .whitespaces).isEmpty } ?? parts[0]
        let options = [first, parts[1]].map { side -> String in
            var s = side.trimmingCharacters(in: .whitespacesAndNewlines.union(.punctuationCharacters))
            var lower = s.lowercased()
            var stripped = true
            while stripped {
                stripped = false
                for asking in askingWords where lower.hasPrefix(asking + " ") {
                    s = String(s.dropFirst(asking.count + 1))
                    lower = s.lowercased()
                    stripped = true
                }
            }
            if s.split(separator: " ").count > 5 { s = lastPhrase(s) }
            return s.prefix(1).uppercased() + s.dropFirst()
        }
        guard options.allSatisfy({ !$0.isEmpty }) else { return nil }
        return options
    }

    /// Words that ask rather than say what to do, longest first.
    private static let askingWords = [
        "do you want to", "would you like to", "would you rather", "should i", "shall i", "should we", "shall we",
        "do you", "do we", "can you", "could you", "can we", "are we", "is it", "we", "i", "you", "to", "either",
    ]

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
