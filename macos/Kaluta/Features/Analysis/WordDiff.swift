import SwiftUI

/// The words that differ between what the AI wrote and what the user sent
/// (spec §14.10): a longest-common-subsequence over words, so text the
/// user kept reads plainly and their changes stand out. Pure.
enum WordDiff {
    /// Each side split into runs: the text, and whether it is changed.
    static func runs(ai: String, sent: String) -> (ai: [(String, Bool)], sent: [(String, Bool)]) {
        let a = tokens(ai), b = tokens(sent)
        let keyA = a.map(key), keyB = b.map(key)
        // Too long to compare word by word: show both plainly.
        guard keyA.count * keyB.count <= Self.limit else { return ([(ai, false)], [(sent, false)]) }
        var table = Array(repeating: Array(repeating: 0, count: keyB.count + 1), count: keyA.count + 1)
        for i in stride(from: keyA.count - 1, through: 0, by: -1) {
            for j in stride(from: keyB.count - 1, through: 0, by: -1) {
                table[i][j] = keyA[i] == keyB[j] ? table[i + 1][j + 1] + 1 : max(table[i + 1][j], table[i][j + 1])
            }
        }
        var keptA = Set<Int>(), keptB = Set<Int>()
        var i = 0, j = 0
        while i < keyA.count, j < keyB.count {
            if keyA[i] == keyB[j], !keyA[i].isEmpty {
                keptA.insert(i); keptB.insert(j); i += 1; j += 1
            } else if table[i + 1][j] >= table[i][j + 1] {
                i += 1
            } else {
                j += 1
            }
        }
        return (merge(a, kept: keptA), merge(b, kept: keptB))
    }

    /// The AI's text with its replaced words struck through, and the sent
    /// text with the user's words marked.
    static func attributed(ai: String, sent: String) -> (ai: AttributedString, sent: AttributedString) {
        let (a, b) = runs(ai: ai, sent: sent)
        func build(_ runs: [(String, Bool)], changed: (inout AttributedString) -> Void) -> AttributedString {
            var out = AttributedString()
            for (text, isChanged) in runs {
                var piece = AttributedString(text)
                if isChanged { changed(&piece) }
                out += piece
            }
            return out
        }
        return (build(a) { $0.strikethroughStyle = .single; $0.foregroundColor = .secondary },
                build(b) { $0.backgroundColor = Tone.changedText })
    }

    /// Words with the spaces after them, so joining gives the text back.
    static func tokens(_ text: String) -> [String] {
        var out: [String] = []
        var current = ""
        for ch in text {
            if ch.isWhitespace {
                current.append(ch)
            } else if let last = current.last, last.isWhitespace {
                out.append(current)
                current = String(ch)
            } else {
                current.append(ch)
            }
        }
        if !current.isEmpty { out.append(current) }
        return out
    }

    /// How a token compares: lower case, without spaces or end punctuation.
    private static func key(_ token: String) -> String {
        token.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            .trimmingCharacters(in: .punctuationCharacters)
    }

    /// Neighbouring tokens with the same state as one run.
    private static func merge(_ tokens: [String], kept: Set<Int>) -> [(String, Bool)] {
        var out: [(String, Bool)] = []
        for (i, t) in tokens.enumerated() {
            let changed = !kept.contains(i) && !key(t).isEmpty
            if let last = out.last, last.1 == changed {
                out[out.count - 1].0 += t
            } else {
                out.append((t, changed))
            }
        }
        return out
    }

    /// Word pairs compared at most (about 500 words a side).
    private static let limit = 250_000
}
