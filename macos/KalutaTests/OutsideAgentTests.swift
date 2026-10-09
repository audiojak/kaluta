import Foundation
import Testing
@testable import Kaluta

/// Agents outside Kaluta on an agent mailbox (spec §10.1): their approvals
/// in the panel, their name in the activity log, and what Connect an
/// Agent… says. Writing the agents' config files is tested in Rust with a
/// scratch home; nothing here reads the real ~/.claude.json or ~/.codex.
@MainActor
struct OutsideAgentTests {
    @Test func anOutsideAgentsProposalShowsInWhicheverPanelIsOpenAndNothingElseOfItDoes() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        let agent = model.agent
        agent.isPresented = false
        let summary = "Claude Code outside Kaluta, as scout@abc.primitive.email: Send “Hi” to ada@example.com"
        await agent.applyOutside([
            .turnStarted,
            .textDelta(text: "not shown"),
            .actionProposed(actionId: 41, tool: "mail_send", summary: summary, draftId: 9),
        ])
        #expect(agent.isPresented, "the user is asked")
        #expect(agent.pendingProposals == [41])
        #expect(agent.entries.map(\.kind) == [
            .proposal(actionID: 41, tool: "mail_send", summary: summary, draftID: nil, state: .pending),
        ], "no Review…: the draft is in another account's store")
        #expect(!agent.isRunning, "the outside agent's turn is not the panel's")
        await agent.applyOutside([.actionResolved(actionId: 41, approved: false)])
        #expect(agent.pendingProposals.isEmpty)
    }

    @Test func outsideSessionsAreNamedInTheActivityLog() {
        #expect(AgentStore.outsideAgentName("outside-claude-code-1a2b3c4d5e6f") == "Claude Code (outside Kaluta)")
        #expect(AgentStore.outsideAgentName("outside-codex-mcp-client-1a2b3c4d5e6f") == "Codex (outside Kaluta)")
        #expect(AgentStore.outsideAgentName("outside-my-script-00ff00ff00ff") == "my-script (outside Kaluta)")
        #expect(AgentStore.outsideAgentName("agent-1a2b-1") == nil)
        #expect(AgentStore.toolTitle("guide_rules") == "Read the writing guide")
    }

    @Test func connectingSaysWhatIsWrittenWhereAndWhereTheCopyIs() {
        let plan = AgentConnection(
            client: .codex, serverName: "kaluta-scout", configPath: NSHomeDirectory() + "/.codex/config.toml",
            entry: "[mcp_servers.kaluta-scout]\n", fileExists: true, replaces: false, replacesOldName: nil,
            paste: "codex mcp add kaluta-scout -- /Applications/Kaluta.app/Contents/MacOS/kaluta-mcp --mailbox scout@abc.primitive.email")
        #expect(ConnectAgentSheet.whereText(plan) == "Adds this to ~/.codex/config.toml, after copying the file as it is:")
        var replacing = plan
        replacing.replaces = true
        #expect(ConnectAgentSheet.whereText(replacing).hasPrefix("Replaces the kaluta-scout entry in ~/.codex/config.toml"))
        var renamed = plan
        renamed.replacesOldName = "openagc-scout"
        #expect(ConnectAgentSheet.whereText(renamed)
            == "Replaces the openagc-scout entry OpenAGC wrote in ~/.codex/config.toml with this one, after copying the file as it is:")
        var fresh = plan
        fresh.fileExists = false
        #expect(ConnectAgentSheet.whereText(fresh) == "Creates ~/.codex/config.toml with:")
        #expect(ConnectAgentSheet.doneText(plan, backup: NSHomeDirectory() + "/.codex/config.toml.kaluta-backup-1")
            == "Added to Codex; the file as it was is at ~/.codex/config.toml.kaluta-backup-1. Start a new Codex session to use it.")
        #expect(ConnectAgentSheet.doneText(fresh, backup: nil) == "Added to Codex. Start a new Codex session to use it.")
        #expect(ConnectAgentSheet.message("scout@abc.primitive.email").contains("even when Kaluta is closed"))
    }
}
