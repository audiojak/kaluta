import SwiftUI

// Workflows (spec §14.13): the type, colours and parts every workflow is
// built from. Each workflow looks like its own place, with the same
// vocabulary: content in large reading type, context quieter, controls
// smallest and consistent, and the same key for the same verb.

extension TypeRole {
    /// The item's title and an end's headline: what the screen is about.
    static let workflowDisplay = Font.system(size: 34, weight: .semibold)
    /// The start screen's and an end's one-line headline.
    static let workflowHeadline = Font.system(size: 40, weight: .semibold)
    /// The email, the draft, a summary: read at arm's length.
    static let workflowContent = Font.system(size: 21)
    /// A card's or rail row's name; a sub-heading.
    static let workflowHeading = Font.system(size: 20, weight: .semibold)
    /// Who, when and why: quieter than the content.
    static let workflowContext = Font.system(size: 15)
    /// Buttons in a decision bar.
    static let workflowControl = Font.system(size: 17, weight: .semibold)
    /// Keycaps, pills and section labels.
    static let workflowKey = Font.system(size: 13)
    /// A score tile's figure.
    static let workflowFigure = Font.system(size: 34, weight: .semibold).monospacedDigit()
}

extension WorkflowKind {
    /// In the start screen's order.
    static let all: [WorkflowKind] = [.triage, .replies, .tasks, .waiting, .reading, .agents, .teach]

    var title: String {
        switch self {
        case .triage: "Triage"
        case .replies: "Replies"
        case .tasks: "Tasks"
        case .waiting: "Waiting on others"
        case .reading: "Weekly reading"
        case .agents: "Agents"
        case .teach: "Teach Kaluta"
        }
    }

    /// The start screen's verb for the large card.
    var startTitle: String {
        switch self {
        case .triage: "Triage what's new"
        case .replies: "Answer what's waiting"
        case .tasks: "Work today's tasks"
        case .waiting: "Chase what you're waiting on"
        case .reading: "Catch up on reading"
        case .agents: "See what your agents need"
        case .teach: "Teach Kaluta"
        }
    }

    /// Its key on the start screen (1–7).
    var number: Int { Self.all.firstIndex(of: self).map { $0 + 1 } ?? 0 }

    var symbol: String {
        switch self {
        case .triage: "tray.and.arrow.down"
        case .replies: "arrowshape.turn.up.left"
        case .tasks: "checklist"
        case .waiting: "hourglass"
        case .reading: "newspaper"
        case .agents: "person.2.badge.gearshape"
        case .teach: "graduationcap"
        }
    }
}

extension Tone {
    /// A workflow's own colour (spec §14.13): white text on it meets 4.5:1
    /// in both appearances.
    static func workflow(_ kind: WorkflowKind) -> Color {
        switch kind {
        case .triage: Color(red: 0.290, green: 0.275, blue: 0.784) // indigo #4a46c8
        case .replies: Color(red: 0.059, green: 0.490, blue: 0.447) // teal #0f7d72
        case .tasks: Color(red: 0.722, green: 0.361, blue: 0.039) // orange #b85c0a
        case .waiting: Color(red: 0.482, green: 0.310, blue: 0.788) // purple #7b4fc9
        case .reading: Color(red: 0.494, green: 0.392, blue: 0.110) // sand #7e641c
        case .agents: Color(red: 0.290, green: 0.333, blue: 0.400) // graphite #4a5566
        case .teach: Color(red: 0.184, green: 0.490, blue: 0.231) // green #2f7d3b
        }
    }

    /// A workflow's ground: its colour faintly over the window's.
    static func workflowGround(_ kind: WorkflowKind) -> some ShapeStyle {
        workflow(kind).opacity(0.06)
    }

    /// Cards and fields on a workflow's ground.
    static let workflowSurface = Color(nsColor: .textBackgroundColor)
}

/// A key the command answers to, drawn on its control ("E", "⌘Return").
struct Keycap: View {
    let key: String
    var onFill = false

    var body: some View {
        Text(key)
            .font(TypeRole.workflowKey)
            .lineLimit(1)
            .fixedSize()
            .foregroundStyle(onFill ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
            .padding(.horizontal, Space.s)
            .padding(.vertical, Space.hair)
            .background(onFill ? AnyShapeStyle(.white.opacity(0.22)) : AnyShapeStyle(.clear),
                        in: RoundedRectangle(cornerRadius: Radius.chip))
            .overlay {
                if !onFill { RoundedRectangle(cornerRadius: Radius.chip).strokeBorder(.separator) }
            }
            .accessibilityHidden(true)
    }
}

/// A decision bar's button: the primary one filled in the workflow's
/// colour, the others plain, the quiet ones without a ground; each shows
/// its key.
struct WorkflowButton: View {
    enum Style { case primary, plain, quiet }

