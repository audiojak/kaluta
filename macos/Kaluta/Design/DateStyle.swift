import Foundation

// Dates (docs/design-system.md, Dates): every formatter in one place,
// made once (making them per row stalls scrolling). Design lint flags
// DateFormatter() and RelativeDateTimeFormatter() anywhere else.

/// Mail-style dates: time today, "Mon" this week, "Sep 12" this year, else
/// a short date. Formatters are created once; making them per row stalls
/// scrolling.
enum RowDateFormatter {
    private static let time: DateFormatter = make("jmm")
    private static let weekday: DateFormatter = make("EEE")
    private static let monthDay: DateFormatter = make("MMMd")
    private static let full: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .none
        return f
    }()

    static func string(forMillis millis: Int64, now: Date = .now, calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
        if calendar.isDate(date, inSameDayAs: now) { return time.string(from: date) }
        if let days = calendar.dateComponents([.day], from: date, to: now).day, days < 7, date < now {
            return weekday.string(from: date)
        }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) { return monthDay.string(from: date) }
        return full.string(from: date)
    }

    private static func make(_ template: String) -> DateFormatter {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate(template)
        return f
    }
}

/// The other date styles the app uses.
enum DateStyle {
    /// The reader's header: "Sep 21, 2026 at 1:00 AM".
    static let readerHeader: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .medium
        f.timeStyle = .short
        return f
    }()

    /// "2 hours ago", for activity lines (not rows: made per call).
    static func relative(_ date: Date, to now: Date = .now) -> String {
        RelativeDateTimeFormatter().localizedString(for: date, relativeTo: now)
    }
}
