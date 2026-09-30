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

@MainActor
struct AudienceTests {
    @Test func audiencesAreFilledConfirmedAndFoundForRecipients() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let filled = try await core.fillAudienceGroups()
        #expect(filled.count == 5 && filled.allSatisfy { $0.status == .suggested })
        let customers = try #require(filled.first { $0.name == "Customers" })
        _ = try await core.saveAudienceGroup(AudienceGroup(id: customers.id, name: customers.name, status: .confirmed,
                                                           description: customers.description, members: ["@acme.com"]))
        #expect(try await core.audienceFor(["ann@acme.com", "bob@x.com"]) == ["Customers"])
        let colleagues = try #require(filled.first { $0.name == "Colleagues" })
        let merged = try await core.mergeAudienceGroups(into: customers.id, from: colleagues.id)
        #expect(!merged.contains { $0.id == colleagues.id })
    }
}

struct GuideInterviewTests {
    private func category(_ id: String, learned: Bool, asked: Bool, accepted: UInt32 = 0) -> GuideCategoryInfo {
        GuideCategoryInfo(id: id, group: String(id.prefix(1)), groupName: "", name: "Name \(id)", looksFor: "how \(id)",
                          learned: learned, asked: asked, accepted: accepted, proposed: 0, evidence: 0)
    }

    @Test func askedCategoriesAndEmptyLearnedOnesGetQuestions() {
        let categories = [category("A1", learned: true, asked: false),
                          category("A2", learned: true, asked: false, accepted: 2),
                          category("F1", learned: false, asked: true)]
        let qs = GuideInterview.questions(categories: categories, signature: "John\nCEO", answered: ["F5"])
        #expect(qs.contains { $0.id == "F4" } && qs.contains { $0.id == "empty-A1" })
        #expect(!qs.contains { $0.id == "empty-A2" }, "a covered category is not asked about")
        #expect(!qs.contains { $0.id == "F5" }, "answered questions are not asked again")
        #expect(qs.first { $0.id == "F3" }?.suggestion == "John\nCEO")
    }

    @Test func answersBecomeEntries() throws {
        let qs = GuideInterview.questions(categories: [], signature: nil, answered: [])
        let bans = try #require(qs.first { $0.id == "C8" })
        let banned = GuideInterview.entries(for: bans, choice: nil, text: "circle back,\n per my last email ,", fields: [])
        #expect(banned.map(\.statement) == ["Never write “circle back”", "Never write “per my last email”"])
        #expect(banned.allSatisfy { $0.kind == .rule && $0.check?.kind == .bannedPhrase })
        #expect(banned[1].check?.value == "per my last email")
        let facts = try #require(qs.first { $0.id == "F3" })
        let made = GuideInterview.entries(for: facts, choice: nil, text: "", fields: ["CEO, Actual AI", "", "Pacific"])
        #expect(made.map(\.statement) == ["My role: CEO, Actual AI", "My time zone: Pacific"])
        #expect(made.allSatisfy { $0.kind == .fact && $0.category == "F3" })
        let invent = try #require(qs.first { $0.id == "F4" })
        #expect(GuideInterview.entries(for: invent, choice: 0, text: "", fields: []).first?.kind == .rule)
        #expect(GuideInterview.entries(for: invent, choice: nil, text: "", fields: []).isEmpty)
    }
}

@MainActor
struct GuideFollowingTests {
    @Test func writingHelpFollowsTheGuideForItsRecipients() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        _ = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "B6", kind: .rule, statement: "Sign off with 'John'",
                                          scope: .always, check: nil), status: .accepted, source: .you, origin: nil),
        ], reason: "test")
        let row = try #require(model.threads.rows.first)
        let detail = try #require(try await core.thread(row.id))
        let message = try #require(detail.messages.last)
        let store = ComposerStore(core: core, attachmentsDirectory: CoreClient.testScratch())
        await store.load(.reply(messageID: message.id, all: false))
        #expect(ComposerAssistant.messageType(store) == "reply")

        let assistant = ComposerAssistant()
        assistant.instruction = "Write a reply"
        await assistant.run(store: store, model: model, original: "")
        for _ in 0..<250 where assistant.state == .working { try await Task.sleep(for: .milliseconds(20)) }
        #expect(assistant.state == .done && assistant.followsGuide)
        // The fake agent echoes the prompt: the guide reached it.
        #expect(store.body.string.contains("Sign off with 'John'"))
        #expect(store.body.string.contains("The user's writing guide (version"))
        for _ in 0..<100 where store.draftID == 0 { try await Task.sleep(for: .milliseconds(20)) }
        try await Task.sleep(for: .milliseconds(200))
        #expect(try await core.draftGuideVersion(store.draftID) != nil, "the draft records the version")
    }
}

