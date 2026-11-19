import Foundation
import SwiftUI
import Testing
@testable import Kaluta

/// Remind me later in Triage (spec §14.13, amended 2026-11-09): B, then
/// 1–5 for when; and the task-match key that shows on its button.
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

    /// Gmail's Snooze choices: later today, tomorrow, next week, the weekend.
    @Test func theTimesAreGmailsSnoozeChoices() {
        // Wednesday 11 November 2026, 10 AM.
        let wednesday = AppModel.reminderTimes(now: date(2026, 11, 11, 10), calendar: calendar)
        #expect(wednesday.map(\.title) == ["Later today", "Tomorrow", "Next week", "This weekend"])
        #expect(wednesday.map(\.date) == [date(2026, 11, 11, 18), date(2026, 11, 12, 8), date(2026, 11, 16, 8),
                                          date(2026, 11, 14, 8)])
        #expect(wednesday[0].phrase.hasPrefix("today at") && wednesday[3].phrase.hasPrefix("Saturday at"))
        #expect(wednesday[0].detail.hasPrefix("Wed") && wednesday[0].detail.contains("6:00"))
        // On a Saturday (as Gmail shows it): next weekend, a week on.
        let saturday = AppModel.reminderTimes(now: date(2026, 11, 14, 10), calendar: calendar)
        #expect(saturday.map(\.title) == ["Later today", "Tomorrow", "Next week", "Next weekend"])
        #expect(saturday.map(\.date) == [date(2026, 11, 14, 18), date(2026, 11, 15, 8), date(2026, 11, 16, 8),
                                         date(2026, 11, 21, 8)])
        // On a Friday, the weekend is tomorrow.
        let friday = AppModel.reminderTimes(now: date(2026, 11, 13, 10), calendar: calendar)
        #expect(friday[3].date == date(2026, 11, 14, 8) && friday[3].phrase.hasPrefix("tomorrow at"))
        // On a Monday, next week is the Monday after.
        let monday = AppModel.reminderTimes(now: date(2026, 11, 16, 9), calendar: calendar)
        #expect(monday[2].date == date(2026, 11, 23, 8))
        // On a Sunday, not tomorrow (that is Tomorrow).
        let sunday = AppModel.reminderTimes(now: date(2026, 11, 15, 9), calendar: calendar)
        #expect(sunday[1].date == date(2026, 11, 16, 8) && sunday[2].date == date(2026, 11, 23, 8))
        #expect(sunday[3].date == date(2026, 11, 21, 8))
        // With 6 PM under an hour away or past, later today is three hours on.
        let evening = AppModel.reminderTimes(now: date(2026, 11, 11, 17, 30), calendar: calendar)
        #expect(evening[0].date == date(2026, 11, 11, 20, 30))
        let late = AppModel.reminderTimes(now: date(2026, 11, 11, 23), calendar: calendar)
        #expect(late[0].phrase.hasPrefix("tomorrow at"))
    }

    @Test func aChosenDayAndTimeIsKeptAndNeverInThePast() {
        let now = date(2026, 11, 11, 10)
        let chosen = AppModel.reminderTime(at: date(2026, 12, 3, 15), now: now, calendar: calendar)
        #expect(chosen.date == date(2026, 12, 3, 15))
        #expect(chosen.title.hasPrefix("on Thursday, December 3 at 3:00"))
        #expect(AppModel.reminderTime(at: date(2026, 11, 11, 9), now: now, calendar: calendar).date == date(2026, 11, 12, 8))
        #expect(AppModel.firstReminderMoment(now: now, calendar: calendar) == date(2026, 11, 12, 8))
    }

    @Test func aChosenDayIsEightThatMorningAndNeverBeforeTomorrow() {
        let now = date(2026, 11, 11, 10)
        let chosen = AppModel.reminderTime(on: date(2026, 12, 3, 15), now: now, calendar: calendar)
        #expect(chosen.date == date(2026, 12, 3, 8))
        #expect(chosen.title == "on Thursday, December 3")
        #expect(AppModel.reminderTime(on: now, now: now, calendar: calendar).date == date(2026, 11, 12, 8))
        // Another year says which.
        #expect(AppModel.reminderTime(on: date(2027, 2, 1, 9), now: now, calendar: calendar).title.hasSuffix("2027"))
        #expect(WorkflowCommand.command(key: KeyEquivalent("5"), characters: "5", modifiers: []) == .pick5)
    }

    @Test func aReminderSaysSoOnlyOnceItHasComeDue() {
        // Midday, so an hour before is still today whenever the run is.
        let now = Calendar.current.date(bySettingHour: 12, minute: 0, second: 0, of: .now)!
        let ms = { (d: Date) in Int64(d.timeIntervalSince1970 * 1000) }
        let due = ReminderInfo(remindAt: ms(now.addingTimeInterval(-60)), setAt: ms(now.addingTimeInterval(-3600)))
        #expect(WorkflowItemView.reminderLine(due, now: now) == "You asked to be reminded about this earlier today")
        let toCome = ReminderInfo(remindAt: ms(now.addingTimeInterval(3600)), setAt: ms(now))
        #expect(WorkflowItemView.reminderLine(toCome, now: now) == nil)
        // The choices are one to a row, each with its key and its time.
        let choices = WorkflowItemView.remindChoices(now: now)
        #expect(choices.map(\.key) == ["1", "2", "3", "4", "5", "L"])
        #expect(choices[0].title == "Later today" && !choices[0].detail.isEmpty)
        #expect(choices[4] == TimeChoice(key: "5", title: "Pick date & time…", detail: ""))
        #expect(choices[5] == TimeChoice(key: "L", title: "No reminder", detail: "stays in the Inbox"))
    }

    /// Waiting on others' *Check again* has the same five choices as
    /// *Remind me later*, in days.
    @Test func checkAgainOffersTheSameFiveChoicesInDays() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/Los_Angeles")!
        let now = calendar.date(from: DateComponents(year: 2026, month: 11, day: 11, hour: 15, minute: 30))!
        let times = AppModel.checkAgainTimes(now: now, calendar: calendar)
        #expect(times.map(\.title) == ["Tomorrow", "In 3 days", "Next week", "Next month"])
        let day = { (m: Int, d: Int) in calendar.date(from: DateComponents(year: 2026, month: m, day: d))! }
        #expect(times.map(\.date) == [day(11, 12), day(11, 14), day(11, 18), day(12, 11)], "the start of each day")
        #expect(times[0].phrase == "tomorrow")
        #expect(times[3].phrase.contains("11") && times[3].phrase.contains("Dec"))

        // Pick a date…: the start of the day chosen, never before tomorrow.
        let picked = AppModel.checkAgainTime(on: calendar.date(from: DateComponents(year: 2027, month: 1, day: 5, hour: 17))!,
                                             now: now, calendar: calendar)
        #expect(picked.date == calendar.date(from: DateComponents(year: 2027, month: 1, day: 5))!)
        #expect(picked.title.hasPrefix("on ") && picked.title.contains("2027"), "another year is named")
        #expect(AppModel.checkAgainTime(on: now, now: now, calendar: calendar).date == day(11, 12))
        let choices = WorkflowItemView.checkAgainChoices(now: now)
        #expect(choices.map(\.title) == ["Tomorrow", "In 3 days", "Next week", "Next month", "Pick a date…"])
        #expect(choices[0].detail.isEmpty, "tomorrow needs no date")
        #expect(!choices[1].detail.isEmpty, "the others name their day")
    }

    /// L, then the keys: 5 opens the calendar, Esc leaves it.
    @Test func checkAgainsFifthChoiceIsADayOnACalendar() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let item = WorkflowItem(key: "thread:t1", threadId: "t1", taskId: nil, decision: nil)
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .waiting, items: [item], startedAt: 0, resumed: false))
        model.workflowSession = session
        session.choosingTime = .checkAgain
        #expect(model.workflowActions().map(\.title)
            == ["Tomorrow", "In 3 days", "Next week", "Next month", "Pick a date…", "Cancel"])
        #expect(model.workflowActions().map(\.command) == [.pick1, .pick2, .pick3, .pick4, .pick5, .back])
        await model.performWorkflow(.pick5)
        #expect(session.choosingTime == .checkAgainOn)
        #expect(session.remindDay == AppModel.earliestReminderDay())
        #expect(model.workflowActions().map(\.command) == [.primary, .back])
        #expect(model.workflowActions().first?.title.hasPrefix("Check again on ") == true)
        await model.performWorkflow(.back)
        #expect(session.choosingTime == nil)
        #expect(session.items.first?.decision == nil, "nothing decided")
    }

    @Test func addingToAMatchedTaskShowsItsKey() {
        #expect(TaskMatchesView.keyLabel(1) == "⌘1")
    }

    /// B, then 2: the email leaves the set until tomorrow at 8 AM; ⌘Z
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
        // The bar: the three ways to answer first, in words, then the
        // decisions. Later (L) asks when; Leave in Inbox is its last choice.
        let bar = model.workflowActions().filter(\.shown)
        #expect(bar.first { $0.command == .replyAll }?.title == "Reply all")
        #expect(Array(bar.map(\.command).prefix(3)) == [.replyNow, .replyAll, .forward])
        #expect(Array(bar.map(\.command).dropFirst(3)) == [.task, .done, .later, .skip])
        #expect(bar.allSatisfy { !$0.onEmail })
        #expect(bar.first { $0.command == .later }?.title == "Later")

        await model.performWorkflow(.remind)
        #expect(session.choosingTime == .remind)
        #expect(model.workflowActions().map(\.title)
            == AppModel.reminderTimes().map(\.title) + ["Pick date & time…", "No reminder", "Cancel"])
        await model.performWorkflow(.pick2)
        #expect(session.choosingTime == nil)
        #expect(session.items.first { $0.key == item.key }?.decision == "remind")
        let reminder = try #require(try await core.reminder(thread))
        let morning = AppModel.reminderTimes()[1].date
        #expect(reminder.remindAt == Int64(morning.timeIntervalSince1970 * 1000))

        model.undo.undo(in: model.openAccountID)
        for _ in 0..<100 where (try? await core.reminder(thread)) != nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await core.reminder(thread) == nil)

        // 5: a day and time on a calendar, then Return.
        let next = try #require(session.current)
        let nextThread = try #require(model.threadID(of: next))
        await model.performWorkflow(.remind)
        await model.performWorkflow(.pick5)
        #expect(session.choosingTime == .remindOn)
        let day = Calendar.current.date(byAdding: .day, value: 10, to: .now)!
        session.remindDay = day
        #expect(model.workflowActions().map(\.command) == [.primary, .back])
        await model.performWorkflow(.primary)
        #expect(session.choosingTime == nil)
        let chosen = try #require(try await core.reminder(nextThread))
        #expect(chosen.remindAt == Int64(AppModel.reminderTime(at: day).date.timeIntervalSince1970 * 1000))
        model.undo.undo(in: model.openAccountID)
        for _ in 0..<100 where (try? await core.reminder(nextThread)) != nil { try await Task.sleep(for: .milliseconds(20)) }

        // Esc on the times: nothing decided.
        await model.performWorkflow(.remind)
        await model.performWorkflow(.back)
        #expect(session.choosingTime == nil && model.workflowSession != nil)

        // L, then L again: no reminder, left in the Inbox.
        let kept = try #require(session.current)
        let keptThread = try #require(model.threadID(of: kept))
        await model.performWorkflow(.later)
        #expect(session.choosingTime == .remind)
        await model.performWorkflow(.later)
        #expect(session.choosingTime == nil)
        #expect(session.items.first { $0.key == kept.key }?.decision == "leave")
        #expect(try await core.reminder(keptThread) == nil)
    }
}
