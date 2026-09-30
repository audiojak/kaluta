import Foundation
import Testing
@testable import OpenAGC

@MainActor
struct GuideStoreTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        return model
    }

    @Test func entriesAreAddedDecidedAndUndoneThroughTheCore() async throws {
        let model = try await demo()
        let core = try #require(model.core)
        #expect(try await core.guideCategories().count == 57)
        let revision = model.guideRevision
        let added = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "B6", kind: .guideline, statement: "Sign off with 'John'",
                                          scope: GuideScope(groups: [], people: [], messageTypes: [], languages: []),
                                          check: nil),
                 status: .proposed, source: .learned, origin: nil),
        ], reason: "test")
        let id = try #require(added.entries.first?.id)
        let decided = try await core.applyGuideEdits([.decide(id: id, status: .accepted)], reason: "decide")
        #expect(decided.version >= 1)
        #expect(try await core.guideEntries([.accepted]).map(\.id) == [id])
        try await core.undoGuideChange(decided.changeId)
        #expect(try await core.guideEntry(id)?.status == .proposed)
        for _ in 0..<100 where model.guideRevision == revision { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.guideRevision > revision, "changes reach the window as events")

        let json = try await core.exportGuide(json: true)
        #expect(try core.readGuideExport(json).entries.isEmpty, "only accepted entries are exported")
    }
}

@MainActor
struct GuideRunTests {
    @Test func aLearningRunReportsProgressAndItsProposalsWaitUntilItIsDone() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        let info = try await core.guideSampleInfo(count: 30, filter: GuideSampleFilter(excludePeople: [], excludeLabels: []))
        #expect(info.sent > 0 && info.chosen > 0 && info.batches == (info.chosen + 19) / 20)

        let run = try await core.startGuideRun(GuideRunRequest(kind: .latest, count: 30,
                                                               filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                               focus: nil, agent: model.agent.providerID))
        #expect(run.status == .running)
        let deadline = ContinuousClock.now + .seconds(20)
        while model.guideProgress?.run?.status != .done, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(50))
        }
        let progress = try #require(model.guideProgress, "progress arrives as events")
        #expect(progress.run?.status == .done && progress.run?.done == progress.run?.total)
        let decisions = try await core.guideDecisions()
        #expect(!decisions.isEmpty && Int(progress.decisionsTotal) == decisions.count)
        #expect(decisions.allSatisfy { !$0.evidence.isEmpty }, "every proposal quotes the user's mail")
        #expect(model.agent.entries.isEmpty, "learning stays out of the agent column")
    }
}

@MainActor
struct GuideDecisionTests {
    private func learned() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        let core = try #require(model.core)
        _ = try await core.startGuideRun(GuideRunRequest(kind: .latest, count: 30,
                                                          filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                          focus: nil, agent: model.agent.providerID))
        for _ in 0..<200 where (try await core.guideProgress()).run?.status != .done {
            try await Task.sleep(for: .milliseconds(50))
        }
        model.selectedMailboxID = AppModel.guideMailboxID
        await model.guide.load()
        model.guideProgress = try await core.guideProgress()
        return model
    }

    @Test func decidingIsSavedAsItGoesAndUndoable() async throws {
        let model = try await learned()
        #expect(model.isGuide && model.listMailboxID == nil)
        let first = try #require(model.guide.decisions.first)
        let waiting = model.guideDecisionsWaiting
        #expect(waiting == model.guide.decisions.count && waiting > 0)

        await model.decideGuide(first, accept: true)
        #expect(!model.guide.decisions.contains { $0.id == first.id })
        #expect(model.guide.entries.contains { $0.id == first.id }, "accepted entries are the guide")
        #expect(model.guideDecisionsWaiting == waiting - 1, "the decisions bar moves")
        #expect(model.guide.categories.first { $0.id == first.category }?.accepted == 1)
        let account = try #require(model.openAccountID)
        #expect(model.undo.undoTitle(in: account) == "Undo Accept Entry")

        model.undo.undo(in: account)
        for _ in 0..<100 where model.guide.decisions.first?.id != first.id {
            try await Task.sleep(for: .milliseconds(20))
            await model.guide.load()
        }
        #expect(model.guide.decisions.contains { $0.id == first.id }, "undo puts the decision back")

        let second = try #require(model.guide.decisions.last)
        await model.decideGuide(second, accept: false)
        #expect(!model.guide.entries.contains { $0.id == second.id })
        #expect(try await model.core!.guideEntry(second.id)?.status == .rejected)
    }

    @Test func aProposalCanReplaceTheEntryItContradicts() async throws {
        let model = try await learned()
        let core = try #require(model.core)
        let scope = GuideScope.always
        let mine = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "B6", kind: .guideline, statement: "Sign off with 'Best, John'",
                                          scope: scope, check: nil), status: .accepted, source: .you, origin: nil),
            .add(fields: GuideEntryFields(category: "B6", kind: .guideline, statement: "Sign off with 'Cheers'",
                                          scope: scope, check: nil), status: .proposed, source: .learned, origin: nil),
        ], reason: "test").entries
        await model.replaceGuideEntry(mine[0], with: mine[1])
        #expect(try await core.guideEntry(mine[1].id)?.status == .accepted)
        #expect(try await core.guideEntry(mine[0].id)?.status == .rejected)
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Replace Entry")
    }
}
