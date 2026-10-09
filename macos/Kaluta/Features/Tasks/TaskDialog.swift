import SwiftUI

/// The task dialog's state (spec §14.8): one email, Claude's suggestion
/// for it once it arrives, and whatever the user changes. A field the user
/// has edited is not overwritten by a later suggestion.
@MainActor
@Observable
final class TaskDraft: Identifiable {
    let id = UUID()
    let threadID: String
    let subject: String
    let sender: String
    let categories: [String]

    var title = "" { didSet { if !applying { editedTitle = true } } }
    var category: String { didSet { if !applying { editedOther = true } } }
    var hasDue = false { didSet { if !applying { editedOther = true } } }
    var due: Date { didSet { if !applying { editedOther = true } } }
    var action: TaskAction = .reply { didSet { if !applying { editedOther = true } } }
    var notes = ""
    /// Claude's line on why, shown under the fields.
    private(set) var why = ""
    /// The title came from Claude and was not changed.
    private(set) var fromAI = false
    /// Why adding it failed, shown in the dialog.
    var error: String?

    /// The task being changed (`↩` in the task list), or nil for a new one.
    let editing: TaskItem?

    let suggester = TaskSuggester()
    @ObservationIgnored private var applying = false
    @ObservationIgnored private var editedTitle = false
    /// Category, due day or action changed by the user: a later
    /// suggestion leaves them as they are.
    @ObservationIgnored private var editedOther = false

    init(threadID: String, subject: String, sender: String, categories: [String], now: Date = .now) {
        editing = nil
        self.threadID = threadID
        self.subject = subject
        self.sender = sender
        self.categories = categories
        category = categories.first ?? "Reply"
        due = Calendar.current.startOfDay(for: now)
        suggester.onDone = { [weak self] found in
            if let first = found.first { self?.apply(first) }
        }
    }

    /// The dialog for changing a task: its fields, no request to Claude.
    init(editing task: TaskItem, categories: [String]) {
        editing = task
        threadID = task.threadId
        subject = task.subject.isEmpty ? "(no subject)" : task.subject
        sender = task.senderName ?? task.senderEmail ?? ""
        self.categories = categories.contains(task.category) ? categories : categories + [task.category]
        category = task.category
        due = DueDay.date(task.dueDay) ?? Calendar.current.startOfDay(for: .now)
        hasDue = task.dueDay != nil
        action = task.action
        notes = task.notes
        why = task.why
        fromAI = task.fromAi
        applying = true
        title = task.title
        applying = false
    }

    var edit: TaskEdit {
        TaskEdit(title: title, notes: notes, category: category, dueDay: hasDue ? DueDay.string(due) : nil,
                 action: action)
    }

    /// Fill the fields from Claude's suggestion; a title the user typed stays.
    func apply(_ s: TaskSuggestion) {
        applying = true
        defer { applying = false }
        if !editedTitle || title.trimmingCharacters(in: .whitespaces).isEmpty {
            title = s.title
            fromAI = true
        }
        if !editedOther {
            category = s.category
            if let day = s.dueDay, let date = DueDay.date(day) {
                hasDue = true
                due = date
            } else {
                hasDue = false
            }
            action = s.action
        }
        why = s.why
    }

    var canAdd: Bool { !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    var newTask: NewTask {
        NewTask(threadId: threadID, messageId: nil, title: title, notes: notes, category: category,
                dueDay: hasDue ? DueDay.string(due) : nil, action: action,
                why: why, fromAi: fromAI && !editedTitle)
    }

    static let actions: [(TaskAction, String)] = [
        (.reply, "Reply"), (.replyAll, "Reply All"), (.forward, "Forward"), (.noEmail, "No Email"),
    ]
}

/// `t`: a task for the email being read, with Claude's guess filled in
/// (spec §14.8). Return adds it, Escape cancels, Ask Again asks Claude
/// once more.
struct TaskDialog: View {
    @Environment(AppModel.self) private var model
    @Bindable var draft: TaskDraft
    @FocusState private var titleFocused: Bool

    var body: some View {
        Dialog(title: draft.editing == nil ? "New Task" : "Edit Task",
               message: draft.sender.isEmpty ? draft.subject : "\(draft.sender) · \(draft.subject)") {
            status
            TextField("What to do", text: $draft.title)
                .textFieldStyle(.roundedBorder)
                .focused($titleFocused)
            ChipFlow(spacing: Space.xs) {
                ForEach(draft.categories, id: \.self) { name in
                    Button { draft.category = name } label: {
                        CategoryChip(name: name, selected: name == draft.category)
                    }
                    .buttonStyle(.plain)
                    .hoverHelp("Category: \(name)")
                    .accessibilityAddTraits(name == draft.category ? [.isSelected] : [])
                }
            }
            HStack(spacing: Space.m) {
                Toggle("Due", isOn: $draft.hasDue)
                    .hoverHelp("Give the task a due day")
                DatePicker("Due day", selection: $draft.due, displayedComponents: .date)
                    .labelsHidden()
                    .disabled(!draft.hasDue)
                    .hoverHelp("The day it should be done by")
                if draft.hasDue {
                    Text(DueDay.label(DueDay.string(draft.due)))
                        .foregroundStyle(DueDay.urgency(DueDay.string(draft.due)) == .overdue ? Tone.caution : .secondary)
                }
                Spacer(minLength: 0)
            }
            Picker("When it's done", selection: $draft.action) {
                ForEach(TaskDraft.actions, id: \.0) { action, title in Text(title).tag(action) }
            }
            .hoverHelp("What you will do with the email to finish the task")
            TextField("Notes", text: $draft.notes, axis: .vertical)
                .lineLimit(2...4)
                .textFieldStyle(.roundedBorder)
        } leading: {
            Button("Ask Again") { ask() }
                .disabled(!TaskSuggester.canAsk(model) || draft.suggester.state == .working)
                .hoverHelp("Ask \(model.agent.providerName) for another suggestion")
        } buttons: {
            CancelButton(help: "Close without adding a task (Esc)") { model.closeTaskDialog() }
            Button(draft.editing == nil ? "Add Task" : "Save") { Task { await model.acceptTask(draft) } }
                .keyboardShortcut(.defaultAction)
                .disabled(!draft.canAdd)
                .hoverHelp(draft.editing == nil ? "Add it to your tasks and label the email Task (Return)"
                                                : "Save your changes to the task (Return)")
        }
        .onAppear { titleFocused = true }
    }

    @ViewBuilder private var status: some View {
        if let error = draft.error {
            Label(error, systemImage: "exclamationmark.triangle")
                .font(TypeRole.meta)
                .foregroundStyle(Tone.failure)
        }
        switch draft.suggester.state {
        case .working:
            HStack(spacing: Space.s) {
                ProgressView().controlSize(.small)
                Text("Asking \(model.agent.providerName)…").foregroundStyle(.secondary)
            }
            .font(TypeRole.meta)
        case let .failed(message):
            Label(message, systemImage: "exclamationmark.triangle")
                .font(TypeRole.meta)
                .foregroundStyle(Tone.failure)
        case .done where !draft.why.isEmpty:
            Label(draft.why, systemImage: "sparkles")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
                .accessibilityLabel("\(model.agent.providerName) suggests this because: \(draft.why)")
        case .idle where !TaskSuggester.canAsk(model):
            Text("\(model.agent.providerName) is not set up, so write the task yourself.")
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
        default:
            EmptyView()
        }
    }

    private func ask() {
        Task { await draft.suggester.run(threadIDs: [draft.threadID], model: model) }
    }
}
