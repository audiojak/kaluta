import AppKit
import SwiftUI

/// Which agent-mailbox sheet is open (spec §7.9).
enum AgentMailboxRequest: Identifiable, Equatable {
    /// Create an Agent Mailbox…, or Add Agent… on a service account: the
    /// steps are `AppModel.agentMailboxFlow`'s.
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

/// What the create sheet asks of the core: a seam, so tests can see that
/// adding an agent never signs up (ADR 0015).
protocol AgentMailboxCalls: AnyObject, Sendable {
    func createAgentMailbox(service: AgentService, name: String, humanEmail: String?,
                            requestID: String) async throws(CoreClientError) -> AgentMailboxCreated
    func addAgent(toServiceAccount serviceAccountID: String, name: String, domain: String?,
                  requestID: String) async throws(CoreClientError) -> AgentAdded
    func serviceAccountPlan(_ serviceAccountID: String) async throws(CoreClientError) -> AgentMailboxPlan
}

extension CoreClient: AgentMailboxCalls {}

/// How a create sheet ended.
enum AgentMailboxOutcome: Equatable {
    /// A new service account with its first agent; verification follows,
    /// at the code when the service sent one at sign-up (AgentMail).
    case created(accountID: String, address: String, codeSentTo: String?)
    /// Another agent on a service account that has agreed and verified.
    case added(accountID: String, address: String)
}

/// Agent mailboxes (spec §7.9): created through the service's own API, no
/// website and no agent involved.
extension AppModel {
    func beginAgentMailbox() {
        agentMailboxFlow = AgentMailboxFlow(serviceAccounts: serviceAccounts, email: preferredHumanEmail)
        agentMailboxSheet = .create
        // The list may be older than the agents on disk.
        Task {
            await reloadServiceAccounts()
            agentMailboxFlow?.serviceAccounts = serviceAccounts
        }
    }

    /// Settings › Add Agent…: the sheet at the name, on that service account.
    func beginAddAgent(to serviceAccountID: String) {
        guard let account = serviceAccounts.first(where: { $0.id == serviceAccountID }) else { return }
        let flow = AgentMailboxFlow(serviceAccounts: serviceAccounts, email: preferredHumanEmail)
        flow.choose(account.service)
        flow.add(to: account)
        agentMailboxFlow = flow
        agentMailboxSheet = .create
    }

    func beginAgentVerification(_ accountID: String) {
        agentMailboxSheet = .verify(accountID: accountID)
    }

    var agentMailboxCalls: (any AgentMailboxCalls)? { agentMailboxCallsOverride ?? core }

    /// The email AgentMail is asked to verify with: the open Gmail account's,
    /// else the first of the user's.
    var preferredHumanEmail: String {
        if let open = codeAccounts.first(where: { $0.id == openAccountID }) { return open.email }
        return codeAccounts.first?.email ?? ""
    }

    /// Create a mailbox in a new service account, then show it and sync it
    /// with the others. `requestID` stays the same across retries from one
    /// sheet, so a retry never makes a second account at the service.
    func createAgentMailbox(service: AgentService = .primitive, name: String, humanEmail: String? = nil,
                            requestID: String = UUID().uuidString) async throws(CoreClientError)
        -> AgentMailboxCreated {
        guard let calls = agentMailboxCalls else {
            throw CoreClientError(kind: .notFound, message: "OpenAGC is still starting")
        }
        let created = try await calls.createAgentMailbox(service: service, name: name, humanEmail: humanEmail,
                                                         requestID: requestID)
        // A new service account takes its first agent's id.
        servicePlans[created.accountId] = created.plan
        fetchedServicePlans.insert(created.accountId)
        await show(newAgent: created.accountId)
        return created
    }

    /// Add an agent to a service account: its name only, no terms, sign-up
    /// or code (ADR 0015).
    func addAgent(to serviceAccountID: String, name: String, domain: String? = nil,
                  requestID: String = UUID().uuidString) async throws(CoreClientError) -> AgentAdded {
        guard let calls = agentMailboxCalls else {
            throw CoreClientError(kind: .notFound, message: "OpenAGC is still starting")
        }
        let added = try await calls.addAgent(toServiceAccount: serviceAccountID, name: name, domain: domain,
                                             requestID: requestID)
        await show(newAgent: added.accountId)
        return added
    }

