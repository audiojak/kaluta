import AppKit
import SwiftUI

/// What Connect a Cloud Agent… and the list of cloud agents ask of the core
/// (spec §10.6): a seam, so tests record the calls and snapshots answer with
/// samples, contacting no server.
protocol RulesAgentCalls: AnyObject, Sendable {
    func rulesConnectInfo(_ accountID: String) async throws(CoreClientError) -> RulesConnectInfo
    func rulesConnectCodeMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesConnectCode
    func rulesAgentTokenMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesAgentToken
    func rulesAgents(_ accountID: String) async throws(CoreClientError) -> [RulesAgent]
    func rulesAgentRevoke(_ accountID: String, agentID: String) async throws(CoreClientError)
}

extension CoreClient: RulesAgentCalls {}

extension AppModel {
    var rulesAgentCalls: (any RulesAgentCalls)? { rulesAgentCallsOverride ?? core }
}

/// How a cloud agent reaches the rules server.
enum CloudAgentRoute: Hashable {
    /// OAuth with a one-time connect code: a claude.ai connector, which
    /// cloud routines use.
    case connector
    /// A static bearer token: Claude Code, the Agent SDK, a script.
    case token
}

/// Connect a Cloud Agent…'s steps (spec §10.6). What it mints lives here
/// only, while the sheet is open: never in the Keychain, a file, the model
/// or a log. Closing the sheet forgets it; it cannot be shown again.
@MainActor @Observable
final class CloudAgentFlow: Identifiable {
    let id = UUID()
    let accountID: String
    /// The mailbox's address.
    let address: String
    @ObservationIgnored private let calls: any RulesAgentCalls

    var name = ""
    var route: CloudAgentRoute = .connector
    private(set) var info: RulesConnectInfo?
    private(set) var code: RulesConnectCode?
    private(set) var token: RulesAgentToken?
    private(set) var working = false
    var error: String?
    /// The connector that signed in with this sheet's code, once the
    /// server lists it.
    private(set) var connected: RulesAgent?
    /// Agents there before this sheet: a connector not among them, named
    /// as the code was, is the one it connected.
    @ObservationIgnored private var known: Set<String>?

    init(accountID: String, address: String, calls: any RulesAgentCalls) {
        self.accountID = accountID
        self.address = address
        self.calls = calls
    }

    /// Ask the server how agents reach it. Without a public URL it signs
    /// no connector in, so the token is the way.
    func load() async {
        error = nil
        do throws(CoreClientError) {
            let info = try await calls.rulesConnectInfo(accountID)
            self.info = info
            if !info.oauth { route = .token }
            known = Set(try await calls.rulesAgents(accountID).map(\.id))
        } catch {
            self.error = error.message
        }
    }

    /// The server signs claude.ai connectors in.
    var offersConnector: Bool { info?.oauth == true }

    var trimmedName: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    var canMint: Bool {
        !trimmedName.isEmpty && info != nil && !working && (route == .token || offersConnector)
    }

    /// A code or token is on screen.
    var isShowing: Bool { code != nil || token != nil }

    func mint() async {
        guard canMint else { return }
        working = true
        defer { working = false }
        do throws(CoreClientError) {
            switch route {
            case .connector: code = try await calls.rulesConnectCodeMint(accountID, name: trimmedName)
            case .token: token = try await calls.rulesAgentTokenMint(accountID, name: trimmedName)
            }
            error = nil
        } catch {
            self.error = error.message
        }
    }

    /// Another code for the same agent: the last one expired, or was mistyped.
    func newCode() async {
        guard code != nil, !working else { return }
        working = true
        defer { working = false }
        do throws(CoreClientError) {
            code = try await calls.rulesConnectCodeMint(accountID, name: code?.name ?? trimmedName)
            error = nil
        } catch {
            self.error = error.message
        }
    }

    /// Whether the connector has signed in with the code yet.
    func checkConnected() async {
        guard let code, connected == nil, let known else { return }
        guard let agents = try? await calls.rulesAgents(accountID) else { return }
        connected = agents.first {
            $0.kind == .connector && $0.revokedAt == nil && $0.name == code.name && !known.contains($0.id)
        }
    }

    /// The sheet closed: what was minted is gone for good.
    func forget() {
        code = nil
        token = nil
    }
}