    let title: String
    let key: String?
    var style: Style = .plain
    let kind: WorkflowKind
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: Space.m) {
                Text(title).lineLimit(1)
                if let key { Keycap(key: key, onFill: style == .primary) }
            }
            .fixedSize(horizontal: true, vertical: false)
            .font(TypeRole.workflowControl)
            .foregroundStyle(style == .primary ? AnyShapeStyle(.white)
                             : style == .quiet ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
            .padding(.horizontal, Space.xxl)
            .frame(minHeight: Self.height)
            .background(background, in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .hoverHelp(key.map { "\(title) (\($0))" } ?? title)
        .accessibilityLabel(title)
    }

    private var background: AnyShapeStyle {
        switch style {
        case .primary: AnyShapeStyle(Tone.workflow(kind))
        case .plain: AnyShapeStyle(Tone.workflowSurface)
        case .quiet: AnyShapeStyle(.clear)
        }
    }

    /// Touch-target height (44 pt and more).
    static let height: CGFloat = 48
}

/// A workflow's header: back to Mail, the workflow's name with "n of N",
/// and at most one status pill.
struct WorkflowHeader: View {
    let kind: WorkflowKind
    let progress: String
    var backTitle = "Mail"
    var pill: String?
    let back: () -> Void

    var body: some View {
        HStack(spacing: Space.xl) {
            Button(action: back) {
                HStack(spacing: Space.xs) {
                    Image(systemName: "chevron.left")
                    Text(backTitle)
                    Keycap(key: "Esc")
                }
                .font(TypeRole.workflowKey)
                .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .hoverHelp("\(backTitle) (Esc)")
            Spacer(minLength: Space.m)
            HStack(spacing: Space.xs) {
                Text(kind.title).fontWeight(.semibold)
                if !progress.isEmpty { Text("· \(progress)").foregroundStyle(.secondary) }
            }
            .font(TypeRole.workflowContext)
            .accessibilityElement(children: .combine)
            Spacer(minLength: Space.m)
            Text(pill ?? "")
                .font(TypeRole.workflowKey)
                .foregroundStyle(Tone.workflow(kind))
                .padding(.horizontal, Space.m)
                .padding(.vertical, Space.hair)
                .background(Tone.workflow(kind).opacity(pill == nil ? 0 : 0.12), in: Capsule())
                .opacity(pill == nil ? 0 : 1)
                .accessibilityHidden(pill == nil)
        }
        .padding(.horizontal, Space.xxl)
        .frame(height: Self.height)
    }

    static let height: CGFloat = 44
}

/// Progress under the header: a segment per item up to 20, else one bar;
/// decided in the workflow's colour, the current one lighter.
struct WorkflowProgress: View {
    let kind: WorkflowKind
    let decided: Int
    let total: Int

    var body: some View {
        Group {
            if total <= Self.maxSegments {
                HStack(spacing: Space.xs) {
                    ForEach(0..<max(total, 1), id: \.self) { i in
                        Capsule().fill(fill(i)).frame(height: Self.thickness)
                    }
                }
            } else {
                GeometryReader { geo in
                    ZStack(alignment: .leading) {
                        Capsule().fill(.quaternary)
                        Capsule().fill(Tone.workflow(kind))
                            .frame(width: geo.size.width * CGFloat(decided) / CGFloat(max(total, 1)))
                    }
                }
                .frame(height: Self.thickness)
            }
        }
        .padding(.horizontal, Space.xxl)
        .accessibilityElement()
        .accessibilityLabel("\(decided) of \(total) decided")
    }

    private func fill(_ i: Int) -> AnyShapeStyle {
        if i < decided { return AnyShapeStyle(Tone.workflow(kind)) }
        if i == decided { return AnyShapeStyle(Tone.workflow(kind).opacity(0.35)) }
        return AnyShapeStyle(.quaternary)
    }

    static let maxSegments = 20
    static let thickness: CGFloat = 5
}

/// A rail row: an item of the set with its state.
struct WorkflowRailRow: View {
    enum State { case current, waiting, done }

    let title: String
    let detail: String
    let state: State
    let kind: WorkflowKind

