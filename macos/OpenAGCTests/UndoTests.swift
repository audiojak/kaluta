import Foundation
import Testing
@testable import OpenAGC

struct UndoWordingTests {
    @Test func noticesAndMenuNamesReadNaturally() {
        #expect(UndoableAction(kind: .archive, count: 3).noticeText == "Archived 3 conversations")
        #expect(UndoableAction(kind: .archive, count: 1).noticeText == "Archived 1 conversation")
        #expect(UndoableAction(kind: .archive, count: 1_200).noticeText == "Archived 1,200 conversations")
        #expect(UndoableAction(kind: .moveToInbox, count: 2).noticeText == "Moved 2 conversations to the Inbox")
        #expect(UndoableAction(kind: .trash, count: 1).noticeText == "Moved 1 conversation to the Trash")
        #expect(UndoableAction(kind: .read, count: 4).noticeText == "Marked 4 conversations as read")
        #expect(UndoableAction(kind: .unread, count: 1).noticeText == "Marked 1 conversation as unread")
        #expect(UndoableAction(kind: .star, count: 1).noticeText == "Starred 1 conversation")
        #expect(UndoableAction(kind: .unstar, count: 2).noticeText == "Unstarred 2 conversations")
        #expect(UndoableAction(kind: .label("Receipts"), count: 2).noticeText == "Labeled 2 conversations “Receipts”")
        #expect(UndoableAction(kind: .unlabel("Receipts"), count: 1).noticeText == "Removed “Receipts” from 1 conversation")
        #expect(UndoableAction(kind: .junk, count: 2).noticeText == "Moved 2 conversations to Spam")
        #expect(UndoableAction(kind: .notJunk, count: 1).noticeText == "Moved 1 conversation out of Spam to the Inbox")
        #expect(UndoableAction(kind: .junk, count: 1).actionName == "Mark as Junk")
        #expect(UndoableAction(kind: .archive, count: 1).actionName == "Archive")
        #expect(UndoableAction(kind: .read, count: 1).actionName == "Mark as Read")
        #expect(UndoableAction(kind: .unlabel("x"), count: 1).actionName == "Remove Label")
    }
}

@MainActor
struct UndoNoticeTimerTests {
    private func undo() -> MailUndo {
        let undo = MailUndo(core: nil)
        undo.runsClock = false
        return undo
    }

    @Test func theNoticeLastsEightSecondsAndPausesWhileHoveredFocusedOrInactive() {
        let undo = undo()
        undo.show("Archived 1 conversation", accountID: "a")
        undo.advance(by: .seconds(5))
        #expect(undo.notice != nil)
        undo.setPaused(.hover, true)
        undo.advance(by: .seconds(30))
        #expect(undo.notice != nil, "paused while the pointer is over it")
        undo.setPaused(.hover, false)
        undo.setPaused(.inactiveWindow, true)
        undo.advance(by: .seconds(30))
        #expect(undo.notice != nil, "paused while the window is inactive")
        undo.setPaused(.inactiveWindow, false)
        undo.setPaused(.focus, true)
        undo.advance(by: .seconds(30))
        #expect(undo.notice != nil, "paused while it has keyboard focus")
        undo.setPaused(.focus, false)
        undo.advance(by: .seconds(2))
        #expect(undo.notice != nil)
        undo.advance(by: .seconds(1))
        #expect(undo.notice == nil, "gone after eight seconds in all")
    }

    @Test func aNewActionReplacesTheNoticeAndRestartsItsTime() {
        let undo = undo()
        undo.show("Archived 1 conversation", accountID: "a")
        undo.advance(by: .seconds(7))
        undo.show("Starred 1 conversation", accountID: "a")
        #expect(undo.notice?.text == "Starred 1 conversation")
        undo.advance(by: .seconds(7))
        #expect(undo.notice != nil, "time restarted")
        undo.dismissNotice()
        #expect(undo.notice == nil)
    }
}

