import Foundation
import Testing
@testable import OpenAGC

@MainActor
struct ThreadWindowTests {
    @Test func returnOpensTheSelectedThreadsInWindowsOfTheirOwn() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        var opened: [ThreadWindowRequest] = []
        model.openThreadWindow = { opened.append($0) }
        let rows = model.threads.rows.filter { !$0.labelIds.contains("DRAFT") }
        let first = try #require(rows.first)
        model.selectedThreadID = first.id
        model.openThreads()
        #expect(opened.map(\.threadID) == [first.id])
        #expect(opened.first?.accountID == model.openAccountID)

        opened = []
        model.selectedThreadIDs = Set(rows.map(\.id))
        model.openThreads()
        #expect(opened.count == min(rows.count, 10), "at most ten windows at once")
    }

    @Test func aThreadWindowShowsItsOwnThreadWhateverIsSelected() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        let rows = model.threads.rows
        #expect(rows.count >= 2)
        let store = ReaderStore(core: model.core)
        await store.show(threadID: rows[1].id)
        model.selectedThreadID = rows[0].id
        #expect(store.detail?.thread.id == rows[1].id)
    }
}

@MainActor
struct ReadOnShowTests {
    @Test func aThreadLookedAtForAMomentIsMarkedReadAndPassingOverItIsNot() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        model.readDelay = .milliseconds(100)
        let core = try #require(model.core)
        let unread = model.threads.rows.filter { $0.unreadCount > 0 }
        #expect(unread.count >= 2)
        let (passed, looked) = (unread[0].id, unread[1].id)

        // Passed over: another thread is chosen before the delay is up.
        model.selectedThreadID = passed
        model.threadShown(passed, hasUnread: true, inMainWindow: true)
        model.selectedThreadID = looked
        model.threadShown(looked, hasUnread: true, inMainWindow: true)
        try await Task.sleep(for: .milliseconds(400))

        let detail = { (id: String) in try await core.thread(id)?.messages.contains { !$0.isRead } }
        #expect(try await detail(looked) == false, "the thread looked at is read")
        #expect(try await detail(passed) == true, "the one passed over stays unread")
        #expect(model.threads.rows.first { $0.id == looked }?.unreadCount == 0)
    }
}
