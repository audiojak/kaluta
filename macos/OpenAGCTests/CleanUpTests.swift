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

    @Test func theWindowListsTheReadyViews() {
        #expect(CleanUpViewKind.shown == [.sender, .people, .subject, .time, .social, .promotions, .size])
        #expect(CleanUpViewKind.social.filterPrompt == "Type a domain…")
        #expect(CleanUpViewKind.promotions.isCategory && !CleanUpViewKind.sender.isCategory)
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

    // MARK: Social and Promotions (spec §14.12)

    @Test func socialAndPromotionsGroupGmailsCategoriesByDomain() async throws {
        let model = try await demo()
        let store = model.cleanUp
        for view in [CleanUpViewKind.social, .promotions] {
            store.view = view
            await store.reload()
            #expect(!store.groups.isEmpty, "the demo's services are sorted into \(view.title)")
            #expect(!store.noCategoryMail)
            for group in store.groups {
                #expect(group.title == group.key, "titled by the domain")
                #expect(!group.title.contains("@"))
                #expect(group.aka.isEmpty)
            }
            let named = try #require(store.groups.first { $0.detail != nil })
            #expect(CleanUpGroupRowView.detailLine(named) == named.detail, "the senders' names, no aka")
        }
        // Ticking a domain lists its category mail and acts on it.
        store.view = .promotions
        await store.reload()
        let domain = try #require(store.groups.first)
        store.toggle(domain.key)
        await store.refreshMessages()
        #expect(store.messageCount == Int(domain.count))
        await eventually { store.message(at: 0) != nil }
        #expect(store.message(at: 0)?.from?.email.hasSuffix("@" + domain.key) == true)
        await store.apply(.archive)
        #expect(model.undo.notice?.text == "Archived \(Int(domain.count).formatted()) messages from \(domain.title)")
        store.filter = String(domain.key.prefix(5))
        await store.loadGroups()
        #expect(!store.groups.contains { $0.key == domain.key }, "out of the Inbox")
    }

    @Test func emptyCategoryViewsSayWhy() {
        let none = CleanUpGroupsColumnText.empty(.social, scope: .inbox, noCategoryMail: true)
        #expect(none == "This view groups the mail Gmail sorts into Social, by the sender's domain. This mailbox has none.")
        #expect(CleanUpGroupsColumnText.empty(.promotions, scope: .inbox, noCategoryMail: false)
            == "No promotions in the Inbox.")
        #expect(CleanUpGroupsColumnText.empty(.social, scope: .allMail, noCategoryMail: false)
            == "No social mail outside Spam and Trash.")
        #expect(CleanUpGroupsColumnText.empty(.sender, scope: .inbox, noCategoryMail: false) == "The Inbox is empty.")
    }

    // MARK: The progress card (spec §14.12)

    @Test func openingSetsTheBaselineAndArchivingCountsAsRemoved() async throws {
        let model = try await demo()
        let store = model.cleanUp
        let first = try #require(store.progress)
        #expect(first.now > 0)
        #expect(first.baseline == first.now, "the Inbox when Clean Up first opened")
        #expect(first.percent == 0)
        #expect(first.removedToday == 0)
        #expect(first.days.last?.count == first.atMidnight, "today, recorded on opening")

        let group = try #require(store.groups.first)
        store.toggle(group.key)
        await store.apply(.archive)
        let after = try #require(store.progress)
        #expect(after.now == first.now - group.count)
        #expect(after.removedToday == group.count, "follows the action")
        #expect(after.baseline == first.baseline)
        #expect(UInt64(after.percent) == group.count * 100 / first.baseline)

        model.undoMailAction()
        await eventually { store.progress?.now == first.now }
        #expect(store.progress?.removedToday == 0, "and its undo")
    }

    @Test func theCardsWordsAndPoints() {
        #expect(CleanUpProgressCard.percentText(62) == "62%")
        #expect(CleanUpProgressCard.signed(12, plus: true) == "+12")
        #expect(CleanUpProgressCard.signed(1_310, plus: false) == "\u{2212}1,310")
        #expect(CleanUpProgressCard.signed(0, plus: false) == "0")
        let progress = CleanupProgress(baseline: 900, percent: 33, atMidnight: 640, receivedToday: 12, removedToday: 52,
                                       now: 600, days: [CleanupDay(day: "2026-10-07", count: 700),
                                                        CleanupDay(day: "2026-10-08", count: 640)])
        #expect(CleanUpProgressCard.points(progress) == [700, 640, 600], "the days, then now")
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

    // MARK: Loading every header (spec §14.12)

    private func status(window: SyncWindow, cheap: Bool, gmail: Bool = true) -> CleanupLoadStatus {
        CleanupLoadStatus(hasSyncWindow: gmail, window: window, cheapHeaders: cheap, headersWaiting: 0,
                          bodiesWaiting: 0)
    }

    @Test func openingDecidesWhetherToWidenAskOrDoNothing() {
        #expect(CleanUpLoadPlan.of(status(window: .halfYear, cheap: true), declined: false) == .widen, "IMAP: at once")
        #expect(CleanUpLoadPlan.of(status(window: .year, cheap: true), declined: true) == .widen,
                "Not Now is about the API's cost only")
        #expect(CleanUpLoadPlan.of(status(window: .month, cheap: false), declined: false) == .ask, "the API: ask first")
        #expect(CleanUpLoadPlan.of(status(window: .month, cheap: false), declined: true) == .nothing,
                "Not Now holds for the session")
        #expect(CleanUpLoadPlan.of(status(window: .everything, cheap: true), declined: false) == .nothing)
        #expect(CleanUpLoadPlan.of(status(window: .everything, cheap: false), declined: false) == .nothing)
        #expect(CleanUpLoadPlan.of(status(window: .halfYear, cheap: false, gmail: false), declined: false) == .nothing,
                "imported, agent and demo mailboxes have no sync window")
    }

    /// A store with no core: the band's and the question's logic alone.
    private func bare() -> CleanUpStore {
        CleanUpStore(core: nil, undo: MailUndo(core: nil))
    }

    @Test func theDemoHasNothingToLoadAndItsSizeGroupsShowTheirRange() async throws {
        let model = try await demo()
        let store = model.cleanUp
        #expect(store.headerLoad == nil)
        #expect(store.loadQuestion == nil)
        let status = try await #require(model.core).cleanupLoadStatus(accountID: try #require(model.openAccountID))
        #expect(!status.hasSyncWindow)

        store.view = .size
        await store.reload()
        #expect(CleanUpViewKind.size.hasDetailLine)
        let detail = try #require(store.groups.first).detail
        #expect(["Less than 1 KB", "1 KB to 10 KB", "10 KB to 100 KB", "100 KB to 1 MB", "1 MB to 10 MB",
                 "More than 10 MB"].contains(detail ?? ""))
    }

    @Test func theBandCountsHeadersAsSyncReportsThem() {
        let store = bare()
        store.headerLoad = CleanUpHeaderLoad(total: 43_000, remaining: 43_000, widened: true)
        #expect(store.headerLoad?.text == "Loading headers for all mail — 0 of 43,000")
        store.syncChanged(pending: 900, headers: 41_800, accountID: nil)
        #expect(store.headerLoad?.text == "Loading headers for all mail — 1,200 of 43,000", "bodies are not counted")
        store.syncChanged(pending: 0, headers: 10, accountID: "someone-else")
        #expect(store.headerLoad?.remaining == 41_800, "another account's sync")
        store.syncChanged(pending: 0, headers: 0, accountID: nil)
        #expect(store.headerLoad?.done == true)
        #expect(store.headerLoad?.text == "Headers for all mail are on this Mac", "the note stays until put away")
        store.dismissHeaderLoad()
        #expect(store.headerLoad == nil)

        // Over the API everything comes down whole; a load found under way
        // (not started here) goes when it is done.
        store.headerLoad = CleanUpHeaderLoad(total: 10, remaining: 10, whole: true, widened: false)
        store.syncChanged(pending: 4, headers: 0, accountID: nil)
        #expect(store.headerLoad?.text == "Loading all mail — 6 of 10")
        store.syncChanged(pending: 0, headers: 0, accountID: nil)
        #expect(store.headerLoad == nil)
    }

    @Test func notNowIsRememberedForTheSession() {
        let store = bare()
        store.loadQuestion = CleanUpLoadQuestion(accountID: "acct", messages: 38_412, seconds: 9_219)
        #expect(store.loadQuestion?.message == "Clean Up groups the mail on this Mac. 38,412 older messages are still "
            + "only in Gmail; over the Gmail API they take about 2 hours, 34 minutes to download.")
        store.answerLoadQuestion(load: false)
        #expect(store.loadQuestion == nil)
        #expect(store.declinedLoads.contains("acct"))
        #expect(CleanUpLoadQuestion(accountID: "acct", messages: nil, seconds: nil).message.contains("all older mail"))
        #expect(CleanUpLoadQuestion.duration(30) == "a minute")
        #expect(CleanUpLoadQuestion.duration(3 * 86_400 + 7_200) == "3 days, 2 hours")
    }
}
