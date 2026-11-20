import Foundation
import Testing
@testable import Kaluta

/// Typed times and the picker that lists them (plan: inbox ideas, F3).
/// The system's date detector reads against the real clock, so these
/// times are relative to now.
struct TimePickerTests {
    private let calendar = Calendar.current

    @Test func typedTimesAreReadByTheSystemsDetector() throws {
        let now = Date.now
        // A day with no time: 8 AM that day.
        let days = try #require(TimeInput.date(from: "in 3 days", now: now))
        #expect(calendar.component(.hour, from: days) == 8 && calendar.component(.minute, from: days) == 0)
        let wanted = try #require(calendar.date(byAdding: .day, value: 3, to: calendar.startOfDay(for: now)))
        #expect(calendar.isDate(days, inSameDayAs: wanted))
        // A day and a time: as typed.
        let tomorrow = try #require(TimeInput.date(from: "tomorrow at 3pm", now: now))
        #expect(calendar.component(.hour, from: tomorrow) == 15)
        #expect(calendar.isDate(tomorrow, inSameDayAs: try #require(calendar.date(byAdding: .day, value: 1, to: now))))
        // A time alone is still to come: today, or tomorrow once it has passed.
        let eight = try #require(TimeInput.date(from: "8am", now: now))
        #expect(eight > now && eight.timeIntervalSince(now) <= 24 * 3600)
        #expect(calendar.component(.hour, from: eight) == 8)
        // Not a time.
        #expect(TimeInput.date(from: "banana", now: now) == nil)
        #expect(TimeInput.date(from: "   ", now: now) == nil)
    }

    @MainActor @Test func thePickerListsTheTypedTimeThenTheChoicesThatMatch() {
        let presets = AppModel.reminderTimes()
        // Nothing typed: every ready choice, in order.
        let all = TimePickerModel(text: "", presets: presets)
        #expect(all.rows.map(\.title) == presets.map(\.title))
        #expect(all.rows.map(\.time) == presets)
        // A choice's name filters the list.
        let week = TimePickerModel(text: "next w", presets: presets)
        #expect(week.rows.contains { $0.title == "Next week" })
        #expect(week.rows.allSatisfy { $0.title.localizedCaseInsensitiveContains("next w") || $0.detail.isEmpty })
        // A typed time comes first, as its own row.
        let typed = TimePickerModel(text: "in 3 days", presets: presets)
        #expect(typed.rows.first?.time.date == TimeInput.date(from: "in 3 days"))
        // Not a time, not a choice: nothing to take.
        #expect(TimePickerModel(text: "banana", presets: presets).rows.isEmpty)
        // ↑ and ↓ stay inside the list.
        #expect(all.moved(from: 0, by: -1) == 0)
        #expect(all.moved(from: 0, by: 1) == 1)
        #expect(all.moved(from: presets.count - 1, by: 1) == presets.count - 1)
    }

    @MainActor @Test func keyHintsNameWhatReturnDoes() {
        #expect(KeyHints.choose.map(\.keys) == ["↑↓", "Return", "Esc"])
        #expect(LabelPickerView.hints(.create("Work/New")).map(\.does) == ["Create", "Close"])
        #expect(LabelPickerView.hints(.nothing).map(\.does) == ["Close"])
        #expect(AppModel.remindNotice(count: 1, phrase: "tomorrow") == "Back in Triage tomorrow")
        #expect(AppModel.remindNotice(count: 3, phrase: "tomorrow") == "3 conversations back in Triage tomorrow")
    }

    /// `b` in the mail window: the selected conversation gets a reminder,
    /// and undo takes it back.
    @MainActor @Test func remindingFromTheMailWindowSetsAReminderAndUndoes() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let thread = try #require(model.threads.rows.first?.id)
        model.selectedThreadID = thread
        let time = AppModel.reminderTimes()[1]
        await model.remindSelection(time)
        let reminder = try #require(try await core.reminder(thread))
        #expect(AppModel.date(of: reminder) == time.date)
        model.undo.undo(in: model.openAccountID)
        await model.undo.settled()
        #expect(try await core.reminder(thread) == nil)
    }
}