    private func show(newAgent accountID: String) async {
        await reloadAccounts()
        await reloadServiceAccounts()
        await switchAccount(to: accountID)
        // A new mailbox is empty: the keyboard goes to its list, so c
        // starts a message at once.
        focusThreadList()
        // Every account syncs in the background; this starts the new one.
        if let core { Task { _ = try? await core.startAllSync() } }
    }

    /// The sheet's Agree and Create, or Add Agent.
    func submitAgentMailbox(_ flow: AgentMailboxFlow) async throws(CoreClientError) -> AgentMailboxOutcome {
        switch flow.step {
        case .add:
            guard let target = flow.target else { throw CoreClientError(kind: .invalidInput, message: "Choose a service account") }
            let added = try await addAgent(to: target.id, name: flow.name, domain: flow.domain,
                                           requestID: flow.requestID)
            return .added(accountID: added.accountId, address: added.address)
        case .new:
            guard let service = flow.service else { throw CoreClientError(kind: .invalidInput, message: "Choose a service") }
            let email = flow.asksForEmail ? flow.humanEmail.trimmingCharacters(in: .whitespaces) : nil
            let created = try await createAgentMailbox(service: service, name: flow.name, humanEmail: email,
                                                       requestID: flow.requestID)
            return .created(accountID: created.accountId, address: created.address,
                            codeSentTo: service == .agentMail && !created.plan.verified ? email : nil)
        case .service, .path:
            throw CoreClientError(kind: .invalidInput, message: "Choose a service first")
        }
    }

    /// Service accounts, read again from disk.
    func reloadServiceAccounts() async {
        guard let core else { return }
        if let fresh = try? await core.listServiceAccounts(), fresh != serviceAccounts { serviceAccounts = fresh }
    }

    /// The service account an agent belongs to.
    func serviceAccount(of agentID: String) -> ServiceAccountSummary? {
        serviceAccounts.first { $0.agentAccountIds.contains(agentID) }
    }

    /// An agent's service account id, from the list or else the core.
    func serviceAccountID(of agentID: String) -> String? {
        serviceAccount(of: agentID)?.id ?? core?.agentServiceAccount(agentID)
    }

    /// The plan of an agent's service account: read from the service once
    /// per service account (all its agents share it), or again with `force`.
    func refreshAgentPlan(_ agentID: String, force: Bool = false) async {
        if serviceAccount(of: agentID) == nil { await reloadServiceAccounts() }
        guard let id = serviceAccountID(of: agentID) else { return }
        await refreshServicePlan(id, force: force)
    }

    func refreshServicePlan(_ serviceAccountID: String, force: Bool = false) async {
        guard let calls = agentMailboxCalls, force || !fetchedServicePlans.contains(serviceAccountID) else { return }
        fetchedServicePlans.insert(serviceAccountID)
        guard let plan = try? await calls.serviceAccountPlan(serviceAccountID) else {
            fetchedServicePlans.remove(serviceAccountID)
            return
        }
        servicePlans[serviceAccountID] = plan
        // The list carries `verified` too.
        if serviceAccounts.first(where: { $0.id == serviceAccountID })?.verified != plan.verified {
            await reloadServiceAccounts()
        }
    }

    /// A service account was verified: every agent in it is.
    func serviceAccountVerified(_ agentID: String, plan: AgentMailboxPlan) async {
        guard let id = serviceAccountID(of: agentID) else { return }
        servicePlans[id] = plan
        fetchedServicePlans.insert(id)
        await reloadServiceAccounts()
    }

    /// The user's own accounts whose mail can carry a code: Gmail ones.
    var codeAccounts: [AccountSummary] {
        accounts.filter { $0.kind == .gmail }
    }

    /// The open agent mailbox's service account, unverified: the banner
    /// says what it cannot do.
    var unverifiedAgentPlan: AgentMailboxPlan? {
        guard let id = openAccountID, isAgentMailbox, let service = serviceAccountID(of: id),
              let plan = servicePlans[service], !plan.verified else { return nil }
        return plan
    }

