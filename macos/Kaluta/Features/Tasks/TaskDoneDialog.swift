import SwiftUI

/// What the app asks after a task's reply is sent (spec §14.8).
struct TaskDoneQuestion: Identifiable {
    let task: TaskItem
    let accountID: String
    /// The message is held for Undo Send.
    let held: Bool
    var id: Int64 { task.id }
}

/// "Mark the task done?": Y or Return marks it done, N or Escape keeps it
/// open.
struct TaskDoneDialog: View {
    @Environment(AppModel.self) private var model
    let question: TaskDoneQuestion

    var body: some View {
        Dialog(title: "Mark the Task Done?", message: "You sent the reply for “\(question.task.title)”.") {
            EmptyView()
        } buttons: {
            Button("Keep Open") { answer(false) }
                .keyboardShortcut(.cancelAction)
                .hoverHelp("Leave the task open (N or Esc)")
            Button("Mark Done") { answer(true) }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Finish the task; Undo brings it back (Y or Return)")
        }
        .background {
            // Y and N answer too.
            Button("Yes") { answer(true) }.keyboardShortcut("y", modifiers: []).hidden() // no-help: hidden
            Button("No") { answer(false) }.keyboardShortcut("n", modifiers: []).hidden() // no-help: hidden
        }
    }

    private func answer(_ done: Bool) {
        Task { await model.answerTaskDone(done) }
    }
}
