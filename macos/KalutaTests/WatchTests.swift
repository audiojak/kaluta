import Foundation
import Testing
import UserNotifications
@testable import Kaluta

/// Reply-landed alerts (spec §14.7 amendment 2026-11-18).
@MainActor
struct WatchTests {
    @Test func theAlertNamesWhoAnsweredAndOffersReplyAndRemind() {
        let mail = CoreClientEvent.NewMail(messageID: "m9", threadID: "t1", senderName: "Sam", subject: "Offsite",
                                           snippet: "Friday works.")
        let request = NewMailNotifier.replyRequest(mail, accountID: "work")
        #expect(request.content.title == "Sam answered")
        #expect(request.content.subtitle == "Offsite" && request.content.body == "Friday works.")
        #expect(request.content.categoryIdentifier == NewMailNotifier.replyCategory)
        #expect(request.content.userInfo["threadID"] as? String == "t1")
        let actions = NewMailNotifier.replyLandedCategory.actions.map(\.title)
        #expect(actions == ["Reply", "Remind Me Tomorrow"])
    }

    @Test func watchingIsToggledAndUndone() async throws {
        var posted: [UNNotificationRequest] = []
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        model.notifier.post = { posted.append($0) }
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let thread = try #require(model.threads.rows.first?.id)
        model.selectedThreadID = thread
        await model.toggleWatchSelection()
        #expect(model.watched == [thread])
        #expect(model.undo.notice?.text == "Kaluta will tell you when someone answers this conversation")
        model.undo.undo(in: model.openAccountID)
        await model.undo.settled()
        #expect(model.watched.isEmpty)
        // Mail that arrives where nothing is watched tells nothing extra.
        await model.announceReplies([CoreClientEvent.NewMail(messageID: "x", threadID: thread, senderName: "Sam",
                                                             subject: "", snippet: "")], accountID: nil)
        #expect(posted.isEmpty)
        #expect(try await core.watchedThreads().isEmpty)
    }
}
