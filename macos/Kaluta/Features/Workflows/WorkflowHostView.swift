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
            WorkflowHeader(kind: session.kind, progress: session.progress, pill: model.workflowPill(session)) {
                model.leaveWorkflow()
            }
            WorkflowProgress(kind: session.kind, decided: session.decided, total: session.total)
            HStack(spacing: 0) {
            // Tasks: today's set beside the task, done ones ticked.
            // Writing needs the room: the rail steps aside meanwhile.
            if [.tasks, .waiting, .reading].contains(session.kind), !session.isFinished, !session.showsReport,
               session.writer == nil {
                WorkflowRail(session: session)
            }
            Group {
                if session.isFinished {
                    WorkflowEndView(session: session)
                } else if session.kind == .reading, session.showsReport {
                    ReadingReportView(session: session)
                        .safeAreaInset(edge: .bottom, spacing: 0) { decisionBar(actions) }
                } else if let item = session.current {
                    ZStack {
                        ScrollViewReader { proxy in
                            ScrollView {
                                WorkflowItemView(session: session, item: item)
                                    .padding(.horizontal, Space.page)
                                    .padding(.vertical, Space.xxl)
                                    .frame(maxWidth: Self.readingWidth, alignment: .leading)
                                    .frame(maxWidth: .infinity)
                            }
                            // Each writing step comes into view above the bar.
                            .onChange(of: [session.writer.map { "\($0.step)" }, session.taskForm.map { _ in "task" }]) { _, now in
                                guard now.contains(where: { $0 != nil }) else { return }
                                withAnimation(AppModel.itemMotion) {
                                    proxy.scrollTo(WorkflowWriteView.scrollID, anchor: .bottom)
                                }
                            }
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
        }
        .overlay(alignment: writing ? .top : .bottom) {
            // While writing, the notice keeps clear of the text: under the header.
            UndoNoticeView()
                .padding(writing ? .top : .bottom, writing ? Self.headerInset : session.isFinished ? Space.m : Self.noticeInset)
        }
        .background {
            // The start screen's card, grown into the window.
            ZStack {
                Color(nsColor: .windowBackgroundColor)
                Rectangle().fill(Tone.workflowGround(session.kind))
            }
            .workflowMorph(session.kind)
        }
        // Not while there is text to type: a focusable screen keeps SwiftUI
        // from handing the keyboard to the field inside it.
        .focusable(!session.isTyping)
        .focused($focused)
        .defaultFocus($focused, true)
        .focusEffectDisabled()
        // Taken a turn after appearing: focus set while a view appears
        // is lost to the one it replaces.
        .onAppear { Task { @MainActor in focused = true } }
        // Each item takes the keyboard back, whatever was clicked before,
        // and so does the item when its writing closes.
        .onChange(of: session.current?.key) { focused = true }
        // Typing: the field takes the keys (the screen lets go); done, the
        // screen takes them back (Kaluta's answers' 1–3 and / are its).
        .onChange(of: session.isTyping) { _, typing in focused = !typing }
        .onKeyPress(phases: .down) { press in
            // Typing or dictating in a field: the keys are the text's, not
            // the workflow's (⌘Return still sends, from the field's own
            // shortcuts).
            if Self.isEditingText { return .ignored }
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

    /// A text field or editor has the keyboard (its field editor is a text
    /// view first responder).
    static var isEditingText: Bool {
        NSApp.keyWindow?.firstResponder is NSTextView
    }

    static let readingWidth: CGFloat = 900
    /// How far the next item rises from.
    static let rise: CGFloat = 36
    /// The undo notice clears the decision bar, or the header and progress.
    static let noticeInset: CGFloat = 84
    static let headerInset: CGFloat = 64

    private var writing: Bool { session.writer != nil || session.taskForm != nil }
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
            if session.kind == .agents {
                AgentItemView(session: session, item: item)
            }
            if session.kind == .reading, let bucket = session.bucket(of: item) {
                Text("\(bucket.title) · \(bucket.verb)")
                    .font(TypeRole.workflowKey).fontWeight(.semibold)
                    .padding(.horizontal, Space.m).padding(.vertical, Space.hair)
                    .background(Tone.workflow(session.kind).opacity(0.14), in: Capsule())
            }
            if session.kind == .waiting, let threadID, let wait = session.waits[threadID] {
                WaitingLine(wait: wait, kind: session.kind)
            }
            if session.kind == .triage, let threadID {
                TriageSuggestionLine(suggestion: session.suggestions[threadID],
                                     task: model.openTask(onThread: threadID), kind: session.kind)
            }
            if let detail {
                VStack(alignment: .leading, spacing: Space.l) {
                    let latest = AppModel.shownMessage(detail, session.kind)
                    Text(Self.contextLine(latest))
                        .font(TypeRole.workflowContext)
                        .foregroundStyle(.secondary)
                    if model.task(of: item) == nil {
                        Text(detail.thread.subject.isEmpty ? "(no subject)" : detail.thread.subject)
                            .font(TypeRole.workflowDisplay)
                    }
                    // Weekly reading: Kaluta's summary until it is opened in full.
                    let summary = session.kind == .reading && !session.showsInFull
                        ? session.summaries[detail.thread.id]?.summary : nil
                    if summary != nil {
                        Label("Kaluta's summary", systemImage: "sparkle")
                            .font(TypeRole.workflowContext).foregroundStyle(.secondary)
                    }
                    let text = summary ?? session.bodies[detail.thread.id] ?? latest?.snippet ?? ""
                    // What the email asks, marked where an answer is written.
                    Text(Self.asksAnswer(session.kind)
                        ? WritingText.highlighted(text, questions: WritingText.questions(in: text),
                                                  tint: Tone.workflow(session.kind).opacity(0.16))
                        : AttributedString(text))
                        .font(TypeRole.workflowContent)
                        .lineSpacing(Space.xs)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(Space.xxxl)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                if session.choosingCheckAgain {
                    Text("Check again when? 1 tomorrow, 2 in three days, 3 next week")
                        .font(TypeRole.workflowHeading)
                        .id(WorkflowWriteView.scrollID)
                }
                if let writer = session.writer {
                    WorkflowWriteView(session: session, writer: writer)
                        .transition(.opacity)
                } else if let form = session.taskForm {
                    WorkflowTaskFormView(session: session, form: form)
                        .transition(.opacity)
                }
            } else if threadID != nil {
                ProgressView()
            }
        }
    }

    /// Workflows where the user answers the email: its questions are marked.
    static func asksAnswer(_ kind: WorkflowKind) -> Bool {
        kind == .replies || kind == .tasks
    }

    static func contextLine(_ m: MessageInfo?) -> String {
        guard let m else { return "" }
        // The user's own message (what they asked, in Waiting) is theirs.
        let who = m.isSentByMe ? "You wrote" : m.from.map { $0.name?.isEmpty == false ? $0.name! : $0.email } ?? ""
        let when = Date(timeIntervalSince1970: TimeInterval(m.date) / 1000)
        return "\(who) · \(when.formatted(date: .abbreviated, time: .shortened))"
    }

    /// "Due today", "Carried over from yesterday", "Carried over from
    /// Friday" (within the week), "Carried over from 3 Oct".
    static func due(_ task: TaskItem, now: Date = .now) -> String {
        guard let day = task.dueDay, let date = DueDay.date(day) else { return "No date" }
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: now)
        let days = calendar.dateComponents([.day], from: date, to: today).day ?? 0
        switch days {
        case ..<0: return "Due \(date.formatted(.dateTime.weekday(.wide).day().month(.abbreviated)))"
        case 0: return "Due today"
        case 1: return "Carried over from yesterday"
        case 2...6: return "Carried over from \(date.formatted(.dateTime.weekday(.wide)))"
        default: return "Carried over from \(date.formatted(.dateTime.day().month(.abbreviated)))"
        }
    }
}

/// Tasks' rail (spec §14.13): today's set, the task on screen marked,
/// finished ones ticked (those finished elsewhere too). What is left for
/// later in the week is the header's pill.
struct WorkflowRail: View {
    let session: WorkflowSession

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(railTitle).font(TypeRole.workflowKey).fontWeight(.semibold).foregroundStyle(.secondary)
                    .padding(.horizontal, Space.l)
                ForEach(railItems, id: \.key) { item in
                    if session.kind == .reading, let id = item.threadId {
                        WorkflowRailRow(title: session.summaries[id]?.line ?? session.threads[id]?.thread.subject
                                            ?? session.readingSubject(id) ?? "…",
                                        detail: Self.readingDetail(item), state: state(item), kind: session.kind)
                    } else if let id = item.taskId, let task = session.taskInfo[id] {
                        WorkflowRailRow(title: task.title, detail: Self.detail(task, item), state: state(item),
                                        kind: session.kind)
                    } else if let id = item.threadId, let wait = session.waits[id] {
                        WorkflowRailRow(title: wait.who.isEmpty ? "Someone" : wait.who, detail: Self.detail(wait, item),
                                        state: state(item), kind: session.kind)
                    }
                }
            }
            .padding(.vertical, Space.xl)
            .padding(.horizontal, Space.m)
        }
        .frame(width: Self.width)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(session.kind == .waiting ? "Waiting on others" : "Today's tasks")
    }

    /// Weekly reading lists the bucket on screen, one line each.
    private var railItems: [WorkflowItem] {
        guard session.kind == .reading, let current = session.current, let bucket = session.bucket(of: current) else {
            return session.items
        }
        return session.items.filter { session.bucket(of: $0)?.id == bucket.id }
    }

    private var railTitle: String {
        switch session.kind {
        case .waiting: "Waiting on"
        case .reading: session.current.flatMap(session.bucket(of:))?.title ?? "Reading"
        default: "Today"
        }
    }

    static func readingDetail(_ item: WorkflowItem) -> String {
        switch item.decision {
        case "read": "Read"
        case "kept": "Kept for later"
        case "inbox": "Back in the Inbox"
        case "task": "Made a task"
        case .some: "Done"
        case nil: "Unread"
        }
    }

    private func state(_ item: WorkflowItem) -> WorkflowRailRow.State {
        if item.decision != nil { return .done }
        return item.key == session.current?.key ? .current : .waiting
    }

    static func detail(_ task: TaskItem, _ item: WorkflowItem) -> String {
        switch item.decision {
        case "elsewhere": "Done elsewhere"
        case "later": "Tomorrow"
        case .some: "Done"
        case nil: "\(task.category) · \(WorkflowItemView.due(task))"
        }
    }

    /// A wait: how long, or how it ended ("Answered · off the list").
    static func detail(_ wait: WaitingInfo, _ item: WorkflowItem, now: Date = .now) -> String {
        switch item.decision {
        case "answered": "Answered · off the list"
        case "let_go": "Let go"
        case "later": "Check again later"
        case "nudged": "Nudged"
        case .some: "Done"
        case nil: WaitingLine.waited(wait, now: now)
        }
    }

    static let width: CGFloat = 280
}

