import SwiftUI

/// `⇧T` (spec §14.8): tasks for many emails at once. The highlighted
/// threads, or else the latest 20 in the open list; one request to Claude
/// for all of them; each row editable, with a checkbox. Threads that
/// already have an open task are marked and start unchecked.
@MainActor
@Observable
final class BulkTaskDraft: Identifiable {
    @MainActor
    @Observable
    final class Row: Identifiable {
        let threadID: String
        let sender: String
        let subject: String
        let hasTask: Bool
        var included: Bool
        var title = ""
        var category: String
        var hasDue = false
        var due: Date
        var action: TaskAction = .reply
        var why = ""
        var fromAI = false
        let id: String

        init(threadID: String, sender: String, subject: String, hasTask: Bool, category: String, now: Date) {
            self.threadID = threadID
            id = threadID
            self.sender = sender
            self.subject = subject
            self.hasTask = hasTask
            included = !hasTask
            self.category = category
            due = Calendar.current.startOfDay(for: now)
        }

        func apply(_ s: TaskSuggestion) {
            title = s.title
            category = s.category
            if let day = s.dueDay, let date = DueDay.date(day) {
                hasDue = true
                due = date
            } else {
                hasDue = false
            }
            action = s.action
            why = s.why
            fromAI = true
        }

        var newTask: NewTask {
            NewTask(threadId: threadID, messageId: nil, title: title, notes: "", category: category,
                    dueDay: hasDue ? DueDay.string(due) : nil, action: action, why: why, fromAi: fromAI)
        }
    }

    /// Threads asked about when nothing is highlighted.
    static let defaultCount = 20

    let id = UUID()
    let rows: [Row]
    let categories: [String]
    let suggester = TaskSuggester()
    var error: String?

    init(rows: [Row], categories: [String]) {
        self.rows = rows
        self.categories = categories
        suggester.onDone = { [weak self] found in
            guard let self else { return }
            for s in found {
                // A title typed while Claude worked stays.
                if let row = self.rows.first(where: { $0.threadID == s.threadId }), row.title.isEmpty { row.apply(s) }
            }
        }
    }

    /// Checked rows with a title: what "Add" adds.
    var ready: [Row] {
        rows.filter { $0.included && !$0.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }
}

struct BulkTaskSheet: View {
    @Environment(AppModel.self) private var model
    let draft: BulkTaskDraft

    var body: some View {
        Dialog(title: "Create Tasks", message: message, width: Self.width) {
            status
            ScrollView {
                VStack(alignment: .leading, spacing: Space.l) {
                    ForEach(draft.rows) { row in
                        BulkTaskRow(row: row, categories: draft.categories)
                        if row.id != draft.rows.last?.id { InsetRule() }
                    }
                }
            }
            .frame(height: Self.listHeight)
        } leading: {
            Button("Ask Again") { ask() }
                .disabled(!TaskSuggester.canAsk(model) || draft.suggester.state == .working)
                .hoverHelp("Ask \(model.agent.providerName) again about these emails")
        } buttons: {
            CancelButton(help: "Close without adding tasks (Esc)") { model.closeBulkTasks() }
            let count = draft.ready.count
            Button(count == 1 ? "Add 1 Task" : "Add \(count) Tasks") { Task { await model.acceptBulkTasks(draft) } }
                .keyboardShortcut(.defaultAction)
                .disabled(count == 0)
                .hoverHelp("Add the checked tasks and label their emails Task (Return)")
        }
    }

    private var message: String {
        let n = draft.rows.count
        return n == 1 ? "1 email" : "\(n) emails"
    }

    @ViewBuilder private var status: some View {
        if let error = draft.error {
            Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
        }
        switch draft.suggester.state {
        case .working:
            HStack(spacing: Space.s) {
                ProgressView().controlSize(.small)
                Text("Asking \(model.agent.providerName) about \(draft.rows.count) emails…").foregroundStyle(.secondary)
            }
            .font(TypeRole.meta)
        case let .failed(message):
            Label(message, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
        case .idle where !TaskSuggester.canAsk(model):
            Text("\(model.agent.providerName) is not set up, so write the tasks yourself.")
                .font(TypeRole.meta).foregroundStyle(.secondary)
        default:
            EmptyView()
        }
    }

    private func ask() {
        Task { await draft.suggester.run(threadIDs: draft.rows.map(\.threadID), model: model) }
    }

    private static let width: CGFloat = 640
    private static let listHeight: CGFloat = 420
}

private struct BulkTaskRow: View {
    @Bindable var row: BulkTaskDraft.Row
    let categories: [String]

    var body: some View {
        HStack(alignment: .top, spacing: Space.m) {
            Toggle("Add", isOn: $row.included)
                .labelsHidden()
                .hoverHelp(row.included ? "Leave this email out" : "Add a task for this email")
            VStack(alignment: .leading, spacing: Space.xs) {
                HStack(spacing: Space.s) {
                    Text("\(row.sender) · \(row.subject)")
                        .font(TypeRole.meta).foregroundStyle(.secondary).lineLimit(1)
                    if row.hasTask {
                        Text("Has a task").font(TypeRole.caption).foregroundStyle(Tone.caution)
                    }
                }
                TextField("What to do", text: $row.title)
                    .textFieldStyle(.roundedBorder)
                HStack(spacing: Space.m) {
                    Picker("Category", selection: $row.category) {
                        ForEach(categories, id: \.self) { Text($0).tag($0) }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .hoverHelp("The task's category")
                    Toggle("Due", isOn: $row.hasDue)
                        .hoverHelp("Give the task a due day")
                    DatePicker("Due day", selection: $row.due, displayedComponents: .date)
                        .labelsHidden()
                        .disabled(!row.hasDue)
                        .hoverHelp("The day it should be done by")
                    Spacer(minLength: 0)
                }
                if !row.why.isEmpty {
                    Text(row.why).font(TypeRole.caption).foregroundStyle(.secondary)
                }
            }
            .disabled(!row.included)
        }
    }
}
