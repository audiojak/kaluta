import Foundation
import Testing
@testable import Kaluta

@MainActor
struct AppModelTests {
    private func demoModel() async throws -> AppModel {
        let dir = CoreClient.testScratch()
        let model = AppModel(core: try CoreClient(dataDirectory: dir))
        await model.start(openDemo: true)
        return model
    }

    @Test func demoModeOpensAnAccountAndFillsTheSidebarAndList() async throws {
        let model = try await demoModel()
        #expect(model.accountState == .open(accountID: AppModel.demoAccountID))
        let inbox = try #require(model.mailboxes.systemMailboxes.first { $0.kind == .inbox })
        #expect(inbox.unreadCount > 0)
        #expect(model.mailboxes.systemMailboxes.map(\.kind) == [.inbox, .starred, .important, .sent, .drafts, .archive, .spam, .trash])
        #expect(model.mailboxes.labels.map(\.name) == ["Customers", "Customers/Acme", "Customers/Globex", "Hiring", "Newsletters",
                                                  "Projects/Launch", "Projects/Launch/Press", "Receipts", "Travel"])
        // The demo uses Gmail's categories, so the Inbox opens on Primary.
        #expect(model.threads.mailboxID == "INBOX+CATEGORY_PERSONAL")
        let primary = try #require(model.inboxCategoryTabs.first)
        #expect(model.threads.rows.count == min(Int(ThreadListStore.pageSize), Int(primary.totalCount)))
    }

    @Test func switchingMailboxesReloadsTheListAndClearsSelection() async throws {
        let model = try await demoModel()
        model.selectedThreadID = model.threads.rows.first?.id
        model.selectedMailboxID = "@archive"
        #expect(model.selectedThreadID == nil)
        try await waitUntil { model.threads.mailboxID == "@archive" && !model.threads.rows.isEmpty }
        let archive = try #require(model.mailboxes.mailboxes.first { $0.kind == .archive })
        #expect(model.threads.rows.count == min(Int(ThreadListStore.pageSize), Int(archive.totalCount)))
    }

    @Test func scrollingNearTheEndLoadsTheNextPage() async throws {
        let model = try await demoModel()
        model.selectedMailboxID = "@archive"
        try await waitUntil { model.threads.mailboxID == "@archive" && !model.threads.rows.isEmpty }
        let first = model.threads.rows.count
        #expect(model.threads.hasMore)
        model.threads.rowWillAppear(at: first - 1)
        try await waitUntil { model.threads.rows.count > first }
        #expect(Set(model.threads.rows.map(\.id)).count == model.threads.rows.count, "no duplicates after paging")
    }

    private func waitUntil(timeout: Duration = .seconds(5), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            guard ContinuousClock.now < deadline else { throw WaitTimeout() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    private struct WaitTimeout: Error {}
}

/// A model that goes away takes its core and the core's open stores with it
/// (oagc-4rrl): the event loop held the core for good, so every test's
/// stores stayed open until the test host ran out of file descriptors.
@MainActor
struct ModelReleaseTests {
    /// The files this process has open under `directory`.
    static func openFiles(under directory: URL) -> [String] {
        let prefix = directory.resolvingSymlinksInPath().path
        var found: [String] = []
        var path = [CChar](repeating: 0, count: Int(MAXPATHLEN))
        for fd in 0..<getdtablesize() where fcntl(fd, F_GETPATH, &path) != -1 {
            let name = String(decoding: path.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
            if name.hasPrefix(prefix) || name.hasPrefix("/private" + prefix) { found.append(name) }
        }
        return found
    }

    @Test func aModelThatGoesAwayClosesItsCoreAndStores() async throws {
        let dir = CoreClient.testScratch()
        weak var weakCore: CoreClient?
        weak var weakModel: AppModel?
        do {
            let core = try CoreClient(dataDirectory: dir)
            let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
            await model.start(openDemo: true)
            #expect(!model.threads.rows.isEmpty)
            #expect(!Self.openFiles(under: dir).isEmpty, "the demo's store is open")
            weakCore = core
            weakModel = model
        }
        // Work the model started (loads, the event loop) winds down.
        for _ in 0..<250 where weakCore != nil || !Self.openFiles(under: dir).isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(weakModel == nil, "the model is gone")
        #expect(weakCore == nil, "and its core")
        #expect(Self.openFiles(under: dir).isEmpty, "and its stores are closed")
    }
}
