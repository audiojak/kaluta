import SwiftUI

/// Answer Questions… (spec §14.9): one question at a time for what mail
/// cannot show. Each answer is saved as it is given (undoable) and not
/// asked again; Skip leaves a question for next time. `only` asks one
/// category's question again, from that category.
struct InterviewSheet: View {
    @Environment(AppModel.self) private var model
    let only: String?
    @State private var questions: [GuideQuestion] = []
    @State private var index = 0
    @State private var choice: Int?
    @State private var text = ""
    @State private var fields: [String] = []
    @State private var saved = 0
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
            if current != nil {
                Button("Skip") { advance() }
                    .hoverHelp("Leave this question for another time")
            }
        } buttons: {
            CancelButton(title: saved > 0 || questions.isEmpty ? "Done" : "Cancel",
                         help: "Close; answers given so far are kept (Esc)") { model.guideSheet = nil }
            if current != nil {
                Button("Save") { Task { await save() } }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Save this answer to your guide (Return)")
            }
        }
        .task { await load() }
    }

    private var current: GuideQuestion? { questions.indices.contains(index) ? questions[index] : nil }
    private var title: String { only == nil ? "A Few Questions" : "Question" }
    private var progress: String? {
        guard !questions.isEmpty else { return nil }
        return current == nil ? "All done: \(saved) saved." : "\(index + 1) of \(questions.count). Mail cannot show these."
    }

    @ViewBuilder private func control(for q: GuideQuestion) -> some View {
        switch q.answer {
        case let .choice(options):
            Picker("Answer", selection: $choice) {
                ForEach(Array(options.enumerated()), id: \.offset) { i, o in Text(o.title).tag(Int?.some(i)) }
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            .hoverHelp("Choose the answer that fits")
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

    private func reset() {
        choice = nil
        text = ""
        error = nil
        if case let .facts(labels) = current?.answer { fields = Array(repeating: "", count: labels.count) } else { fields = [] }
    }

    private func advance() {
        index += 1
        reset()
    }

    private func save() async {
        guard let q = current else { return }
        let facts = GuideInterview.facts(for: q, fields: fields)
        if !facts.isEmpty {
            if let failure = await model.applyFactEdits(facts, actionName: "Answer Question",
                                                         notice: facts.count == 1 ? "Added a fact" : "Added \(facts.count) facts") {
                error = failure.message
                return
            }
            saved += facts.count
        }
        let entries = GuideInterview.entries(for: q, choice: choice, text: text, fields: fields)
        if !entries.isEmpty {
            let result = await model.applyGuideEdits(entries.map { .add(fields: $0, status: .accepted, source: .you, origin: nil) },
                                                     reason: "interview", actionName: "Answer Question",
                                                     notice: "Added your answer to the writing guide")
            if case let .failure(e) = result {
                error = e.message
                return
            }
            saved += entries.count
        }
        if let account = model.openAccountID {
            let defaults = CoreClient.appDefaults()
            let key = GuideInterview.answeredKey(account)
            defaults.set(Array(answered.union([q.id])).sorted(), forKey: key)
        }
        advance()
    }

    private static let width: CGFloat = 520
}
