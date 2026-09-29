import SwiftUI

/// The prompt capsule floating over the reader (spec §14.3, amended):
/// "Ask Claude…" with the agent switcher. Sending opens the inspector.
struct AgentPromptBar: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @FocusState private var focused: Bool
    /// The chip ↑/↓/Tab moved to; Return chooses it.
    @State private var highlighted: Int?
    /// Escape hid the chips until the field is left or typed in.
    @State private var chipsHidden = false

    private var chips: [AgentSuggestion] {
        guard focused, model.agentPromptDraft.isEmpty, !chipsHidden, model.agent.isProviderReady,
              !model.agent.isRunning else { return [] }
        return model.agentChips
    }

    var body: some View {
        @Bindable var model = model
        let agent = model.agent
        let chips = chips
        VStack(alignment: .leading, spacing: Space.m) {
            if !chips.isEmpty {
                SuggestionChips(chips: chips, highlighted: highlighted) { model.choose($0) }
                    .transition(reduceMotion ? .identity : .opacity)
            }
            HStack(spacing: Space.m) {
                Menu {
                    ForEach(agent.providers, id: \.id) { provider in
                        Button {
                            agent.providerID = provider.id
                        } label: {
                            if provider.id == agent.providerID {
                                Label(provider.name, systemImage: "checkmark")
                            } else {
                                Text(provider.name)
                            }
                        }
                        .disabled({ if case .ready = provider.status { false } else { true } }())
                    }
                    Divider() // menu
                    Button("Agent Settings…") { NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil) }
                } label: {
                    Image(systemName: "sparkles")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .hoverHelp("Choose the agent")
                .accessibilityLabel("Choose the agent")

                TextField("Ask \(agent.providerName)…", text: $model.agentPromptDraft)
                    .textFieldStyle(.plain)
                    .focused($focused)
                    .onSubmit(send)
                    .disabled(!agent.isProviderReady)
                    .accessibilityLabel("Ask \(agent.providerName)")
                    .onKeyPress(.downArrow) { move(1, in: chips) }
                    .onKeyPress(.tab) { move(1, in: chips) }
                    .onKeyPress(.upArrow) { move(-1, in: chips) }
                    .onKeyPress(.return) {
                        guard let highlighted, chips.indices.contains(highlighted) else { return .ignored }
                        self.highlighted = nil
                        model.choose(chips[highlighted])
                        return .handled
                    }
                    .onKeyPress(.escape) {
                        guard !chips.isEmpty else { return .ignored }
                        chipsHidden = true
                        highlighted = nil
                        return .handled
                    }

                if agent.isRunning {
                    Button("Stop", systemImage: "stop.circle.fill") { agent.cancel() }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .hoverHelp("Stop the agent")
                } else {
                    Button("Send", systemImage: "arrow.up.circle.fill", action: send)
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .disabled(model.agentPromptDraft.trimmingCharacters(in: .whitespaces).isEmpty
                                  || !agent.isProviderReady)
                        .hoverHelp("Ask \(agent.providerName) (Return)")
                }
            }
            .glassCapsule()
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: chips)
        .task { await agent.loadProviders() }
        .onChange(of: model.agentFocusRequests) { focused = true }
        .onChange(of: focused) { _, isFocused in
            if !isFocused { chipsHidden = false }
            highlighted = nil
        }
        .onChange(of: model.agentPromptDraft) { _, text in
            if !text.isEmpty { chipsHidden = false }
            highlighted = nil
        }
        .onChange(of: chips.count) { old, count in
            // VoiceOver hears how many appeared, without focus moving.
            guard count > 0, old == 0, let window = NSApp.mainWindow else { return }
            NSAccessibility.post(element: window, notification: .announcementRequested, userInfo: [
                .announcement: count == 1 ? "1 suggestion" : "\(count) suggestions",
                .priority: NSAccessibilityPriorityLevel.medium.rawValue,
            ])
        }
    }

    private func move(_ step: Int, in chips: [AgentSuggestion]) -> KeyPress.Result {
        guard !chips.isEmpty else { return .ignored }
        let next = (highlighted ?? (step > 0 ? -1 : chips.count)) + step
        highlighted = chips.indices.contains(next) ? next : (step > 0 ? 0 : chips.count - 1)
        return .handled
    }

    private func send() {
        let prompt = model.agentPromptDraft
        guard !prompt.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        model.agentPromptDraft = ""
        Task { await model.askAgent(prompt) }
    }
}