    var body: some View {
        HStack(spacing: Space.m) {
            Group {
                switch state {
                case .current: Circle().fill(Tone.workflow(kind))
                case .waiting: Circle().strokeBorder(Tone.workflow(kind).opacity(0.5), lineWidth: 2)
                case .done: Image(systemName: "checkmark").foregroundStyle(Tone.approved)
                }
            }
            .frame(width: Self.mark, height: Self.mark)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(title)
                    .font(TypeRole.workflowContext)
                    .fontWeight(state == .current ? .semibold : .regular)
                    .strikethrough(state == .done)
                Text(detail).font(TypeRole.workflowKey).foregroundStyle(.secondary)
            }
            .lineLimit(1)
            Spacer(minLength: 0)
        }
        .foregroundStyle(state == .done ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
        .padding(.horizontal, Space.l)
        .padding(.vertical, Space.m)
        .background(state == .current ? AnyShapeStyle(Tone.workflowSurface) : AnyShapeStyle(.clear),
                    in: RoundedRectangle(cornerRadius: Radius.panel))
        .accessibilityElement(children: .combine)
    }

    static let mark: CGFloat = 16
}

/// One figure of a score.
struct ScoreTile: View {
    let figure: String
    let label: String

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xs) {
            Text(figure).font(TypeRole.workflowFigure)
            Text(label).font(TypeRole.workflowContext).foregroundStyle(.secondary)
        }
        .frame(minWidth: Self.minWidth, maxWidth: Self.maxWidth, alignment: .leading)
        .padding(Space.xl)
        .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
        .accessibilityElement(children: .combine)
    }

    /// Tiles share a row; one alone stays tile-sized.
    static let minWidth: CGFloat = 120
    static let maxWidth: CGFloat = 220
}

/// The ring a set closes: in the workflow's colour, a check inside, with
/// a burst of ticks around it while `celebrating`.
struct CompletionRing: View {
    let kind: WorkflowKind
    var celebrating = true
    /// Closes, then bursts, as it appears (spec §14.13); Reduce Motion
    /// draws it closed.
    var animated = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var closed = false
    @State private var burst = false

    var body: some View {
        let still = !animated || reduceMotion
        let isClosed = still || closed
        let isBurst = still || burst
        ZStack {
            ForEach(0..<Self.ticks, id: \.self) { i in
                Capsule()
                    .fill(i % 3 == 0 ? Tone.workflow(kind) : i % 3 == 1 ? Tone.workflow(.replies) : Tone.workflow(.tasks))
                    .frame(width: Self.tickWidth, height: i.isMultiple(of: 2) ? Self.tickLong : Self.tickShort)
                    .offset(y: -(isBurst ? Self.burstRadius : Self.diameter / 2))
                    .rotationEffect(.degrees(Double(i) * 360 / Double(Self.ticks)))
                    .opacity(celebrating && isBurst ? 1 : 0)
            }
            Circle()
                .fill(Tone.workflowSurface)
                .frame(width: Self.diameter, height: Self.diameter)
            Circle()
                .trim(from: 0, to: isClosed ? 1 : 0)
                .stroke(Tone.workflow(kind), style: StrokeStyle(lineWidth: Self.ringWidth, lineCap: .round))
                .rotationEffect(.degrees(-90))
                .frame(width: Self.diameter - Self.ringWidth, height: Self.diameter - Self.ringWidth)
            Image(systemName: "checkmark")
                .font(TypeRole.workflowHeadline)
                .foregroundStyle(Tone.workflow(kind))
                .scaleEffect(isClosed ? 1 : 0.6)
                .opacity(isClosed ? 1 : 0)
        }
        .frame(width: Self.burstRadius * 2 + Self.tickLong, height: Self.burstRadius * 2 + Self.tickLong)
        .accessibilityHidden(true)
        .task {
            guard !still else { return }
            withAnimation(.easeInOut(duration: Self.closeSeconds)) { closed = true }
            try? await Task.sleep(for: .seconds(Self.closeSeconds))
            withAnimation(.spring(duration: Self.burstSeconds, bounce: 0.4)) { burst = true }
        }
    }

    static let closeSeconds = 0.4
    static let burstSeconds = 0.6
    static let ticks = 12
    static let diameter: CGFloat = 150
    static let ringWidth: CGFloat = 10
    static let burstRadius: CGFloat = 104
    static let tickWidth: CGFloat = 5
    static let tickLong: CGFloat = 18
    static let tickShort: CGFloat = 12
}

extension EnvironmentValues {
    /// Shared by the start screen's cards and a workflow's window, so a
    /// card grows into its workflow (spec §14.13).
    @Entry var workflowMorph: Namespace.ID?
}

extension View {
    /// This view and the other with the same `kind` share a frame across
    /// start ↔ workflow; nothing under Reduce Motion.
    func workflowMorph(_ kind: WorkflowKind) -> some View {
        modifier(WorkflowMorph(kind: kind))
    }
}

private struct WorkflowMorph: ViewModifier {
    let kind: WorkflowKind
    @Environment(\.workflowMorph) private var namespace
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func body(content: Content) -> some View {
        if let namespace, !reduceMotion {
            content.matchedGeometryEffect(id: kind, in: namespace)
        } else {
            content
        }
    }
}
