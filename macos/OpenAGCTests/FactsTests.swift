import Foundation
import Testing
@testable import OpenAGC

/// Facts (spec §14.11): editing with Undo, Make Global, and what the
/// interview keeps.
@MainActor
struct FactsTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        model.undo.runsClock = false
        await model.facts.load()
        return model
    }

    private func role(_ value: String) -> FactEdit {
        .add(fields: FactFields(category: "work", label: "Occupation or role", value: value, use: .free, asOf: nil),
             status: .accepted, source: .you)
    }

    @Test func addingEditingAndUndoing() async throws {
        let model = try await demo()
        let account = model.openAccountID
        #expect(model.facts.categories.prefix(7).map(\.key)
                == ["identity", "contact", "availability", "people", "work", "preferences", "other"])
        #expect(await model.applyFactEdits([role("CEO")], actionName: "Add Fact", notice: "Added") == nil)
        #expect(model.facts.sections.map(\.category.key) == ["work"], "only categories with facts")
        let fact = try #require(model.facts.facts.first)
        #expect(model.undo.undoTitle(in: account) == "Undo Add Fact")
        let duplicate = await model.applyFactEdits([role("CTO")], actionName: "Add Fact", notice: "Added")
        #expect(duplicate != nil, "one fact per label in a category")

        model.undo.undo(in: account)
        for _ in 0..<100 where !model.facts.facts.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
            await model.facts.load()
        }
        #expect(model.facts.facts.isEmpty)
        model.undo.redo(in: account)
        for _ in 0..<100 where model.facts.facts.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
            await model.facts.load()
        }
        #expect(model.facts.facts.first?.id == fact.id, "redo brings back the same fact")
    }

    @Test func makeGlobalFromTheTabAndBack() async throws {
        let model = try await demo()
        let account = model.openAccountID
        await model.applyFactEdits([role("CEO")], actionName: "Add Fact", notice: "Added")
        let fact = try #require(model.facts.facts.first)
        await model.moveFact(fact)
        let global = try #require(model.facts.facts.first)
        #expect(global.scope == .global && global.value == "CEO")
        #expect(try await model.core!.globalFacts().map(\.value) == ["CEO"], "listed in Settings › Facts")
        #expect(model.undo.undoTitle(in: account) == "Undo Make Global")
        model.undo.undo(in: account)
        for _ in 0..<100 where model.facts.facts.first?.scope != .account {
            try await Task.sleep(for: .milliseconds(20))
            await model.facts.load()
        }
        #expect(model.facts.facts.map(\.scope) == [.account])
        #expect(try await model.core!.globalFacts().isEmpty)
    }

    @Test func theInterviewsFactsLandInFacts() async throws {
        let model = try await demo()
        let questions = GuideInterview.questions(categories: [], signature: nil, answered: [])
        let q = try #require(questions.first { $0.id == "F3" })
        let edits = GuideInterview.facts(for: q, fields: ["", "https://cal.com/j", "Pacific"])
        await model.applyFactEdits(edits, actionName: "Answer Question", notice: "Added")
        #expect(model.facts.facts.map { "\($0.category)/\($0.label)" }.sorted()
                == ["availability/Calendar link", "availability/Time zone"])
    }

    @Test func starterSetsAndCategories() async throws {
        let model = try await demo()
        let household = try #require(model.core?.factStarterSets().first { $0.name == "Household" })
        await model.addFactStarterSet(household)
        #expect(model.facts.categories.contains { $0.name == "Family logistics" && $0.defaultUse == .ask })
        let failure = await model.editFactCategories([.add(name: "home", description: "")], actionName: "Add Category",
                                                     notice: "Added")
        #expect(failure != nil, "a name like one there is already is refused")
    }
}

@MainActor
struct FactsMergeTests {
    @Test func mergingAnotherAccountsFactsIsUndoableAndDifferencesWaitInAnalysis() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        model.undo.runsClock = false
        let core = try #require(model.core)
        _ = try await core.applyFactEdits([.add(fields: FactFields(category: "work", label: "Occupation or role", value: "CTO",
                                                                   use: .free, asOf: nil), status: .accepted, source: .you)],
                                          reason: "test")
        let json = """
        {"openagc_facts": 1, "categories": [], "facts": [
          {"category": "work", "label": "Occupation or role", "value": "CEO", "use": "free", "as_of": null},
          {"category": "availability", "label": "Time zone", "value": "Pacific", "use": "free", "as_of": null}]}
        """
        await model.mergeFacts(json)
        #expect(model.analysisError == nil)
        #expect(model.facts.facts.map(\.label).sorted() == ["Occupation or role", "Time zone"])
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Merge Facts")
        #expect(try await core.analysisQueue().facts.map(\.value) == ["CEO"])
    }
}
