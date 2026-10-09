import SwiftUI

/// Answer Questions… (spec §14.9): one question at a time for what mail
/// cannot show. Each answer is saved as it is given (undoable) and not
/// asked again; choosing an answer saves it and goes on. Skip leaves a
/// question for next time; Back shows the last one with its answer, and a
/// different answer replaces it. `only` asks one category's question
/// again, from that category.
struct InterviewSheet: View {
    @Environment(AppModel.self) private var model
    let only: String?
    @State private var questions: [GuideQuestion] = []
    @State private var index = 0
    @State private var choice: Int?
    @State private var text = ""
    @State private var fields: [String] = []
    /// What was answered in this sitting, by question id.
    @State private var given: [String: Given] = [:]
    @State private var error: String?
    @State private var loaded = false

    var body: some View {
        Dialog(title: title, message: loaded && questions.isEmpty ? "Nothing to ask right now." : progress,
               width: Self.width) {
            if let q = current {
                VStack(alignment: .leading, spacing: Space.m) {
                    Text(q.prompt).font(TypeRole.heading)
                    if !q.detail.isEmpty {
                        Text(q.detail).font(TypeRole.meta).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let suggestion = q.suggestion {
                        Text(suggestion)
                            .font(TypeRole.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .padding(Space.m)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .card(.neutral)
                            .textSelection(.enabled)
                    }
                    control(for: q)
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } leading: {
            if index > 0, !questions.isEmpty {
                Button("Back") { back() }
                    .hoverHelp("The question before, with your answer")
            }
            if let q = current {
                Button(given[q.id] == nil ? "Skip" : "Next") { advance() }
                    .hoverHelp(given[q.id] == nil ? "Leave this question for another time" : "Keep your answer and go on")
            }
        } buttons: {
            CancelButton(title: saved > 0 || questions.isEmpty ? "Done" : "Cancel",
                         help: "Close; answers given so far are kept (Esc)") { model.guideSheet = nil }
            if let q = current, !q.isChoice {
                Button("Save") { Task { await save() } }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Save this answer to your guide (Return)")
            }
        }
        .task { await load() }
    }

    private var current: GuideQuestion? { questions.indices.contains(index) ? questions[index] : nil }
    private var title: String { only == nil ? "A Few Questions" : "Question" }
    private var saved: Int { given.values.filter(\.savedSomething).count }

    private var progress: String? {
        guard !questions.isEmpty else { return nil }
        return current == nil ? "All done: \(saved) saved." : "\(index + 1) of \(questions.count). Mail cannot show these."
    }

    @ViewBuilder private func control(for q: GuideQuestion) -> some View {
        switch q.answer {
        case let .choice(options):
            VStack(alignment: .leading, spacing: Space.s) {
                ForEach(Array(options.enumerated()), id: \.offset) { i, o in
                    AnswerButton(title: o.title, number: i + 1, chosen: given[q.id]?.choice == i) {
                        choice = i
                        Task { await save() }
                    }
                }
            }
        case .list, .text:
            TextField("Your answer", text: $text, axis: .vertical)
                .lineLimit(2...5)
                .textFieldStyle(.roundedBorder)
        case let .facts(labels):
            Grid(alignment: .leading, horizontalSpacing: Space.m, verticalSpacing: Space.s) {
                ForEach(Array(labels.enumerated()), id: \.offset) { i, l in
                    GridRow {
                        Text(l.label).foregroundStyle(.secondary)
                        TextField(l.label, text: Binding(get: { fields.indices.contains(i) ? fields[i] : "" },
                                                         set: { if fields.indices.contains(i) { fields[i] = $0 } }))
                            .textFieldStyle(.roundedBorder)
                    }
                }
            }
        }
    }

    private var answered: Set<String> {
        guard let account = model.openAccountID else { return [] }
        return Set(CoreClient.appDefaults().stringArray(forKey: GuideInterview.answeredKey(account)) ?? [])
    }

    private func load() async {
        let signature = try? await model.core?.guideSignature()
        await model.guide.load()
        var all = GuideInterview.questions(categories: model.guide.categories, signature: signature ?? nil,
                                           answered: only == nil ? answered : [])
        if let only { all = all.filter { $0.category == only } }
        questions = all
        loaded = true
        reset()
    }

    /// The fields as last answered, else empty.
    private func reset() {
        error = nil
        let earlier = current.flatMap { given[$0.id] }
        choice = earlier?.choice
        text = earlier?.text ?? ""
        if case let .facts(labels) = current?.answer {
            fields = earlier?.fields ?? Array(repeating: "", count: labels.count)
        } else {
            fields = []
        }
    }

    private func advance() {
        index += 1
        reset()
    }

    private func back() {
        index = max(index - 1, 0)
        reset()
    }

    private func save() async {
        guard let q = current else { return }
        let earlier = given[q.id]
        // The same answer again: nothing to change.
        if let earlier, earlier.choice == choice, earlier.text == text, earlier.fields == fields {
            advance()
            return
        }
        await model.facts.load()
        let facts = GuideInterview.facts(for: q, fields: fields).map { edit -> FactEdit in
            // A fact by that label already: the answer replaces its value.
            guard case let .add(f, _, _) = edit, let old = model.facts.facts.first(where: {
                $0.scope == .account && $0.status == .accepted && $0.category == f.category
                    && $0.label.lowercased() == f.label.lowercased()
            }) else { return edit }
            return .update(id: old.id, fields: FactFields(category: f.category, label: old.label, value: f.value,
                                                          use: old.use, asOf: old.asOf))
        }
        if !facts.isEmpty {
            if let failure = await model.applyFactEdits(facts, actionName: "Answer Question",
                                                         notice: facts.count == 1 ? "Added a fact" : "Added \(facts.count) facts") {
                error = failure.message
                return
            }
        }
        let entries = GuideInterview.entries(for: q, choice: choice, text: text, fields: fields)
        // A changed answer replaces what the earlier one added, in the same change.
        let replaced = earlier?.entryIDs ?? []
        var entryIDs: [Int64] = []
        if !entries.isEmpty || !replaced.isEmpty {
            let edits = replaced.map { GuideEdit.delete(id: $0) }
                + entries.map { .add(fields: $0, status: .accepted, source: .you, origin: nil) }
            let result = await model.applyGuideEdits(edits, reason: "interview", actionName: "Answer Question",
                                                     notice: replaced.isEmpty ? "Added your answer to the writing guide"
                                                         : "Changed your answer in the writing guide")
            switch result {
            case let .failure(e):
                error = e.message
                return
            case let .success(change):
                entryIDs = change.entries.map(\.id)
            }
        }
        given[q.id] = Given(choice: choice, text: text, fields: fields, entryIDs: entryIDs,
                            savedSomething: !entryIDs.isEmpty || !facts.isEmpty || earlier?.savedSomething == true)
        if let account = model.openAccountID {
            let defaults = CoreClient.appDefaults()
            let key = GuideInterview.answeredKey(account)
            defaults.set(Array(answered.union([q.id])).sorted(), forKey: key)
        }
        advance()
    }

    private static let width: CGFloat = 520
}

/// An answer given in this sitting: what was chosen or typed, and the
/// guide entries it added, which a different answer replaces.
private struct Given {
    var choice: Int?
    var text: String
    var fields: [String]
    var entryIDs: [Int64]
    var savedSomething: Bool
}

extension GuideQuestion {
    /// Answered by choosing one of its answers.
    var isChoice: Bool {
        if case .choice = answer { return true }
        return false
    }
}
