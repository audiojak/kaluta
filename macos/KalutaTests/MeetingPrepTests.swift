import Foundation
import Testing
import UserNotifications
@testable import Kaluta

/// Meeting prep (spec §14.14 amendment 2026-11-18): the brief on the
/// Dashboard, and its reminder 15 minutes before.
@MainActor
struct MeetingPrepTests {
    private func brief(in seconds: TimeInterval, now: Date) -> MeetingBrief {
        MeetingBrief(meetingId: "evt_1", title: "Pricing call", meetingAt: CoreClient.millis(now.addingTimeInterval(seconds)),
                     people: ["Reese Park"], brief: "- Reese: wants pricing.")
    }

    @Test func theReminderComesFifteenMinutesBefore() throws {
        let now = Date.now
        let request = try #require(NewMailNotifier.briefRequest(brief(in: 3600, now: now), accountID: "work", now: now))
        let trigger = try #require(request.trigger as? UNTimeIntervalNotificationTrigger)
        #expect(abs(trigger.timeInterval - 45 * 60) < 2)
        #expect(request.content.title == "Pricing call in 15 minutes")
        #expect(request.content.body == "- Reese: wants pricing.")
        #expect(request.identifier == "brief-evt_1", "the same id replaces it when it moves")
        #expect(NewMailNotifier.briefRequest(brief(in: 600, now: now), accountID: nil, now: now) == nil, "too late")
    }

    @Test func theDashboardLineSaysWhenTheNextMeetingIs() {
        let now = Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: .now)!
        let line = MeetingBriefLine.line(brief(in: 3600, now: now), now: now)
        #expect(line.hasPrefix("Next: Pricing call at ") && line.hasSuffix(" · brief ready"))
        let tomorrow = MeetingBriefLine.line(brief(in: 26 * 3600, now: now), now: now)
        #expect(tomorrow.contains("Pricing call tomorrow at "), "\(tomorrow)")
    }
}