extension CloudAgentFlow {
    /// The server's host, for messages: `rules.example.com`.
    var host: String {
        guard let base = info?.baseUrl, let url = URL(string: base), let host = url.host() else { return "the server" }
        return url.port.map { "\(host):\($0)" } ?? host
    }

    static func message(_ address: String) -> String {
        "Let an agent in the cloud read \(address)'s published writing guide and the facts you share with cloud agents. It can read nothing else, and you can revoke it here."
    }

    static func noPublicURL(_ host: String) -> String {
        "\(host) has no public address set, so claude.ai cannot sign in to it. Its operator sets OPENAGC_RULES_PUBLIC_URL; until then, use a token."
    }

    /// The MCP server's name in an agent's settings: `openagc-scout-rules`
    /// for `scout@…`, apart from the local `openagc-scout` (Connect an Agent…).
    static func serverName(_ address: String) -> String {
        let local = address.split(separator: "@").first.map(String.init) ?? address
        let slug = local.lowercased().map { $0.isLetter || $0.isNumber ? String($0) : "-" }.joined()
        return "openagc-\(slug.trimmingCharacters(in: CharacterSet(charactersIn: "-")))-rules"
    }

    static func claudeMCPAdd(address: String, mcpURL: String, token: String) -> String {
        "claude mcp add --transport http \(serverName(address)) \(mcpURL) --header \"Authorization: Bearer \(token)\""
    }

    /// The read-only REST, for scripts.
    static func curl(address: String, baseURL: String, token: String) -> String {
        "curl -H \"Authorization: Bearer \(token)\" \"\(baseURL)/v1/m/\(address)/guide?message_type=new\""
    }

    static func tokenWarning(_ address: String) -> String {
        "This token is shown only now; OpenAGC does not keep it. Whoever holds it can read \(address)'s published writing guide and shared facts, and nothing else, until you revoke it here."
    }

    /// "Works once, for 9:42 more."
    static func expiry(_ code: RulesConnectCode, now: Date = .now) -> String {
        let left = Int((Double(code.expiresAt) / 1000 - now.timeIntervalSince1970).rounded(.down))
        guard left > 0 else { return "This code has expired. New Code makes another." }
        return "Works once, for \(left / 60):\(String(format: "%02d", left % 60)) more."
    }

    static func connectedText(_ agent: RulesAgent) -> String {
        "Connected: \(agent.name)\(agent.clientName.map { ", from \($0)" } ?? ""). It appears under Cloud agents."
    }

    /// What the user pastes into a routine's (or agent's) instructions.
    static func instructions(address: String) -> String {
        var lines = [
            "You write email as \(address). Its writing guide and facts come from the OpenAGC rules server's tools.",
            "Before writing each email, call guide_rules with the recipients' addresses (to) and the message type (new, reply or forward), and follow the guide it returns.",
            "When you need a fact about the user or their work (a calendar link, a role, an address), call facts_lookup with a category or a query. Use only facts it returns, and leave out any marked ask before using.",
        ]
        // oagc-gmn7.6: once the server has check_draft and report_send, add
        // a line to check each draft with check_draft and fix what it
        // reports before sending, and one to call report_send after each
        // send with its Message-ID.
        lines += afterSendLines
        return lines.joined(separator: "\n")
    }

    /// Lines about checking drafts and reporting sends; none until the
    /// server has the tools (oagc-gmn7.6).
    static let afterSendLines: [String] = []
}