/// An Agents item (spec §14.13): a proposed send on its editable email
/// with the request that produced it, another proposed action, a code
/// from an agent mailbox shown large, or this morning's sorting run.
struct AgentItemView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let item: WorkflowItem

    var body: some View {
        VStack(alignment: .leading, spacing: Space.xxl) {
            switch model.agentItem(item, in: session) {
            case let .action(_, tool, summary, draftID, request):
                if let request, !request.isEmpty {
                    Label("You asked: \(request)", systemImage: "text.bubble")
                        .font(TypeRole.workflowContext).foregroundStyle(.secondary).lineLimit(2)
                }
                Text(summary).font(TypeRole.workflowDisplay).fixedSize(horizontal: false, vertical: true)
                if AgentStore.isSend(tool), draftID != nil {
                    TextEditor(text: Binding(get: { session.agentDrafts[item.key] ?? "" },
                                             set: { session.agentDrafts[item.key] = $0 }))
                        .font(TypeRole.workflowContent)
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: WorkflowWriteView.editorHeight)
                        .padding(Space.xl)
                        .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
                        .accessibilityLabel("The email that will be sent")
                }
            case .elsewhere:
                Text("Waiting in another account's agent panel").font(TypeRole.workflowDisplay)
                Text("Open that account in Mail to decide it.").font(TypeRole.workflowContent).foregroundStyle(.secondary)
            case let .code(code):
                Text("\(code.from) · to \(code.mailbox)").font(TypeRole.workflowContext).foregroundStyle(.secondary)
                Text(code.subject).font(TypeRole.workflowContent)
                Text(Self.spaced(code.code))
                    .font(TypeRole.workflowCode)
                    .textSelection(.enabled)
                    .accessibilityLabel("Code \(code.code.map(String.init).joined(separator: " "))")
                if let expires = code.expiresAt {
                    let at = Date(timeIntervalSince1970: TimeInterval(expires) / 1000)
                    Text(at < .now ? "Expired" : "Expires at \(at.formatted(date: .omitted, time: .shortened))")
                        .font(TypeRole.workflowContext).foregroundStyle(.secondary)
                }
            case .run:
                Text("This morning's sorting run").font(TypeRole.workflowDisplay)
                if let run = session.agentRuns[item.key] {
                    let at = Date(timeIntervalSince1970: TimeInterval(run.startedAt) / 1000)
                    Text("At \(at.formatted(date: .omitted, time: .shortened)) it filed \(run.threadCount) conversation\(run.threadCount == 1 ? "" : "s").")
                        .font(TypeRole.workflowContent)
                    if let report = run.reportText?.split(separator: "\n").first {
                        Text(String(report)).font(TypeRole.workflowContext).foregroundStyle(.secondary)
                    }
                }
                Text("Keep what it filed, or undo the run to put it all back.")
                    .font(TypeRole.workflowContent).foregroundStyle(.secondary)
            case nil:
                Text("This is no longer waiting.").font(TypeRole.workflowContent).foregroundStyle(.secondary)
            }
        }
    }

    /// "482 913".
    static func spaced(_ code: String) -> String {
        guard code.count == 6 else { return code }
        return "\(code.prefix(3)) \(code.suffix(3))"
    }
}