/// Up to four example prompts over the empty prompt field (spec §14.6b).
struct SuggestionChips: View {
    let chips: [AgentSuggestion]
    let highlighted: Int?
    let choose: (AgentSuggestion) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            ForEach(Array(chips.enumerated()), id: \.element.id) { index, chip in
                Button { choose(chip) } label: {
                    Text(chip.text)
                        .lineLimit(1)
                        .padding(.horizontal, Space.l)
                        .padding(.vertical, Space.s)
                        .background(index == highlighted ? Tone.highlight : AnyShapeStyle(.clear),
                                    in: .capsule)
                }
                .buttonStyle(.plain)
                .glassEffect(.regular.interactive(), in: .capsule)
                .hoverHelp(chip.fillsOnly ? "Put this in the field for you to finish" : "Ask the agent this")
                .accessibilityLabel(chip.text)
                .accessibilityHint(chip.fillsOnly ? "Puts this in the field for you to finish" : "Asks the agent")
            }
        }
        .font(.callout)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Suggestions")
    }
}

/// The agent column before a conversation: what the agent can do, as
/// example prompts in groups (spec §14.6b). Choosing one puts it in the
/// prompt, to send as it is or change first.
struct AgentCapabilitiesView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxl) {
                VStack(alignment: .leading, spacing: Space.xs) {
                    Label("Ask \(model.agent.providerName)", systemImage: "sparkles").font(.headline)
                    Text("It reads your mail here, on this Mac. Drafts wait for you, and sending always asks first.")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                ForEach(AgentSuggestions.groups(canDraft: !model.isArchive)) { group in
                    VStack(alignment: .leading, spacing: Space.s) {
                        Label(group.title, systemImage: group.symbol)
                            .font(TypeRole.groupLabel)
                            .foregroundStyle(.secondary)
                        ForEach(group.examples) { example in
                            Button { model.fillPrompt(example) } label: {
                                Text("“\(example.text)”")
                                    .multilineTextAlignment(.leading)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(.tint)
                            .hoverHelp(example.fillsOnly ? "Put this in the prompt for you to finish"
                                                         : "Put this in the prompt; press Return to ask")
                            .accessibilityLabel(example.text)
                            .accessibilityHint("Puts this in the prompt")
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(group.title)
                }
            }
            .padding(Space.xl)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// The inspector column: transcript, results as thread rows, cancel.
struct AgentInspector: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let agent = model.agent
        VStack(spacing: 0) {
            if agent.entries.isEmpty {
                AgentCapabilitiesView()
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: Space.m) {
                            ForEach(agent.entries) { entry in
                                EntryView(entry: entry).id(entry.id)
                            }
                        }
                        .padding(Space.l)
                    }
                    .onChange(of: agent.entries.last) { _, last in
                        if let last { proxy.scrollTo(last.id, anchor: .bottom) }
                    }
                }
            }
            if let usage = agent.lastUsage {
                InsetRule()
                Text(usage).font(.caption).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .padding(.horizontal, Space.l).padding(.vertical, Space.xs)
            }
        }
        // The transcript scrolls under the header; the system draws the edge.
        .columnHeader { header(agent) }
    }

    private func header(_ agent: AgentStore) -> some View {
        HStack {
            Text(agent.providerName).font(TypeRole.heading)
            Spacer()
            if agent.pendingProposals.count > 1 {
                Button("Approve All (\(agent.pendingProposals.count))") { agent.approveAll() }
                    .hoverHelp("Approve every action the agent is waiting on")
                    .controlSize(.small)
            }
            if agent.isRunning {
                ProgressView().controlSize(.small)
                Button("Stop") { agent.cancel() }
                    .hoverHelp("Stop the agent's current turn")
                    .controlSize(.small)
            }
            Menu {
                if agent.history.isEmpty {
                    Text("No earlier conversations")
                }
                ForEach(agent.history, id: \.sessionId) { conversation in
                    Button(conversation.title.isEmpty ? "Untitled" : conversation.title) {
                        Task { await agent.open(conversation) }
                    }
                }
            } label: {
                Image(systemName: "clock.arrow.circlepath")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .hoverHelp("Earlier conversations")
            .accessibilityLabel("Earlier conversations")
            .onAppear { Task { await agent.loadHistory() } }
            Button("New Conversation", systemImage: "square.and.pencil") { agent.newConversation() }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .hoverHelp("Start a new conversation")
                .disabled(agent.entries.isEmpty)
        }
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
    }
}

private struct EntryView: View {
    let entry: AgentStore.Entry
    @Environment(AppModel.self) private var model
    @State private var expanded = false

