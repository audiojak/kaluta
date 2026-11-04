import Foundation
import Testing
@testable import Kaluta

/// The queue of changes for Gmail (spec §7.4, the queue) and the
/// Dashboard's Inbox card (§14.12, on the Dashboard), in words.
@MainActor
struct ChangesQueueTests {
    private func change(_ state: QueuedChangeState, attempts: UInt32 = 0, next: Int64? = nil,
                        error: String? = nil) -> QueuedChange {
        QueuedChange(id: Int64.random(in: 1...1_000_000), action: "Add “Waiting”", target: "“Q3 planning”",
                     state: state, attempts: attempts, createdAt: 0, nextAttemptAt: next, error: error)
    }

    @Test func theFooterSpeaksOnlyWhenAChangeNeedsTheUser() {
        #expect(QueueStatusFooter.line([], service: "Gmail") == nil)
        // A change waiting its turn a moment is not worth a line.
        #expect(QueueStatusFooter.line([change(.waiting), change(.sending)], service: "Gmail") == nil)
        let retrying = QueueStatusFooter.line([change(.retrying, attempts: 2), change(.waiting)], service: "Gmail")
        #expect(retrying?.text == "2 changes waiting for Gmail")
        #expect(retrying?.failed == false)
        let failed = QueueStatusFooter.line([change(.retrying, attempts: 1), change(.failed)], service: "Gmail")
        #expect(failed?.text == "1 change didn't reach Gmail")
        #expect(failed?.failed == true)
    }

    @Test func eachChangeSaysWhereItIs() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let soon = Int64((now.timeIntervalSince1970 + 240) * 1000)
        let retrying = QueuedChangeRow.status(change(.retrying, attempts: 3, next: soon, error: "network error: offline"),
                                              service: "Gmail", now: now)
        #expect(retrying.hasPrefix("Tried 3 times · trying again in 4 minutes"), "\(retrying)")
        #expect(retrying.hasSuffix(" · network error: offline"))
        #expect(QueuedChangeRow.status(change(.failed, error: "Invalid ids value"), service: "Gmail", now: now)
            == "Didn't reach Gmail and was undone here: Invalid ids value")
        #expect(QueuedChangeRow.status(change(.waiting), service: "Gmail", now: now) == "Waiting its turn")
        #expect(QueuedChangeRow.status(change(.sending), service: "Gmail", now: now) == "Sending now")
    }

    @Test func theDashboardIsNamedSo() {
        #expect(StartScreenView.headline(nil) == "Dashboard")
    }

    @Test func theInboxCardCountsMessages() {
        let tidy = InboxTidy(inbox: 6706, suggestions: [])
        #expect(InboxTidyCard.summary(tidy).hasPrefix("6,706 messages in the Inbox."))
        #expect(InboxTidyCard.messages(1) == "1 message")
    }
}
