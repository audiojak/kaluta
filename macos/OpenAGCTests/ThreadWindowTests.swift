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
