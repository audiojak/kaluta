import AppKit
import SwiftUI

/// Settings › Accounts: what a service account's agents share (spec §7.9,
/// ADR 0015): the service and its plan, verification, the limits, adding an
/// agent, the API key and, on Primitive, whom it writes to and its own
/// domains. The agents' own rows follow it in the same section.
struct ServiceAccountPane: View {
    @Environment(AppModel.self) private var model
    let service: ServiceAccountSummary

    /// Copy API Key asks first: whoever holds the key can use the mailboxes.
    @State private var confirmingCopy = false
    @State private var sendRules: [AgentSendRule]?
    @State private var domains: [AgentDomain]?
    @State private var copyError: String?
    @State private var copied: String?

    private var plan: AgentMailboxPlan? { model.servicePlans[service.id] ?? service.plan }
    private var verified: Bool { plan?.verified ?? service.verified }
    private var agents: [AccountSummary] {
        service.agentAccountIds.compactMap { id in model.accounts.first { $0.id == id } }
    }
    private var serviceName: String { AppModel.serviceName(service.service) }

    var body: some View {
        LabeledContent("Service") {
            Text(Self.planText(plan, service: service.service)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .task(id: service.id) { await load() }
        LabeledContent("Verified") {
            HStack(spacing: Space.m) {
                Text(Self.verifiedText(verified: verified, email: plan?.email ?? service.humanEmail))
                    .foregroundStyle(.secondary)
                if !verified, let first = agents.first {
                    Button("Verify…") { model.beginAgentVerification(first.id) }
                        .hoverHelp("Verify the service account with your email; every agent in it is verified with it")
                }
            }
        }
        if let limits = model.core?.serviceAccountLimits(service.id) {
            // A paragraph: under its label, not squeezed beside it.
            VStack(alignment: .leading, spacing: Space.xs) {
                Text("Limits")
                Text(limits).font(TypeRole.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
        }
        if service.service == .primitive {
            LabeledContent("Can write to") {
                Text(Self.sendRulesText(sendRules)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .hoverHelp("Whom Primitive lets these agents write to. Sending to anyone is something Primitive grants on request; ask them at support, naming the service account's address")
            LabeledContent("Domains") {
                HStack(spacing: Space.m) {
                    Text(Self.domainsText(domains)).foregroundStyle(.secondary)
                    if let first = agents.first {
                        Button("Add Domain…") { model.beginAgentDomain(first.id) }
                            .hoverHelp("Put the service account on a domain you own, such as agents.example.com; its agents can then take addresses there")
                    }
                }
            }
        }
        HStack {
            Button("Add Agent…") { model.beginAddAgent(to: service.id) }
                .hoverHelp("Give another agent an address on this service account: a name only, no sign-up or code")
            Button("Copy API Key…") { confirmingCopy = true }
                .hoverHelp("Copy a \(serviceName) key, for an agent that calls \(serviceName) itself")
            if let dashboard = model.core?.agentServiceDashboardURL(service.service) {
                Button("Open \(serviceName)…") { NSWorkspace.shared.open(dashboard) }
                    .hoverHelp(Self.dashboardHelp(plan, service: service.service))
            }
        }
        .confirmationDialog(Self.copyTitle(service), isPresented: $confirmingCopy) {
            // AgentMail, verified: a key for one inbox is the safer choice,
            // so it comes first.
            if Self.offersInboxKeys(service) {
                ForEach(agents, id: \.id) { agent in
                    Button("Copy Key for \(agent.displayName ?? agent.email) Only") { // no-help: confirmation dialog button
                        Task { await copyInboxKey(agent) }
                    }
                }
            }
            Button(Self.offersInboxKeys(service) ? "Copy Organisation Key" : "Copy API Key") { // no-help: confirmation dialog button
                do throws(CoreClientError) {
                    if let key = try model.core?.serviceAccountAPIKey(service.id) { put(key, copied: serviceName) }
                } catch {
                    copyError = error.message
                }
            }
        } message: {
            Text(Self.copyMessage(service, agents: agents.map { $0.displayName ?? $0.email }))
        }
        if let copyError {
            Text(copyError).font(TypeRole.caption).foregroundStyle(Tone.failure)
        } else if let copied {
            Text("Copied the \(copied) key.").font(TypeRole.caption).foregroundStyle(.secondary)
        }
    }

    /// The plan once per service account; Primitive's send rules and domains.
    private func load() async {
        await model.refreshServicePlan(service.id)
        guard service.service == .primitive, let core = model.core else { return }
        sendRules = (try? await core.serviceAccountSendRules(service.id)) ?? []
        domains = try? await core.serviceAccountDomains(service.id)
    }

    private func copyInboxKey(_ agent: AccountSummary) async {
        do {
            if let key = try await model.core?.agentInboxAPIKey(agent.id) {
                put(key, copied: "\(agent.displayName ?? agent.email) inbox")
            }
        } catch {
            copyError = error.message
        }
    }

    private func put(_ key: String, copied what: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(key, forType: .string)
        // Clipboard managers leave concealed items out.
        NSPasteboard.general.setString("", forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
        copyError = nil
        copied = what
    }
}

extension ServiceAccountPane {
    /// Verified AgentMail makes keys for one inbox (`agentInboxApiKey`).
    static func offersInboxKeys(_ service: ServiceAccountSummary) -> Bool {
        service.service == .agentMail && service.verified
    }

    static func copyTitle(_ service: ServiceAccountSummary) -> String {
        offersInboxKeys(service) ? "Copy an API key?" : "Copy the service account's API key?"
    }

    /// What the key reaches: every agent in the service account.
    static func copyMessage(_ service: ServiceAccountSummary, agents: [String]) -> String {
        let who = agents.isEmpty ? "its agents" : ListFormatter.localizedString(byJoining: agents)
        let reach = "The service account's key reaches every agent in it (\(who)): whoever has it can read their mail and send as them."
        if offersInboxKeys(service) {
            return "A key for one agent's inbox reaches only that inbox, and is the safer choice. " + reach
                + " Give a key only to an agent you run."
        }
        return reach + " Give it only to an agent you run."
    }

    /// The service and its plan in a line.
    static func planText(_ plan: AgentMailboxPlan?, service: AgentService) -> String {
        switch service {
        case .primitive:
            guard let plan else { return "Primitive" }
            // Primitive still limits whom it writes to (spec §7.9).
            if plan.verified { return "Primitive · \(plan.name) plan · writes to you, people who wrote first and your own domains" }
            return "Primitive · \(plan.name) plan · replies only, \(plan.sendPerHour) an hour"
        case .agentMail:
            // Its limits are monthly and per new recipient: the Limits row.
            guard let plan else { return "AgentMail" }
            return "AgentMail · \(plan.name) plan"
        }
    }

    static func verifiedText(verified: Bool, email: String?) -> String {
        if verified { return email.map { "With \($0)" } ?? "Yes" }
        return "Not yet"
    }

    /// Where the service account may send, in a line (spec §7.9).
    static func sendRulesText(_ rules: [AgentSendRule]?) -> String {
        guard let rules else { return "Checking…" }
        if rules.contains(where: { $0.kind == "any_recipient" }) { return "Anyone" }
        var parts: [String] = []
        let addresses = rules.filter { $0.kind == "address" }.count
        if addresses > 0 { parts.append(addresses == 1 ? "1 address that wrote to it" : "\(addresses) addresses that wrote to it") }
        let domains = rules.filter { $0.kind == "your_domain" }.compactMap(\.value)
        if !domains.isEmpty { parts.append("anyone at " + domains.joined(separator: ", ")) }
        if rules.contains(where: { $0.kind == "managed_zone" }) { parts.append("other Primitive mailboxes") }
        return parts.isEmpty ? "Nobody yet" : parts.joined(separator: " · ")
    }

    /// The own domains in a line: "agents.example.com · waiting.example.com
    /// (records not found yet)".
    static func domainsText(_ domains: [AgentDomain]?) -> String {
        guard let domains else { return "Checking…" }
        if domains.isEmpty { return "None" }
        return domains.map { $0.verified ? $0.domain : "\($0.domain) (records not found yet)" }.joined(separator: " · ")
    }

    /// How to sign in to the service's dashboard.
    static func dashboardHelp(_ plan: AgentMailboxPlan?, service: AgentService) -> String {
        let name = AppModel.serviceName(service)
        return switch plan?.email {
        case let .some(email): "Open \(name)'s dashboard in your browser; sign in as \(email), the email this service account was verified with"
        default: "Open \(name)'s dashboard in your browser; verify the service account first, then sign in with that email"
        }
    }
}
