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
            // ⌘Return and ⌥⌘Return send from inside the text, too; Enter
            // as Return.
            ForEach([KeyEquivalent.return, .enter], id: \.character) { key in
                Button("Send") { perform(.send) } // no-help: hidden
                    .keyboardShortcut(key, modifiers: .command)
                    .hidden()
                Button("Send, keep the task open") { perform(.sendKeepTask) } // no-help: hidden
                    .keyboardShortcut(key, modifiers: [.command, .option])
                    .hidden()
            }
            // Esc goes back a step and ⌘. discards, from inside the text too
            // (the field would otherwise keep Esc for itself).
            if writer.step == .say || writer.step == .final {
                Button("Back") { perform(.back) } // no-help: hidden
                    .keyboardShortcut(.escape, modifiers: [])
                    .hidden()
                // ⇧⌘M from inside the field: the words as written.
                Button("Use my words") { perform(.ownWords) } // no-help: hidden
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                    .hidden()
                Button("Discard") { perform(.discard) } // no-help: hidden
                    .keyboardShortcut(".", modifiers: .command)
                    .hidden()
            }
        }
        .task(id: writer.step) {
            // The field or the text takes the keys. Focus asked for while
            // the view is still appearing (it slides in with the item) can
            // be lost, so ask until a text view has the keyboard.
            let step = writer.step
            guard step == .say || step == .final else { return }
            for _ in 0..<10 {
                try? await Task.sleep(for: .milliseconds(60))
                guard !Task.isCancelled else { return }
                fieldFocused = step == .say
                textFocused = step == .final
                if WorkflowHostView.isEditingText { return }
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
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(writer.ownWordsOnly ? "Write your email" : Self.askTitle(session.kind))
                    .font(TypeRole.workflowHeading)
                Text(model.agent.isProviderReady && !writer.ownWordsOnly
                     ? "Kaluta writes the email from it, for you to read and change before it goes."
                     : "What you type is the email.")
                    .font(TypeRole.workflowContext).foregroundStyle(.secondary)
            }
            HStack(alignment: .top, spacing: Space.m) {
                TextField("Type it, or dictate it", text: $writer.field, axis: .vertical)
                    .font(TypeRole.workflowContent)
                    .textFieldStyle(.plain)
                    .lineLimit(2...6)
                    .focused($fieldFocused)
                    .onSubmit { perform(.primary) }
                    .onExitCommand { perform(.back) }
                    .accessibilityLabel(writer.ownWordsOnly ? "Write your email" : Self.askTitle(session.kind))
                // macOS dictation into the field, as Edit › Start Dictation
                // (or the dictation key) does.
                Button { startDictation() } label: {
                    Image(systemName: "mic")
                        .font(TypeRole.workflowContent)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .hoverHelp("Dictate (or press the dictation key, by default Fn twice)")
                .accessibilityLabel("Dictate")
            }
            .padding(Space.xl)
            .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
            // Starters say what to tell Kaluta; writing it oneself, there is none.
            if !writer.starters.isEmpty, !writer.ownWordsOnly {
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
            if let to = recipients, !to.isEmpty {
                Text("To: \(to)")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .accessibilityLabel("It goes to \(to)")
            }
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

    /// Put the cursor in the field and start macOS dictation there.
    private func startDictation() {
        fieldFocused = true
        Task { @MainActor in
            // A turn later: the field must hold the keyboard first.
            NSApp.sendAction(Selector(("startDictation:")), to: nil, from: nil)
        }
    }

    /// Who the email goes to: the sender, and everyone on it with Reply all.
    private var recipients: String? {
        guard let detail = session.threads[writer.threadID],
              let message = detail.messages.first(where: { $0.id == writer.messageID })
                ?? AppModel.shownMessage(detail, session.kind) else { return nil }
        return AppModel.replyRecipients(message, all: writer.replyAll, own: model.ownAddresses)
            .map { $0.name?.isEmpty == false ? $0.name! : $0.email }
            .joined(separator: ", ")
    }

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
