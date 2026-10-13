import SwiftUI

/// The start screen, *What's next* (spec §14.13): the day in one line, the
/// next workflow as a large card (Return), the others on 1–7, Done today,
/// and the mail view on M.
struct StartScreenView: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        let overview = model.workflowOverview
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxxl) {
                header(overview)
                if let overview, let next = overview.next,
                   let summary = overview.workflows.first(where: { $0.kind == next }) {
                    NextWorkflowCard(summary: summary) { start(next) }
                } else if overview != nil {
                    Text("Nothing is waiting. New mail will be here for triage.")
                        .font(TypeRole.workflowContent)
                        .foregroundStyle(.secondary)
                }
                if let overview {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(minimum: Self.cardMinimum), spacing: Space.xl),
                                             count: 3),
                              alignment: .leading, spacing: Space.xl) {
                        ForEach(overview.workflows.filter { $0.kind != overview.next }, id: \.kind) { summary in
                            WorkflowCard(summary: summary) { start(summary.kind) }
                        }
                    }
                    footer(overview)
                }
            }
            .padding(Space.page)
            .frame(maxWidth: Self.width, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(Color(nsColor: .windowBackgroundColor))
        .focusable()
        .focused($focused)
        .defaultFocus($focused, true)
        .focusEffectDisabled()
        // Taken a turn after appearing: focus set while a view appears
        // is lost to the one it replaces.
        .onAppear { Task { @MainActor in focused = true } }
        .task(id: [model.workflowsRevision, model.tasksRevision, model.inboxRevision, model.guideRevision,
                   model.factsRevision]) {
            // Counts settle as mail, tasks and proposals change: at most
            // once a second.
            try? await Task.sleep(for: .milliseconds(300))
            await model.refreshWorkflowOverview()
        }
        .onKeyPress(phases: .down) { press in
            guard press.modifiers.isDisjoint(with: [.command, .control, .option]) else { return .ignored }
            if press.key == .return, let next = overview?.next {
                start(next)
                return .handled
            }
            switch press.characters.lowercased() {
            case "m":
                model.showMail()
                return .handled
            case let c where Int(c).map { (1...7).contains($0) } == true:
                start(WorkflowKind.all[Int(c)! - 1])
                return .handled
            default:
                return .ignored
            }
        }
        .accessibilityAction(named: "Open Mail") { model.showMail() }
    }

    private func start(_ kind: WorkflowKind) {
        Task { await model.beginWorkflow(kind) }
    }

    private func header(_ overview: WorkflowOverview?) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(Date.now, format: .dateTime.weekday(.wide).day().month(.wide).hour().minute())
                .font(TypeRole.workflowContext)
                .foregroundStyle(.secondary)
            Text(Self.headline(overview))
                .font(TypeRole.workflowHeadline)
            if let next = overview?.next {
                Text(next == .triage ? "Start with what's new: answer what takes 30 seconds, make tasks of the rest."
                                     : "Start with \(next.title.lowercased()).")
                    .font(TypeRole.workflowContent)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private func footer(_ overview: WorkflowOverview) -> some View {
        HStack(spacing: Space.l) {
            Text(Self.doneToday(overview)).foregroundStyle(.secondary)
            Spacer(minLength: Space.l)
            Text("Return starts · 1–7 chooses · M opens Mail").foregroundStyle(.secondary)
        }
        .font(TypeRole.workflowContext)
    }

    static func headline(_ overview: WorkflowOverview?) -> String {
        guard let overview else { return "What's next" }
        let minutes = (Int(overview.estimateSecs) + 30) / 60
        if overview.next == nil { return "You're all caught up." }
        return minutes < 1 ? "A minute or two of mail today." : "About \(minutes) minutes of mail today."
    }

    static func doneToday(_ overview: WorkflowOverview) -> String {
        let sets = overview.doneToday.reduce(0) { $0 + Int($1.setsFinished) }
        let decided = overview.doneToday.reduce(0) { $0 + Int($1.decisions) }
        guard decided > 0 else { return "Nothing done yet today." }
        return "Done today: \(sets) set\(sets == 1 ? "" : "s") · \(decided) decided"
    }

    static let width: CGFloat = 1040
    static let cardMinimum: CGFloat = 220
}

/// The next workflow, large, in its colour.
private struct NextWorkflowCard: View {
    let summary: WorkflowSummary
    let start: () -> Void

    var body: some View {
        Button(action: start) { label }
        .buttonStyle(.plain)
        .hoverHelp("\(summary.kind.startTitle) (Return)")
        .accessibilityLabel("Next: \(summary.kind.startTitle). \(WorkflowCard.detail(summary))")
    }

