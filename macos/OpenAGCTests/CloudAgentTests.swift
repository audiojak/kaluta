import Foundation
import Testing
@testable import OpenAGC

/// Records what Connect a Cloud Agent… and the cloud agents' list ask of
/// the core, and answers as a rules server would. No server is contacted.
private final class RecordingRulesCalls: RulesAgentCalls, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    private var listed: [RulesAgent]
    private var minted = 0
    let oauth: Bool
    var failRevoke = false

    init(oauth: Bool = true, agents: [RulesAgent] = []) {
        self.oauth = oauth
        listed = agents
    }

    var calls: [String] { lock.withLock { recorded } }
    private func record(_ call: String) { lock.withLock { recorded.append(call) } }

    /// The connector signs in on claude.ai with the code: the server lists it.
    func signIn(_ name: String) {
        lock.withLock {
            listed.append(RulesAgent(id: "grant\(listed.count)", name: name, kind: .connector, clientName: "Claude",
                                     createdAt: 1_791_500_000_000, revokedAt: nil, lastUsedAt: nil))
        }
    }

    func rulesConnectInfo(_ accountID: String) async throws(CoreClientError) -> RulesConnectInfo {
        record("info \(accountID)")
        return oauth
            ? RulesConnectInfo(baseUrl: "https://rules.example.com", mcpUrl: "https://rules.example.com/mcp", oauth: true)
            : RulesConnectInfo(baseUrl: "http://127.0.0.1:8787", mcpUrl: "http://127.0.0.1:8787/mcp", oauth: false)
    }

    func rulesConnectCodeMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesConnectCode {
        record("code \(name)")
        guard oauth else {
            throw CoreClientError(kind: .invalidInput, message: "set OPENAGC_RULES_PUBLIC_URL; use an agent token instead")
        }
        let n = lock.withLock { minted += 1; return minted }
        return RulesConnectCode(code: n == 1 ? "ABCDE-FGHJK" : "MNPQR-STUVW", name: name,
                                expiresAt: Int64((Date().timeIntervalSince1970 + 600) * 1000))
    }

    func rulesAgentTokenMint(_ accountID: String, name: String) async throws(CoreClientError) -> RulesAgentToken {
        record("token \(name)")
        let token = RulesAgentToken(id: "0123456789abcdef", name: name, token: "oagc_agt_0123456789abcdef_secret")
        lock.withLock {
            listed.append(RulesAgent(id: token.id, name: name, kind: .token, clientName: nil, createdAt: 1, revokedAt: nil,
                                     lastUsedAt: nil))
        }
        return token
    }

    func rulesAgents(_ accountID: String) async throws(CoreClientError) -> [RulesAgent] {
        record("agents")
        return lock.withLock { listed }
    }

    func rulesAgentRevoke(_ accountID: String, agentID: String) async throws(CoreClientError) {
        record("revoke \(agentID)")
        if failRevoke { throw CoreClientError(kind: .network, message: "Could not reach rules.example.com") }
        lock.withLock {
            listed = listed.map { a in
                var a = a
                if a.id == agentID { a.revokedAt = 2 }
                return a
            }
        }
    }
}

/// Connect a Cloud Agent… (spec §10.6): the connector's code and the token,
/// each shown once and never kept; a server without a public URL; the list
/// of agents with Revoke.
@MainActor
@Suite(.serialized)
struct CloudAgentTests {
    private let address = "research-scout@demo.primitive.email"

    @Test func aConnectorGetsTheURLAOneTimeCodeAndTheRoutineInstructions() async throws {
        let calls = RecordingRulesCalls()
        let flow = CloudAgentFlow(accountID: "a1", address: address, calls: calls)
        await flow.load()
        #expect(flow.route == .connector, "recommended")
        #expect(flow.offersConnector)
        #expect(!flow.canMint, "a name first")
        flow.name = "  Weekly outreach routine "
        #expect(flow.canMint)
        await flow.mint()
        #expect(calls.calls == ["info a1", "agents", "code Weekly outreach routine"])
        let code = try #require(flow.code)
        #expect(code.code == "ABCDE-FGHJK")
        #expect(flow.token == nil && flow.isShowing)
        #expect(flow.info?.mcpUrl == "https://rules.example.com/mcp")
        #expect(flow.host == "rules.example.com")
        #expect(CloudAgentFlow.expiry(code).hasPrefix("Works once, for 9:5") || CloudAgentFlow.expiry(code)
            .hasPrefix("Works once, for 10:00"), "\(CloudAgentFlow.expiry(code))")

