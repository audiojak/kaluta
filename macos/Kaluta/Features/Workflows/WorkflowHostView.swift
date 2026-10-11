import SwiftUI

/// A workflow over the whole window (spec §14.13): the header, progress,
/// one item at a time with its decision bar, and the end. Keys, buttons
/// and VoiceOver's actions all go through `WorkflowCommand`.
struct WorkflowHostView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    @FocusState private var focused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let actions = model.workflowActions()
        VStack(spacing: 0) {
            WorkflowHeader(kind: session.kind, progress: session.progress) { model.leaveWorkflow() }
            WorkflowProgress(kind: session.kind, decided: session.decided, total: session.total)
            Group {
                if session.isFinished {
                    WorkflowEndView(session: session)
                } else if let item = session.current {
                    ZStack {
                        ScrollView {
                            WorkflowItemView(session: session, item: item)
                                .padding(.horizontal, Space.page)
                                .padding(.vertical, Space.xxl)
                                .frame(maxWidth: Self.readingWidth, alignment: .leading)
                                .frame(maxWidth: .infinity)
                        }
                        .id(item.key)
                        .transition(itemTransition)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .clipped()
                    .safeAreaInset(edge: .bottom, spacing: 0) { decisionBar(actions) }
                } else {
                    ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .overlay(alignment: .bottom) { UndoNoticeView().padding(.bottom, session.isFinished ? Space.m : Self.noticeInset) }
        .background {
            // The start screen's card, grown into the window.
            ZStack {
                Color(nsColor: .windowBackgroundColor)
                Rectangle().fill(Tone.workflowGround(session.kind))
            }
            .workflowMorph(session.kind)
        }
        .focusable()
        .focused($focused)
        .defaultFocus($focused, true)
        .focusEffectDisabled()
        // Taken a turn after appearing: focus set while a view appears
        // is lost to the one it replaces.
        .onAppear { Task { @MainActor in focused = true } }
        // Each item takes the keyboard back, whatever was clicked before.
        .onChange(of: session.current?.key) { focused = true }
        .onKeyPress(phases: .down) { press in
            guard let command = WorkflowCommand.command(key: press.key, characters: press.characters,
                                                        modifiers: press.modifiers) else { return .ignored }
            if session.isFinished {
                switch command {
                case .back: model.leaveWorkflow()
                case .primary: model.continueAfter(session)
                default: return .ignored
                }
                return .handled
            }
            Task { await model.performWorkflow(command) }
            return .handled
        }
        .accessibilityElement(children: .contain)
        .accessibilityActions {
            ForEach(actions) { action in
                Button(action.title) { Task { await model.performWorkflow(action.command) } } // no-help: VoiceOver action
            }
            Button(WorkflowCommand.whatsLeft.title(in: session.kind)) { // no-help: VoiceOver action
                Task { await model.performWorkflow(.whatsLeft) }
            }
            Button(WorkflowCommand.whereAmI.title(in: session.kind)) { // no-help: VoiceOver action
                Task { await model.performWorkflow(.whereAmI) }
            }
        }
    }

    /// The decided item leaves by its verb's direction and the next rises
    /// (spec §14.13); Reduce Motion cross-fades.
    private var itemTransition: AnyTransition {
        if reduceMotion { return .opacity }
        return .asymmetric(insertion: .offset(y: Self.rise).combined(with: .opacity),
                           removal: .offset(session.exit.offset).combined(with: .opacity))
    }

    private func decisionBar(_ actions: [WorkflowAction]) -> some View {
        HStack(spacing: Space.m) {
            ForEach(actions.filter(\.shown)) { action in
                WorkflowButton(title: action.title, key: action.command.keyLabel, style: action.style,
                               kind: session.kind) {
                    Task { await model.performWorkflow(action.command) }
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, Space.page)
        .padding(.vertical, Space.xl)
        .frame(maxWidth: Self.readingWidth + Space.page * 2)
        .frame(maxWidth: .infinity)
    }

    static let readingWidth: CGFloat = 900
    /// How far the next item rises from.
    static let rise: CGFloat = 36
    /// The undo notice clears the decision bar.
    static let noticeInset: CGFloat = 84
}

/// The current item: a conversation (its sender, subject and latest
/// message at reading size) or a task above its email.
struct WorkflowItemView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let item: WorkflowItem

    var body: some View {
        let threadID = model.threadID(of: item)
        let detail = threadID.flatMap { session.threads[$0] }
        VStack(alignment: .leading, spacing: Space.xxl) {
            if let task = model.task(of: item) {
                VStack(alignment: .leading, spacing: Space.s) {
                    HStack(spacing: Space.m) {
                        Text(task.category)
                            .font(TypeRole.workflowKey).fontWeight(.semibold)
                            .padding(.horizontal, Space.m).padding(.vertical, Space.hair)
                            .background(Tone.workflow(session.kind).opacity(0.14), in: Capsule())
                        Text(Self.due(task)).font(TypeRole.workflowContext).foregroundStyle(.secondary)
                    }
                    Text(task.title).font(TypeRole.workflowDisplay)
                    if !task.why.isEmpty {
                        Label(task.why, systemImage: "sparkle")
                            .font(TypeRole.workflowContext)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            if let detail {
                VStack(alignment: .leading, spacing: Space.l) {
                    let latest = AppModel.latestReceived(detail)
                    Text(Self.contextLine(latest))
                        .font(TypeRole.workflowContext)
                        .foregroundStyle(.secondary)
                    if model.task(of: item) == nil {
                        Text(detail.thread.subject.isEmpty ? "(no subject)" : detail.thread.subject)
                            .font(TypeRole.workflowDisplay)
                    }
                    Text(session.bodies[detail.thread.id] ?? latest?.snippet ?? "")
                        .font(TypeRole.workflowContent)
                        .lineSpacing(Space.xs)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(Space.xxxl)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
            } else if threadID != nil {
                ProgressView()
            }
        }
    }

    static func contextLine(_ m: MessageInfo?) -> String {
        guard let m else { return "" }
        let who = m.from.map { $0.name?.isEmpty == false ? $0.name! : $0.email } ?? ""
        let when = Date(timeIntervalSince1970: TimeInterval(m.date) / 1000)
        return "\(who) · \(when.formatted(date: .abbreviated, time: .shortened))"
    }

    static func due(_ task: TaskItem) -> String {
        guard let day = task.dueDay else { return "No date" }
        let today = DueDay.string(.now)
        if day < today { return "Carried over" }
        if day == today { return "Due today" }
        return "Due \(day)"
    }
}

/// The end of a set: the ring, the score, the next workflow and back.
struct WorkflowEndView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let session: WorkflowSession

    var body: some View {
        let score = session.score
        let earned = (score?.decided ?? 0) > 0
        ScrollView {
            VStack(spacing: Space.xxxl) {
                CompletionRing(kind: session.kind, celebrating: earned, animated: earned)
                VStack(spacing: Space.s) {
                    Text(Self.headline(session)).font(TypeRole.workflowHeadline)
                    if let line = Self.subline(score) {
                        Text(line).font(TypeRole.workflowContent).foregroundStyle(.secondary)
                    }
                }
                .multilineTextAlignment(.center)
                if let score, !score.counts.isEmpty {
                    HStack(spacing: Space.l) {
                        ForEach(score.counts, id: \.decision) { c in
                            ScoreTile(figure: "\(c.count)", label: Self.words(c.decision, count: Int(c.count)))
                        }
                    }
                }
                if let next = model.nextWorkflow(after: session.kind) {
                    WorkflowButton(title: "Next: \(next.title)", key: "Return", style: .primary, kind: next) {
                        model.continueAfter(session)
                    }
                }
                WorkflowButton(title: "Back to the start", key: "Esc", style: .quiet, kind: session.kind) {
                    model.leaveWorkflow()
                }
            }
            .padding(Space.page)
            .frame(maxWidth: Self.width)
            .frame(maxWidth: .infinity)
        }
        .accessibilityElement(children: .contain)
        .task {
            // The chord sounds as the ring closes and bursts.
            guard earned else { return }
            if !reduceMotion { try? await Task.sleep(for: .seconds(CompletionRing.closeSeconds)) }
            WorkflowSound.playChord(defaults: model.defaults)
        }
    }

    static func headline(_ session: WorkflowSession) -> String {
        guard session.total > 0 else { return "Nothing waiting here." }
        switch session.kind {
        case .triage: return "Everything new is decided."
        case .tasks: return "Today's tasks are done."
        case .replies: return "Everyone has an answer."
        default: return "All done."
        }
    }

    static func subline(_ score: WorkflowScore?) -> String? {
        guard let score, score.decided > 0 else { return nil }
        let minutes = max(1, Int((score.millis + 30_000) / 60_000))
        return "\(score.decided) decided in about \(minutes) minute\(minutes == 1 ? "" : "s")."
    }

    static func words(_ decision: String, count: Int) -> String {
        switch decision {
        case "archive": "archived"
        case "reply": count == 1 ? "reply" : "replies"
        case "task": count == 1 ? "task made" : "tasks made"
        case "leave": "left in Inbox"
        case "done": "done"
        case "later": "moved to tomorrow"
        default: decision
        }
    }

    static let width: CGFloat = 880
}

extension AppModel {
    /// The workflow after `kind` on the start screen that has anything in it.
    func nextWorkflow(after kind: WorkflowKind) -> WorkflowKind? {
        guard let overview = workflowOverview else { return nil }
        let order = WorkflowKind.all
        let start = (order.firstIndex(of: kind) ?? 0) + 1
        return order[start...].first { k in overview.workflows.first { $0.kind == k }.map { $0.count > 0 } ?? false }
    }

    /// Return at an end: the next workflow, else the start screen.
    func continueAfter(_ session: WorkflowSession) {
        let next = nextWorkflow(after: session.kind)
        workflowSession = nil
        if let next { Task { await beginWorkflow(next) } } else { Task { await refreshWorkflowOverview() } }
    }
}
