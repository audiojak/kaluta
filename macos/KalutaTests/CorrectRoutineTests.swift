import Foundation
import Testing
@testable import Kaluta

/// Correct it once (spec §14.13 amendment 2026-11-18): the notice asks why,
/// and the reason goes with the proposal.
@MainActor
struct CorrectRoutineTests {
    @Test func theReasonIsAskedOnTheNoticeAndKeptWithTheProposal() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        // A conversation not filed by a routine: an ordinary move, no question.
        let thread = try #require(model.threads.rows.first?.id)
        await model.correctRoutine(threadID: thread)
        #expect(model.reasonRequest == nil)
        // Asked: the typed reason is kept with that conversation.
        let notice = try #require(model.undo.notice)
        model.reasonRequest = AppModel.ReasonRequest(noticeID: notice.id, proposalID: 999, threadID: thread)
        await model.saveRoutineReason("  ")
        #expect(model.reasonRequest == nil, "nothing typed: the question goes")
        #expect(try await core.explainRoutineEvidence(999, threadID: thread, reason: "x") == false, "no such proposal")
    }
}
