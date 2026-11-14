import SwiftUI

/// Forwarding rules (spec §14.15, ADR 0021): the rule editor, the band a
/// conversation carries when a rule asks or has forwarded, and Settings ›
/// Forwarding. A rule is proposed in Teach Kaluta (`ForwardDecisionBody`)
/// or written here by hand; no agent makes or changes one.

/// What the rule editor is open on.
enum ForwardRuleSheet: Identifiable {
    /// A proposal: saving accepts it as edited.
    case proposal(ForwardRuleProposalInfo)
    case rule(ForwardRuleInfo)
    case new

    var id: String {
        switch self {
        case let .proposal(p): "proposal:\(p.id)"
        case let .rule(r): "rule:\(r.id)"
        case .new: "new"
        }
    }
}

extension ForwardMode {
    var title: String {
        switch self {
        case .automatic: "Forward automatically"
        case .askEachTime: "Ask each time"
        }
    }

    var explanation: String {
        switch self {
        case .automatic: "Kaluta forwards matching mail as it arrives, with its attachments, and marks the conversation."
        case .askEachTime: "Kaluta suggests the forward on the conversation; nothing is sent until you say so."
        }
    }
}

extension ForwardRuleProposalInfo {
    /// The proposal as a rule, in the mode the user chose.
    func edit(mode: ForwardMode) -> ForwardRuleEdit {
        ForwardRuleEdit(sender: sender, senderIsDomain: false, subjectPrefix: subjectPrefix, recipient: recipient,
                        mode: mode, enabled: true)
    }
}

extension AppModel {
    /// The rules and what is proposed, as the store holds them now.
    func loadForwardRules() async {
        forwardRules = (try? await core?.forwardRules()) ?? []
        forwardRuleProposals = (try? await core?.forwardRuleProposals()) ?? []
    }

    /// Take a rule's suggestion: the forward goes now.
    func takeForwardSuggestion(_ suggestion: ForwardSuggestionInfo) async {
        guard let core, let accountID = openAccountID else { return }
        do {
            try await core.takeForwardSuggestion(suggestion)
            undo.show("Forwarded to \(suggestion.recipient)", accountID: accountID, offersUndo: false)
        } catch {
            undo.show(error.message, accountID: accountID, offersUndo: false)
        }
    }

    /// *Not this one*: the suggestion goes and is not made again.
    func dismissForwardSuggestion(_ suggestion: ForwardSuggestionInfo) async {
        guard let core, let accountID = openAccountID else { return }
        do {
            try await core.dismissForwardSuggestion(suggestion)
        } catch {
            undo.show(error.message, accountID: accountID, offersUndo: false)
        }
    }
}

// MARK: - The editor

/// Who it is from, how its subject starts, where it goes, and whether
/// Kaluta forwards on its own or asks. A proposal has no mode chosen
/// ahead: the user picks one before it can be accepted.
struct ForwardRuleEditor: View {
    @Environment(AppModel.self) private var model
    let sheet: ForwardRuleSheet
    let close: () -> Void