    var body: some View {
        switch entry.kind {
        case let .prompt(text):
            Text(text)
                .frame(maxWidth: .infinity, alignment: .leading)
                .card(.info, padding: Space.m)
                .textSelection(.enabled)
        case let .reply(text):
            Text(LocalizedStringKey(text))
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)
        case let .thinking(text):
            DisclosureGroup("Thinking", isExpanded: $expanded) {
                Text(text).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
            }
            .font(.callout)
            .foregroundStyle(.secondary)
        case let .tool(name, arguments, state, summary):
            DisclosureGroup(isExpanded: $expanded) {
                VStack(alignment: .leading, spacing: Space.xs) {
                    if !arguments.isEmpty { Text(arguments) }
                    if !summary.isEmpty { Text(summary).foregroundStyle(.secondary) }
                }
                .font(.caption.monospaced())
                .textSelection(.enabled)
            } label: {
                HStack(spacing: Space.s) {
                    switch state {
                    case .running: ProgressView().controlSize(.mini)
                    case .succeeded: Image(systemName: "checkmark.circle").foregroundStyle(.secondary)
                    case .failed: Image(systemName: "xmark.octagon").foregroundStyle(Tone.failure)
                    }
                    Text(AgentStore.toolTitle(name))
                    if !arguments.isEmpty {
                        Text(arguments).foregroundStyle(.tertiary).lineLimit(1)
                    }
                }
                .font(.callout)
                .foregroundStyle(.secondary)
            }
        case let .results(rows):
            VStack(alignment: .leading, spacing: 0) {
                ForEach(rows, id: \.id) { row in
                    ResultRow(row: row, selected: model.selectedThreadID == row.id)
                        .contentShape(.rect)
                        .onTapGesture { model.selectedThreadID = row.id }
                    if row.id != rows.last?.id { InsetRule(inset: Space.m) }
                }
            }
            .background(.background, in: .rect(cornerRadius: Radius.card))
            .overlay(RoundedRectangle(cornerRadius: Radius.card).strokeBorder(.separator))
        case let .error(message):
            Label(message, systemImage: "exclamationmark.triangle.fill")
                .font(.callout)
                .foregroundStyle(Tone.failure)
        case let .proposal(actionID, tool, summary, draftID, state):
            ProposalCard(actionID: actionID, tool: tool, summary: summary, draftID: draftID, state: state)
        }
    }
}

/// An action waiting for the user (spec §10.4): what it does, a way to
/// review the message for sends and forwards, and the decision.
private struct ProposalCard: View {
    let actionID: Int64
    let tool: String
    let summary: String
    let draftID: Int64?
    let state: AgentStore.Entry.ProposalState
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: Space.m) {
            Label(summary, systemImage: Self.symbol(tool))
                .font(.callout.weight(.medium))
                .fixedSize(horizontal: false, vertical: true)
            switch state {
            case .pending:
                HStack {
                    if let draftID {
                        Button("Review…") {
                            model.compose(.review(draftID: draftID, agent: model.agent.providerName))
                        }
                        .hoverHelp("Open the message in a composer to read or edit it first")
                    }
                    Spacer()
                    Button("Reject", role: .destructive) { model.agent.resolve(actionID, approve: false) }
                        .hoverHelp("Don't let the agent do this")
                    Button("Approve") { model.agent.resolve(actionID, approve: true) }
                        .hoverHelp("Let the agent do this")
                        .buttonStyle(.borderedProminent)
                }
                .controlSize(.small)
            case .approved:
                Label("Approved", systemImage: "checkmark.circle.fill").font(.caption).foregroundStyle(Tone.approved)
            case let .sending(until):
                HStack {
                    ProgressView().controlSize(.mini)
                    Text("Sending…").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Undo") { model.agent.undoSend(actionID) }
                        .controlSize(.small)
                        .hoverHelp("Take the message back and open it (until \(until.formatted(date: .omitted, time: .standard)))")
                }
                .accessibilityElement(children: .contain)
            case .undoing:
                HStack {
                    ProgressView().controlSize(.mini)
                    Text("Taking it back…").font(.caption).foregroundStyle(.secondary)
                }
            case .takenBack:
                Label("Not sent. The draft is open for you.", systemImage: "arrow.uturn.backward.circle")
                    .font(.caption).foregroundStyle(.secondary)
            case .rejected:
                Label("Declined", systemImage: "xmark.circle").font(.caption).foregroundStyle(.secondary)
            }
        }
        .card(state == .pending ? .attention : .neutral, padding: Space.m)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Proposed: \(summary)")
    }

    static func symbol(_ tool: String) -> String {
        switch tool {
        case "mail_send": "paperplane"
        case "mail_forward": "arrowshape.turn.up.right"
        case "mail_delete": "trash"
        default: "hand.raised"
        }
    }
}

/// A thread from the agent's results, like a row in the main list.
private struct ResultRow: View {
    let row: ThreadRow
    let selected: Bool
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: Space.hair) {
            HStack {
                Text(ThreadRowView.senderLine(row, me: model.ownAddresses))
                    .fontWeight(row.unreadCount > 0 ? .semibold : .regular)
                    .lineLimit(1)
                Spacer()
                Text(RowDateFormatter.string(forMillis: row.lastMessageAt))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Text(row.subject.isEmpty ? "(no subject)" : row.subject).font(.callout).lineLimit(1)
            Text(row.snippet).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(Space.m)
        .background(selected ? AnyShapeStyle(.selection) : AnyShapeStyle(.clear))
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}
