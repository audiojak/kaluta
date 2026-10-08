import Charts
import SwiftUI

/// The Inbox Zero card at the foot of Clean Up's views (spec §14.12): how
/// much of the Inbox is gone since Clean Up was first opened, the Inbox's
/// daily count over the last 30 days, and today's numbers.
struct CleanUpProgressCard: View {
    let progress: CleanupProgress

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            HStack(alignment: .firstTextBaseline) {
                Text("Inbox Zero").font(TypeRole.groupLabel)
                Spacer(minLength: Space.xs)
                Text(Self.percentText(progress.percent)).font(TypeRole.figure)
            }
            ProgressView(value: Double(progress.percent), total: 100)
                .progressViewStyle(.linear)
                .controlSize(.small)
                .accessibilityLabel("Inbox Zero")
                .accessibilityValue(Self.percentText(progress.percent))
            CleanUpSparkline(points: Self.points(progress))
                .frame(height: Self.sparklineHeight)
                .padding(.vertical, Space.xs)
            VStack(spacing: Space.hair) {
                row("At Midnight", Self.count(progress.atMidnight))
                row("Received Today", Self.signed(progress.receivedToday, plus: true))
                row("Removed Today", Self.signed(progress.removedToday, plus: false))
                row("Now", Self.count(progress.now))
            }
        }
        .card(.neutral)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Inbox Zero")
        .hoverHelp("Inbox Zero since Clean Up was first opened, when the Inbox held \(Self.count(progress.baseline))")
    }

    private func row(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.s) {
            Text(label).foregroundStyle(.secondary).lineLimit(1)
            Spacer(minLength: 0)
            Text(value).lineLimit(1).layoutPriority(1)
        }
        .font(TypeRole.numeric)
        .accessibilityElement(children: .combine)
    }

    static let sparklineHeight: CGFloat = 28

    /// "62%".
    static func percentText(_ percent: UInt8) -> String { "\(percent)%" }

    static func count(_ n: UInt64) -> String { Int(clamping: n).formatted() }

    /// "+12" received, "−310" removed (a real minus sign); "0" for none.
    static func signed(_ n: UInt64, plus: Bool) -> String {
        guard n > 0 else { return "0" }
        return (plus ? "+" : "\u{2212}") + count(n)
    }

    /// The sparkline's points: each recorded day's count at midnight,
    /// then the Inbox now.
    static func points(_ progress: CleanupProgress) -> [UInt64] {
        progress.days.map(\.count) + [progress.now]
    }
}

/// A line over the Inbox's daily counts: no axes, no labels, the tint
/// colour; a single point (a first day) draws as a dot.
struct CleanUpSparkline: View {
    let points: [UInt64]

    var body: some View {
        let low = Double(points.min() ?? 0)
        let high = Double(points.max() ?? 0)
        // A flat line sits in the middle rather than on the floor.
        let domain = high > low ? low...high : (low - 1)...(high + 1)
        Chart(Array(points.enumerated()), id: \.offset) { point in
            LineMark(x: .value("Day", point.offset), y: .value("Inbox", Double(point.element)))
                .interpolationMethod(.monotone)
                .lineStyle(StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
            if point.offset == points.count - 1 {
                PointMark(x: .value("Day", point.offset), y: .value("Inbox", Double(point.element)))
                    .symbolSize(16)
            }
        }
        .foregroundStyle(.tint)
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .chartLegend(.hidden)
        .chartYScale(domain: domain)
        .chartXScale(domain: 0...max(points.count - 1, 1))
        .accessibilityLabel("The Inbox over the last \(points.count == 1 ? "day" : "\(points.count) days")")
        .accessibilityValue(points.map { Int(clamping: $0).formatted() }.joined(separator: ", "))
    }
}