    /// The banner's line for the open agent mailbox, when unverified.
    var unverifiedAgentLimits: String? {
        guard let plan = unverifiedAgentPlan, let id = openAccountID else { return nil }
        let account = serviceAccount(of: id)
        return Self.limitsText(plan, service: account?.service ?? .primitive,
                               words: account.flatMap { core?.serviceAccountLimits($0.id) })
    }

    /// The open agent mailbox's limits in full, for the composer.
    func agentLimits(_ accountID: String?) -> String? {
        guard let accountID, accounts.first(where: { $0.id == accountID })?.kind == .agent,
              let id = serviceAccountID(of: accountID) else { return nil }
        return core?.serviceAccountLimits(id)
    }

    /// One line on an unverified mailbox's limits. Primitive's come from
    /// its plan; AgentMail's plan has no hourly or daily numbers (its limits
    /// are monthly and per new recipient), so its line is the core's words
    /// (`words`, `serviceAccountLimits`): their first sentence, which is the
    /// one about verifying.
    static func limitsText(_ plan: AgentMailboxPlan, service: AgentService = .primitive, words: String? = nil) -> String {
        switch service {
        case .primitive:
            "Until it's verified, this mailbox can only reply to people who wrote first, up to \(plan.sendPerHour) an hour and \(plan.sendPerDay) a day."
        case .agentMail:
            words.map(firstSentence) ?? "Until it's verified, this mailbox can write only to the email it was created with."
        }
    }

    /// The text up to and including its first full stop.
    static func firstSentence(_ text: String) -> String {
        guard let end = text.range(of: ". ") else { return text }
        return String(text[..<end.lowerBound]) + "."
    }

    /// "Primitive" or "AgentMail".
    static func serviceName(_ service: AgentService) -> String {
        switch service {
        case .primitive: "Primitive"
        case .agentMail: "AgentMail"
        }
    }

    /// What a service is, in a line, with its free tier.
    static func serviceBlurb(_ service: AgentService) -> String {
        switch service {
        case .primitive:
            "Addresses for agents on a shared domain. Free; until you verify it with your email, it can only reply, 10 messages an hour."
        case .agentMail:
            "Inboxes for agents at agentmail.to. Free for 3 inboxes and 3,000 messages a month; verified with your email."
        }
    }

    /// A service account as the switcher and Settings name it: "AgentMail ·
    /// you@example.com", "Primitive · jade-emu.primitive.email". AgentMail
    /// is known by the user's email (an organisation per email); Primitive
    /// by its own subdomain, which it has before it is verified.
    static func serviceAccountTitle(_ account: ServiceAccountSummary) -> String {
        let detail = switch account.service {
        case .agentMail: account.humanEmail ?? account.managedDomain
        case .primitive: account.managedDomain ?? account.humanEmail
        }
        return [serviceName(account.service), detail].compactMap { $0 }.joined(separator: " · ")
    }

    /// An agent's address from its name, as the core makes it: lower case,
    /// letters and digits, dashes between words.
    static func localPart(_ name: String) -> String {
        let local = name.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : "-" }.joined()
            .split(separator: "-").joined(separator: "-")
        return local.isEmpty ? "agent" : local
    }
}

/// The steps of Create an Agent Mailbox (spec §7.9, ADR 0015): the service
/// first; then, when a service account for it exists, adding to it or a new
/// one; then the name (and, for a new AgentMail account, the user's email).
@MainActor @Observable
final class AgentMailboxFlow {
    enum Step: Equatable {
        /// Primitive or AgentMail.
        case service
        /// Add to an existing service account, or make a new one.
        case path
        /// A name for an agent on `target`: no terms, sign-up or code.
        case add
        /// A new service account: the name, AgentMail's email, the terms.
        case new
    }

    private(set) var step: Step = .service
    private(set) var service: AgentService?
    /// The service account an agent is added to.
    private(set) var target: ServiceAccountSummary?
    var serviceAccounts: [ServiceAccountSummary]
    var name = ""
    /// The user's email, for a new AgentMail account.
    var humanEmail: String
    /// Adding on Primitive: one of the service account's verified own
    /// domains, else `nil` for its managed subdomain.
    var domain: String?
    /// Its verified own domains, when known.
    var ownDomains: [String] = []
    /// One per sheet: retries after a failure reuse it.
    let requestID = UUID().uuidString

