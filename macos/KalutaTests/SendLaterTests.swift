import Foundation
import Testing
@testable import Kaluta

/// Send later (spec §14.5 amendment 2026-11-18, ADR 0022), against the
/// demo mailbox: a scheduled draft stays a draft and undo takes the time
/// back.
@MainActor
struct SendLaterTests {
    private func model() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        model.overduePrompt = { _ in .askLater }
        await model.start(openDemo: true)
        return model
    }

    @Test func aComposerSchedulesItsDraftAndUndoTakesTheTimeBack() async throws {
        let model = try await model()
        let core = try #require(model.core)
        let account = try #require(model.openAccountID)
        let store = ComposerStore(core: core, account: account)
        await store.load(.prefilled(to: ["sam@example.com"], cc: [], subject: "Numbers", body: "Attached."))
        let when = AppModel.reminderTimes()[1].date

        await store.sendLater(at: when)
        #expect(store.phase == .sent, "the window closes")
        #expect(!store.didSend, "nothing was sent")
        let scheduled = try #require(store.scheduledNow)
        #expect(scheduled.at == when && scheduled.before == nil)
        let draft = try #require(try await core.draft(store.draftID))
        #expect(draft.status == .editing, "still a draft")
        #expect(try await core.scheduledSend(store.draftID, in: account) == when)

        // What the composer's window does next: the notice, with Undo.
        model.sendScheduled(draftID: store.draftID, at: when, before: nil, accountID: account)
        await model.refreshScheduled()
        #expect(model.scheduledSends.map(\.draftId) == [store.draftID])
        #expect(model.undo.notice?.text.hasPrefix("Will send ") == true)
        #expect(AppModel.scheduledLine(model.scheduledSends[0]).hasPrefix("“Numbers” sends "))
        // Its time has not come: nothing goes.
        await model.sendDueScheduled()
        #expect(model.scheduledSends.count == 1)

        model.undo.undo(in: account)
        await model.undo.settled()
        #expect(try await core.scheduledSend(store.draftID, in: account) == nil)
        #expect(try await core.draft(store.draftID) != nil, "the draft is kept")

        // Reopened and unscheduled from its banner.
        try await core.scheduleSend(store.draftID, at: when, in: account)
        let again = ComposerStore(core: core, account: account)
        await again.load(.draft(id: store.draftID, task: nil))
        #expect(again.scheduledAt == when)
        await again.unschedule()
        #expect(again.scheduledAt == nil)
        #expect(try await core.scheduledSend(store.draftID, in: account) == nil)
    }

    @Test func noticesNameTheMessage() {
        let send = ScheduledSendInfo(accountId: "a", draftId: 1, sendAt: 0, subject: "Numbers")
        #expect(AppModel.scheduledNotice(send, went: true) == "Sent “Numbers” as scheduled")
        #expect(AppModel.scheduledNotice(send, went: false) == "Could not send “Numbers” as scheduled. It is in Drafts.")
        let blank = ScheduledSendInfo(accountId: "a", draftId: 1, sendAt: 0, subject: " ")
        #expect(AppModel.scheduledNotice(blank, went: true) == "Sent a message as scheduled")
    }
}