@MainActor
struct UndoModelTests {
    private func demo() async throws -> AppModel {
        let dir = CoreClient.testScratch()
        let model = AppModel(core: try CoreClient(dataDirectory: dir),
                             defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        model.undo.runsClock = false
        await model.start(openDemo: true)
        return model
    }

    private func waitUntil(_ what: String, _ condition: () async throws -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while try await !condition() {
            guard ContinuousClock.now < deadline else { Issue.record("timed out: \(what)"); return }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    private func inbox(_ model: AppModel) async throws -> [String] {
        try await model.core!.threads(in: "INBOX", limit: 500).rows.map(\.id)
    }

    @Test func archiveThenUndoAndRedo() async throws {
        let model = try await demo()
        let id = model.threads.rows[0].id
        model.selectedThreadID = id
        model.archiveSelection()
        try await waitUntil("recorded") { model.undo.canUndo(in: model.openAccountID) }
        #expect(model.undo.notice?.text == "Archived 1 conversation")
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Archive")

        model.undoMailAction()
        #expect(model.undo.notice == nil, "undoing clears the notice")
        try await waitUntil("back in the Inbox") { try await inbox(model).contains(id) }
        #expect(model.undo.redoTitle(in: model.openAccountID) == "Redo Archive")

        model.redoMailAction()
        try await waitUntil("archived again") { try await !inbox(model).contains(id) }
        #expect(model.undo.canUndo(in: model.openAccountID))
    }

    @Test func everyActionPathIsUndoable() async throws {
        let model = try await demo()
        let row = try #require(model.threads.rows.first { $0.unreadCount > 0 })
        model.selectedThreadID = row.id

        model.toggleReadSelection()
        try await waitUntil("read recorded") { model.undo.notice?.text == "Marked 1 conversation as read" }
        model.toggleStarSelection()
        try await waitUntil("star recorded") { model.undo.notice?.text.hasSuffix("1 conversation") == true
            && model.undo.notice?.text.contains("tarred") == true }
        let label = try #require(model.mailboxes.labels.first)
        model.setLabel(label.labelId!, applied: true)
        try await waitUntil("label recorded") { model.undo.notice?.text.hasPrefix("Labeled 1 conversation") == true }

        // Undo all three, newest first: label, star, read.
        model.undoMailAction()
        try await waitUntil("label removed") {
            try await !(model.core!.thread(row.id)?.thread.labelIds.contains(label.labelId!) ?? true)
        }
        model.undoMailAction()
        try await waitUntil("star restored") { try await model.core!.thread(row.id)?.thread.isStarred == row.isStarred }
        model.undoMailAction()
        try await waitUntil("unread again") { try await (model.core!.thread(row.id)?.thread.unreadCount ?? 0) > 0 }

        // Trash, then Move to Inbox from the Trash.
        model.selectedThreadID = row.id
        model.trashSelection()
        try await waitUntil("trash recorded") { model.undo.notice?.text == "Moved 1 conversation to the Trash" }
        model.selectedMailboxID = "TRASH"
        model.selectedThreadID = row.id
        model.moveSelectionToInbox()
        try await waitUntil("inbox recorded") { model.undo.notice?.text == "Moved 1 conversation to the Inbox" }
        model.undoMailAction()
        model.undoMailAction()
        try await waitUntil("out of the Trash, back in the Inbox") {
            let thread = try await model.core!.thread(row.id)?.thread
            return thread.map { !$0.labelIds.contains("TRASH") && $0.labelIds.contains("INBOX") } ?? false
        }

        // Dropping threads on a sidebar label.
        model.addLabel(label.labelId!, toThreads: [row.id])
        try await waitUntil("drop recorded") { model.undo.notice?.text.hasPrefix("Labeled 1 conversation") == true }
    }

    @Test func junkMovesToSpamAndNotJunkBringsItBack() async throws {
        let model = try await demo()
        let id = model.threads.rows[0].id
        model.selectedThreadID = id
        #expect(!model.isSpamMailbox)
        model.toggleJunkSelection()
        #expect(!model.threads.rows.contains { $0.id == id }, "gone from the list at once")
        try await waitUntil("junk recorded") { model.undo.notice?.text == "Moved 1 conversation to Spam" }
        #expect(try await model.core!.threads(in: "SPAM", limit: 50).rows.map(\.id) == [id])
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Mark as Junk")

        model.selectedMailboxID = "SPAM"
        try await waitUntil("Spam listed") { model.threads.mailboxID == "SPAM" && model.threads.rows.contains { $0.id == id } }
        #expect(model.isSpamMailbox)
        model.selectedThreadID = id
        model.toggleJunkSelection()
        try await waitUntil("not junk recorded") { model.undo.notice?.text == "Moved 1 conversation out of Spam to the Inbox" }
        try await waitUntil("back in the Inbox") { try await inbox(model).contains(id) }

        model.undoMailAction()
        try await waitUntil("in Spam again") {
            try await model.core!.threads(in: "SPAM", limit: 50).rows.contains { $0.id == id }
        }
    }

    @Test func nothingChangedMeansNothingToUndo() async throws {
        let model = try await demo()
        let id = model.threads.rows[0].id
        model.selectedThreadID = id
        model.archiveSelection()
        try await waitUntil("recorded") { model.undo.notice != nil }
        #expect(try await model.core!.archive([id]) == nil, "already archived: no token")
        model.undo.dismissNotice()
        model.selectedThreadID = id
        model.archiveSelection()
        try await Task.sleep(for: .milliseconds(300))
        #expect(model.undo.notice == nil, "no acknowledgement for a change that did not happen")
    }

    @Test func eachAccountHasItsOwnStack() async throws {
        let dir = CoreClient.testScratch()
        let core = try CoreClient(dataDirectory: dir)
        try await core.addDemoAccount("work", email: "work@example.com", threads: 10)
        try await core.addDemoAccount("home", email: "home@example.com", threads: 10)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        model.undo.runsClock = false
        await model.start(openDemo: false)
        await model.switchAccount(to: "work")
        model.selectedThreadID = model.threads.rows[0].id
        model.archiveSelection()
        try await waitUntil("work recorded") { model.undo.canUndo(in: "work") }
        await model.switchAccount(to: "home")
        #expect(!model.undo.canUndo(in: model.openAccountID), "home has nothing to undo")
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo")
        await model.switchAccount(to: "work")
        #expect(model.undo.undoTitle(in: model.openAccountID) == "Undo Archive")
    }
}

@MainActor
struct UndoSendTests {
    @Test func aHeldSendIsUndoneByTakingItBackWithoutRedo() async throws {
        let undo = MailUndo(core: nil)
        undo.runsClock = false
        var tookBack = false
        undo.recordSend(accountID: "a", holdFor: .seconds(10)) { tookBack = true }
        #expect(undo.notice?.text == "Sending…")
        #expect(undo.remaining == .seconds(10), "the notice lasts as long as the hold")
        undo.setPaused(.hover, true)
        undo.advance(by: .seconds(4))
        #expect(undo.remaining == .seconds(6), "the hold does not pause, so neither does its notice")
        undo.setPaused(.hover, false)
        #expect(undo.undoTitle(in: "a") == "Undo Send")
        undo.undo(in: "a")
        try await Task.sleep(for: .milliseconds(50))
        #expect(tookBack)
        #expect(!undo.canRedo(in: "a"), "no redo: the user sends again")
    }

    @Test func theDelayDefaultsToTenSecondsAndIsRemembered() throws {
        let suite = "openagc-tests-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        let model = AppModel(core: nil, defaults: defaults)
        #expect(model.undoSendSeconds == 10)
        model.undoSendSeconds = 0
        #expect(AppModel(core: nil, defaults: defaults).undoSendSeconds == 0)
        #expect(AppModel.undoSendChoices == [0, 5, 10, 20, 30])
    }
}
