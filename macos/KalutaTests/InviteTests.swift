import Foundation
import Testing
@testable import Kaluta

/// Calendar invitations in mail (spec §14.9, amended 2026-11-04), in words.
@MainActor
struct InviteTests {
    private let la = TimeZone(identifier: "America/Los_Angeles")!

    private func invite(start: Int64, end: Int64?, allDay: Bool = false, state: InviteState = .answerable,
                        response: InviteResponse? = .needsAction) -> CalendarInvite {
        CalendarInvite(threadId: "t1", title: "Founders Meetup", startMs: start, endMs: end, allDay: allDay,
                       organizer: "Events", location: "Example Hall", response: response, state: state)
    }

    @Test func theBarSaysWhenWhereAndWho() {
        // Fri 6 Nov 2026, 18:00–20:00 UTC: 10 am to noon in California.
        let start: Int64 = 1_793_988_000_000
        let i = invite(start: start, end: start + 2 * 3_600_000)
        let details = InviteBar.details(i, zone: la)
        #expect(details.hasPrefix("Fri, Nov 6, 10:00"), "\(details)")
        #expect(details.contains("12:00"))
        #expect(details.hasSuffix(" · Example Hall · from Events"))
        let day = invite(start: 1_798_156_800_000, end: nil, allDay: true)
        #expect(InviteBar.when(day, zone: la).hasSuffix(", all day"))
        #expect(InviteBar.when(day, zone: la).contains("Dec 25"), "an all-day date is not shifted by the zone")
    }

    @Test func whyAnInvitationCannotBeAnsweredHere() {
        let i = invite(start: 0, end: nil)
        #expect(InviteBar.why(invite(start: 0, end: nil, state: .notConnected))
            == "Connect Google Calendar in Settings › Accounts to answer here")
        #expect(InviteBar.why(invite(start: 0, end: nil, state: .past, response: .accepted)) == "Over · you accepted")
        #expect(InviteBar.why(invite(start: 0, end: nil, state: .cancelled)) == "Cancelled by the organiser")
        #expect(InviteBar.why(i).isEmpty)
        #expect(InviteBar.notice(invite(start: 0, end: nil, response: .tentative))
            == "Said maybe to “Founders Meetup” on your calendar")
    }
}
