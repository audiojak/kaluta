import AppKit
import SwiftUI

/// Which agent-mailbox sheet is open (spec §7.9).
enum AgentMailboxRequest: Identifiable, Equatable {
    /// Create an Agent Mailbox…
    case create
    /// Verify an existing one.
    case verify(accountID: String)
    /// Put it on one of the user's own domains.
    case domain(accountID: String)

    var id: String {
        switch self {
        case .create: "create"
        case let .verify(accountID): "verify-\(accountID)"
        case let .domain(accountID): "domain-\(accountID)"
        }
    }
}

/// Agent mailboxes (spec §7.9): created through the service's own API, no
/// website and no agent involved.
extension AppModel {
    func beginAgentMailbox() {
        agentMailboxSheet = .create
    }

    func beginAgentVerification(_ accountID: String) {
        agentMailboxSheet = .verify(accountID: accountID)
    }

    /// Create the mailbox, then show it and sync it with the others.
    /// `requestID` stays the same across retries from one sheet, so a retry
    /// never makes a second account at the service.
    func createAgentMailbox(name: String, requestID: String = UUID().uuidString) async throws(CoreClientError)
        -> AgentMailboxCreated {
        guard let core else { throw CoreClientError(kind: .notFound, message: "OpenAGC is still starting") }
        let created = try await core.createAgentMailbox(service: .primitive, name: name, requestID: requestID)
        agentPlans[created.accountId] = created.plan
        await reloadAccounts()
        await switchAccount(to: created.accountId)
        // Every account syncs in the background; this starts the new one.
        Task { _ = try? await core.startAllSync() }
        return created
    }

    func refreshAgentPlan(_ accountID: String) async {
        guard let core, let plan = try? await core.agentMailboxPlan(accountID) else { return }
        agentPlans[accountID] = plan
    }

    /// The user's own accounts whose mail can carry a code: Gmail ones.
    var codeAccounts: [AccountSummary] {
        accounts.filter { $0.kind == .gmail }
    }

    /// The open agent mailbox, unverified: the banner says what it cannot do.
    var unverifiedAgentPlan: AgentMailboxPlan? {
        guard let id = openAccountID, isAgentMailbox, let plan = agentPlans[id], !plan.verified else { return nil }
        return plan
    }

    /// One line on an unverified mailbox's limits.
    static func limitsText(_ plan: AgentMailboxPlan) -> String {
        "Until it's verified, this mailbox can only reply to people who wrote first, up to \(plan.sendPerHour) an hour and \(plan.sendPerDay) a day."
    }
}

/// Accounts › Create an Agent Mailbox…: a name, the service's terms, then
/// verification in the same sheet.
struct AgentMailboxSheet: View {
    @Environment(AppModel.self) private var model
    let request: AgentMailboxRequest

    @State private var name = ""
    /// One per sheet: retries after a failure reuse it.
    @State private var requestID = UUID().uuidString
    @State private var busy = false
    @State private var error: String?
    /// Set once the mailbox exists (or when verifying an existing one).
    @State private var created: (accountID: String, address: String)?

    var body: some View {
        Group {
            if case let .domain(accountID) = request {
                AgentDomainForm(accountID: accountID)
            } else if let created {
                AgentVerificationForm(accountID: created.accountID, address: created.address, justCreated: request == .create)
            } else {
                createForm
            }
        }
        .onAppear {
            if case let .verify(accountID) = request {
                let address = model.accounts.first { $0.id == accountID }?.email ?? ""
                created = (accountID, address)
            }
        }
    }