/// Connect a Cloud Agent… (spec §10.6): name the agent, choose how it
/// connects, then the connect code (with the connector's URL) or the token
/// (with the `claude mcp add` line), shown once, and the routine's
/// instructions.
struct ConnectCloudAgentSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Bindable var flow: CloudAgentFlow
    @FocusState private var focused: Bool

    static let width: CGFloat = 600
    static let help = URL(string: "https://github.com/audiojak/openagc/blob/main/docs/rules-server.md#connect-a-claudeai-connector-or-a-cloud-routine")!
    /// How often the sheet asks whether the connector has signed in.
    static let poll: Duration = .seconds(5)

    var body: some View {
        Dialog(title: "Connect a Cloud Agent", message: CloudAgentFlow.message(flow.address), width: Self.width) {
            if let code = flow.code, let info = flow.info {
                connectorSteps(code, info: info)
            } else if let token = flow.token, let info = flow.info {
                tokenSteps(token, info: info)
            } else {
                form
            }
            if let error = flow.error {
                Label(error, systemImage: "exclamationmark.triangle").font(TypeRole.meta).foregroundStyle(Tone.failure)
                    .fixedSize(horizontal: false, vertical: true)
            }
        } buttons: {
            if flow.isShowing {
                CancelButton(title: "Done", help: "Close; the \(flow.code != nil ? "code" : "token") is not shown again (Esc)") {
                    close()
                }
            } else {
                CancelButton(help: "Close without connecting an agent (Esc)") { close() }
                Button(flow.route == .connector ? "Make Code" : "Make Token") { Task { await flow.mint() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!flow.canMint)
                    .hoverHelp(flow.route == .connector
                        ? "Make a one-time code to enter when claude.ai connects (Return)"
                        : "Make a token for the agent, shown once (Return)")
            }
        }
        .task { await flow.load() }
        .task(id: flow.code?.code) {
            // The connector signs in on claude.ai: say so when it has.
            while flow.code != nil, flow.connected == nil, !Task.isCancelled {
                try? await Task.sleep(for: Self.poll)
                await flow.checkConnected()
            }
        }
        .onAppear { focused = true }
        .onDisappear { flow.forget() }
    }

    private func close() {
        flow.forget()
        dismiss()
    }

    // MARK: Name and route

    private var form: some View {
        VStack(alignment: .leading, spacing: Space.l) {
            TextField("Name, such as Weekly outreach routine", text: $flow.name)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit { Task { await flow.mint() } }
            VStack(alignment: .leading, spacing: Space.s) {
                Text("It connects as").font(TypeRole.groupLabel)
                Picker("It connects as", selection: $flow.route) {
                    Text("A claude.ai connector or cloud routine (recommended)").tag(CloudAgentRoute.connector)
                        .disabled(!flow.offersConnector)
                    Text("Claude Code, the Agent SDK or a script").tag(CloudAgentRoute.token)
                }
                .pickerStyle(.radioGroup)
                .labelsHidden()
                .hoverHelp("A connector signs in with a one-time code; anything else is given a token")
                Text(flow.route == .connector
                    ? "OpenAGC shows a one-time code, which you enter when claude.ai connects to the server."
                    : "OpenAGC shows a token once, for the agent's settings.")
                    .font(TypeRole.caption).foregroundStyle(.secondary)
            }
            if flow.info == nil, flow.error == nil {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text("Asking the server how agents connect…").foregroundStyle(.secondary)
                }
                .font(TypeRole.meta)
            } else if let info = flow.info, !info.oauth {
                Label(CloudAgentFlow.noPublicURL(flow.host), systemImage: "exclamationmark.triangle")
                    .font(TypeRole.caption).foregroundStyle(Tone.caution)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if flow.working {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text(flow.route == .connector ? "Making a code…" : "Making a token…").foregroundStyle(.secondary)
                }
                .font(TypeRole.meta)
            }
        }
    }

    // MARK: A connector

    private func connectorSteps(_ code: RulesConnectCode, info: RulesConnectInfo) -> some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            step("1. In claude.ai, open Customize › Connectors › Add custom connector, with this URL. Choose Sign in now, and Register automatically for the OAuth client.") {
                CopyableText(text: info.mcpUrl, help: "Copy the server's MCP address, for the connector")
            }
            step("2. Choose Connect. On the server's page, enter this code:") {
                VStack(alignment: .leading, spacing: Space.xs) {
                    HStack(spacing: Space.m) {
                        Text(code.code).font(TypeRole.display).monospaced().textSelection(.enabled)
                            .accessibilityLabel("Connect code \(code.code)")
                        Spacer(minLength: 0)
                        CopyButton(text: code.code, concealed: true, help: "Copy the code, to paste on the server's page")
                        Button("New Code") { Task { await flow.newCode() } }
                            .disabled(flow.working || flow.connected != nil)
                            .hoverHelp("Make another code for this agent; use it if this one expired or was mistyped")
                    }
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Text(CloudAgentFlow.expiry(code, now: context.date))
                            .font(TypeRole.caption).foregroundStyle(.secondary).monospacedDigit()
                    }
                }
            }
            step("3. Add the connector to the routine, and paste this into its instructions:") {
                CopyableText(text: CloudAgentFlow.instructions(address: flow.address), monospaced: false,
                             help: "Copy the instructions, for the routine's prompt")
            }
            if let connected = flow.connected {
                Label(CloudAgentFlow.connectedText(connected), systemImage: "checkmark.circle")
                    .foregroundStyle(Tone.approved).fixedSize(horizontal: false, vertical: true)
            } else {
                HStack(spacing: Space.m) {
                    ProgressView().controlSize(.small)
                    Text("Waiting for the agent to sign in…").foregroundStyle(.secondary)
                    Link("Steps in the guide", destination: Self.help)
                        .hoverHelp("Open the rules server's guide at connecting a claude.ai connector")
                }
                .font(TypeRole.meta)
            }
        }
    }

    // MARK: A token

    private func tokenSteps(_ token: RulesAgentToken, info: RulesConnectInfo) -> some View {
        VStack(alignment: .leading, spacing: Space.xl) {
            Label(CloudAgentFlow.tokenWarning(flow.address), systemImage: "exclamationmark.triangle")
                .font(TypeRole.meta)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .card(.caution)
            step("The token, for an agent's Authorization: Bearer header:") {
                CopyableText(text: token.token, concealed: true, help: "Copy the token")
            }
            step("Claude Code, in Terminal:") {
                CopyableText(text: CloudAgentFlow.claudeMCPAdd(address: flow.address, mcpURL: info.mcpUrl,
                                                               token: token.token),
                             concealed: true, help: "Copy the command that adds the server to Claude Code")
            }
            step("A script, through the read-only REST:") {
                CopyableText(text: CloudAgentFlow.curl(address: flow.address, baseURL: info.baseUrl, token: token.token),
                             concealed: true, help: "Copy an example request for the writing guide")
            }
            step("Instructions for the agent:") {
                CopyableText(text: CloudAgentFlow.instructions(address: flow.address), monospaced: false,
                             help: "Copy the instructions, for the agent's prompt")
            }
        }
    }

    private func step(_ title: String, @ViewBuilder content: () -> some View) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(title).font(TypeRole.meta).fixedSize(horizontal: false, vertical: true)
            content()
        }
    }
}

