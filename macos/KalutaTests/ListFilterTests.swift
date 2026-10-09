import Foundation
import Testing
@testable import Kaluta

@MainActor
struct ListFilterTests {
    private func demo() async throws -> AppModel {
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        defaults.set(false, forKey: AppModel.showCategoriesKey("demo"))
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()), defaults: defaults)
        await model.start(openDemo: true)
        return model
    }

    private func waitFor(_ what: String, _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition(), ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        #expect(condition(), "timed out: \(what)")
    }

    @Test func filtersNarrowTheMailboxCombineAndFollowItAround() async throws {
        let model = try await demo()
        #expect(model.threads.rows.contains { $0.unreadCount == 0 }, "the Inbox has read mail")
        model.listFilters = [.unread]
        try await waitFor("unread listing") { model.threads.mailboxID == "INBOX+@unread" }
        #expect(!model.threads.rows.isEmpty)
        #expect(model.threads.rows.allSatisfy { $0.unreadCount > 0 })

        model.listFilters.insert(.attachments)
        try await waitFor("combined") { model.threads.mailboxID == "INBOX+@unread+@attachments" }
        #expect(model.threads.rows.allSatisfy { $0.unreadCount > 0 && $0.hasAttachments })

        model.selectedMailboxID = "@archive"
        #expect(model.listMailboxID == "@archive+@unread+@attachments", "kept when changing mailboxes")

        model.listFilters = []
        #expect(model.listMailboxID == "@archive")
    }

    @Test func aSearchGetsTheFiltersAsOperators() async throws {
        let model = try await demo()
        model.listFilters = [.starred]
        try await waitFor("starred listing") { model.threads.mailboxID == "INBOX+@starred" }
        model.searchText = "plan"
        #expect(model.filteredSearch == "(plan) is:starred", "grouped, so an OR is filtered whole")
        try await waitFor("search ran") { model.threads.searchQuery == "(plan) is:starred" }
        model.listFilters = [.unread]
        try await waitFor("search ran again") { model.threads.searchQuery == "(plan) is:unread" }
        // Clearing the search shows the listing as it is now.
        model.searchText = ""
        try await waitFor("back to the filtered Inbox") {
            model.threads.searchQuery == nil && model.threads.mailboxID == "INBOX+@unread"
        }
    }

    @Test func revealingAThreadClearsTheFilters() async throws {
        let model = try await demo()
        let read = try #require(model.threads.rows.first { $0.unreadCount == 0 })
        model.listFilters = [.unread]
        model.reveal(threadID: read.id)
        #expect(model.listFilters.isEmpty)
        #expect(model.selectedThreadID == read.id)
    }
}
