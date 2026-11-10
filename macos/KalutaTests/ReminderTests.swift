import Foundation
import SwiftUI
import Testing
@testable import Kaluta

/// Remind me later in Triage (spec §14.13, amended 2026-11-09): B, then
/// 1–3 for when; and the task-match key that shows on its button.
@MainActor
struct ReminderTests {
    private var calendar: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "America/New_York")!
        return c
    }

    private func date(_ y: Int, _ m: Int, _ d: Int, _ h: Int, _ min: Int = 0) -> Date {
        calendar.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min))!
    }

    @Test func bIsRemindMeLater() {
        #expect(WorkflowCommand.command(key: KeyEquivalent("b"), characters: "b", modifiers: []) == .remind)
        #expect(WorkflowCommand.remind.keyLabel == "B")
        #expect(WorkflowCommand.remind.title(in: .triage) == "Remind me later")
    }

    @Test func theTimesAreInThreeHoursTomorrowMorningAndNextWeek() {
        // Wednesday 11 November 2026, 10 AM.
        let wednesday = AppModel.reminderTimes(now: date(2026, 11, 11, 10), calendar: calendar)
        #expect(wednesday.map(\.title) == ["In 3 hours", "Tomorrow morning", "Next week"])
        #expect(wednesday.map(\.date) == [date(2026, 11, 11, 13), date(2026, 11, 12, 8), date(2026, 11, 16, 8)])
        // On a Monday, next week is the Monday after.
        let monday = AppModel.reminderTimes(now: date(2026, 11, 16, 9), calendar: calendar)
        #expect(monday[2].date == date(2026, 11, 23, 8))
        // On a Sunday, not tomorrow (that is Tomorrow morning).
        let sunday = AppModel.reminderTimes(now: date(2026, 11, 15, 9), calendar: calendar)
        #expect(sunday[1].date == date(2026, 11, 16, 8) && sunday[2].date == date(2026, 11, 23, 8))
        // Late in the evening, three hours is tomorrow.
        let late = AppModel.reminderTimes(now: date(2026, 11, 11, 23), calendar: calendar)
        #expect(late[0].phrase.hasPrefix("tomorrow at"))
    }

    @Test func aReminderSaysSoOnlyOnceItHasComeDue() {
        let now = Date.now
        let ms = { (d: Date) in Int64(d.timeIntervalSince1970 * 1000) }
        let due = ReminderInfo(remindAt: ms(now.addingTimeInterval(-60)), setAt: ms(now.addingTimeInterval(-3600)))
        #expect(WorkflowItemView.reminderLine(due, now: now) == "You asked to be reminded about this earlier today")
        let toCome = ReminderInfo(remindAt: ms(now.addingTimeInterval(3600)), setAt: ms(now))
        #expect(WorkflowItemView.reminderLine(toCome, now: now) == nil)
        #expect(WorkflowItemView.remindQuestion().hasPrefix("Remind you when? 1 in 3 hours, 2 tomorrow morning"))
    }

    @Test func addingToAMatchedTaskShowsItsKey() {
        #expect(TaskMatchesView.keyLabel(1) == "⌘1")
    }

    /// B, then 2: the email leaves the set until tomorrow morning; ⌘Z
    /// takes the reminder off again.
    @Test func remindMeLaterPutsTheEmailOffAndUndoTakesItBack() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        model.undo.runsClock = false
        let core = try #require(model.core)
        // Today's mail, so Triage has a set.
        _ = try await core.seedWorkflowDemo(threads: 30)
        await model.beginWorkflow(.triage)
        let session = try #require(model.workflowSession)
        let item = try #require(session.current)
        let thread = try #require(model.threadID(of: item))
        #expect(model.workflowActions().contains { $0.command == .remind && $0.title == "Remind me later" })

        await model.performWorkflow(.remind)
        #expect(session.choosingTime == .remind)
        #expect(model.workflowActions().map(\.title) == ["In 3 hours", "Tomorrow morning", "Next week", "Cancel"])
        await model.performWorkflow(.pick2)
        #expect(session.choosingTime == nil)
        #expect(session.items.first { $0.key == item.key }?.decision == "remind")
        let reminder = try #require(try await core.reminder(thread))
        let morning = AppModel.reminderTimes()[1].date
        #expect(reminder.remindAt == Int64(morning.timeIntervalSince1970 * 1000))

        model.undo.undo(in: model.openAccountID)
        for _ in 0..<100 where (try? await core.reminder(thread)) != nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await core.reminder(thread) == nil)

        // Esc on the times: nothing decided.
        await model.performWorkflow(.remind)
        await model.performWorkflow(.back)
        #expect(session.choosingTime == nil && model.workflowSession != nil)
    }
}
