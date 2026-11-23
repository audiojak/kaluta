import SwiftUI

/// A composer window's content (spec §14.5): header fields, the rich-text
/// body, the message being answered (shown by default, in a pane that can
/// be resized or hidden), writing help from the agent, attachments, and
/// Send.
struct ComposerView: View {
    let request: ComposeRequest
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var store: ComposerStore?
    @State private var showsQuote = true
    @State private var importing = false
    /// Discarding cannot be undone: asked first when the draft has anything in it.
    @State private var confirmingDiscard = false
    @State private var choosingTime = false
    @State private var calendarConnected = false
    @State private var assistant = ComposerAssistant()
    @State private var rewriter = SelectionRewriter()
    @State private var formatting = RichTextCommands()
    @FocusState private var assistantFocused: Bool
    @State private var factAnswers: [String: String] = [:]

    var body: some View {
        Group {
            if let store {
                switch store.phase {
                case .loading:
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                case let .failed(message):
                    ContentUnavailableView("Can't Open Message", systemImage: "exclamationmark.triangle",
                                           description: Text(message))
                default:
                    editor(store)
                }
            } else {
                // Waiting for the account to open (a composer restored at launch).
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 520, minHeight: 380)
        .navigationTitle(store?.windowTitle ?? "New Message")
        .task(id: model.openAccountID) {
            // Restored by macOS at launch, a composer can appear before the
            // account opens: it waits for it rather than failing.
            guard store == nil, model.openAccountID != nil || model.accountState != .starting else { return }
            let store = ComposerStore(core: model.core, account: model.openAccountID)
            self.store = store
            await store.load(request)
            // The open account's tasks: a composer pinned to another
            // account answers none of them.
            if store.accountID == nil || store.accountID == model.openAccountID,
               let open = try? await model.core?.listTasks() {
                store.findTask(in: open)
            }
            if model.agent.providers.isEmpty { await model.agent.loadProviders() }
            calendarConnected = (try? await model.core?.calendarStatus())?.connected == true
            await assistant.loadAudiences(model.core)
        }
        .onChange(of: store?.phase) { _, phase in
            guard phase == .sent else { return }
            if let store, let scheduled = store.scheduledNow, let account = store.accountID ?? model.openAccountID {
                model.sendScheduled(draftID: store.draftID, at: scheduled.at, before: scheduled.before, accountID: account)
            }
            if let store, store.didSend, let account = store.accountID ?? model.openAccountID {
                Task {
                    await model.messageSent(heldDraftID: store.heldSend, taskID: store.taskID,
                                            finishTask: store.task != nil && !store.keepsTask,
                                            archiveThreadID: store.archivesThread ? store.threadID : nil,
                                            accountID: account)
                }
            }
            dismiss()
        }
        .onDisappear {
            guard let store else { return }
            Task {
                await store.close()
                if model.selectedMailboxID == "DRAFT" { await model.loadKalutaWritten() }
            }
        }
    }

    /// Send: an agent's proposed send is approved with the draft as edited
    /// here (the agent sends it); anything else sends as usual.
    private func send(_ store: ComposerStore) {
        Task {
            if let action = request.agentAction {
                await store.handToAgent()
                if store.phase == .sent { model.agent.resolve(action, approve: true) }
            } else {
                await store.send()
            }
        }
    }

    /// *Send and Archive* is offered on a reply in the open account's
    /// mailbox that no agent proposed (the agent sends those itself).
    private func canSendAndArchive(_ store: ComposerStore) -> Bool {
        store.threadID != nil && request.agentAction == nil && request.agentName == nil
            && (store.accountID ?? model.openAccountID) == model.openAccountID
    }

    /// Discard: an agent's proposed send is rejected with it.
    private func discard(_ store: ComposerStore) async {
        if let action = request.agentAction { model.agent.resolve(action, approve: false) }
        await store.discard()
    }

    /// The task this reply answers, and the two ways to send it (spec
    /// §14.13): finish the task, or keep it open.
    private func taskBar(_ task: TaskItem, _ store: ComposerStore) -> some View {
        HStack(spacing: Space.s) {
            Image(systemName: "checklist")
                .foregroundStyle(.secondary)
                .accessibilityHidden(true)
            Text("Answers the task “\(task.title)”")
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: Space.s)
            Button("Send, Keep Task Open") { Task { await store.send(keepTask: true) } }
                .disabled(!store.canSend)
                .hoverHelp("Send the message; the task stays open (⌥⌘Return)")
            Button("Send and Finish Task") { Task { await store.send() } }
                .buttonStyle(.borderedProminent)
                .disabled(!store.canSend)
                .hoverHelp("Send the message and finish the task; Undo brings it back (⌘Return)")
        }
        .controlSize(.small)
        .padding(.horizontal, Space.m)
        .padding(.vertical, Space.xs)
        .background(.bar)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Task")
    }