    private var createForm: some View {
        Dialog(title: "Create an Agent Mailbox",
               message: "An address of its own for one of your agents, to sign up for things and write to people as itself. You can read its mail here and send as it.") {
            TextField("Agent's name", text: $name, prompt: Text("Research Scout"))
                .onSubmit { Task { await create() } }
            LabeledContent("Service") {
                VStack(alignment: .leading, spacing: Space.hair) {
                    Text("Primitive")
                    Text("Free. Until you verify it with your email, it can only reply, 10 messages an hour.")
                        .font(TypeRole.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if let terms = model.core?.agentServiceTermsURL(.primitive) {
                HStack(spacing: Space.xs) {
                    Text("Creating it accepts Primitive's")
                    Link("Terms of Service", destination: terms)
                }
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
            }
        } buttons: {
            if busy { ProgressView().controlSize(.small) }
            CancelButton(help: "Close without creating a mailbox (Esc)") { model.agentMailboxSheet = nil }
            Button("Agree and Create") { Task { await create() } }
                .keyboardShortcut(.defaultAction)
                .disabled(busy || name.trimmingCharacters(in: .whitespaces).isEmpty)
                .hoverHelp("Accept Primitive's terms and create the mailbox (Return)")
        }
    }

    private func create() async {
        guard !busy, !name.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        busy = true
        defer { busy = false }
        do {
            let mailbox = try await model.createAgentMailbox(name: name, requestID: requestID)
            error = nil
            created = (mailbox.accountId, mailbox.address)
        } catch {
            self.error = error.message
        }
    }
}

/// Verify a mailbox with the user's email: send a code, fill it from their
/// own mail when it arrives there, confirm.
struct AgentVerificationForm: View {
    @Environment(AppModel.self) private var model
    let accountID: String
    let address: String
    let justCreated: Bool

    @State private var email = ""
    @State private var code = ""
    @State private var sentTo: String?
    /// Bumped by each code sent, so the search for it starts again.
    @State private var sends = 0
    @State private var resendAt: Date?
    @State private var foundCode: String?
    @State private var busy = false
    @State private var error: String?
    @State private var now = Date()

    var body: some View {
        Dialog(title: justCreated ? "Mailbox Created" : "Verify the Agent's Mailbox", message: message) {
            if sentTo == nil {
                TextField("Your email", text: $email, prompt: Text("you@example.com"))
                    .onSubmit { Task { await sendCode() } }
            } else {
                TextField("Code", text: $code, prompt: Text("6-digit code"))
                    .onSubmit { Task { await verify() } }
                if let foundCode, code != foundCode, let sentTo {
                    Button("Fill Code from \(sentTo)") { code = foundCode }
                        .hoverHelp("Use the code Primitive sent to \(sentTo); it arrived in your mail here")
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
            }
        } leading: {
            if sentTo != nil {
                Button("Resend") { Task { await sendCode() } }
                    .disabled(busy || (resendAt.map { now < $0 } ?? false))
                    .hoverHelp("Send another code")
            }
        } buttons: {
            if busy { ProgressView().controlSize(.small) }
            CancelButton(title: "Later", help: "Close; verify later from the mailbox or its settings (Esc)") {
                model.agentMailboxSheet = nil
            }
            if sentTo == nil {
                Button("Send Code") { Task { await sendCode() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || !email.contains("@"))
                    .hoverHelp("Primitive emails you a code (Return)")
            } else {
                Button("Verify") { Task { await verify() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || code.trimmingCharacters(in: .whitespaces).isEmpty)
                    .hoverHelp("Confirm the code (Return)")
            }
        }
        .onAppear {
            if email.isEmpty { email = model.codeAccounts.first?.email ?? "" }
        }
        .task(id: sends) { await watchForCode() }
    }

    private var message: String {
        let lead = justCreated ? "\(address) is ready and syncing. " : ""
        if let sentTo { return lead + "Primitive sent a code to \(sentTo)." }
        return lead + "Verify it with your email to lift the limits: until then it can only reply to people who wrote first."
    }

    /// The account the code is going to, if it is one of the user's here.
    private var codeAccountID: String? {
        guard let sentTo else { return nil }
        return model.codeAccounts.first { $0.email.caseInsensitiveCompare(sentTo) == .orderedSame }?.id
    }

    private func sendCode() async {
        guard let core = model.core, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let started = try await core.startAgentMailboxVerification(accountID, email: email)
            error = nil
            sentTo = email.trimmingCharacters(in: .whitespaces)
            foundCode = nil
            sends += 1
            resendAt = Date().addingTimeInterval(TimeInterval(started.resendAfterSecs))
            // A sync now brings the code's message sooner.
            core.syncNow()
        } catch {
            self.error = error.message
        }
    }

    /// Look for the code in the user's own account while the sheet waits.
    private func watchForCode() async {
        guard let core = model.core, let inAccount = codeAccountID else { return }
        while !Task.isCancelled {
            now = Date()
            if foundCode == nil, let found = await core.findAgentMailboxCode(accountID, in: inAccount) {
                foundCode = found
            }
            try? await Task.sleep(for: .seconds(2))
        }
    }

    private func verify() async {
        guard let core = model.core, !busy else { return }
        busy = true
        defer { busy = false }
        do {
            let plan = try await core.verifyAgentMailbox(accountID, code: code)
            model.agentPlans[accountID] = plan
            model.agentMailboxSheet = nil
        } catch {
            self.error = error.message
        }
    }
}

/// The open agent mailbox is not verified yet: what it cannot do, and Verify.
struct AgentLimitsBanner: View {
    @Environment(AppModel.self) private var model
    let plan: AgentMailboxPlan

    var body: some View {
        Banner(AppModel.limitsText(plan), systemImage: "person.badge.shield.checkmark", intent: .attention) {
            if let id = model.openAccountID {
                Button("Verify…") { model.beginAgentVerification(id) }
                    .hoverHelp("Verify the mailbox with your email to lift the limits")
            }
        }
    }
}
