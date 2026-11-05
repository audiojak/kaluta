import Foundation
import Testing
@testable import Kaluta

/// Follow-ups from Granola meetings (spec §14.14), in words and keys.
@MainActor
struct FollowUpTests {
    private func person(_ name: String) -> MeetingPerson {
        MeetingPerson(name: name, email: "\(name.lowercased())@example.org")
    }

    @Test func aMeetingSaysWhoItWasWith() {
        #expect(FollowUpItemView.with([]) == "no one else")
        #expect(FollowUpItemView.with([person("Reese")]) == "with Reese")
        #expect(FollowUpItemView.with([person("Reese"), person("Sam")]) == "with Reese and Sam")
        #expect(FollowUpItemView.with([person("Reese"), person("Sam"), person("Avery"), person("Kai")])
            == "with Reese, Sam and 2 more")
        #expect(FollowUpItemView.with([MeetingPerson(name: "", email: "x@example.org")]) == "with x@example.org")
    }

    @Test func theRailSaysHowEachWasDecided() {
        let meeting = MeetingFollowUp(meetingId: "m1", title: "Pilot", meetingAt: 0, people: [person("Reese")],
                                      reason: "", options: [], url: nil)
        let item = { (decision: String?) in
            WorkflowItem(key: "meeting:m1", threadId: nil, taskId: nil, decision: decision)
        }
        #expect(item(nil).meetingID == "m1")
        #expect(WorkflowRail.followUpDetail(meeting, item(nil)) == "with Reese")
        #expect(WorkflowRail.followUpDetail(meeting, item("sent")) == "Follow-up opened")
        #expect(WorkflowRail.followUpDetail(meeting, item("none")) == "No follow-up")
        #expect(WorkflowRail.followUpDetail(meeting, item("later")) == "Tomorrow")
    }

    @Test func followUpsAreTheEighthWorkflow() {
        #expect(WorkflowKind.followups.number == 8)
        #expect(WorkflowKind.followups.title == "Follow-ups")
        #expect(WorkflowCommand.done.title(in: .followups) == "No follow-up")
        #expect(FollowUpItemView.pick(1) == .pick2)
    }
}
