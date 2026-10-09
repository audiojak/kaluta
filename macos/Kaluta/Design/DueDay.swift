import Foundation

/// How a task's due day reads (spec §14.8): the words in its row and the
/// group it falls in. Days are `YYYY-MM-DD` in the user's calendar, so a
/// task due "today" stays due today wherever the Mac travels.
enum DueDay {
    enum Group: Int, CaseIterable, Comparable {
        case overdue, today, thisWeek, later, noDate

        var title: String {
            switch self {
            case .overdue: "Overdue"
            case .today: "Today"
            case .thisWeek: "This Week"
            case .later: "Later"
            case .noDate: "No Date"
            }
        }

        static func < (a: Group, b: Group) -> Bool { a.rawValue < b.rawValue }
    }

    /// Whether the day is past (drawn in the caution tone, never red:
    /// red is for failure).
    enum Urgency: Equatable { case overdue, today, upcoming, none }

    static func date(_ day: String?, calendar: Calendar = .current) -> Date? {
        guard let day else { return nil }
        let parts = day.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return nil }
        return calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
    }

    static func string(_ date: Date, calendar: Calendar = .current) -> String {
        let c = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    /// Whole days from `now`'s day to `day` (negative when past).
    static func days(until day: String?, now: Date = .now, calendar: Calendar = .current) -> Int? {
        guard let due = date(day, calendar: calendar) else { return nil }
        return calendar.dateComponents([.day], from: calendar.startOfDay(for: now), to: due).day
    }

    static func group(_ day: String?, now: Date = .now, calendar: Calendar = .current) -> Group {
        guard let days = days(until: day, now: now, calendar: calendar) else { return .noDate }
        if days < 0 { return .overdue }
        if days == 0 { return .today }
        if days < 7 { return .thisWeek }
        return .later
    }

    static func urgency(_ day: String?, now: Date = .now, calendar: Calendar = .current) -> Urgency {
        switch group(day, now: now, calendar: calendar) {
        case .overdue: .overdue
        case .today: .today
        case .noDate: .none
        case .thisWeek, .later: .upcoming
        }
    }

    /// "Today", "Tomorrow", "Yesterday", a weekday within the week ahead,
    /// else "Sep 12" (with the year when it is not this year's).
    static func label(_ day: String?, now: Date = .now, calendar: Calendar = .current) -> String {
        guard let due = date(day, calendar: calendar), let days = days(until: day, now: now, calendar: calendar)
        else { return "No date" }
        switch days {
        case 0: return "Today"
        case 1: return "Tomorrow"
        case -1: return "Yesterday"
        case 2..<7: return due.formatted(Date.FormatStyle(calendar: calendar).weekday(.wide))
        default:
            let sameYear = calendar.component(.year, from: due) == calendar.component(.year, from: now)
            var style = Date.FormatStyle(calendar: calendar).month(.abbreviated).day()
            if !sameYear { style = style.year() }
            return due.formatted(style)
        }
    }
}
