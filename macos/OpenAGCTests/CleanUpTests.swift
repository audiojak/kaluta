import Foundation
import Testing
@testable import OpenAGC

/// The Clean Up window's model (spec §14.12) against the demo mailbox.
@MainActor
struct CleanUpTests {
    private func demo() async throws -> AppModel {
        let dir = CoreClient.testScratch()
        let model = AppModel(core: try CoreClient(dataDirectory: dir),
                             defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        model.undo.runsClock = false
        await model.start(openDemo: true)
        await model.cleanUp.open(accountID: model.openAccountID)
        return model
    }

    /// Wait for something an unawaited task finishes (an undo's replay).
    private func eventually(_ condition: () -> Bool) async {
        for _ in 0..<200 where !condition() {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test func theWindowListsTheFiveReadyViews() {
        #expect(CleanUpViewKind.shown == [.sender, .people, .subject, .time, .size])
        #expect(CleanUpViewKind.sender.filterPrompt == "Type a sender…")
        #expect(CleanUpViewKind.time.filterPrompt == nil)
        #expect(CleanUpViewKind.size.filterPrompt == nil)
        #expect(CleanUpViewKind.size.showsSize)
        #expect(!CleanUpViewKind.sender.showsSize)
    }

    @Test func theDemoHasBulkSendersWithOtherNames() async throws {
        let model = try await demo()
        let store = model.cleanUp
        #expect(store.groupsLoaded)
        #expect(store.groups.filter { $0.count >= 24 }.count >= 4, "several senders with dozens of messages")
        #expect(store.groups.map(\.count) == store.groups.map(\.count).sorted(by: >), "largest first")
        let aka = try #require(store.groups.first { !$0.aka.isEmpty })
        #expect(CleanUpGroupRowView.detailLine(aka).contains("aka "))
        #expect(model.cleanUpTitle == "Clean Up — Demo Mailbox")
    }

    @Test func spaceTicksTheHighlightedGroupsAndTheCountFollows() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let groups = Array(store.groups.prefix(2))
        try #require(groups.count == 2)
        #expect(store.summary == nil)
        #expect(!store.canAct, "nothing ticked: nothing to act on")

        store.toggle(groups[0].key)
        await store.refreshMessages()
        #expect(store.messageCount == Int(groups[0].count))
        #expect(store.summary?.groups == "1 group")
        #expect(store.canAct)

        // Space with one of two ticked ticks both; again unticks both.
        store.toggleTicks(groups.map(\.key))
        await store.refreshMessages()
        #expect(store.ticked == Set(groups.map(\.key)))
        #expect(store.messageCount == Int(groups[0].count + groups[1].count))
        #expect(store.summary?.groups == "2 groups")
        store.toggleTicks(groups.map(\.key))
        await store.refreshMessages()
        #expect(!store.hasTicks)
        #expect(store.messageCount == 0)
    }

    @Test func ticksStayThroughTheFilterAndGoWithTheView() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.groups.first)
        store.toggle(first.key)
        store.filter = "zzz-no-such-sender"
        await store.loadGroups()
        #expect(store.groups.isEmpty)
        #expect(store.isTicked(first.key), "kept while filtered away")
        store.filter = String(first.title.prefix(4))
        await store.loadGroups()
        #expect(store.groups.allSatisfy { group in
            ([group.title, group.detail ?? ""] + group.aka).contains { $0.localizedCaseInsensitiveContains(store.filter) }
        })
        store.view = .size
        #expect(!store.hasTicks, "keys belong to a view")
        #expect(store.filter.isEmpty)
        await store.reload()
        #expect(!store.groups.isEmpty)
    }

    @Test func messagesArriveAPageAtATime() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.groups.first)
        store.toggle(first.key)
        await store.refreshMessages()
        #expect(store.message(at: 0) == nil, "asked for")
        await eventually { store.message(at: 0) != nil }
        let message = try #require(store.message(at: 0))
        #expect(message.from?.email.lowercased() == first.detail?.lowercased())
        let last = store.messageCount - 1
        _ = store.message(at: last)
        await eventually { store.message(at: last) != nil }
        #expect(store.message(at: last) != nil)
    }

    @Test func allMailHoldsAtLeastTheInboxAndKeepsTicks() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.groups.first)
        store.toggle(first.key)
        await store.refreshMessages()
        let inboxCount = store.messageCount
        store.scope = .allMail
        await store.reload()
        #expect(store.isTicked(first.key))
        #expect(store.messageCount >= inboxCount)
        let wider = try #require(store.groups.first { $0.key == first.key })
        #expect(wider.count >= first.count)
    }

    @Test func archivingIsOneUndoInTheWindowsOwnNotice() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.groups.first)
        store.toggle(first.key)
        await store.refreshMessages()

        await store.apply(.archive)
        #expect(store.error == nil)
        #expect(!store.hasTicks, "an action clears the ticks")
        #expect(!store.groups.contains { $0.key == first.key }, "gone from the Inbox")
        let notice = try #require(model.undo.notice)
        #expect(notice.origin == .cleanUp, "shown in Clean Up, not over the mail list")
        #expect(notice.text == "Archived \(Int(first.count).formatted()) messages from \(first.title)")
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Archive")

        model.undoMailAction()
        await eventually { store.groups.contains { $0.key == first.key } }
        #expect(store.groups.first { $0.key == first.key }?.count == first.count, "every message back")

        model.redoMailAction()
        await eventually { !store.groups.contains { $0.key == first.key } }
        #expect(!store.groups.contains { $0.key == first.key })
    }

    @Test func trashAndSpamTakeMessagesOutOfAllMail() async throws {
        let model = try await demo()
        let store = model.cleanUp
        store.scope = .allMail
        await store.reload()
        let groups = Array(store.groups.prefix(2))
        try #require(groups.count == 2)
        store.toggle(groups[0].key)
        await store.apply(.trash)
        #expect(!store.groups.contains { $0.key == groups[0].key })
        #expect(model.undo.notice?.text.hasPrefix("Moved") == true)
        store.toggle(groups[1].key)
        await store.apply(.spam)
        #expect(!store.groups.contains { $0.key == groups[1].key })
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Mark as Spam")
    }

    @Test func movingToALabelLeavesTheInbox() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.groups.first)
        let label = try #require(model.mailboxes.labels.first { $0.labelId != nil && $0.name == "Receipts" }?.labelId)
        store.toggle(first.key)
        await store.apply(.move(labelId: label))
        #expect(store.error == nil)
        #expect(model.undo.notice?.text.contains("“Receipts”") == true)
        #expect(!store.groups.contains { $0.key == first.key })
    }

    @Test func toolbarWordsWhileWorking() {
        #expect(CleanUpStore.workingText(.archive, count: 813) == "Archiving 813 messages…")
        #expect(CleanUpStore.workingText(.trash, count: 1) == "Moving 1 message to the Trash…")
        #expect(CleanUpStore.workingText(.spam, count: 2_500) == "Moving 2,500 messages to Spam…")
    }

    @Test func pendingChangesBelongToTheCleanedAccount() async throws {
        let model = try await demo()
        let store = model.cleanUp
        store.outboxChanged(pending: 3, accountID: "someone-else")
        #expect(store.pending == 0)
        store.outboxChanged(pending: 3, accountID: model.openAccountID)
        #expect(store.pending == 3)
    }
}
