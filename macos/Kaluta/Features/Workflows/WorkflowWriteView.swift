import SwiftUI

/// Kaluta's writing on a workflow item (spec §14.13): up to three answers,
/// or *What do you want to say?*; then the email that will be sent,
/// editable, with what the user said above it and quick changes. Every
/// action goes through `WorkflowCommand`, as the decision bar's do.
struct WorkflowWriteView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    @Bindable var writer: WorkflowWriter
    @FocusState private var fieldFocused: Bool
    @FocusState private var textFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            switch writer.step {
            case .choose: choose
            case .say: say
            case .writing: working(writer.directions.count > 1 ? "Kaluta is changing it…" : "Kaluta is writing…")
            case .final: finalEmail
            case .sending: working("Sending…")
            }
            if let problem = writer.problem {
                Label(problem, systemImage: "exclamationmark.circle")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .id(Self.scrollID)
        .background {
            // ⌘Return and ⌥⌘Return send from inside the text, too.
            Button("Send") { perform(.send) } // no-help: hidden
                .keyboardShortcut(.return, modifiers: .command)
                .hidden()
            Button("Send, keep the task open") { perform(.sendKeepTask) } // no-help: hidden
                .keyboardShortcut(.return, modifiers: [.command, .option])
                .hidden()
        }
        .onChange(of: writer.step, initial: true) { _, step in
            // The field or the text takes the keys, a turn later (focus set
            // while a view appears is lost to the one it replaces).
            Task { @MainActor in
                fieldFocused = step == .say
                textFocused = step == .final
            }
        }
    }

    // MARK: Steps

    private var choose: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text("Kaluta's answers").font(TypeRole.workflowHeading)
            ForEach(Array(writer.answers.enumerated()), id: \.element.id) { i, answer in
                Button { perform([.pick1, .pick2, .pick3][i]) } label: {
                    HStack(alignment: .top, spacing: Space.l) {
                        Keycap(key: "\(i + 1)")
                        VStack(alignment: .leading, spacing: Space.xs) {
                            Text(answer.label).font(TypeRole.workflowHeading)
                            Text(Self.preview(answer.text))
                                .font(TypeRole.workflowContext)
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(Space.xl)
                    .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                    .contentShape(RoundedRectangle(cornerRadius: Radius.panel))
                }
                .buttonStyle(.plain)
                .hoverHelp("Use this answer (\(i + 1))")
            }
            Button { perform(.tellKaluta) } label: {
                HStack(spacing: Space.l) {
                    Keycap(key: "/")
                    Text("None of these? Tell Kaluta what to say…").font(TypeRole.workflowContext)
                }
            }
            .buttonStyle(.plain)
            .hoverHelp("Say what to answer instead (/)")
        }
    }

    private var say: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Text(Self.askTitle(session.kind)).font(TypeRole.workflowHeading)
            TextField("Type it, or dictate it", text: $writer.field, axis: .vertical)
                .font(TypeRole.workflowContent)
                .textFieldStyle(.plain)
                .lineLimit(2...6)
                .padding(Space.xl)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                .focused($fieldFocused)
                .onSubmit { perform(.primary) }
                .onExitCommand { perform(.back) }
                .accessibilityLabel(Self.askTitle(session.kind))
            if !writer.starters.isEmpty {
                HStack(spacing: Space.s) {
                    ForEach(writer.starters, id: \.self) { starter in
                        Button(starter) { writer.addStarter(starter) }
                            .buttonStyle(.bordered)
                            .hoverHelp("Start with “\(starter)”")
                    }
                }
                .controlSize(.large)
            }
        }
    }

    private var finalEmail: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            HStack(spacing: Space.m) {
                Text(Self.saidLine(writer.said))
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                Button("Change") { perform(.back) }
                    .buttonStyle(.link)
                    .hoverHelp("Change what you told Kaluta (Esc)")
            }
            TextEditor(text: $writer.text)
                .font(TypeRole.workflowContent)
                .scrollContentBackground(.hidden)
                .frame(minHeight: Self.editorHeight)
                .padding(Space.xl)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                .focused($textFocused)
                .accessibilityLabel("The email that will be sent")
            if model.agent.isProviderReady {
                HStack(spacing: Space.s) {
                    Button("Shorter") { revise("Make it shorter") }
                        .hoverHelp("Ask Kaluta to make it shorter")
                    Button("Warmer") { revise("Make it warmer") }
                        .hoverHelp("Ask Kaluta to make it warmer")
                    TextField("Tell Kaluta what to change…", text: $writer.field)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit { revise(writer.field) }
                        .frame(maxWidth: Self.changeWidth)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
            }
        }
    }

    private func working(_ title: String) -> some View {
        HStack(spacing: Space.m) {
            ProgressView().controlSize(.small)
            Text(title).font(TypeRole.workflowContext).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: Acting

    private func perform(_ command: WorkflowCommand) {
        Task { await model.performWorkflow(command) }
    }

    private func revise(_ request: String) {
        Task { await writer.revise(request, model: model) }
    }

    /// An answer on one line: its lines run together.
    static func preview(_ text: String) -> String {
        text.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }.joined(separator: " ")
    }

    /// Above the final email: what the user said, or which answer they chose.
    static func saidLine(_ said: String) -> String {
        if said.isEmpty { return "Written by you" }
        if said.hasPrefix("Chose: ") { return "You chose: " + said.dropFirst("Chose: ".count) }
        return "You said: \(said)"
    }

    /// A task's reply asks what the user decided (spec §14.13).
    static func askTitle(_ kind: WorkflowKind) -> String {
        kind == .tasks ? "What did you decide?" : "What do you want to say?"
    }

    static let editorHeight: CGFloat = 180
    static let scrollID = "workflow-writing"
    static let changeWidth: CGFloat = 320
}