/// Weekly reading's report (spec §14.13): what the routine sorted this
/// week, what is due today and how long it takes, a card per due bucket
/// with its verb, and the routine's settings and this morning's run.
struct ReadingReportView: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    @State private var morningRun: RoutineRunInfo?

    var body: some View {
        let report = session.reading
        let due = report?.buckets.filter { $0.dueToday && !$0.threadIds.isEmpty } ?? []
        let count = due.reduce(0) { $0 + $1.threadIds.count }
        ScrollView {
            VStack(alignment: .leading, spacing: Space.xxxl) {
                VStack(alignment: .leading, spacing: Space.s) {
                    Text("\(report?.sortedThisWeek ?? 0) sorted this week").font(TypeRole.workflowHeadline)
                    Text(count == 0 ? "Nothing is due today."
                                    : "\(count) due today, about \(max(1, (count * 20 + 30) / 60)) min.")
                        .font(TypeRole.workflowContent).foregroundStyle(.secondary)
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: Self.cardWidth), spacing: Space.xl)], spacing: Space.xl) {
                    ForEach(due, id: \.id) { bucket in card(bucket) }
                }
                HStack(spacing: Space.l) {
                    Button("Routine settings…") { model.openRoutines?() }
                        .hoverHelp("Change the sorting routine: its buckets, their days and what it leaves alone")
                    if let run = morningRun {
                        Button("Undo this morning's run") { Task { await undoRun(run) } }
                            .hoverHelp("Put back what the routine sorted this morning")
                    }
                }
                .controlSize(.large)
            }
            .padding(Space.page)
            .frame(maxWidth: WorkflowHostView.readingWidth + Space.page * 2, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .task { morningRun = await model.morningRoutineRun() }
    }

    private func card(_ bucket: ReadingBucket) -> some View {
        VStack(alignment: .leading, spacing: Space.s) {
            Text(bucket.title).font(TypeRole.workflowHeading)
            Text("\(bucket.threadIds.count) · \(bucket.verb)").font(TypeRole.workflowContext).foregroundStyle(.secondary)
            ForEach(Array(bucket.threadIds.prefix(3).enumerated()), id: \.element) { i, id in
                // Kaluta's line once it is written, the subject until then.
                Text(session.summaries[id]?.line ?? (i < bucket.subjects.count ? bucket.subjects[i] : ""))
                    .font(TypeRole.workflowKey).lineLimit(1)
            }
        }
        .padding(Space.xxl)
        .frame(maxWidth: .infinity, minHeight: Self.cardHeight, alignment: .topLeading)
        .background(Tone.workflowSurface, in: RoundedRectangle(cornerRadius: Radius.panel))
        .accessibilityElement(children: .combine)
    }

    private func undoRun(_ run: RoutineRunInfo) async {
        guard let core = model.core, (try? await core.undoRoutineRun(run.runId)) != nil else { return }
        morningRun = nil
        await model.loadReading(session)
    }

    static let cardWidth: CGFloat = 260
    static let cardHeight: CGFloat = 130
}