@MainActor
struct GuideCheckTests {
    @Test func aDraftThatBreaksACheckIsRewrittenOnceThenFlagged() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        // The fake agent echoes its prompt, and the writing-help prompt says
        // "Answer with only the text": a banned "answer" fails every draft.
        _ = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "C8", kind: .rule, statement: "Never write 'answer'", scope: .always,
                                          check: GuideCheck(kind: .bannedPhrase, value: "answer")),
                 status: .accepted, source: .you, origin: nil),
        ], reason: "test")
        let row = try #require(model.threads.rows.first)
        let detail = try #require(try await core.thread(row.id))
        let store = ComposerStore(core: core, attachmentsDirectory: CoreClient.testScratch())
        await store.load(.reply(messageID: try #require(detail.messages.last).id, all: false))
        let before = store.body.string
        let assistant = ComposerAssistant()
        assistant.instruction = "Write a reply"
        await assistant.run(store: store, model: model, original: "")
        for _ in 0..<250 where assistant.state == .working { try await Task.sleep(for: .milliseconds(20)) }
        #expect(assistant.state == .done)
        #expect(store.body.string.hasPrefix("You said: Your draft breaks the user's writing guide"), "rewritten once")
        #expect(assistant.checkFailures == ["Uses “answer”, which your rules ban"], "then flagged")
        assistant.undo()
        #expect(store.body.string == before, "the user's own text is never checked or lost")
        #expect(try await core.checkGuideDraft("Thanks, see you then.", recipients: [], messageType: "reply",
                                               audiences: nil).isEmpty)
    }
}

@MainActor
struct AudienceDraftTests {
    @Test func switchingAudienceWritesANewDraftAndSwitchingBackIsImmediate() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        _ = try await core.saveAudienceGroup(AudienceGroup(id: 0, name: "Investors", status: .confirmed,
                                                           description: "", members: ["@fund.com"]))
        _ = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "D1", kind: .guideline, statement: "Lead with the numbers",
                                          scope: GuideScope(groups: ["Investors"], people: [], messageTypes: [], languages: []),
                                          check: nil), status: .accepted, source: .you, origin: nil),
            .add(fields: GuideEntryFields(category: "A1", kind: .guideline, statement: "Be warm", scope: .always, check: nil),
                 status: .accepted, source: .you, origin: nil),
        ], reason: "test")
        let row = try #require(model.threads.rows.first)
        let detail = try #require(try await core.thread(row.id))
        let store = ComposerStore(core: core, attachmentsDirectory: CoreClient.testScratch())
        await store.load(.reply(messageID: try #require(detail.messages.last).id, all: false))
        store.body = NSAttributedString(string: "My own words", attributes: [.font: ComposerHTML.bodyFont])
        let assistant = ComposerAssistant()
        await assistant.loadAudiences(core)
        #expect(assistant.audienceChoices == ["Investors"])
        assistant.instruction = "Write a reply"
        await assistant.run(store: store, model: model, original: "")
        for _ in 0..<250 where assistant.state == .working { try await Task.sleep(for: .milliseconds(20)) }
        let first = store.body.string
        #expect(!first.contains("Lead with the numbers") && assistant.writtenFor.isEmpty)

        await assistant.switchAudience(to: ["Investors"], store: store, model: model)
        for _ in 0..<250 where assistant.state == .working { try await Task.sleep(for: .milliseconds(20)) }
        #expect(store.body.string.contains("Lead with the numbers"), "the investors' guideline reached the prompt")
        #expect(store.body.string.contains("My own words"), "from the user's own text, not the first draft")
        #expect(assistant.writtenFor == ["Investors"])

        await assistant.switchAudience(to: nil, store: store, model: model)
        #expect(assistant.state == .done && store.body.string == first, "switching back is immediate")
        assistant.undo()
        #expect(store.body.string == "My own words")
    }
}

@MainActor
struct ChangeGuideTests {
    @Test func aRequestBecomesQuestionsAndOnlyTheYesesApply() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        let core = try #require(model.core)
        _ = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "C1", kind: .rule, statement: "Use US spelling", scope: .always, check: nil),
                 status: .accepted, source: .you, origin: nil),
        ], reason: "test")
        let questions = try await core.proposeGuideChange("Use British spelling", agent: model.agent.providerID)
        #expect(questions.count == 2)
        #expect(ChangeGuideSheet.statement(questions[0].edits[0]) == "Use British spelling")
        #expect(try await core.guideEntries([.accepted]).count == 1, "nothing changed yet")
        // Yes to the first only.
        _ = await model.applyGuideEdits(questions[0].edits, reason: "change by prompt", actionName: "Change Guide",
                                        notice: "Changed")
        let now = try await core.guideEntries([.accepted]).map(\.statement)
        #expect(Set(now) == ["Use US spelling", "Use British spelling"])
        model.undo.undo(in: model.openAccountID)
        for _ in 0..<100 where try await core.guideEntries([.accepted]).count > 1 { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await core.guideEntries([.accepted]).map(\.statement) == ["Use US spelling"])
    }
}
