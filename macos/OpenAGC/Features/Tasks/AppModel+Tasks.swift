import Foundation

/// Tasks from email (spec §14.8): the dialog, the task list and what each
/// action does. Every change to a task can be undone.
extension AppModel {
    /// The sidebar's Tasks entry, in place of a mailbox id.
    static let tasksMailboxID = "@tasks"

    /// The list column shows tasks (and not search results).
    var isTaskList: Bool { selectedMailboxID == Self.tasksMailboxID && threads.searchQuery == nil }

    /// The task a reply or forward answers: the one selected in the task
    /// list, if it is still open.
    var answeringTaskID: Int64? {
        guard isTaskList, let task = tasks.selected, !task.done, task.threadId == selectedThreadID else { return nil }
        return task.id
    }

    /// Choose a task: the reader shows its email.
    func selectTask(_ id: Int64?) {
        tasks.selectedID = id
        selectedThreadIDs = []
        selectedThreadID = id.flatMap { id in tasks.tasks.first { $0.id == id } }?.threadId
    }

    /// `e` in the task list: done (or, among finished ones, open again).
    func toggleSelectedTaskDone() async {
        guard let task = tasks.selected, let core, let accountID = openAccountID else { return }
        let done = !task.done
        let next = tasks.neighbour(of: task.id)
        guard (try? await core.setTaskDone(task.id, done)) != nil else { return }
        await tasks.load()
        selectTask(next)
        undo.record(accountID: accountID, actionName: done ? "Complete Task" : "Reopen Task",
                    noticeText: done ? "Done: “\(task.title)”" : "Reopened “\(task.title)”",
                    undo: { _ = try? await core.setTaskDone(task.id, !done) },
                    redo: { _ = try? await core.setTaskDone(task.id, done) })
    }

    /// `⌫` in the task list.
    func deleteSelectedTask() async {
        guard let task = tasks.selected, let core, let accountID = openAccountID else { return }
        let next = tasks.neighbour(of: task.id)
        guard let deleted = try? await core.deleteTask(task.id) else { return }
        await tasks.load()
        selectTask(next)
        undo.record(accountID: accountID, actionName: "Delete Task", noticeText: "Deleted “\(task.title)”",
                    undo: { _ = try? await core.restoreTask(deleted) },
                    redo: { _ = try? await core.deleteTask(deleted.id) })
    }

    /// `c` in the task list: another category.
    func setSelectedTaskCategory(_ name: String) async {
        guard let task = tasks.selected, task.category != name else { return }
        var edit = task.edit
        edit.category = name
        await update(task, to: edit, actionName: "Change Category", notice: "Moved “\(task.title)” to \(name)")
    }

    /// Change a task, undoably.
    func update(_ task: TaskItem, to edit: TaskEdit, actionName: String, notice: String) async {
        guard let core, let accountID = openAccountID else { return }
        guard (try? await core.updateTask(task.id, edit)) != nil else { return }
        await tasks.load()
        let before = task.edit
        undo.record(accountID: accountID, actionName: actionName, noticeText: notice,
                    undo: { _ = try? await core.updateTask(task.id, before) },
                    redo: { _ = try? await core.updateTask(task.id, edit) })
    }

    /// `↩` in the task list: the task dialog, to change it.
    func editSelectedTask() async {
        guard taskDraft == nil, let task = tasks.selected, let core else { return }
        let categories = (try? await core.taskCategories()) ?? []
        taskDraft = TaskDraft(editing: task, categories: categories)
    }

    /// The thread `t` makes a task for: the one being read, else the first
    /// highlighted.
    var taskTarget: ThreadRow? {
        let id = selectedThreadID ?? actionTargets.first
        return id.flatMap { id in threads.rows.first { $0.id == id } }
    }

    /// `t` and Message › New Task from Email: open the dialog and ask
    /// Claude, or open it empty when no agent is ready.
    func openTaskDialog(now: Date = .now) async {
        guard taskDraft == nil, let row = taskTarget, let core else { return }
        let categories = (try? await core.taskCategories()) ?? []
        let draft = TaskDraft(threadID: row.id, subject: row.subject.isEmpty ? "(no subject)" : row.subject,
                              sender: ThreadRowView.senderLine(row), categories: categories, now: now)
        taskDraft = draft
        if TaskSuggester.canAsk(self) {
            await draft.suggester.run(threadIDs: [row.id], model: self, now: now)
        }
    }

    func closeTaskDialog() {
        taskDraft?.suggester.cancel()
        taskDraft = nil
    }

    /// Add the dialog's task (the core labels the email `Task`); Undo
    /// removes it again, and Redo puts it back.
    func acceptTask(_ draft: TaskDraft) async {
        guard draft.canAdd, let core, let accountID = openAccountID else { return }
        if let task = draft.editing {
            draft.suggester.cancel()
            if taskDraft === draft { taskDraft = nil }
            await update(task, to: draft.edit, actionName: "Edit Task", notice: "Changed “\(draft.edit.title)”")
            return
        }
        do {
            let made = try await core.createTasks([draft.newTask])
            draft.suggester.cancel()
            if taskDraft === draft { taskDraft = nil }
            recordTaskUndo(made, accountID: accountID)
        } catch let error as CoreClientError {
            draft.error = error.message
        } catch {
            draft.error = error.localizedDescription
        }
    }

    /// Undo for tasks just added: deleting them takes the label off with
    /// the last one; redo restores them as they were.
    func recordTaskUndo(_ made: [TaskItem], accountID: String) {
        guard let core, !made.isEmpty else { return }
        let text = made.count == 1 ? "Added a task: “\(made[0].title)”" : "Added \(made.count) tasks"
        undo.record(accountID: accountID, actionName: made.count == 1 ? "Add Task" : "Add Tasks", noticeText: text,
                    undo: { for t in made { _ = try? await core.deleteTask(t.id) } },
                    redo: { for t in made { _ = try? await core.restoreTask(t) } })
    }
}

extension TaskItem {
    /// Its editable fields.
    var edit: TaskEdit {
        TaskEdit(title: title, notes: notes, category: category, dueDay: dueDay, action: action)
    }
}

extension AppModel {
    /// Snapshots and tests: a few tasks on the demo mailbox's first
    /// threads, due at different times, one finished.
    func seedDemoTasks(now: Date = .now) async {
        guard let core else { return }
        let rows = Array(threads.rows.prefix(6))
        let plan: [(String, String, Int?, TaskAction)] = [
            ("Send Emerson the revised onboarding plan", "Reply", -2, .reply),
            ("Decide on the offsite venue", "Decide", 0, .replyAll),
            ("Find last quarter's figures for the review", "Gather Info", 0, .reply),
            ("Book a call about the contract", "Schedule", 3, .reply),
            ("Review the design proposal", "Review", 12, .noEmail),
            ("Forward the invoice to accounts", "Admin", nil, .forward),
        ]
        let calendar = Calendar.current
        let made = (try? await core.createTasks(zip(rows, plan).map { row, p in
            NewTask(threadId: row.id, messageId: nil, title: p.0, notes: "", category: p.1,
                    dueDay: p.2.flatMap { calendar.date(byAdding: .day, value: $0, to: now) }.map { DueDay.string($0) },
                    action: p.3, why: "", fromAi: true)
        })) ?? []
        if let last = made.last { _ = try? await core.setTaskDone(last.id, true) }
        await tasks.load()
    }
}
