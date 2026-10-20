import Foundation
import Testing
@testable import Kaluta

struct AgentSettingsTests {
    private func info(_ id: String, _ status: AgentStatusInfo) -> AgentProviderInfo {
        AgentProviderInfo(id: id, name: id == "codex" ? "Codex" : "Claude", status: status)
    }

    @Test func statusesReadClearly() {
        #expect(AgentStatusText(info("claude-code", .ready(version: "2.1.34"))).isReady)
        #expect(AgentStatusText(info("claude-code", .ready(version: "2.1.34"))).detail == "Ready — version 2.1.34")
        #expect(AgentStatusText(info("codex", .notInstalled)).detail.contains("brew install codex"))
        #expect(AgentStatusText(info("claude-code", .notInstalled)).detail.contains("claude-code"))
        #expect(AgentStatusText(info("codex", .notAuthenticated(version: "0.150.0"))).detail.contains("codex login"))
        let old = AgentStatusText(info("claude-code", .updateRequired(version: "1.0.0", minimum: "2.1.0")))
        #expect(!old.isReady && old.detail.contains("2.1.0"))
        #expect(AgentStatusText(info("codex", .error(message: "boom"))).detail.hasSuffix("boom"))
    }

    /// A refused model says what Kaluta used instead.
    @Test func aRefusedModelSaysWhatWasUsedInstead() {
        let own = AgentModelFallback(requested: nil, refused: "gpt-6.1-sol", used: "gpt-5.5")
        #expect(AgentModelSection.fallbackNote(own, agent: "Codex")
            == "Codex's own setting, gpt-6.1-sol, isn't available to your account, so Kaluta is using gpt-5.5 instead.")
        let chosen = AgentModelFallback(requested: "gpt-x", refused: "gpt-x", used: "gpt-5.5")
        #expect(AgentModelSection.fallbackNote(chosen, agent: "Codex")
            == "gpt-x isn't available to your account, so Kaluta is using gpt-5.5 instead.")
    }
}

@MainActor
struct AgentPermissionTests {
    @Test func thePolicyFollowsTheUsersChoices() async throws {
        let dir = CoreClient.testScratch()
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        let model = AppModel(core: try CoreClient(dataDirectory: dir), defaults: defaults)
        defaults.set(["mail_archive"], forKey: AppModel.agentApprovalKey)
        model.applyAgentPolicy()
        #expect(model.core?.agentPolicy == ["mail_archive"])
        #expect(model.core?.configurableAgentTools.contains("mail_send") == false, "sends always ask")
        defaults.set([String](), forKey: AppModel.agentApprovalKey)
        model.applyAgentPolicy()
        #expect(model.core?.agentPolicy == [])
    }

    @Test func activityExportsAsJSONLinesOldestFirst() throws {
        let actions = [
            AgentActionInfo(actionId: 2, sessionId: "s", tool: "mail_send", argumentsJson: #"{"draft_id":3}"#,
                            risk: "external", state: "rejected", resultSummary: nil, createdAt: 20, resolvedAt: 25),
            AgentActionInfo(actionId: 1, sessionId: "s", tool: "mail_search", argumentsJson: #"{"query":"x"}"#,
                            risk: "read_only", state: "done", resultSummary: "t1, t2", createdAt: 10, resolvedAt: 11),
        ]
        let lines = AgentActivityView.jsonLines(actions).split(separator: "\n")
        #expect(lines.count == 2)
        let first = try #require(try JSONSerialization.jsonObject(with: Data(lines[0].utf8)) as? [String: Any])
        #expect(first["action_id"] as? Int == 1)
        #expect((first["arguments"] as? [String: Any])?["query"] as? String == "x")
        #expect(first["result"] as? String == "t1, t2")
        #expect(AgentActivityView.stateText("denied") == "Blocked")
    }
}