        // Not signed in yet; then claude.ai connects with the code.
        await flow.checkConnected()
        #expect(flow.connected == nil)
        calls.signIn("Someone else")
        await flow.checkConnected()
        #expect(flow.connected == nil, "only the agent this code names")
        calls.signIn("Weekly outreach routine")
        await flow.checkConnected()
        let connected = try #require(flow.connected)
        #expect(CloudAgentFlow.connectedText(connected).hasPrefix("Connected: Weekly outreach routine, from Claude."))

        await flow.newCode()
        #expect(flow.code?.code == "MNPQR-STUVW", "New Code mints another for the same agent")
        #expect(calls.calls.last == "code Weekly outreach routine")

        let instructions = CloudAgentFlow.instructions(address: address)
        #expect(instructions.contains("You write email as \(address)"))
        #expect(instructions.contains("call guide_rules with the recipients' addresses (to) and the message type"))
        #expect(instructions.contains("call facts_lookup"))
        #expect(!instructions.contains("check_draft"), "not on the server yet (oagc-gmn7.6)")

        flow.forget()
        #expect(flow.code == nil && !flow.isShowing)
    }

    @Test func aTokenIsShownOnceWithTheClaudeCodeLineAndNeverAgain() async throws {
        let calls = RecordingRulesCalls()
        let model = try await modelWith(calls)
        let opened = try #require(model.rulesAgentCalls)
        let flow = CloudAgentFlow(accountID: "a1", address: address, calls: opened)
        await flow.load()
        flow.route = .token
        flow.name = "Nightly digest script"
        await flow.mint()
        let token = try #require(flow.token)
        #expect(flow.code == nil)
        #expect(calls.calls.filter { $0.hasPrefix("token") } == ["token Nightly digest script"])
        #expect(CloudAgentFlow.claudeMCPAdd(address: address, mcpURL: "https://rules.example.com/mcp", token: token.token)
            == "claude mcp add --transport http openagc-research-scout-rules https://rules.example.com/mcp --header \"Authorization: Bearer oagc_agt_0123456789abcdef_secret\"")
        #expect(CloudAgentFlow.curl(address: address, baseURL: "https://rules.example.com", token: token.token)
            == "curl -H \"Authorization: Bearer oagc_agt_0123456789abcdef_secret\" \"https://rules.example.com/v1/m/\(address)/guide?message_type=new\"")
        let warning = CloudAgentFlow.tokenWarning(address)
        #expect(warning.contains("shown only now") && warning.contains("nothing else") && warning.contains("revoke"))

        // Closed: the token is gone, and nothing can show it again.
        flow.forget()
        #expect(flow.token == nil)
        let again = CloudAgentFlow(accountID: "a1", address: address, calls: opened)
        await again.load()
        #expect(again.token == nil && again.code == nil && again.name.isEmpty)
        #expect(calls.calls.filter { $0.hasPrefix("token") }.count == 1, "only minting shows a token")
        let list = CloudAgentList(accountID: "a1", calls: opened)
        await list.load()
        #expect(list.agents.map(\.name) == ["Nightly digest script"])
        #expect(!String(describing: list.agents).contains("secret"), "the list carries ids and names only")
    }

    @Test func aServerWithoutAPublicURLOffersTheTokenInstead() async throws {
        let calls = RecordingRulesCalls(oauth: false)
        let flow = CloudAgentFlow(accountID: "a1", address: address, calls: calls)
        flow.name = "Weekly outreach routine"
        await flow.load()
        #expect(!flow.offersConnector)
        #expect(flow.route == .token, "the token is the way")
        #expect(flow.canMint)
        flow.route = .connector
        #expect(!flow.canMint, "no code without OAuth")
        await flow.mint()
        #expect(!calls.calls.contains { $0.hasPrefix("code") })
        #expect(CloudAgentFlow.noPublicURL(flow.host)
            == "127.0.0.1:8787 has no public address set, so claude.ai cannot sign in to it. Its operator sets OPENAGC_RULES_PUBLIC_URL; until then, use a token.")
        flow.route = .token
        await flow.mint()
        #expect(flow.token != nil)
    }

    @Test func theListShowsLiveAgentsRefreshesAndRevokes() async throws {
        let calls = RecordingRulesCalls(agents: [
            RulesAgent(id: "aa", name: "Weekly outreach routine", kind: .connector, clientName: "Claude",
                       createdAt: 1_000, revokedAt: nil, lastUsedAt: 2_000),
            RulesAgent(id: "bb", name: "Old script", kind: .token, clientName: nil, createdAt: 1_000, revokedAt: 1_500,
                       lastUsedAt: nil),
        ])
        let list = CloudAgentList(accountID: "a1", calls: calls)
        await list.load()
        #expect(list.agents.map(\.id) == ["aa"], "revoked agents are gone from the list")

        // Connecting another (the sheet closes): the list reads again.
        _ = try await calls.rulesAgentTokenMint("a1", name: "Nightly digest script")
        await list.load()
        #expect(list.agents.map(\.name) == ["Weekly outreach routine", "Nightly digest script"])

        let connector = list.agents[0]
        #expect(CloudAgentList.revokeTitle(connector) == "Revoke Weekly outreach routine?")
        #expect(CloudAgentList.revokeMessage(connector).contains("the connector's sessions end"))
        await list.revoke(connector)
        #expect(calls.calls.suffix(2) == ["revoke aa", "agents"])
        #expect(list.agents.map(\.name) == ["Nightly digest script"])
        #expect(list.error == nil)

        calls.failRevoke = true
        await list.revoke(list.agents[0])
        #expect(list.error == "Could not revoke Nightly digest script: Could not reach rules.example.com")
        #expect(list.agents.count == 1)
    }

    @Test func eachAgentSaysItsKindAndWhenItWasLastUsed() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let day: Int64 = 86_400_000
        let connector = RulesAgent(id: "aa", name: "Weekly outreach routine", kind: .connector, clientName: "Claude",
                                   createdAt: 1_000_000_000 - 2 * day, revokedAt: nil, lastUsedAt: 1_000_000_000 - 3_600_000)
        let detail = CloudAgentList.detail(connector, now: now)
        #expect(detail.hasPrefix("Connector · Claude · connected "))
        #expect(detail.contains("· last used "))
        let token = RulesAgent(id: "bb", name: "Script", kind: .token, clientName: nil, createdAt: 1_000_000_000 - day,
                               revokedAt: nil, lastUsedAt: nil)
        #expect(CloudAgentList.detail(token, now: now).hasPrefix("Token · made "))
        #expect(CloudAgentList.detail(token, now: now).hasSuffix("· not used yet"))
        #expect(CloudAgentFlow.serverName("Research.Scout@x.example") == "openagc-research-scout-rules")
    }

    @Test func anExpiredCodeSaysSo() {
        let code = RulesConnectCode(code: "ABCDE-FGHJK", name: "x", expiresAt: 1_000_000)
        #expect(CloudAgentFlow.expiry(code, now: Date(timeIntervalSince1970: 1_000)) == "This code has expired. New Code makes another.")
        #expect(CloudAgentFlow.expiry(code, now: Date(timeIntervalSince1970: 1_000 - 65)) == "Works once, for 1:05 more.")
    }

    private func modelWith(_ calls: RecordingRulesCalls) async throws -> AppModel {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        model.rulesAgentCallsOverride = calls
        return model
    }
}
