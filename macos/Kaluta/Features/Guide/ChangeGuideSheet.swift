import SwiftUI

/// Ask Claude to Change the Guide… (spec §14.9): a request in the user's
/// words; the agent answers with questions, each one decision with before
/// and after. Nothing changes until the user answers; the yeses apply as
/// one change, undoable, and a new version of the guide.
struct ChangeGuideSheet: View {
    @Environment(AppModel.self) private var model
    @State private var request = ""
    @State private var questions: [GuideChangeQuestion] = []
    /// Yes or no, by question.
    @State private var answers: [Int: Bool] = [:]
    /// Reworded statements of added or edited entries, by question and edit.
    @State private var wording: [String: String] = [:]
    @State private var asking = false
    @State private var error: String?
    @FocusState private var focused: Bool

    var body: some View {
        Dialog(title: "Change Your Writing Guide",
               message: questions.isEmpty ? "Say what you want changed. \(model.agent.providerName) works out the changes and asks you about each before anything changes." : "Answer each question. Only the ones you say yes to change your guide.",
               width: Self.width) {
            if let why = notReady {
                Label(why, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if questions.isEmpty {
                TextField("Make everything for customers more formal", text: $request, axis: .vertical)
                    .lineLimit(2...5)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                if asking {
                    HStack(spacing: Space.s) {
                        ProgressView().controlSize(.small)
                        Text("Asking \(model.agent.providerName)…").foregroundStyle(.secondary)
                    }
                    .font(TypeRole.meta)
                }
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: Space.m) {
                        ForEach(Array(questions.enumerated()), id: \.offset) { i, q in question(i, q) }
                    }
                }
                .frame(maxHeight: Self.listHeight)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
            }
        } buttons: {
            CancelButton(help: "Close without changing anything (Esc)") { model.guideSheet = nil }
            if questions.isEmpty {
                Button("Ask") { Task { await ask() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(asking || notReady != nil || request.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .hoverHelp("Ask \(model.agent.providerName) what to change (Return)")
            } else {
                let yes = answers.values.filter { $0 }.count
                Button(yes == 1 ? "Apply 1 Change" : "Apply \(yes) Changes") { Task { await apply() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(yes == 0 || answers.count < questions.count)
                    .hoverHelp("Change your guide as you answered; Undo reverses it (Return)")
            }
        }
        .onAppear { focused = true }
    }

    private func question(_ i: Int, _ q: GuideChangeQuestion) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(q.question).font(TypeRole.heading).fixedSize(horizontal: false, vertical: true)
            if !q.before.isEmpty || !q.after.isEmpty {
                Grid(alignment: .leading, horizontalSpacing: Space.m, verticalSpacing: Space.xs) {
                    GridRow {
                        Text("Now").foregroundStyle(.secondary)
                        Text(q.before.isEmpty ? "nothing" : q.before)
                    }
                    GridRow {
                        Text("After").foregroundStyle(.secondary)
                        Text(q.after.isEmpty ? "nothing" : q.after)
                    }
                }
                .font(TypeRole.meta)
            }
            ForEach(Array(q.edits.enumerated()), id: \.offset) { j, edit in
                if let statement = Self.statement(edit) {
                    TextField("Statement", text: Binding(get: { wording["\(i)-\(j)"] ?? statement },
                                                         set: { wording["\(i)-\(j)"] = $0 }))
                        .textFieldStyle(.roundedBorder)
                        .disabled(answers[i] != true)
                }
            }
            Picker("Answer", selection: Binding(get: { answers[i] }, set: { answers[i] = $0 })) {
                Text("Yes").tag(Bool?.some(true))
                Text("No").tag(Bool?.some(false))
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .fixedSize()
            .hoverHelp("Yes changes the guide as shown; no leaves it")
        }
        .padding(Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(answers[i] == nil ? .attention : .neutral)
    }

    private var notReady: String? {
        model.agent.isProviderReady ? nil
            : "\(model.agent.providerName) is not ready. Connect Claude Code or Codex in Settings › Agents to change the guide by asking."
    }

    static func statement(_ edit: GuideEdit) -> String? {
        switch edit {
        case let .add(fields, _, _, _), let .update(_, fields): fields.statement
        default: nil
        }
    }

    private func ask() async {
        guard let core = model.core else { return }
        asking = true
        defer { asking = false }
        do {
            questions = try await core.proposeGuideChange(request, agent: model.agent.providerID)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    private func apply() async {
        var edits: [GuideEdit] = []
        for (i, q) in questions.enumerated() where answers[i] == true {
            for (j, edit) in q.edits.enumerated() {
                let reworded = wording["\(i)-\(j)"]
                switch (edit, reworded) {
                case let (.add(fields, status, source, origin), .some(text)):
                    var f = fields
                    f.statement = text
                    edits.append(.add(fields: f, status: status, source: source, origin: origin))
                case let (.update(id, fields), .some(text)):
                    var f = fields
                    f.statement = text
                    edits.append(.update(id: id, fields: f))
                default:
                    edits.append(edit)
                }
            }
        }
        let result = await model.applyGuideEdits(edits, reason: "change by prompt", actionName: "Change Guide",
                                                 notice: "Changed your writing guide")
        switch result {
        case .success: model.guideSheet = nil
        case let .failure(e): error = e.message
        }
    }

    private static let width: CGFloat = 600
    private static let listHeight: CGFloat = 440
}