    init(serviceAccounts: [ServiceAccountSummary], email: String) {
        self.serviceAccounts = serviceAccounts
        humanEmail = email
    }

    static let services: [AgentService] = [.primitive, .agentMail]

    /// The service accounts an agent could be added to.
    func existing(_ service: AgentService) -> [ServiceAccountSummary] {
        serviceAccounts.filter { $0.service == service }
    }

    func choose(_ chosen: AgentService) {
        service = chosen
        target = nil
        domain = nil
        step = existing(chosen).isEmpty ? .new : .path
    }

    func add(to account: ServiceAccountSummary) {
        service = account.service
        target = account
        domain = nil
        step = .add
    }

    func newServiceAccount() {
        target = nil
        step = .new
    }

    func back() {
        switch step {
        case .service: break
        case .path: step = .service
        case .add, .new:
            let alone = service.map { existing($0).isEmpty } ?? true
            step = alone ? .service : .path
        }
    }

    /// AgentMail's sign-up takes the user's email with the name: without it
    /// the inbox can only receive and a lost key cannot be recovered.
    var asksForEmail: Bool { step == .new && service == .agentMail }

    var canSubmit: Bool {
        guard !name.trimmingCharacters(in: .whitespaces).isEmpty else { return false }
        switch step {
        case .add: return target != nil
        case .new: return !asksForEmail || humanEmail.contains("@")
        case .service, .path: return false
        }
    }

    /// The address the agent will have: `writer@jade-emu.primitive.email`.
    var addressPreview: String? {
        let local = AppModel.localPart(name)
        switch (step, service) {
        case (.add, _):
            guard let host = domain ?? target?.managedDomain else { return nil }
            return "\(local)@\(host)"
        case (.new, .agentMail?):
            return "\(local)@agentmail.to"
        default:
            return nil
        }
    }
}

/// Accounts › Create an Agent Mailbox…: the service, then adding to a
/// service account or creating one (name, terms, AgentMail's email), then
/// verification in the same sheet.
struct AgentMailboxSheet: View {
    @Environment(AppModel.self) private var model
    let request: AgentMailboxRequest

    @State private var busy = false
    @State private var error: String?
    /// Set once the mailbox exists (or when verifying an existing one).
    @State private var created: (accountID: String, address: String, codeSentTo: String?)?

    var body: some View {
        Group {
            if case let .domain(accountID) = request {
                AgentDomainForm(accountID: accountID)
            } else if let created {
                AgentVerificationForm(accountID: created.accountID, address: created.address,
                                      justCreated: request == .create, codeSentTo: created.codeSentTo)
            } else if let flow = model.agentMailboxFlow {
                steps(flow)
            }
        }
        .onAppear {
            if case let .verify(accountID) = request {
                let address = model.accounts.first { $0.id == accountID }?.email ?? ""
                created = (accountID, address, nil)
            }
        }
        .onDisappear { if request == .create { model.agentMailboxFlow = nil } }
    }

    @ViewBuilder private func steps(_ flow: AgentMailboxFlow) -> some View {
        switch flow.step {
        case .service: AgentServiceStep(flow: flow)
        case .path: AgentPathStep(flow: flow)
        case .add, .new: nameStep(flow)
        }
    }

