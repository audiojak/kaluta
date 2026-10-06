import Foundation

/// The interview (spec §14.9): short questions for what sent mail cannot
/// show (content rules, facts, what to do when unsure) and for any category
/// left without evidence. Fixed in the app, so it needs no agent. Each
/// answer becomes entries with source "you". Pure, so it is unit-tested.
struct GuideQuestion: Identifiable, Equatable {
    enum Answer: Equatable {
        /// One of these; each option's statement (nil: nothing to add).
        case choice([(title: String, statement: String?, kind: GuideKind)])
        /// Free text turned into one statement per line or comma-separated
        /// item by `template` ("Never write '%@'"), with an optional check.
        case list(template: String, kind: GuideKind, check: GuideCheckKind?)
        /// Free text, used as the statement itself.
        case text(kind: GuideKind)
        /// Several labelled fields, each a fact when filled (spec §14.11):
        /// its category key and label in Facts.
        case facts([(label: String, category: String, fact: String)])

        static func == (a: Answer, b: Answer) -> Bool {
            switch (a, b) {
            case let (.choice(x), .choice(y)): x.map(\.title) == y.map(\.title)
            case let (.list(t1, k1, c1), .list(t2, k2, c2)): t1 == t2 && k1 == k2 && c1 == c2
            case let (.text(k1), .text(k2)): k1 == k2
            case let (.facts(x), .facts(y)): x.map(\.label) == y.map(\.label)
            default: false
            }
        }
    }

    let id: String
    let category: String
    let prompt: String
    let detail: String
    let answer: Answer
    /// A suggestion drawn from the user's mail (their signature).
    var suggestion: String?
}

enum GuideInterview {
    /// The questions to ask, in order: the fixed ones, then one for each
    /// learned category that has nothing yet. `answered` are left out.
    static func questions(categories: [GuideCategoryInfo], signature: String?,
                          answered: Set<String>) -> [GuideQuestion] {
        var out: [GuideQuestion] = [
            GuideQuestion(id: "F4", category: "F4", prompt: "When a draft needs a fact nobody gave it…",
                          detail: "A name, a figure, a date, a price.", answer: .choice([
                              ("Leave a [bracket] for me to fill in",
                               "Never invent facts, names, figures or dates; leave a [bracket] for anything not known", .rule),
                              ("Ask me before drafting",
                               "Never invent facts, names, figures or dates; ask me for anything not known", .rule),
                          ])),
            GuideQuestion(id: "H1", category: "H1", prompt: "When the request is unclear…",
                          detail: "What should the draft do?", answer: .choice([
                              ("Ask me a question first", "When a request is unclear, ask me before drafting", .guideline),
                              ("Draft the likeliest reading", "When a request is unclear, draft the likeliest reading and say what was assumed", .guideline),
                              ("Offer two short options", "When a request is unclear, offer two short alternative drafts", .guideline),
                          ])),
            GuideQuestion(id: "F1", category: "F1", prompt: "What should a draft never promise without asking you?",
                          detail: "One per line or separated by commas: delivery dates, prices, discounts, contract terms…",
                          answer: .list(template: "Never promise %@ without asking me", kind: .rule, check: nil)),
            GuideQuestion(id: "F2", category: "F2", prompt: "Anything never to mention?",
                          detail: "Topics, projects or figures, one per line or separated by commas.",
                          answer: .list(template: "Never mention %@", kind: .rule, check: nil)),
            GuideQuestion(id: "F3", category: "F3", prompt: "Facts a draft may use about you",
                          detail: "Only what you fill in is used. They are kept in Analysis › Facts.", answer: .facts([
                              ("Role and company", "work", "Occupation or role"),
                              ("Calendar link", "availability", "Calendar link"),
                              ("Time zone", "availability", "Time zone"),
                              ("Working hours", "availability", "Usual hours"),
                              ("Phone", "contact", "Phone"),
                              ("Pronouns", "identity", "Pronouns"),
                          ]), suggestion: signature),
            GuideQuestion(id: "F5", category: "F5", prompt: "Should a message say an AI helped write it?",
                          detail: "", answer: .choice([
                              ("Never mention it", "Never say that an AI helped write a message", .rule),
                              ("Only if someone asks", "Say that an assistant helped only if someone asks", .guideline),
                              ("Say so at the end", "End messages an assistant helped write with a short note saying so", .rule),
                          ])),
            GuideQuestion(id: "F6", category: "F6", prompt: "Wording some messages must include?",
                          detail: "Legal or compliance text, exactly as it must appear. Leave empty if none.",
                          answer: .list(template: "Include “%@” where it applies", kind: .rule, check: .requiredPhrase)),
            GuideQuestion(id: "C8", category: "C8", prompt: "Words or phrases you never use?",
                          detail: "Drafts are checked for them. One per line or separated by commas: circle back, per my last email…",
                          answer: .list(template: "Never write “%@”", kind: .rule, check: .bannedPhrase)),
            GuideQuestion(id: "H4", category: "H4", prompt: "What may an assistant do with a draft?",
                          detail: "Settings › Permissions still decides what it is allowed to do.", answer: .choice([
                              ("Only draft; I send", "Only draft messages; I review and send them myself", .rule),
                              ("Send routine replies, ask for the rest",
                               "Send only routine acknowledgements without asking; ask me before sending anything else", .guideline),
                              ("Don't reply to automated mail", "Never reply to newsletters or automated mail", .rule),
                          ])),
            GuideQuestion(id: "B7", category: "B7", prompt: "Your signature",
                          detail: signature == nil ? "When should drafts include a signature block?"
                              : "Your mail ends with this. When should drafts include it?",
                          answer: .choice([
                              ("In new messages only", "Include my signature block in new messages, not in replies", .guideline),
                              ("In every message", "Include my signature block in every message", .guideline),
                              ("Never", "Never add a signature block; my mail app adds it", .rule),
                          ]), suggestion: signature),
            GuideQuestion(id: "D5", category: "D5", prompt: "Anything different when you write to someone senior?",
                          detail: "Leave empty if not.", answer: .text(kind: .guideline)),
            GuideQuestion(id: "E4", category: "E4", prompt: "When you set up a meeting…",
                          detail: "", answer: .choice([
                              ("Offer two or three times", "When scheduling, offer two or three specific times with the time zone", .guideline),
                              ("Send my calendar link", "When scheduling, send my calendar link rather than proposing times", .guideline),
                          ])),
        ]
        let fixed = Set(out.map(\.category))
        for category in categories where category.learned && !category.asked && category.accepted == 0 && !fixed.contains(category.id) {
            out.append(GuideQuestion(id: "empty-\(category.id)", category: category.id,
                                     prompt: "\(category.name): how do you handle it?",
                                     detail: "Your mail showed nothing yet about \(category.looksFor). Leave empty to skip.",
                                     answer: .text(kind: .guideline)))
        }
        return out.filter { !answered.contains($0.id) }
    }

