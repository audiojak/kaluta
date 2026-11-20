import Foundation
import Testing
@testable import Kaluta

/// Free times beside a conversation and in the composer (spec §14.9
/// amendment 2026-11-18): only free times, never the calendar's events.
@MainActor
struct FreeTimesTests {
    private let calendar: Calendar = {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/Los_Angeles")!
        return c
    }()

    private func slot(day: Int, hour: Int, label: String) -> CalendarSlot {
        let start = calendar.date(from: DateComponents(year: 2026, month: 11, day: day, hour: hour))!
        return CalendarSlot(startMs: CoreClient.millis(start), endMs: CoreClient.millis(start.addingTimeInterval(3600)),
                            label: label)
    }

    @Test func theNextThreeDaysAreShownElseTheFirstAfter() {
        let now = calendar.date(from: DateComponents(year: 2026, month: 11, day: 18, hour: 9))!
        let slots = [slot(day: 18, hour: 14, label: "Wed 2–3 pm"), slot(day: 20, hour: 10, label: "Fri 10–11 am"),
                     slot(day: 25, hour: 9, label: "Wed 9–10 am")]
        #expect(FreeTimes.shown(slots, now: now, calendar: calendar).map(\.label) == ["Wed 2–3 pm", "Fri 10–11 am"])
        #expect(FreeTimes.shown(Array(slots.suffix(1)), now: now, calendar: calendar).map(\.label) == ["Wed 9–10 am"],
                "none soon: the first after")
        #expect(FreeTimes.shown(slots, now: now, limit: 1, calendar: calendar).count == 1)
        #expect(FreeTimes.minutes(slots[0]) == 60)
    }

    @Test func insertingWritesALineATimeWithTheZone() {
        let slots = [slot(day: 18, hour: 14, label: "Wed 2–3 pm"), slot(day: 20, hour: 10, label: "Fri 10–11 am")]
        #expect(FreeTimes.insertion(slots, zone: "Pacific Time") == "- Wed 2–3 pm\n- Fri 10–11 am\n(Pacific Time)\n")
        #expect(FreeTimes.insertion([], zone: "Pacific Time") == "")
    }

    @Test func writingHelpOffersTheFreeTimesWhenTheEmailAsksToMeet() {
        let prompt = ComposerAssistant.prompt(instruction: "Say yes", from: "me@example.com", to: ["sam@example.com"],
                                              subject: "Re: coffee", draft: "", original: "Can we meet next week?",
                                              freeTimes: ("Pacific Time", ["Wed 2–3 pm"]))
        #expect(prompt.contains("- Wed 2–3 pm"))
        #expect(!prompt.contains(ComposerAssistant.meetingRule), "the times, not the link")
    }
}
