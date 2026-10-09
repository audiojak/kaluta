import Foundation
import Testing
@testable import Kaluta

struct InboxCategoryRuleTests {
    private func counts(_ pairs: [(String, UInt32)]) -> [InboxCategory] {
        pairs.map { InboxCategory(id: $0.0, totalCount: $0.1, unreadCount: 0) }
    }

    @Test func tabsShowOnlyWhenAnotherCategoryHasMail() {
        let none = counts([("CATEGORY_PERSONAL", 40), ("CATEGORY_PROMOTIONS", 0), ("CATEGORY_SOCIAL", 0)])
        #expect(InboxCategories.visible(none).isEmpty, "Primary alone is no choice")
        let some = counts([("CATEGORY_PERSONAL", 0), ("CATEGORY_PROMOTIONS", 3), ("CATEGORY_SOCIAL", 0),
                           ("CATEGORY_UPDATES", 5)])
        #expect(InboxCategories.visible(some).map(\.id) == ["CATEGORY_PERSONAL", "CATEGORY_PROMOTIONS", "CATEGORY_UPDATES"],
                "Primary stays even when empty; empty others go")
    }

    @Test func theChosenTabHoldsWhileItHasMail() {
        let tabs = counts([("CATEGORY_PERSONAL", 1), ("CATEGORY_SOCIAL", 2)])
        #expect(InboxCategories.active(chosen: "CATEGORY_SOCIAL", visible: tabs) == "CATEGORY_SOCIAL")
        #expect(InboxCategories.active(chosen: "CATEGORY_FORUMS", visible: tabs) == "CATEGORY_PERSONAL")
        #expect(InboxCategories.active(chosen: "CATEGORY_SOCIAL", visible: []) == nil)
    }

    @Test func aThreadBelongsToItsFirstCategoryElsePrimary() {
        let order = ["CATEGORY_PERSONAL", "CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL"]
        #expect(InboxCategories.category(of: ["INBOX", "CATEGORY_SOCIAL"], in: order) == "CATEGORY_SOCIAL")
        #expect(InboxCategories.category(of: ["INBOX"], in: order) == "CATEGORY_PERSONAL")
    }
}

@MainActor
struct CategoryTabModelTests {
    private func demo(_ defaults: UserDefaults) async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()), defaults: defaults)
        await model.start(openDemo: true)
        return model
    }

    private func waitFor(_ model: AppModel, _ mailbox: String) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while model.threads.mailboxID != mailbox, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.threads.mailboxID == mailbox)
    }

    @Test func theDemoInboxOpensOnPrimaryAndTabsNarrowIt() async throws {
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        let model = try await demo(defaults)
        #expect(model.inboxCategoryTabs.first?.id == InboxCategories.primary)
        #expect(model.inboxCategoryTabs.count > 2, "the demo seeds several categories")
        #expect(model.listMailboxID == "INBOX+CATEGORY_PERSONAL")
        try await waitFor(model, "INBOX+CATEGORY_PERSONAL")
        let primary = Set(model.threads.rows.map(\.id))

        model.inboxCategory = "CATEGORY_PROMOTIONS"
        try await waitFor(model, "INBOX+CATEGORY_PROMOTIONS")
        #expect(model.threads.rows.allSatisfy { $0.labelIds.contains("CATEGORY_PROMOTIONS") })
        #expect(primary.isDisjoint(with: model.threads.rows.map(\.id)))
        #expect(defaults.string(forKey: AppModel.inboxCategoryKey("demo")) == "CATEGORY_PROMOTIONS", "remembered")

        model.inboxImportantOnly = true
        #expect(model.listMailboxID?.hasPrefix("INBOX+IMPORTANT") == true, "tabs combine with Important only")

        model.inboxImportantOnly = false
        model.showCategories = false
        try await waitFor(model, "INBOX")
        #expect(model.inboxCategoryTabs.isEmpty)
    }

    @Test func searchIgnoresTabsAndOtherMailboxesAreNotNarrowed() async throws {
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        let model = try await demo(defaults)
        model.searchText = "invoice"
        let fromOtherTabs = { model.threads.rows.contains { !$0.labelIds.contains(InboxCategories.primary) } }
        let deadline = ContinuousClock.now + .seconds(5)
        while !fromOtherTabs(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.threads.searchQuery == "invoice")
        #expect(fromOtherTabs(), "results come from every category, not the Primary tab")

        model.selectedMailboxID = "STARRED"
        #expect(model.listMailboxID == "STARRED", "only the Inbox has tabs")
    }

    @Test func revealingAThreadSwitchesToItsTab() async throws {
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        let model = try await demo(defaults)
        let social = try #require(try await model.core!.threads(in: "INBOX+CATEGORY_SOCIAL", limit: 1).rows.first)
        model.reveal(threadID: social.id)
        try await waitFor(model, "INBOX+CATEGORY_SOCIAL")
        #expect(model.selectedThreadID == social.id)
    }
}
