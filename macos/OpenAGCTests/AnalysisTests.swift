import Foundation
import Testing
@testable import OpenAGC

/// Proposed rules in the Writing Guide (spec §14.10): its dot, the learning
/// decisions beside the review's proposals, and deciding them with Undo.
@MainActor
struct AnalysisTests {
    /// The demo account after a learning run, with a day of reviews seeded.
    private func reviewed() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        let core = try #require(model.core)
        #expect(!model.reviewsAvailable, "no reviews until the guide has learned")
        _ = try await core.startGuideRun(GuideRunRequest(kind: .latest, count: 30,
                                                          filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                          focus: nil, agent: model.agent.providerID))
        for _ in 0..<200 where (try await core.guideProgress()).run?.status != .done {
            try await Task.sleep(for: .milliseconds(50))
        }
        // One learned guideline accepted, so a review can propose changing it.
        if let learned = try await core.guideDecisions().first(where: { $0.kind == .guideline }) {
            _ = try await core.applyGuideEdits([.decide(id: learned.id, status: .accepted)], reason: "test")
        }
        try await core.debugSeedAnalysis()
        model.analysisProgress = try await core.analysisProgress()
        await model.guide.load()
        await model.analysis.load()
        model.guidePrompt = nil
        return model
    }

    @Test func theWritingGuidesDotShowsUntilItIsOpened() async throws {
        let model = try await reviewed()
        #expect(model.reviewsAvailable)
        #expect(model.analysis.unseenRules)
        #expect(model.analysis.proposals.count >= 2 && !model.analysis.watching.isEmpty)
        #expect(model.analysis.learningDecisions > 0, "the learning run's decisions wait with them")
        #expect(model.analysis.rulesWaiting == model.guide.decisions.count + model.analysis.proposals.count)
        model.openProposedRules(learning: true)
        await model.proposalsShown(.rules)
        #expect(model.isGuide && model.analysis.reviewingRules && !model.analysis.unseenRules)
        let first = try #require(model.guide.decisions.first)
        #expect(model.analysis.selection == AnalysisStore.tag(first), "the first learning decision is chosen")
        #expect(model.selectedDecision?.id == first.id)
    }

    @Test func decidingAProposedRuleChoosesTheNext() async throws {
        let model = try await reviewed()
        model.openProposedRules()
        let tags = model.proposedRuleTags
        let first = try #require(model.guide.decisions.first)
        model.analysis.selection = AnalysisStore.tag(first)
        await model.decideProposedRule(reject: first)
        #expect(!model.guide.decisions.contains { $0.id == first.id })
        #expect(model.analysis.selection == tags[1], "the next proposed rule")
        // A category chosen: the review flow gives way to it; the header's
        // button brings it back.
        #expect(model.analysis.reviewingRules)
        model.showGuideCategory("A1")
        #expect(!model.analysis.reviewingRules && model.guide.selectedCategory == "A1")
        model.openProposedRules()
        #expect(model.analysis.reviewingRules && model.analysis.selection == tags[1])
    }

    @Test func acceptingChangesTheGuideAndUndoPutsItBack() async throws {
        let model = try await reviewed()
        let account = model.openAccountID
        let proposal = try #require(model.analysis.proposals.first { $0.op == .add })
        await model.decideAnalysis([proposal], accept: true)
        #expect(model.analysisError == nil, "\(model.analysisError ?? "")")
        #expect(model.guide.entries.contains { $0.statement == proposal.statement })
        #expect(!model.analysis.proposals.contains { $0.id == proposal.id })
        #expect(model.undo.undoTitle(in: account) == "Undo Accept Proposal")
        model.undo.undo(in: account)
        for _ in 0..<100 where !model.analysis.proposals.contains(where: { $0.id == proposal.id }) {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.analysis.proposals.contains { $0.id == proposal.id }, "back in the queue")
        await model.guide.load()
        #expect(!model.guide.entries.contains { $0.statement == proposal.statement })
    }

    @Test func acceptAllIsOneChangeAndRejectingIsUndoable() async throws {
        let model = try await reviewed()
        let account = model.openAccountID
        let all = model.analysis.proposals
        await model.decideAnalysis(all, accept: true)
        #expect(model.analysis.proposals.isEmpty)
        #expect(model.undo.undoTitle(in: account) == "Undo Accept Proposals")
        model.undo.undo(in: account)
        for _ in 0..<100 where model.analysis.proposals.count < all.count {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.analysis.proposals.count == all.count, "one Undo takes them all back")

        let first = try #require(model.analysis.proposals.first)
        await model.decideAnalysis([first], accept: false)
        #expect(model.undo.undoTitle(in: account) == "Undo Reject Proposal")
        #expect(!model.analysis.proposals.contains { $0.id == first.id })
    }

    @Test func theDetailMarksWhatTheUserChanged() async throws {
        let model = try await reviewed()
        let proposal = try #require(model.analysis.proposals.first { $0.op == .add && $0.support >= 2 })
        model.analysis.selection = AnalysisStore.tag(proposal)
        await model.analysis.loadPairs()
        let pair = try #require(model.analysis.pairs.first)
        let runs = WordDiff.runs(ai: try #require(pair.aiText), sent: try #require(pair.sentText))
        #expect(runs.ai.contains { $0.1 }, "the AI's replaced words are marked")
        #expect(runs.sent.contains { !$0.1 }, "what the user kept is not")
        await model.ignoreAnalysisPair(pair)
        let after = model.analysis.proposals.first { $0.id == proposal.id }
        #expect(after?.support == proposal.support - 1)
    }

    @Test func aReviewWithNothingToCompareSaysSo() {
        let run = AnalysisRunInfo(id: 1, day: "2026-10-05", daily: false, status: .done, matched: 0, unmatched: 0,
                                  unchanged: 0, total: 0, done: 0, batches: 0, batchesDone: 0, agent: nil, error: nil,
                                  startedAt: 0, finishedAt: Int64(Date().timeIntervalSince1970 * 1000), secondsLeft: nil)
        #expect(ReviewStatus.summary(run, daily: false) == "Last reviewed today: no edited AI drafts to compare.")
    }

    @Test func wordDiffKeepsTheTextAndMarksOnlyChanges() {
        let runs = WordDiff.runs(ai: "Hi Ann, Friday works well. Best regards, John", sent: "Hi Ann, Friday works. J")
        #expect(runs.ai.map(\.0).joined() == "Hi Ann, Friday works well. Best regards, John")
        #expect(runs.sent.map(\.0).joined() == "Hi Ann, Friday works. J")
        #expect(runs.sent.filter(\.1).map(\.0).joined().trimmingCharacters(in: .whitespaces) == "J")
        #expect(runs.ai.filter(\.1).map(\.0).joined().contains("regards"))
    }
}
