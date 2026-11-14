import Foundation
import SwiftUI
import Testing
@testable import Kaluta

/// Forwarding rules in the app (spec §14.15): accepting one in Teach
/// Kaluta asks for its mode, with none chosen ahead.
@MainActor
struct ForwardRuleTests {
    private let proposal = ForwardRuleProposalInfo(
        id: 7, statement: "Forward mail from receipts@shop.example starting “Your receipt” to books@example.com",
        sender: "receipts@shop.example", subjectPrefix: "Your receipt", recipient: "books@example.com",
        evidence: [ForwardEvidenceInfo(threadId: "t1", messageId: "m1", subject: "Your receipt #1", at: 0)])

    @Test func acceptingAsksForTheModeAndEscGoesBack() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        // No account is opened and nothing is started: the question and its
        // keys are the session's own state.
        model.forwardRuleProposals = [proposal]
        let item = WorkflowItem(key: "forward:7", threadId: nil, taskId: nil, decision: nil)
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .teach, items: [item], startedAt: 0, resumed: false))
        model.workflowSession = session

        guard case .forward = model.teachItem(item) else {
            Issue.record("the key names the proposal")
            return
        }
        #expect(AppModel.teachLine(.forward(proposal)).kind == "Forwarding rule")
        #expect(model.workflowActions().map(\.command) == [.done, .edit, .reject, .openInFull, .skip])

        // Accept (E) decides nothing yet: it asks, 1 or 2.
        await model.performWorkflow(.done)
        #expect(session.choosingTime == .forwardMode)
        #expect(session.items.first?.decision == nil)
        #expect(model.workflowActions().map(\.title) == ["Forward automatically", "Ask each time", "Cancel"])
        #expect(model.workflowActions().map(\.command) == [.pick1, .pick2, .back])
        await model.performWorkflow(.back)
        #expect(session.choosingTime == nil)
        #expect(model.workflowSession != nil, "Esc leaves the question, not the workflow")

        // Edit… opens the rule editor on the proposal.
        await model.performWorkflow(.edit)
        #expect(model.forwardRuleSheet?.id == "proposal:7")
    }

    @Test func aProposalBecomesARuleInTheModeChosen() {
        let edit = proposal.edit(mode: .askEachTime)
        #expect(edit.sender == "receipts@shop.example" && edit.subjectPrefix == "Your receipt")
        #expect(edit.recipient == "books@example.com" && edit.mode == .askEachTime && edit.enabled)
        #expect(!edit.senderIsDomain)
    }

    @Test func aRuleSaysWhatItHasForwarded() {
        let rule = ForwardRuleInfo(id: 1, sender: "a@x.example", senderIsDomain: false, subjectPrefix: "", recipient: "b@y.example",
                                   mode: .automatic, enabled: true, statement: "", sentCount: 0, lastSentAt: nil)
        #expect(ForwardingSettings.history(rule) == "Nothing forwarded yet")
        var paused = rule
        paused.enabled = false
        #expect(ForwardingSettings.history(paused) == "Paused · nothing forwarded yet")
        var busy = rule
        busy.sentCount = 3
        busy.lastSentAt = 0
        #expect(ForwardingSettings.history(busy).hasPrefix("Forwarded 3, last on "))
        #expect(ForwardingSettings.edit(rule, mode: .askEachTime).mode == .askEachTime)
        #expect(ForwardingSettings.edit(rule, enabled: false).enabled == false)
        let mark = ForwardMarkInfo(ruleId: 1, messageId: "m", recipient: "b@y.example", at: 0)
        #expect(ForwardRuleBar.markText(mark, count: 1).hasPrefix("Forwarded by a rule · b@y.example · "))
        #expect(ForwardRuleBar.markText(mark, count: 2).hasSuffix("(2 emails)"))
    }
}