/// Text to copy as it is, in a quiet well, with Copy beside it.
struct CopyableText: View {
    let text: String
    var monospaced = true
    /// Kept out of clipboard managers (a token, or a line holding one).
    var concealed = false
    let help: String

    var body: some View {
        HStack(alignment: .top, spacing: Space.m) {
            Text(text)
                .font(monospaced ? TypeRole.codeCaption : TypeRole.caption)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(Space.m)
                .background(.quaternary, in: RoundedRectangle(cornerRadius: Radius.control))
            CopyButton(text: text, concealed: concealed, help: help)
        }
    }
}

/// Copy, then "Copied" for a moment.
struct CopyButton: View {
    let text: String
    var concealed = false
    let help: String
    @State private var copied = false

    var body: some View {
        Button(copied ? "Copied" : "Copy") {
            Self.copy(text, concealed: concealed)
            copied = true
            Task {
                try? await Task.sleep(for: .seconds(2))
                copied = false
            }
        }
        .hoverHelp(help)
    }

    static func copy(_ text: String, concealed: Bool) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        // Clipboard managers leave concealed items out.
        if concealed {
            NSPasteboard.general.setString("", forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType"))
        }
    }
}

/// The cloud agents connected to an agent mailbox on its rules server, for
/// its Settings row: listed, refreshed, revoked.
@MainActor @Observable
final class CloudAgentList {
    let accountID: String
    @ObservationIgnored private let calls: any RulesAgentCalls
    /// Agents not revoked, oldest first.
    private(set) var agents: [RulesAgent] = []
    private(set) var loaded = false
    var error: String?

    init(accountID: String, calls: any RulesAgentCalls) {
        self.accountID = accountID
        self.calls = calls
    }

    func load() async {
        do throws(CoreClientError) {
            agents = try await calls.rulesAgents(accountID).filter { $0.revokedAt == nil }
            error = nil
        } catch {
            self.error = "Could not list cloud agents: \(error.message)"
        }
        loaded = true
    }

