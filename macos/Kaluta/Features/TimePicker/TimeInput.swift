import Foundation

/// A time as the user typed it ("8am", "in 3 days", "Aug 7", "friday
/// 3pm"), read by the system's date detector: no grammar of Kaluta's own.
enum TimeInput {
    /// What the field suggests typing; each of these the detector reads.
    static let prompt = "Try: 8am, in 3 days, Aug 7"

    /// The moment the text names, if it names one still to come. A day
    /// with no time means 8 AM that day; a time already past today means
    /// that time tomorrow.
    static func date(from text: String, now: Date = .now, calendar: Calendar = .current) -> Date? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty,
              let detector = try? NSDataDetector(types: NSTextCheckingResult.CheckingType.date.rawValue),
              let found = detector.firstMatch(in: text, range: NSRange(text.startIndex..., in: text))?.date
        else { return nil }
        // The detector answers noon for a day with no time.
        let clock = calendar.dateComponents([.hour, .minute, .second], from: found)
        let lower = text.lowercased()
        let saysTime = lower.contains(":") || lower.contains("am") || lower.contains("pm")
            || lower.contains("12") || lower.contains("noon")
        let dayOnly = clock.hour == 12 && clock.minute == 0 && clock.second == 0 && !saysTime
        let date = dayOnly ? calendar.date(bySettingHour: 8, minute: 0, second: 0, of: found) ?? found : found
        if date > now { return date }
        guard !dayOnly, calendar.isDate(date, inSameDayAs: now) else { return nil }
        return calendar.date(byAdding: .day, value: 1, to: date)
    }
}

/// What a time picker lists for what is typed: the typed time first when
/// it reads as one, then the ready choices whose names match.
@MainActor
struct TimePickerModel: Equatable {
    struct Row: Equatable, Identifiable {
        let title: String
        let detail: String
        let time: AppModel.ReminderTime
        var id: String { title + detail }
    }

    let rows: [Row]

    init(text: String, presets: [AppModel.ReminderTime], now: Date = .now, calendar: Calendar = .current) {
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        var rows: [Row] = []
        if let date = TimeInput.date(from: query, now: now, calendar: calendar) {
            let time = AppModel.reminderTime(at: date, now: now, calendar: calendar)
            rows.append(Row(title: time.phrase.prefix(1).uppercased() + time.phrase.dropFirst(), detail: "", time: time))
        }
        rows += presets.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) }
            .map { Row(title: $0.title, detail: $0.detail, time: $0) }
        self.rows = rows
    }

    /// The row ↑ or ↓ moves to from `index`, staying inside the list.
    func moved(from index: Int, by delta: Int) -> Int {
        rows.isEmpty ? 0 : min(max(index + delta, 0), rows.count - 1)
    }
}