    private func editor(_ store: ComposerStore) -> some View {
        @Bindable var store = store
        return VStack(spacing: 0) {
            if let agent = request.agentName {
                Banner("Created by \(agent). Edit it if you like, then send it.",
                       systemImage: "sparkles", intent: .info)
            }
            // An agent mailbox: what its service lets it send (spec §7.9).
            if let limits = model.agentLimits(store.accountID ?? model.openAccountID) {
                Banner(limits, systemImage: "person.badge.shield.checkmark", intent: .info)
            }
            header(store)
                .background {
                    // ⌘Return sends too, as in Gmail; with a task, ⌘Return
                    // finishes it and ⌥⌘Return keeps it open. Enter as Return.
                    ForEach([KeyEquivalent.return, .enter], id: \.character) { key in
                        Button("Send") { send(store) } // no-help: hidden
                            .keyboardShortcut(key, modifiers: .command)
                            .disabled(!store.canSend)
                            .hidden()
                        if canSendAndArchive(store) {
                            // ⇧⌘Return: the reply goes and its conversation
                            // leaves the Inbox, as in a workflow.
                            Button("Send and Archive") { Task { await store.send(archive: true) } } // no-help: hidden
                                .keyboardShortcut(key, modifiers: [.command, .shift])
                                .disabled(!store.canSend)
                                .hidden()
                        }
                        if request.agentName == nil {
                            Button("Send, Keep Task Open") { Task { await store.send(keepTask: true) } } // no-help: hidden
                                .keyboardShortcut(key, modifiers: [.command, .option])
                                .disabled(!store.canSend || store.task == nil)
                                .hidden()
                        }
                    }
                    // ⌘J: Kaluta rewrites the selection (or the paragraph) in place.
                    Button("Rewrite Selection") { // no-help: hidden
                        if let textView = formatting.textView { rewriter.present(in: textView, store: store, model: model) }
                    }
                    .keyboardShortcut("j", modifiers: .command)
                    .hidden()
                }
            if let task = store.task, request.agentName == nil {
                taskBar(task, store)
            }
            if let at = store.scheduledAt {
                Banner("Sends \(AppModel.reminderTime(at: at).phrase), if Kaluta is running then.",
                       systemImage: "clock", intent: .info) {
                    Button("Don't Send Later") { Task { await store.unschedule() } }
                        .hoverHelp("Keep it as an ordinary draft")
                }
            }
            if let error = store.saveError {
                Banner(error, systemImage: "exclamationmark.triangle.fill", intent: .caution)
            }
            if let threadID = store.threadID, store.inReplyTo != nil {
                // A reply: the conversation above it, the message being
                // answered open and the others as rows; the divider draggable.
                if showsQuote {
                    VSplitView {
                        VStack(spacing: 0) {
                            ComposerThreadPane(threadID: threadID, answering: store.inReplyTo)
                            conversationToggle
                        }
                        .frame(minHeight: 160, idealHeight: 360, maxHeight: .infinity)
                        .layoutPriority(1)
                        bodyEditor(store)
                            .frame(minHeight: 180, idealHeight: 260, maxHeight: .infinity)
                    }
                } else {
                    conversationToggle
                    bodyEditor(store)
                        .frame(maxHeight: .infinity)
                }
            } else if !store.quotedHTML.isEmpty, showsQuote {
                // The editor and the original, the divider between them
                // draggable.
                VSplitView {
                    bodyEditor(store)
                        .frame(minHeight: 120, maxHeight: .infinity)
                    quote(store)
                        .frame(minHeight: 90, idealHeight: 260, maxHeight: .infinity)
                }
            } else {
                bodyEditor(store)
                    .frame(maxHeight: .infinity)
                if !store.quotedHTML.isEmpty {
                    quoteToggle
                }
            }
            rewriteStatus
            assistantBar(store)
            if !store.attachments.isEmpty {
                attachmentStrip(store)
            }
        }
        .disabled(store.phase == .sending)
        .confirmationDialog("Discard this draft?", isPresented: $confirmingDiscard) {
            Button("Discard Draft", role: .destructive) { Task { await discard(store) } } // no-help: dialog
        } message: {
            Text("It is deleted here and in Gmail's drafts, with its attachments. This cannot be undone.")
        }
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                Button("Attach", systemImage: "paperclip") { importing = true }
                    .keyboardShortcut("a", modifiers: [.command, .shift])
                    .help(ToolbarHelp.composer("Attach")) // toolbar
                // Reviewing an agent's draft: Send approves its proposed
                // send with the edits; Discard rejects it (spec §10.4).
                Button("Discard", systemImage: "trash") {
                    if store.hasContent { confirmingDiscard = true } else { Task { await discard(store) } }
                }
                .help(ToolbarHelp.composer("Discard")) // toolbar
                Button("Insert Free Times", systemImage: "calendar") { Task { await insertFreeTimes() } }
                    .disabled(!calendarConnected)
                    .help(ToolbarHelp.composer("Insert Free Times")) // toolbar
                if request.agentAction == nil {
                    Button("Send Later", systemImage: "clock") { choosingTime = true }
                        .keyboardShortcut("d", modifiers: [.command, .option, .shift])
                        .disabled(!store.canSend)
                        .help(ToolbarHelp.composer("Send Later")) // toolbar
                        .popover(isPresented: $choosingTime, arrowEdge: .bottom) {
                            TimePickerView(title: "Send when?", presets: AppModel.reminderTimes(), verb: "Send") { time in
                                choosingTime = false
                                Task { await store.sendLater(at: time.date) }
                            }
                        }
                }
                if canSendAndArchive(store) {
                    Button("Send and Archive", systemImage: "archivebox") { Task { await store.send(archive: true) } }
                        .disabled(!store.canSend)
                        .help(ToolbarHelp.composer("Send and Archive")) // toolbar
                }
                Button(store.task == nil || request.agentName != nil ? "Send" : "Send and Finish Task",
                       systemImage: "paperplane.fill") {
                    send(store)
                }
                .keyboardShortcut("d", modifiers: [.command, .shift])
                .disabled(!store.canSend)
                .help(ToolbarHelp.composer(store.task == nil || request.agentName != nil ? "Send" : "Send and Finish Task")) // toolbar
            }
        }
        .attachingFiles(choosing: $importing, attach: store.attach)
    }

    private func header(_ store: ComposerStore) -> some View {
        @Bindable var store = store
        let suggest: (String) -> [AddressInfo] = { [drafts = store.drafts] text in drafts?.suggestContactsNow(text) ?? [] }
        return VStack(spacing: 0) {
            row("To:") {
                // With no one to write to yet (a new message, a forward), the
                // cursor starts here; a reply starts in the body.
                RecipientField(addresses: $store.to, suggest: suggest, accessibilityLabel: "To",
                               focusOnAppear: store.to.isEmpty)
                if !store.showsCcBcc {
                    Button("Cc/Bcc") { store.showsCcBcc = true }
                        .hoverHelp("Add Cc and Bcc fields")
                        .buttonStyle(.link)
                        .font(TypeRole.meta)
                }
            }
            if store.showsCcBcc {
                row("Cc:") { RecipientField(addresses: $store.cc, suggest: suggest, accessibilityLabel: "Cc") }
                row("Bcc:") { RecipientField(addresses: $store.bcc, suggest: suggest, accessibilityLabel: "Bcc") }
            }
            row("Subject:") {
                TextField("", text: $store.subject)
                    .textFieldStyle(.plain)
                    .accessibilityLabel("Subject")
            }
            row("From:") {
                Text(store.from).foregroundStyle(.secondary)
                Spacer()
            }
        }
    }

    /// The message body; Tab goes to writing help, as Return goes to a new
    /// line (spec §14.5).
    /// In a frame of its own, with the formatting bar under it as in Gmail,
    /// so it reads as the place to type.
    private func bodyEditor(_ store: ComposerStore) -> some View {
        @Bindable var store = store
        return VStack(alignment: .leading, spacing: 0) {
            RichTextEditor(text: $store.body, focusOnAppear: !store.to.isEmpty, onTab: { assistantFocused = true },
                           commands: formatting,
                           onSlash: { textView, at in
                               Task {
                                   let snippets = (try? await model.core?.snippets()) ?? []
                                   SnippetPopover.show(in: textView, slashAt: at, snippets: snippets,
                                                       recipient: store.to.first)
                               }
                           },
                           onSaveSnippet: { selection in Task { await model.saveSnippet(from: selection) } })
            FormattingBar(commands: formatting)
                .padding(Space.m)
        }
        .background(.background, in: .rect(cornerRadius: Radius.panel))
        .overlay(RoundedRectangle(cornerRadius: Radius.panel).strokeBorder(.separator))
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
    }

    /// Shows or hides the conversation above a reply.
    private var conversationToggle: some View {
        HStack {
            Button {
                showsQuote.toggle()
            } label: {
                Label(showsQuote ? "Hide Conversation" : "Show Conversation",
                      systemImage: showsQuote ? "chevron.up" : "chevron.down")
                    .font(TypeRole.meta)
            }
            .hoverHelp(showsQuote ? "Hide the conversation you are answering" : "Show the conversation you are answering")
            .buttonStyle(.borderless)
            Spacer()
        }
        .padding(.horizontal, Space.xl)
        .padding(.vertical, Space.s)
        .overlay(alignment: .bottom) { InsetRule() }
    }

    private func row(_ label: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(spacing: 0) {
            // Top, not first-baseline: asking the recipient token field for
            // its baseline while the user typed made it re-tokenize and ask
            // for layout again, forever (the app hung and ran out of memory
            // forwarding a message, 2026-09-28).
            HStack(alignment: .top, spacing: Space.m) {
                Text(label)
                    .foregroundStyle(.secondary)
                    .frame(width: 64, alignment: .trailing)
                    .padding(.top, Space.hair)
                content()
            }
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.s)
            InsetRule()
        }
    }

    /// The original, under the editor.
    private func quote(_ store: ComposerStore) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            quoteToggle
            MessageWebView(html: Self.quoteDocument(store.quotedHTML), allowRemoteImages: false)
        }
    }

    private var quoteToggle: some View {
        HStack {
            Button {
                showsQuote.toggle()
            } label: {
                Label(showsQuote ? "Hide Original" : "Show Original",
                      systemImage: showsQuote ? "chevron.down" : "chevron.up")
                    .font(TypeRole.meta)
            }
            .hoverHelp(showsQuote ? "Hide the message you are answering" : "Show the message you are answering")
            .buttonStyle(.borderless)
            Spacer()
        }
        .padding(.horizontal, Space.xl)
        .padding(.vertical, Space.s)
        .overlay(alignment: .top) { InsetRule() }
    }

    /// Writing help: ask the agent to write or change the message.
    /// Insert Free Times: the next free times, a line each, at the cursor
    /// (spec §14.9 amendment 2026-11-18). Only free times, no events.
    private func insertFreeTimes() async {
        guard let core = model.core, let textView = formatting.textView,
              let found = try? await core.calendarFreeSlots() else { return }
        let text = FreeTimes.insertion(FreeTimes.shown(found.slots, limit: 3), zone: found.timeZone)
        guard !text.isEmpty else { return }
        textView.window?.makeFirstResponder(textView)
        textView.insertText(NSAttributedString(string: text, attributes: textView.typingAttributes),
                            replacementRange: textView.selectedRange())
    }

    /// Rewrite in place (⌘J): while Kaluta writes, and why it could not.
    @ViewBuilder private var rewriteStatus: some View {
        switch rewriter.state {
        case .working:
            Banner("Kaluta is rewriting the selection…", systemImage: "sparkles", intent: .info) {
                Button("Stop") { rewriter.cancel() }
                    .hoverHelp("Stop and leave the text as it is")
            }
        case let .failed(message):
            Banner(message, systemImage: "exclamationmark.triangle.fill", intent: .caution)
        case .idle:
            EmptyView()
        }
    }

    private func assistantBar(_ store: ComposerStore) -> some View {
        @Bindable var assistant = assistant
        let ready = model.agent.isProviderReady
        let name = model.agent.providerName
        let run = { Task { await assistant.run(store: store, model: model, original: ComposerAssistant.plainText(fromHTML: store.quotedHTML)) } }
        return VStack(alignment: .leading, spacing: Space.xs) {
            HStack(spacing: Space.m) {
                Image(systemName: "sparkles").foregroundStyle(.tint)
                // As large as the main window's prompt, and it grows with
                // what is typed (⌥Return for a new line).
                TextField(ready ? "Ask \(AgentStore.assistantName) to write or change this message…" : "\(name) is not set up (Settings › Agents)",
                          text: $assistant.instruction, axis: .vertical)
                    .textFieldStyle(.plain)
                    .lineLimit(1...6)
                    .focused($assistantFocused)
                    .onSubmit { run() }
                    .disabled(!ready || assistant.state == .working)
                    .accessibilityLabel("Writing help")
                Menu {
                    ForEach(ComposerAssistant.suggestions(replying: !store.quotedHTML.isEmpty), id: \.self) { suggestion in
                        Button(suggestion) {
                            assistant.instruction = suggestion
                            assistantFocused = true
                        }
                    }
                } label: {
                    Image(systemName: "text.bubble")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .disabled(!ready)
                .hoverHelp("Ideas to ask for; choosing one puts it in the box")
                if assistant.state == .working {
                    ProgressView().controlSize(.small)
                    Button("Stop", systemImage: "stop.circle.fill") { assistant.cancel() }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .hoverHelp("Stop the agent writing")
                } else {
                    Button("Write", systemImage: "arrow.up.circle.fill") { run() }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .disabled(!ready || assistant.instruction.trimmingCharacters(in: .whitespaces).isEmpty)
                        .hoverHelp("Have \(AgentStore.assistantName) write this into the message (Return)")
                }
            }
            .font(TypeRole.body)
            .controlSize(.regular)
            .glassCapsule()
            switch assistant.state {
            case .done:
                HStack(spacing: Space.m) {
                    Text(assistant.followsGuide
                         ? "Written by \(AgentStore.assistantName), following your writing guide. Read it before sending."
                         : "Written by \(AgentStore.assistantName). Read it before sending.").foregroundStyle(.secondary)
                    if assistant.followsGuide {
                        Button("Your Writing Guide") { model.showWritingGuide() }
                            .buttonStyle(.link)
                            .hoverHelp("See the writing guide it followed, in the mail window")
                    }
                    audienceMenu(store)
                    Button("Undo") { assistant.undo() }
                        .buttonStyle(.link)
                        .hoverHelp("Put back the message as it was before")
                }
                .font(TypeRole.caption)
                if !assistant.checkFailures.isEmpty {
                    Label(assistant.checkFailures.joined(separator: "; "), systemImage: "exclamationmark.triangle")
                        .font(TypeRole.caption)
                        .foregroundStyle(Tone.caution)
                        .fixedSize(horizontal: false, vertical: true)
                }
            case let .asking(questions):
                factQuestions(questions)
            case let .failed(message):
                Text(message).font(TypeRole.caption).foregroundStyle(Tone.failure)
            default:
                // An agent's draft under review can be rewritten for an audience.
                if request.agentName != nil {
                    audienceMenu(store).font(TypeRole.caption)
                }
            }
        }
        .controlSize(.small)
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.s)
        .overlay(alignment: .top) { InsetRule() }
    }

    /// The facts the agent needs before it drafts: a field for each, with
    /// the answers the thread offers to pick from. The answers are
    /// proposed in Facts for review (spec §14.9).
    private func factQuestions(_ questions: [ComposerAssistant.FactQuestion]) -> some View {
        return VStack(alignment: .leading, spacing: Space.s) {
            Text("\(AgentStore.assistantName) needs a few facts before it writes this. Answer what you can.")
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
            ForEach(questions) { q in
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text(q.question).font(TypeRole.caption.weight(.medium))
                    TextField(q.fact, text: Binding(get: { factAnswers[q.id] ?? "" }, set: { factAnswers[q.id] = $0 }),
                              axis: .vertical)
                        .textFieldStyle(.roundedBorder)
                        .lineLimit(1...4)
                        .accessibilityLabel(q.question)
                    if !q.choices.isEmpty {
                        HStack(spacing: Space.s) {
                            ForEach(q.choices, id: \.self) { choice in
                                Button(choice) { factAnswers[q.id] = choice }
                                    .hoverHelp("Answer “\(choice)”")
                            }
                        }
                    }
                }
            }
            HStack(spacing: Space.m) {
                Text("Your answers are proposed in Facts, for you to review.")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
                Spacer(minLength: Space.m)
                CancelButton(help: "Stop; the message stays as it is (Esc)") {
                    assistant.cancel()
                    factAnswers = [:]
                }
                Button("Skip") { answerFacts([:]) }
                    .hoverHelp("Write it now, leaving [brackets] for what is not known")
                Button("Write") { answerFacts(factAnswers) }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Write the message with these answers")
            }
        }
    }

    private func answerFacts(_ answers: [String: String]) {
        factAnswers = [:]
        Task { await assistant.answer(answers) }
    }

    /// "Written for Customers": another audience writes a new draft under
    /// its guidelines (spec §14.9).
    @ViewBuilder private func audienceMenu(_ store: ComposerStore) -> some View {
        if !assistant.audienceChoices.isEmpty {
            let current = assistant.writtenFor
            Menu(current.isEmpty ? "Written for everyone" : "Written for \(current.joined(separator: ", "))") {
                Button("The Recipients' Own") { switchAudience(nil, store) } // no-help: menu
                Divider() // menu
                ForEach(assistant.audienceChoices, id: \.self) { name in
                    Button(name) { switchAudience([name], store) } // no-help: menu
                }
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .disabled(assistant.state == .working)
            .hoverHelp("Write it for another audience, following that audience's guidelines")
        }
    }

    private func switchAudience(_ audiences: [String]?, _ store: ComposerStore) {
        Task { await assistant.switchAudience(to: audiences, store: store, model: model) }
    }

    private func attachmentStrip(_ store: ComposerStore) -> some View {
        DraftAttachmentChips(attachments: store.attachments, remove: store.removeAttachment)
            .padding(.horizontal, Space.l)
            .padding(.vertical, Space.m)
            .overlay(alignment: .top) { InsetRule() }
    }

    /// The quote was sanitized by the core; show it under the reader's CSP.
    static func quoteDocument(_ html: String) -> String {
        """
        <!doctype html><html><head><meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="\(EmailDocument.contentSecurityPolicy)">
        <meta name="color-scheme" content="light dark">
        <style>:root{color-scheme:light dark}
        body{font:13px -apple-system;margin:8px 16px;background:Canvas;color:GrayText}
        blockquote{margin:0 0 0 4px;padding-left:8px;border-left:2px solid color-mix(in srgb, CanvasText 25%, transparent)}
        a{color:LinkText}</style>
        </head><body>\(html)</body></html>
        """
    }
}
