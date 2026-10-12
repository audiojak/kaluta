import SwiftUI

/// A task made (or the conversation's task shown) on the item's own screen
/// in Triage (spec §14.13): the fields filled in from Kaluta's suggestion,
/// editable, with *Archive the email* ticked (it stays under the `Task`
/// label). Return makes it, Esc puts it away.
@MainActor
@Observable
final class WorkflowTaskForm {
    let draft: TaskDraft
    var archiveEmail = true

    init(draft: TaskDraft) {
        self.draft = draft
    }

    /// Showing the conversation's open task, not making a second.
    var isExisting: Bool { draft.editing != nil }
}

struct WorkflowTaskFormView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let form: WorkflowTaskForm
    @FocusState private var titleFocused: Bool

    var body: some View {
        @Bindable var draft = form.draft
        @Bindable var form = form
        VStack(alignment: .leading, spacing: Space.l) {
            Text(form.isExisting ? "This conversation's task" : "The task").font(TypeRole.workflowHeading)
            TextField("What to do", text: $draft.title)
                .font(TypeRole.workflowContent)
                .textFieldStyle(.plain)
                .padding(Space.xl)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                .focused($titleFocused)
                .onSubmit { perform(.primary) }
                .onExitCommand { perform(.back) }
                .accessibilityLabel("What to do")
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
            HStack(spacing: Space.l) {
                Toggle("Due", isOn: $draft.hasDue)
                    .hoverHelp("Give the task a due day")
                DatePicker("Due day", selection: $draft.due, displayedComponents: .date)
                    .labelsHidden()
                    .disabled(!draft.hasDue)
                    .hoverHelp("The day it should be done by")
                Picker("When it's done", selection: $draft.action) {
                    ForEach(TaskDraft.actions, id: \.0) { action, title in Text(title).tag(action) }
                }
                .fixedSize()
                .hoverHelp("What you will do with the email to finish the task")
                Spacer(minLength: 0)
            }
            .font(TypeRole.workflowContext)
            Toggle("Archive the email", isOn: $form.archiveEmail)
                .font(TypeRole.workflowContext)
                .hoverHelp("Out of the Inbox; it stays under the Task label until the task is done")
            if draft.suggester.state == .working {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text("Kaluta is filling it in…").font(TypeRole.workflowContext).foregroundStyle(.secondary)
                }
            }
            if let error = draft.error {
                Label(error, systemImage: "exclamationmark.circle")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
            }
        }
        .id(WorkflowWriteView.scrollID)
        .onAppear { Task { @MainActor in titleFocused = true } }
    }

    private func perform(_ command: WorkflowCommand) {
        Task { await model.performWorkflow(command) }
    }
}