    private func nameStep(_ flow: AgentMailboxFlow) -> some View {
        @Bindable var flow = flow
        let service = flow.service ?? .primitive
        let serviceName = AppModel.serviceName(service)
        let adding = flow.step == .add
        return Dialog(title: adding ? "Add an Agent" : "Create an Agent Mailbox",
                      message: adding
                          ? "A new address on \(flow.target.map(AppModel.serviceAccountTitle) ?? serviceName), which has agreed to the terms and shares its key, plan and limits with its other agents."
                          : "An address of its own for one of your agents, to sign up for things and write to people as itself. You can read its mail here and send as it.") {
            TextField("Agent's name", text: $flow.name, prompt: Text("Research Scout"))
                .onSubmit { Task { await submit(flow) } }
            if adding, service == .primitive, !flow.ownDomains.isEmpty, let managed = flow.target?.managedDomain {
                Picker("Domain", selection: $flow.domain) {
                    Text(managed).tag(String?.none)
                    ForEach(flow.ownDomains, id: \.self) { Text($0).tag(String?.some($0)) }
                }
                .hoverHelp("Where the agent's address is: the service account's own subdomain or one of your verified domains")
            }
            if let address = flow.addressPreview {
                LabeledContent("Address") {
                    Text(address).foregroundStyle(.secondary).textSelection(.enabled)
                }
            }
            if flow.asksForEmail {
                VStack(alignment: .leading, spacing: Space.xs) {
                    Text("Your email")
                    HumanEmailField(email: $flow.humanEmail, choices: model.codeAccounts.map(\.email))
                        .onSubmit { Task { await submit(flow) } }
                    Text("AgentMail sends a code to it. Without it the inbox can only receive mail, and a lost key can't be recovered.")
                        .font(TypeRole.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if !adding {
                LabeledContent("Service") {
                    VStack(alignment: .leading, spacing: Space.hair) {
                        Text(serviceName)
                        Text(AppModel.serviceBlurb(service))
                            .font(TypeRole.caption).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            if !adding, let terms = model.core?.agentServiceTermsURL(service) {
                HStack(spacing: Space.xs) {
                    Text("Creating it accepts \(serviceName)'s")
                    Link("Terms of Service", destination: terms)
                }
                .font(TypeRole.meta)
                .foregroundStyle(.secondary)
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } leading: {
            Button("Back") { error = nil; flow.back() }
                .disabled(busy)
                .hoverHelp("Choose the service again")
        } buttons: {
            if busy { ProgressView().controlSize(.small) }
            CancelButton(help: "Close without creating a mailbox (Esc)") { model.agentMailboxSheet = nil }
            Button(adding ? "Add Agent" : "Agree and Create") { Task { await submit(flow) } }
                .keyboardShortcut(.defaultAction)
                .disabled(busy || !flow.canSubmit)
                .hoverHelp(adding ? "Add the agent to the service account (Return)"
                           : "Accept \(serviceName)'s terms and create the mailbox (Return)")
        }
        .task(id: flow.target?.id) { await loadDomains(flow) }
    }

    /// Adding on Primitive: the verified own domains to choose from.
    private func loadDomains(_ flow: AgentMailboxFlow) async {
        guard flow.step == .add, flow.service == .primitive, let target = flow.target,
              let domains = try? await model.core?.serviceAccountDomains(target.id) else { return }
        flow.ownDomains = domains.filter(\.verified).map(\.domain)
    }

    private func submit(_ flow: AgentMailboxFlow) async {
        guard !busy, flow.canSubmit else { return }
        busy = true
        defer { busy = false }
        do {
            switch try await model.submitAgentMailbox(flow) {
            case let .created(accountID, address, codeSentTo):
                error = nil
                created = (accountID, address, codeSentTo)
            case .added:
                model.agentMailboxSheet = nil
            }
        } catch {
            self.error = error.message
        }
    }
}

/// The first step: which service.
private struct AgentServiceStep: View {
    @Environment(AppModel.self) private var model
    let flow: AgentMailboxFlow

    var body: some View {
        Dialog(title: "Create an Agent Mailbox",
               message: "An address of its own for one of your agents, to sign up for things and write to people as itself. Choose where it lives.") {
            VStack(spacing: Space.m) {
                ForEach(Array(AgentMailboxFlow.services.enumerated()), id: \.offset) { index, service in
                    AnswerButton(title: AppModel.serviceName(service), detail: AppModel.serviceBlurb(service),
                                 number: index + 1) { flow.choose(service) }
                }
            }
        } buttons: {
            CancelButton(help: "Close without creating a mailbox (Esc)") { model.agentMailboxSheet = nil }
        }
    }
}

/// A service account for the chosen service exists: add to it, or make
/// another.
private struct AgentPathStep: View {
    @Environment(AppModel.self) private var model
    let flow: AgentMailboxFlow

    var body: some View {
        let service = flow.service ?? .primitive
        let existing = flow.existing(service)
        Dialog(title: "Add to \(AppModel.serviceName(service))?",
               message: "An agent added to a service account shares its key, plan and limits; it needs no terms, sign-up or code.") {
            VStack(spacing: Space.m) {
                ForEach(Array(existing.enumerated()), id: \.element.id) { index, account in
                    AnswerButton(title: "Add to \(AppModel.serviceAccountTitle(account))",
                                 detail: Self.agentsLine(account, model.accounts), number: index + 1) {
                        flow.add(to: account)
                    }
                }
                AnswerButton(title: "New Service Account…", detail: Self.newLine(service),
                             number: existing.count + 1) { flow.newServiceAccount() }
            }
        } leading: {
            Button("Back") { flow.back() }
                .hoverHelp("Choose the service again")
        } buttons: {
            CancelButton(help: "Close without creating a mailbox (Esc)") { model.agentMailboxSheet = nil }
        }
    }

    /// "Scout and Writer · verified".
    static func agentsLine(_ account: ServiceAccountSummary, _ accounts: [AccountSummary]) -> String {
        let names = account.agentAccountIds.compactMap { id in
            accounts.first { $0.id == id }.map { $0.displayName ?? $0.email }
        }
        let who = names.isEmpty ? "No agents yet" : ListFormatter.localizedString(byJoining: names)
        return who + (account.verified ? " · verified" : " · not verified")
    }

    static func newLine(_ service: AgentService) -> String {
        switch service {
        case .primitive: "Its own key and limits. Primitive verifies each email only once."
        case .agentMail: "For another email: AgentMail keeps one organisation per email."
        }
    }
}

/// The user's email, typed or chosen from their accounts here.
private struct HumanEmailField: View {
    @Binding var email: String
    let choices: [String]

    var body: some View {
        HStack(spacing: Space.xs) {
            TextField("Your email", text: $email, prompt: Text("you@example.com"))
            if choices.count > 1 {
                Menu {
                    ForEach(choices, id: \.self) { choice in
                        Button(choice) { email = choice } // no-help: menu item
                    }
                } label: {
                    Image(systemName: "person.crop.circle")
                }
                .menuIndicator(.visible)
                .fixedSize()
                .hoverHelp("Use the address of one of your accounts")
                .accessibilityLabel("Your accounts")
            }
        }
    }
}

/// Verify a service account with the user's email: send a code (or, for
/// AgentMail, take the one sent at sign-up), fill it from their own mail
/// when it arrives there, confirm.
struct AgentVerificationForm: View {
    @Environment(AppModel.self) private var model
    let accountID: String
    let address: String
    let justCreated: Bool
    /// AgentMail sent the code at sign-up, to this email: the sheet starts
    /// at the code.
    var codeSentTo: String?

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

    private var service: AgentService { model.serviceAccount(of: accountID)?.service ?? .primitive }
    private var serviceName: String { AppModel.serviceName(service) }

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
                        .hoverHelp("Use the code \(serviceName) sent to \(sentTo); it arrived in your mail here")
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
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
                    .hoverHelp("\(serviceName) emails you a code (Return)")
            } else {
                Button("Verify") { Task { await verify() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(busy || code.trimmingCharacters(in: .whitespaces).isEmpty)
                    .hoverHelp("Confirm the code (Return)")
            }
        }
        .onAppear {
            if let codeSentTo, sentTo == nil {
                email = codeSentTo
                sentTo = codeSentTo
                sends += 1
            }
            if email.isEmpty {
                // AgentMail sends the code to the email it was made with.
                email = model.serviceAccount(of: accountID)?.humanEmail ?? model.preferredHumanEmail
            }
        }
        .task(id: sends) { await watchForCode() }
    }

    private var message: String {
        let lead = justCreated ? "\(address) is ready and syncing. " : ""
        if let sentTo { return lead + "\(serviceName) sent a code to \(sentTo)." }
        return lead + "Verify it with your email to raise its limits and let it write to you. It can always write to people who wrote to it first."
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
            await model.serviceAccountVerified(accountID, plan: plan)
            model.agentMailboxSheet = nil
        } catch {
            self.error = error.message
        }
    }
}

/// The open agent mailbox is not verified yet: what it cannot do, and Verify.
struct AgentLimitsBanner: View {
    @Environment(AppModel.self) private var model
    let text: String

    var body: some View {
        Banner(text, systemImage: "person.badge.shield.checkmark", intent: .attention) {
            if let id = model.openAccountID {
                Button("Verify…") { model.beginAgentVerification(id) }
                    .hoverHelp("Verify the service account with your email to raise its limits and let it write to you")
            }
        }
    }
}