    @State private var sender = ""
    @State private var wholeDomain = false
    @State private var prefix = ""
    @State private var recipient = ""
    @State private var mode: ForwardMode?
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        Dialog(title: title, message: "Applies to mail that arrives from now on. A rule always names who the mail is from.") {
            Form {
                Picker("From", selection: $wholeDomain) {
                    Text("This address").tag(false)
                    Text("Anyone at this domain").tag(true)
                }
                .hoverHelp("One sender's address, or every address at a company's domain")
                TextField(wholeDomain ? "Domain" : "Address", text: $sender,
                          prompt: Text(wholeDomain ? "example.com" : "receipts@example.com"))
                    .hoverHelp("Who the mail must be from")
                TextField("Subject starts with", text: $prefix, prompt: Text("Your receipt from"))
                    .hoverHelp(wholeDomain ? "How the subject must start; needed for a whole domain"
                               : "How the subject must start; leave empty for all mail from this address")
                TextField("Forward to", text: $recipient, prompt: Text("receipts@example.com"))
                    .hoverHelp("The one address the mail is forwarded to")
                Picker("When mail matches", selection: $mode) {
                    ForEach([ForwardMode.automatic, .askEachTime], id: \.self) { choice in
                        Text(choice.title).tag(Optional(choice))
                    }
                }
                .pickerStyle(.radioGroup)
                .hoverHelp("Forward on its own, or suggest the forward and wait for you")
                if let mode {
                    Text(mode.explanation).font(TypeRole.caption).foregroundStyle(.secondary)
                } else {
                    Text("Choose one: Kaluta does not pick for you.").font(TypeRole.caption).foregroundStyle(.secondary)
                }
                if let error {
                    Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure)
                }
            }
            .formStyle(.columns)
        } leading: {
            EmptyView()
        } buttons: {
            CancelButton(help: "Close without saving (Esc)") { close() }
            Button(saveTitle) { Task { await save() } }
                .keyboardShortcut(.defaultAction)
                .disabled(mode == nil || saving)
                .hoverHelp(mode == nil ? "Choose whether to forward automatically or ask each time first"
                           : "\(saveTitle) (Return)")
        }
        .onAppear(perform: fill)
    }

    private var title: String {
        switch sheet {
        case .proposal: "Edit the Proposed Forwarding Rule"
        case .rule: "Edit Forwarding Rule"
        case .new: "New Forwarding Rule"
        }
    }

    private var saveTitle: String {
        if case .proposal = sheet { "Accept" } else { "Save" }
    }

    private func fill() {
        switch sheet {
        case let .proposal(p):
            (sender, wholeDomain, prefix, recipient, mode) = (p.sender, false, p.subjectPrefix, p.recipient, nil)
        case let .rule(r):
            (sender, wholeDomain, prefix, recipient, mode) = (r.sender, r.senderIsDomain, r.subjectPrefix, r.recipient, r.mode)
        case .new:
            break
        }
    }

    private func save() async {
        guard let mode, let core = model.core else { return }
        saving = true
        defer { saving = false }
        var enabled = true
        if case let .rule(r) = sheet { enabled = r.enabled }
        let edit = ForwardRuleEdit(sender: sender, senderIsDomain: wholeDomain, subjectPrefix: prefix,
                                   recipient: recipient, mode: mode, enabled: enabled)
        do {
            switch sheet {
            case let .proposal(p):
                if let message = await model.acceptForwardProposal(p, edit: edit) {
                    error = message
                    return
                }
            case let .rule(r):
                _ = try await core.saveForwardRule(r.id, edit)
            case .new:
                _ = try await core.saveForwardRule(nil, edit)
            }
            await model.loadForwardRules()
            close()
        } catch {
            self.error = error.message
        }
    }
}

// MARK: - On a conversation

/// What forwarding rules have to say about a conversation: the forward a
/// rule set to ask suggests (*Forward*, *Not this one*), and that a rule
/// forwarded it. Nothing for a conversation no rule touched.
struct ForwardRuleBar: View {
    @Environment(AppModel.self) private var model
    let threadID: String
    var inset: CGFloat = Space.xxl

    @State private var suggestion: ForwardSuggestionInfo?
    @State private var marks: [ForwardMarkInfo] = []
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            if let suggestion {
                HStack(alignment: .center, spacing: Space.l) {
                    Image(systemName: "arrowshape.turn.up.right")
                        .font(TypeRole.title)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: Space.hair) {
                        Text("Forward to \(suggestion.recipient)?").font(TypeRole.body.weight(.semibold)).lineLimit(1)
                        Text("Your forwarding rule asks first.").font(TypeRole.caption).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: Space.l)
                    if working { ProgressView().controlSize(.small) }
                    Button("Not This One") { act { await model.dismissForwardSuggestion(suggestion) } }
                        .controlSize(.small)
                        .disabled(working)
                        .hoverHelp("Leave this one; the rule does not ask about it again")
                    Button("Forward") { act { await model.takeForwardSuggestion(suggestion) } }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        .disabled(working)
                        .hoverHelp("Forward this email, with its attachments, to \(suggestion.recipient) now")
                }
                .padding(Space.l)
                .background(RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.controlFill))
                .accessibilityElement(children: .contain)
                .accessibilityLabel("Forwarding rule: forward to \(suggestion.recipient)?")
            }
            if let mark = marks.last {
                Label(Self.markText(mark, count: marks.count), systemImage: "arrowshape.turn.up.right.fill")
                    .font(TypeRole.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, suggestion == nil && marks.isEmpty ? 0 : inset)
        .padding(.bottom, suggestion == nil && marks.isEmpty ? 0 : Space.s)
        .task(id: "\(threadID)#\(model.forwardRulesRevision)") { await load() }
    }

    static func markText(_ mark: ForwardMarkInfo, count: Int) -> String {
        let when = Date(timeIntervalSince1970: TimeInterval(mark.at) / 1000)
            .formatted(.dateTime.month(.abbreviated).day().hour().minute())
        let more = count > 1 ? " (\(count) emails)" : ""
        return "Forwarded by a rule · \(mark.recipient) · \(when)\(more)"
    }

    private func load() async {
        guard let core = model.core else { return }
        suggestion = (try? await core.forwardSuggestion(threadID)) ?? nil
        marks = (try? await core.forwardMarks(threadID)) ?? []
    }

    private func act(_ work: @escaping @MainActor () async -> Void) {
        working = true
        Task {
            await work()
            working = false
            await load()
        }
    }
}

