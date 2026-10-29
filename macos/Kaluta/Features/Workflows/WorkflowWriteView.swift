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
    @FocusState private var toFocused: Bool
    /// Which of the names offered ↑ and ↓ have reached; Return takes it.
    @State private var pick = 0

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            switch writer.step {
            case .choose: choose
            case .say: say
            case .asking: asking
            case .writing: working(writer.directions.count > 1 ? "Kaluta is changing it…" : "Kaluta is writing…")
            case .final: finalEmail
            case .sending: working("Sending…")
            case .forward: forward
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
            if writer.step == .say || writer.step == .asking || writer.step == .final || writer.step == .forward {
                Button("Back") { perform(.back) } // no-help: hidden
                    .keyboardShortcut(.escape, modifiers: [])
                    .hidden()
                // ⇧Return from inside the answer: skip the question.
                if writer.step == .asking {
                    Button("Skip the question") { perform(.skipQuestion) } // no-help: hidden
                        .keyboardShortcut(.return, modifiers: .shift)
                        .hidden()
                }
                // ⇧⌘M from inside the field: the words as written.
                Button("Use my words") { perform(.ownWords) } // no-help: hidden
                    .keyboardShortcut("m", modifiers: [.command, .shift])
                    .hidden()
                if writer.step == .final {
                    Button("Attach a file") { perform(.attach) } // no-help: hidden
                        .keyboardShortcut("a", modifiers: [.command, .shift])
                        .hidden()
                }
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
            guard step == .say || step == .asking || step == .final || step == .forward else { return }
            for _ in 0..<10 {
                try? await Task.sleep(for: .milliseconds(60))
                guard !Task.isCancelled else { return }
                fieldFocused = step == .say || step == .asking
                textFocused = step == .final
                toFocused = step == .forward
                if WorkflowHostView.isEditingText { return }
            }
        }
    }

    // MARK: Steps

    /// Forward: whom to (names from the mailbox as they are typed), and a
    /// note; the email as it was sent goes below the note.
    private var forward: some View {
        let typed = writer.recipientText.trimmingCharacters(in: .whitespaces)
        let suggestions = typed.isEmpty ? [] : (model.core?.suggestContactsNow(typed, limit: 5) ?? [])
            .filter { s in !writer.recipients.contains { $0.email.lowercased() == s.email.lowercased() } }
        return VStack(alignment: .leading, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text("Forward").font(TypeRole.workflowHeading)
                Text("The email goes as it was sent, under your note if you add one.")
                    .font(TypeRole.workflowContext).foregroundStyle(.secondary)
            }
            VStack(alignment: .leading, spacing: Space.s) {
                HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                    Text("To").font(TypeRole.workflowContext).foregroundStyle(.secondary)
                    ForEach(writer.recipients, id: \.email) { r in
                        HStack(spacing: Space.xs) {
                            Text(r.name?.isEmpty == false ? r.name! : r.email).lineLimit(1)
                            Button { writer.removeRecipient(r.email) } label: {
                                Image(systemName: "xmark").font(TypeRole.workflowKey)
                            }
                            .buttonStyle(.plain)
                            .hoverHelp("Remove \(r.email)")
                            .accessibilityLabel("Remove \(r.email)")
                        }
                        .font(TypeRole.workflowContext)
                        .padding(.horizontal, Space.m).padding(.vertical, Space.hair)
                        .background(Tone.workflow(session.kind).opacity(0.14), in: Capsule())
                    }
                    TextField(writer.recipients.isEmpty ? "Name or address" : "Add another", text: $writer.recipientText)
                        .font(TypeRole.workflowContent)
                        .textFieldStyle(.plain)
                        .focused($toFocused)
                        .onSubmit {
                            // The name reached with ↑ and ↓ (the first, to
                            // start), else what was typed.
                            if !suggestions.isEmpty {
                                take(suggestions[min(pick, suggestions.count - 1)])
                            } else if writer.takeTypedRecipient() {
                                focusNote()
                            }
                        }
                        .onKeyPress(.downArrow) {
                            guard !suggestions.isEmpty else { return .ignored }
                            pick = (pick + 1) % suggestions.count
                            return .handled
                        }
                        .onKeyPress(.upArrow) {
                            guard !suggestions.isEmpty else { return .ignored }
                            pick = (pick + suggestions.count - 1) % suggestions.count
                            return .handled
                        }
                        .onChange(of: writer.recipientText) { pick = 0 }
                        // Tab: on to the note, taking an address typed.
                        .onKeyPress(.tab, phases: .down) { press in
                            guard !press.modifiers.contains(.shift) else { return .ignored }
                            _ = writer.takeTypedRecipient()
                            focusNote()
                            return .handled
                        }
                        .onExitCommand { perform(.back) }
                        .accessibilityLabel("To")
                }
                .padding(Space.xl)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                if !suggestions.isEmpty {
                    HStack(spacing: Space.s) {
                        // Names, with the address on hover; the address when
                        // names repeat or there is none.
                        let names = Dictionary(grouping: suggestions, by: { $0.name ?? "" })
                        ForEach(Array(suggestions.enumerated()), id: \.element.email) { i, s in
                            let name = s.name ?? ""
                            let reached = i == min(pick, suggestions.count - 1)
                            Button { take(s) } label: {
                                Text(name.isEmpty || names[name]?.count ?? 0 > 1 ? s.email : name).lineLimit(1)
                            }
                            .buttonStyle(.bordered)
                            .tint(reached ? Tone.workflow(session.kind) : nil)
                            .hoverHelp("Forward to \(name.isEmpty ? s.email : "\(name) (\(s.email))") (↑↓ then Return)")
                            .accessibilityAddTraits(reached ? .isSelected : [])
                        }
                        Text("↑↓ then Return").foregroundStyle(.secondary).fixedSize()
                    }
                    .font(TypeRole.workflowContext)
                }
            }
            HStack(alignment: .top, spacing: Space.m) {
                TextField("Add a note (optional)", text: $writer.field, axis: .vertical)
                    .font(TypeRole.workflowContent)
                    .textFieldStyle(.plain)
                    .lineLimit(2...6)
                    .focused($fieldFocused)
                    .onSubmit { perform(.primary) }
                    // ⇧Tab: back to To.
                    .onKeyPress(.tab, phases: .down) { press in
                        guard press.modifiers.contains(.shift) else { return .ignored }
                        toFocused = true
                        return .handled
                    }
                    .onExitCommand { perform(.back) }
                    .accessibilityLabel("Note")
                Button { startDictation() } label: {
                    Image(systemName: "mic").font(TypeRole.workflowContent).foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .hoverHelp("Dictate the note")
                .accessibilityLabel("Dictate")
            }
            .padding(Space.xl)
            .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
        }
    }

    /// A recipient chosen: added, and the note takes the keys.
    private func take(_ address: AddressInfo) {
        writer.addRecipient(address)
        writer.recipientText = ""
        focusNote()
    }

    /// The suggestions row goes as the name is taken, so the focus asked
    /// for in the same turn can be lost: ask until the note has it.
    private func focusNote() {
        Task { @MainActor in
            for _ in 0..<8 {
                fieldFocused = true
                try? await Task.sleep(for: .milliseconds(50))
                if fieldFocused { return }
            }
        }
    }

    private var choose: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text("Kaluta's answers").font(TypeRole.workflowHeading)
                Text(Self.answersNote(byWriter: writer.answersByWriter))
                    .font(TypeRole.workflowContext).foregroundStyle(.secondary)
            }
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
                    // 1–3 say an answer, while nothing is typed.
                    .onKeyPress(characters: .decimalDigits) { press in
                        guard writer.field.isEmpty, let digit = press.characters.first?.wholeNumberValue,
                              (1...writer.starters.count).contains(digit), !writer.ownWordsOnly else { return .ignored }
                        perform([.pick1, .pick2, .pick3][digit - 1])
                        return .handled
                    }
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
            // The answers the email takes (1–3): say one and Kaluta writes
            // from it. Writing it oneself, there are none.
            if !writer.starters.isEmpty, !writer.ownWordsOnly {
                HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                    Text("Or just say")
                        .font(TypeRole.workflowContext)
                        .foregroundStyle(.secondary)
                    HStack(spacing: Space.s) {
                        ForEach(Array(writer.starters.enumerated()), id: \.element) { i, starter in
                            Button { perform([.pick1, .pick2, .pick3][i]) } label: {
                                HStack(spacing: Space.s) {
                                    Keycap(key: "\(i + 1)")
                                    Text(starter)
                                }
                            }
                            .buttonStyle(.bordered)
                            .hoverHelp(model.agent.isProviderReady
                                       ? "Say “\(starter)”: Kaluta writes the reply from it (\(i + 1))"
                                       : "Start with “\(starter)” (\(i + 1))")
                        }
                    }
                    .controlSize(.large)
                }
            }
        }
    }

    /// Kaluta asks before it writes (spec §14.9): one question at a time,
    /// in the workflow's colour, with the answers the thread offers to
    /// pick from (1–3 with nothing typed). Return answers, ⇧Return skips.
    private var asking: some View {
        let question = writer.question
        let choices = question?.choices ?? []
        let tint = Tone.workflow(session.kind)
        return VStack(alignment: .leading, spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.xs) {
                HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                    Label("Kaluta asks", systemImage: "questionmark.circle")
                        .font(TypeRole.workflowContext.weight(.semibold))
                        .foregroundStyle(tint)
                    if writer.factQuestions.count > 1 {
                        Text("\(writer.questionIndex + 1) of \(writer.factQuestions.count)")
                            .font(TypeRole.workflowContext)
                            .foregroundStyle(.secondary)
                    }
                }
                Text(question?.question ?? "")
                    .font(TypeRole.workflowHeading)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Kaluta never makes a fact up. Skip what you would rather not say; it leaves a [blank] for it.")
                    .font(TypeRole.workflowContext).foregroundStyle(.secondary)
            }
            HStack(alignment: .top, spacing: Space.m) {
                TextField(choices.isEmpty ? "Your answer, then Return" : "Pick one, or type your answer", text: $writer.field, axis: .vertical)
                    .font(TypeRole.workflowContent)
                    .textFieldStyle(.plain)
                    .lineLimit(1...4)
                    .focused($fieldFocused)
                    .onSubmit { perform(.primary) }
                    .onExitCommand { perform(.back) }
                    // 1–3 pick an answer offered, while nothing is typed.
                    .onKeyPress(characters: .decimalDigits) { press in
                        guard writer.field.isEmpty, let digit = press.characters.first?.wholeNumberValue,
                              (1...choices.count).contains(digit) else { return .ignored }
                        perform([.pick1, .pick2, .pick3][digit - 1])
                        return .handled
                    }
                    .accessibilityLabel(question?.question ?? "Kaluta's question")
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
            if !choices.isEmpty {
                HStack(spacing: Space.s) {
                    ForEach(Array(choices.enumerated()), id: \.element) { i, choice in
                        Button { perform([.pick1, .pick2, .pick3][i]) } label: {
                            HStack(spacing: Space.s) {
                                Keycap(key: "\(i + 1)")
                                Text(choice)
                            }
                        }
                        .buttonStyle(.bordered)
                        .hoverHelp("Answer “\(choice)” (\(i + 1))")
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
            // Under the email, its width: a file to go with it, quick changes,
            // and what else to change (Return asks).
            HStack(spacing: Space.s) {
                Button("Attach a file", systemImage: "paperclip") { writer.choosingFiles = true }
                    .labelStyle(.iconOnly)
                    .hoverHelp("Attach a file, or drop one on the email (⇧⌘A)")
                if model.agent.isProviderReady {
                    Button("Shorter") { revise("Make it shorter") }
                        .hoverHelp("Ask Kaluta to make it shorter")
                    Button("Warmer") { revise("Make it warmer") }
                        .hoverHelp("Ask Kaluta to make it warmer")
                    TextField("Tell Kaluta what to change…", text: $writer.field)
                        .font(TypeRole.workflowContext)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit { revise(writer.field) }
                        .frame(maxWidth: .infinity)
                        .accessibilityLabel("Tell Kaluta what to change")
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.large)
            if !writer.attachments.isEmpty {
                DraftAttachmentChips(attachments: writer.attachments, remove: writer.removeAttachment)
            }
        }
        .attachingFiles(choosing: $writer.choosingFiles, attach: writer.attach)
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
        Task { @MainActor in
            // The field must hold the keyboard first: asked for until it
            // does (the host gives the keys up a turn later).
            for _ in 0..<8 {
                fieldFocused = true
                try? await Task.sleep(for: .milliseconds(50))
                if WorkflowHostView.isEditingText { break }
            }
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

    /// Under *Kaluta's answers*: how they were written.
    static func answersNote(byWriter: Bool) -> String {
        byWriter ? "Written ahead with your writing guide. Pick one to read and change before it goes."
            : "Pick one to read and change before it goes."
    }

    /// Above the final email: what the user said, or which answer they chose.
    static func saidLine(_ said: String) -> String {
        if said.isEmpty { return "Written by you" }
        if said == WorkflowWriter.firstPassInstruction { return "Kaluta's first pass" }
        if said.hasPrefix("Chose: ") { return "You chose: " + said.dropFirst("Chose: ".count) }
        return "You said: \(said)"
    }

    /// A task's reply asks what the user decided (spec §14.13).
    static func askTitle(_ kind: WorkflowKind) -> String {
        kind == .tasks ? "What did you decide?" : "What do you want to say?"
    }

    static let editorHeight: CGFloat = 180
    static let scrollID = "workflow-writing"
}
