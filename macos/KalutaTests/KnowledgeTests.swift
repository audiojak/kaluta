import Foundation
import Testing
@testable import Kaluta

/// What Kaluta Knows (spec §14.11 amendment 2026-11-18).
@MainActor
struct KnowledgeTests {
    @Test func factsAndGuideEntriesAreOneListWithTabsAndScopes() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        await model.facts.load()
        await model.guide.load()
        let items = KnowledgeItem.items(facts: model.facts.facts, guide: model.guide.entries)
        #expect(items.allSatisfy { $0.tabs.contains(.all) })
        #expect(items.allSatisfy { !$0.scope.isEmpty })
        let accepted = model.facts.facts.filter { $0.status == .accepted }.count
            + model.guide.entries.filter { $0.status == .accepted }.count
        #expect(items.count == accepted, "only what was accepted")
        // Guide entries are preferences; a fact is where its category says.
        #expect(items.filter { if case .guide = $0.source { true } else { false } }.allSatisfy { $0.tabs.contains(.preferences) })
        // The sidebar entry opens the page, not a mailbox.
        model.selectedMailboxID = AppModel.knowsMailboxID
        #expect(model.isKnows && model.listMailboxID == nil)
    }
}