    func revoke(_ agent: RulesAgent) async {
        var failure: String?
        do throws(CoreClientError) {
            try await calls.rulesAgentRevoke(accountID, agentID: agent.id)
        } catch {
            failure = "Could not revoke \(agent.name): \(error.message)"
        }
        await load()
        if let failure { error = failure }
    }

    /// "Connector · Claude · connected 2 days ago · last used 1 hour ago".
    static func detail(_ agent: RulesAgent, now: Date = .now) -> String {
        func ago(_ millis: Int64) -> String {
            DateStyle.relative(Date(timeIntervalSince1970: TimeInterval(millis) / 1000), to: now)
        }
        var parts: [String]
        switch agent.kind {
        case .connector:
            parts = ["Connector"]
            if let client = agent.clientName { parts.append(client) }
            parts.append("connected \(ago(agent.createdAt))")
        case .token:
            parts = ["Token", "made \(ago(agent.createdAt))"]
        }
        parts.append(agent.lastUsedAt.map { "last used \(ago($0))" } ?? "not used yet")
        return parts.joined(separator: " · ")
    }

    static func revokeTitle(_ agent: RulesAgent) -> String { "Revoke \(agent.name)?" }

    static func revokeMessage(_ agent: RulesAgent) -> String {
        switch agent.kind {
        case .connector:
            "It stops at its next request: the connector's sessions end, and connecting it again takes a new code."
        case .token:
            "Its token stops working at its next request; whatever uses it needs a new one."
        }
    }

    static let emptyText = "None yet. A connected agent can read this mailbox's published writing guide and shared facts, and nothing else."
}

/// Under the Rules server line while it publishes: Connect a Cloud Agent…
/// and the agents connected, each with Revoke….
struct CloudAgentsSection: View {
    @Environment(AppModel.self) private var model
    let account: AccountSummary
    @State private var list: CloudAgentList?
    @State private var flow: CloudAgentFlow?
    @State private var revoking: RulesAgent?

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            LabeledContent("Cloud agents") {
                Button("Connect a Cloud Agent…") { connect() }
                    .hoverHelp("Give a claude.ai routine, Claude Code elsewhere or a script this mailbox's writing guide and shared facts")
                    .disabled(model.rulesAgentCalls == nil)
            }
            if let list {
                if list.agents.isEmpty, list.loaded, list.error == nil {
                    Text(CloudAgentList.emptyText).font(TypeRole.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                TimelineView(.periodic(from: .now, by: 60)) { context in
                    VStack(alignment: .leading, spacing: Space.s) {
                        ForEach(list.agents, id: \.id) { agent in
                            row(agent, now: context.date)
                        }
                    }
                }
                if let error = list.error {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(TypeRole.caption).foregroundStyle(Tone.failure)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .task(id: "\(account.id) \(model.rulesRevision)") {
            guard let calls = model.rulesAgentCalls else { return }
            let current = list ?? CloudAgentList(accountID: account.id, calls: calls)
            list = current
            await current.load()
        }
        .sheet(item: $flow, onDismiss: { Task { await list?.load() } }) { flow in
            ConnectCloudAgentSheet(flow: flow)
        }
        .confirmationDialog(revoking.map(CloudAgentList.revokeTitle) ?? "", isPresented: Binding(
            get: { revoking != nil }, set: { if !$0 { revoking = nil } }
        ), presenting: revoking) { agent in
            Button("Revoke", role: .destructive) { Task { await list?.revoke(agent) } } // no-help: confirmation dialog button
        } message: { agent in
            Text(CloudAgentList.revokeMessage(agent))
        }
    }

    private func row(_ agent: RulesAgent, now: Date) -> some View {
        HStack(spacing: Space.m) {
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(agent.name)
                Text(CloudAgentList.detail(agent, now: now)).font(TypeRole.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Button("Revoke…") { revoking = agent }
                .hoverHelp("Stop \(agent.name) reading this mailbox's guide and facts")
        }
        .padding(.leading, Space.xl)
    }

    private func connect() {
        guard let calls = model.rulesAgentCalls else { return }
        flow = CloudAgentFlow(accountID: account.id, address: account.email, calls: calls)
    }
}