/// Who a conversation waits on, for how long, and why Kaluta thinks an
/// answer is due (spec §14.13).
struct WaitingLine: View {
    let wait: WaitingInfo
    let kind: WorkflowKind

    var body: some View {
        HStack(spacing: Space.m) {
            Text("Waiting on \(wait.who.isEmpty ? "them" : wait.who) · \(Self.waited(wait))")
                .font(TypeRole.workflowKey).fontWeight(.semibold)
                .padding(.horizontal, Space.m).padding(.vertical, Space.hair)
                .background(Tone.workflow(kind).opacity(0.14), in: Capsule())
            if !wait.reason.isEmpty {
                Label(wait.reason, systemImage: "sparkle")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// "9 days", "1 day", "today".
    static func waited(_ wait: WaitingInfo, now: Date = .now) -> String {
        let since = Date(timeIntervalSince1970: TimeInterval(wait.since) / 1000)
        let days = Calendar.current.dateComponents([.day], from: Calendar.current.startOfDay(for: since),
                                                   to: Calendar.current.startOfDay(for: now)).day ?? 0
        return days <= 0 ? "today" : "\(days) day\(days == 1 ? "" : "s")"
    }
}

/// Kaluta's suggestion for a triage item (spec §14.13): what to do, the
/// time a quick reply takes, and why; and the conversation's task, if it
/// has one.
struct TriageSuggestionLine: View {
    let suggestion: TriageSuggestion?
    let task: TaskItem?
    let kind: WorkflowKind

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            if let suggestion {
                HStack(spacing: Space.m) {
                    Text(Self.verb(suggestion))
                        .font(TypeRole.workflowKey).fontWeight(.semibold)
                        .padding(.horizontal, Space.m).padding(.vertical, Space.hair)
                        .background(Tone.workflow(kind).opacity(0.14), in: Capsule())
                    if !suggestion.reason.isEmpty {
                        Label(suggestion.reason, systemImage: "sparkle")
                            .font(TypeRole.workflowContext)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Kaluta suggests: \(Self.verb(suggestion)). \(suggestion.reason)")
            }
            if let task {
                Label("Has a task: “\(task.title)”", systemImage: "checklist")
                    .font(TypeRole.workflowContext)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// "Quick reply · about 20 seconds", "Make a task", "Archive", "Leave in Inbox".
    static func verb(_ s: TriageSuggestion) -> String {
        switch s.kind {
        case .quickReply:
            guard let seconds = s.seconds else { return "Quick reply" }
            return "Quick reply · about \(max(5, (Int(seconds) + 4) / 5 * 5)) seconds"
        case .task: return "Make a task"
        case .archive: return "Archive"
        case .leave: return "Leave in Inbox"
        }
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
                            ScoreTile(figure: "\(c.count)", label: Self.words(c.decision, count: Int(c.count), kind: session.kind))
                        }
                    }
                }
                if let next = model.nextWorkflow(after: session.kind) {
                    WorkflowButton(title: model.nextTitle(next, after: session.kind), key: "Return", style: .primary,
                                   kind: next) {
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
        case .waiting: return "Nobody is left hanging."
        case .reading: return "The reading is done."
        default: return "All done."
        }
    }

    static func subline(_ score: WorkflowScore?) -> String? {
        guard let score, score.decided > 0 else { return nil }
        let minutes = max(1, Int((score.millis + 30_000) / 60_000))
        return "\(score.decided) decided in about \(minutes) minute\(minutes == 1 ? "" : "s")."
    }

    static func words(_ decision: String, count: Int, kind: WorkflowKind = .tasks) -> String {
        switch decision {
        case "archive": "archived"
        case "reply": count == 1 ? "reply" : "replies"
        case "elsewhere": "done elsewhere"
        case "nudged": "nudged"
        case "let_go": "let go"
        case "answered": "answered meanwhile"
        case "read": "read"
        case "sent": "sent"
        case "declined": "not sent"
        case "drafted": "kept in Drafts"
        case "approved": "approved"
        case "copied": "codes copied"
        case "undone": "runs undone"
        case "kept": kind == .agents ? "runs kept" : "kept for later"
        case "inbox": "back to the Inbox"
        case "task": count == 1 ? "task made" : "tasks made"
        case "leave": "left in Inbox"
        case "done": "done"
        case "later": kind == .waiting ? "to check again" : "moved to tomorrow"
        default: decision
        }
    }

    static let width: CGFloat = 880
}

extension AppModel {
    /// This morning's sorting run, if one ran today and was not undone.
    func morningRoutineRun() async -> RoutineRunInfo? {
        guard let core, let routines = try? await core.routines() else { return nil }
        let start = Calendar.current.startOfDay(for: .now).timeIntervalSince1970 * 1000
        for r in routines {
            let runs = (try? await core.routineRuns(r.id, limit: 5)) ?? []
            if let run = runs.first(where: { Double($0.startedAt) >= start && $0.status == "succeeded" }) { return run }
        }
        return nil
    }

    /// The header's one pill: Tasks says what waits for later this week.
    func workflowPill(_ session: WorkflowSession) -> String? {
        if session.kind == .agents {
            // What agent mailboxes sent on their own since the last look.
            let n = session.agentsWork?.sentOnTheirOwn.reduce(0) { $0 + Int($1.count) } ?? 0
            return n == 0 ? nil : "Agent mailboxes sent \(n) on their own"
        }
        guard session.kind == .tasks,
              let later = workflowOverview?.workflows.first(where: { $0.kind == .tasks })?.later, later > 0 else { return nil }
        return "\(later) more later this week"
    }

    /// The workflow after `kind` on the start screen that has anything in
    /// it; after Triage, the mail that arrived meanwhile comes first (spec
    /// §14.13, arrivals wait for the end).
    func nextWorkflow(after kind: WorkflowKind) -> WorkflowKind? {
        guard let overview = workflowOverview else { return nil }
        if kind == .triage, arrivals(overview) > 0 { return .triage }
        let order = WorkflowKind.all
        let start = (order.firstIndex(of: kind) ?? 0) + 1
        return order[start...].first { k in overview.workflows.first { $0.kind == k }.map { $0.count > 0 } ?? false }
    }

    private func arrivals(_ overview: WorkflowOverview) -> Int {
        Int(overview.workflows.first { $0.kind == .triage }?.count ?? 0)
    }

    /// The end's Next button: "Next: Replies", or "Triage 3 that arrived".
    func nextTitle(_ next: WorkflowKind, after kind: WorkflowKind) -> String {
        guard next == kind, let overview = workflowOverview else { return "Next: \(next.title)" }
        return "Triage \(arrivals(overview)) that arrived"
    }

    /// Return at an end: the next workflow, else the start screen.
    func continueAfter(_ session: WorkflowSession) {
        let next = nextWorkflow(after: session.kind)
        workflowSession = nil
        if let next { Task { await beginWorkflow(next) } } else { Task { await refreshWorkflowOverview() } }
    }
}