    /// Split a list answer: lines or commas, trimmed, empties dropped.
    static func items(_ text: String) -> [String] {
        text.split(whereSeparator: { $0 == "\n" || $0 == "," })
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
    }

    /// The entries an answer makes: `choice` is the chosen option's index,
    /// `text` the typed answer, `fields` the facts' values in order.
    static func entries(for q: GuideQuestion, choice: Int?, text: String, fields: [String]) -> [GuideEntryFields] {
        let make = { (statement: String, kind: GuideKind, check: GuideCheck?) in
            GuideEntryFields(category: q.category, kind: kind, statement: statement, scope: .always, check: check)
        }
        switch q.answer {
        case let .choice(options):
            guard let choice, options.indices.contains(choice), let s = options[choice].statement else { return [] }
            return [make(s, options[choice].kind, nil)]
        case let .list(template, kind, check):
            return items(text).map { item in
                make(String(format: template, item), kind, check.map { GuideCheck(kind: $0, value: item) })
            }
        case let .text(kind):
            let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
            return t.isEmpty ? [] : [make(t, kind, nil)]
        case .facts:
            // Facts have their own store: see `facts(for:fields:)`.
            return []
        }
    }

    /// The facts a facts question's filled fields make (spec §14.11).
    static func facts(for q: GuideQuestion, fields: [String]) -> [FactEdit] {
        guard case let .facts(labels) = q.answer else { return [] }
        return zip(labels, fields).compactMap { label, value in
            let v = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return v.isEmpty ? nil : .add(fields: FactFields(category: label.category, label: label.fact, value: v,
                                                             use: .free, asOf: nil),
                                          status: .accepted, source: .you)
        }
    }

    static func answeredKey(_ accountID: String) -> String { "guideInterviewAnswered.\(accountID)" }
}