// MARK: - Settings

/// Settings › Forwarding (spec §14.15): the open account's rules. Each can
/// be switched between forwarding on its own and asking, paused, edited
/// or removed; one can be written by hand.
struct ForwardingSettings: View {
    @Environment(AppModel.self) private var model
    @State private var sheet: ForwardRuleSheet?
    @State private var removing: ForwardRuleInfo?
    @State private var error: String?

    var body: some View {
        Form {
            Section {
                if model.forwardRules.isEmpty {
                    Text("No forwarding rules yet. When you keep forwarding the same kind of mail to the same address, Kaluta proposes one in Teach Kaluta.")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.forwardRules, id: \.id) { rule in
                    row(rule)
                }
                if let error {
                    Text(error).font(TypeRole.caption).foregroundStyle(Tone.failure)
                }
            } header: {
                Text("Forwarding Rules")
            } footer: {
                Text("A rule forwards mail that arrives after it was made, once each, while Kaluta is open. An automatic rule forwards at most \(Self.dailyCap) a day, then asks.")
            }
            Section {
                Button("Add Rule…") { sheet = .new }
                    .hoverHelp("Write a forwarding rule by hand")
            }
        }
        .formStyle(.grouped)
        .task(id: model.openAccountID) { await model.loadForwardRules() }
        .sheet(item: $sheet) { sheet in
            ForwardRuleEditor(sheet: sheet) { self.sheet = nil }
        }
        .confirmationDialog("Remove this forwarding rule?", isPresented: Binding(get: { removing != nil },
                                                                              set: { if !$0 { removing = nil } }),
                            presenting: removing) { rule in
            Button("Remove", role: .destructive) { // no-help: confirmation dialog button
                change { try await $0.deleteForwardRule(rule.id) }
            }
        } message: { rule in
            Text("\(rule.statement). What it already forwarded stays sent.")
        }
    }

    /// As the core caps it (`forward_rules::DAILY_CAP`).
    static let dailyCap = 20

    private func row(_ rule: ForwardRuleInfo) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text(rule.statement).fixedSize(horizontal: false, vertical: true)
                    Text(Self.history(rule)).font(TypeRole.caption).foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
                Toggle("On", isOn: Binding(get: { rule.enabled }, set: { on in
                    change { _ = try await $0.saveForwardRule(rule.id, Self.edit(rule, enabled: on)) }
                }))
                .labelsHidden()
                .hoverHelp(rule.enabled ? "Pause this rule" : "Turn this rule on again; it applies to mail from now on")
            }
            HStack(spacing: Space.m) {
                Picker("When mail matches", selection: Binding(get: { rule.mode }, set: { mode in
                    change { _ = try await $0.saveForwardRule(rule.id, Self.edit(rule, mode: mode)) }
                })) {
                    Text(ForwardMode.automatic.title).tag(ForwardMode.automatic)
                    Text(ForwardMode.askEachTime.title).tag(ForwardMode.askEachTime)
                }
                .labelsHidden()
                .fixedSize()
                .hoverHelp("Forward on its own, or suggest the forward and wait for you")
                Spacer(minLength: 0)
                Button("Edit…") { sheet = .rule(rule) }
                    .hoverHelp("Change who it is from, the subject or where it goes")
                Button("Remove", systemImage: "minus.circle") { removing = rule }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .hoverHelp("Remove this rule; what it forwarded stays sent")
            }
        }
    }

    static func history(_ rule: ForwardRuleInfo) -> String {
        guard rule.sentCount > 0, let last = rule.lastSentAt else {
            return rule.enabled ? "Nothing forwarded yet" : "Paused · nothing forwarded yet"
        }
        let when = Date(timeIntervalSince1970: TimeInterval(last) / 1000).formatted(.dateTime.month(.abbreviated).day())
        return "\(rule.enabled ? "" : "Paused · ")Forwarded \(rule.sentCount), last on \(when)"
    }

    static func edit(_ rule: ForwardRuleInfo, mode: ForwardMode? = nil, enabled: Bool? = nil) -> ForwardRuleEdit {
        ForwardRuleEdit(sender: rule.sender, senderIsDomain: rule.senderIsDomain, subjectPrefix: rule.subjectPrefix,
                        recipient: rule.recipient, mode: mode ?? rule.mode, enabled: enabled ?? rule.enabled)
    }

    private func change(_ work: @escaping @MainActor (CoreClient) async throws -> Void) {
        guard let core = model.core else { return }
        Task {
            do {
                try await work(core)
                error = nil
            } catch let failure as CoreClientError {
                error = failure.message
            } catch {
                self.error = error.localizedDescription
            }
            await model.loadForwardRules()
        }
    }
}