    private var label: some View {
        HStack(spacing: Space.xxl) {
            VStack(alignment: .leading, spacing: Space.s) {
                Text("NEXT").font(TypeRole.workflowKey).fontWeight(.semibold).opacity(0.85)
                Text(summary.kind.startTitle).font(TypeRole.workflowDisplay)
                Text(WorkflowCard.detail(summary)).font(TypeRole.workflowContent).opacity(0.92)
                if !summary.preview.isEmpty {
                    HStack(spacing: Space.s) {
                        ForEach(summary.preview.prefix(4), id: \.self) { name in
                            Text(Self.initials(name))
                                .font(TypeRole.workflowKey).fontWeight(.semibold)
                                .frame(width: Self.avatar, height: Self.avatar)
                                .background(.white.opacity(0.22), in: Circle())
                        }
                        if summary.count > 4 {
                            Text("+\(summary.count - 4)")
                                .font(TypeRole.workflowKey).fontWeight(.semibold)
                                .padding(.horizontal, Space.m)
                                .frame(height: Self.avatar)
                                .background(.white.opacity(0.22), in: Capsule())
                        }
                    }
                    .accessibilityHidden(true)
                }
            }
            Spacer(minLength: Space.l)
            VStack(spacing: Space.s) {
                Image(systemName: "return")
                    .font(TypeRole.workflowDisplay)
                    .foregroundStyle(Tone.workflow(summary.kind))
                    .frame(width: Self.returnBadge, height: Self.returnBadge)
                    .background(.white, in: Circle())
                Text("Return").font(TypeRole.workflowKey).fontWeight(.semibold)
            }
            .accessibilityHidden(true)
        }
        .foregroundStyle(.white)
        .padding(Space.xxxl)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: Self.radius).fill(Tone.workflow(summary.kind)).workflowMorph(summary.kind)
        }
        .contentShape(RoundedRectangle(cornerRadius: Self.radius))
    }

    static func initials(_ name: String) -> String {
        let parts = name.split(separator: " ").prefix(2)
        return parts.compactMap(\.first).map { String($0).uppercased() }.joined()
    }

    static let avatar: CGFloat = 34
    static let returnBadge: CGFloat = 72
    static let radius: CGFloat = 20
}

/// Another workflow, with its count and key.
private struct WorkflowCard: View {
    let summary: WorkflowSummary
    let start: () -> Void

    var body: some View {
        let empty = summary.count == 0
        Button(action: start) {
            VStack(alignment: .leading, spacing: Space.s) {
                HStack(spacing: Space.m) {
                    Circle().fill(Tone.workflow(summary.kind)).frame(width: Space.l, height: Space.l)
                    Text(summary.kind.title).font(TypeRole.workflowHeading)
                    Spacer(minLength: 0)
                    Keycap(key: "\(summary.kind.number)")
                }
                Text(Self.detail(summary)).font(TypeRole.workflowContext).foregroundStyle(.primary)
                Text(Self.estimate(summary)).font(TypeRole.workflowKey).foregroundStyle(.secondary)
            }
            .padding(Space.xxl)
            .frame(maxWidth: .infinity, minHeight: Self.height, alignment: .topLeading)
            .background {
                RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.workflowSurface).workflowMorph(summary.kind)
            }
            .opacity(empty ? 0.6 : 1)
            .contentShape(RoundedRectangle(cornerRadius: Radius.panel))
        }
        .buttonStyle(.plain)
        .disabled(empty && summary.kind != .teach && summary.kind != .agents)
        .hoverHelp("\(summary.kind.title) (\(summary.kind.number))")
        .accessibilityLabel("\(summary.kind.title). \(Self.detail(summary))")
    }

    static func detail(_ s: WorkflowSummary) -> String {
        let n = Int(s.count)
        if n == 0 { return "Nothing waiting" }
        switch s.kind {
        case .triage: return "\(n) new conversation\(n == 1 ? "" : "s")"
        case .replies: return "\(n) waiting on your answer"
        case .tasks: return "\(n) for today" + (s.later > 0 ? " · \(s.later) later this week" : "")
        case .waiting: return "\(n) with no answer"
        case .reading: return "\(n) due today"
        case .agents: return "\(n) waiting on you"
        case .teach: return "\(n) to decide"
        }
    }

    static func estimate(_ s: WorkflowSummary) -> String {
        guard s.count > 0 else { return s.inProgress ? "Finished for now" : " " }
        let minutes = max(1, (Int(s.estimateSecs) + 30) / 60)
        return (s.inProgress ? "In progress · " : "") + "about \(minutes) min"
    }

    static let height: CGFloat = 120
}
