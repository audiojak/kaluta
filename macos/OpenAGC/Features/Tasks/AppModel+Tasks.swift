import Foundation

/// Tasks from email (spec §14.8): the dialog and what accepting does.
extension AppModel {
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
