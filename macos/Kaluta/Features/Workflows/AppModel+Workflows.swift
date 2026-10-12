import os
import SwiftUI

private let workflowLog = Logger(subsystem: "org.kaluta.Kaluta", category: "workflows")

/// Workflows (spec §14.13): the start screen as home, and a workflow over
/// the whole window. Until each workflow's own screens land (plan
/// `docs/plans/workflows.md`), an item shows its email or task and the
/// commands act with what the app already does: archive, reply, make a
/// task, finish a task.
extension AppModel {
    /// Workflows are a preview until Triage lands: on in the workflow
    /// demo, else by Settings › General.
    static let workflowsEnabledKey = "workflowsEnabled"
    /// Which of start screen and mail showed last, and on which day.
    static let workflowHomeKey = "workflowHome"

    var workflowsEnabled: Bool { CoreClient.demoWorkflows || defaults.bool(forKey: Self.workflowsEnabledKey) }

    /// The window shows the start screen or a workflow, not the mail view.
    var showsWorkflows: Bool { workflowsEnabled && home == .start }

    /// At launch: the start screen, unless the user chose Mail earlier
    /// today (spec §14.13: remembered for the rest of the day).
    func restoreHome(now: Date = .now) {
        // Kaluta's background jobs run only while workflows are on.
        core?.setWorkflowJobsOn(workflowsEnabled)
        let saved = defaults.string(forKey: Self.workflowHomeKey)?.split(separator: "@").map(String.init) ?? []
        if saved.count == 2, saved[1] == DueDay.string(now), let home = WorkflowHome(rawValue: saved[0]) {
            self.home = home
        } else {
            home = .start
        }
    }

    private func rememberHome() {
        defaults.set("\(home.rawValue)@\(DueDay.string(.now))", forKey: Self.workflowHomeKey)
    }

    /// Settings › General › Workflows changed.
    func workflowsSettingChanged() {
        core?.setWorkflowJobsOn(workflowsEnabled)
    }

    /// M, View › Mail: the mail view; any workflow stays where it was.
    func showMail() {
        home = .mail
        rememberHome()
    }

    /// View › What's Next, and back from a workflow's end.
    func showStart() {
        home = .start
        rememberHome()
        Task { await refreshWorkflowOverview() }
    }

    func refreshWorkflowOverview() async {
        guard let core else { return }
        workflowOverview = try? await core.workflowOverview()
    }

    /// Start (or resume) a workflow. Teach Kaluta is Review mode for now
    /// (oagc-k6o8.19); Agents is the agent column (oagc-k6o8.20).
    func beginWorkflow(_ kind: WorkflowKind) async {
        switch kind {
        case .teach:
            await analysis.load()
            await guide.load()
            enterReview(.rules)
            return
        case .agents:
            showMail()
            agent.isPresented = true
            return
        default:
            break
        }
        guard let core else { return }
        do {
            let set = try await core.startWorkflow(kind)
            let session = WorkflowSession(set: set)
            if kind == .tasks { await tasks.load() }
            // The card grows into the window (spec §14.13); the first
            // item loads behind it.
            withAnimation(Self.workflowMotion) { workflowSession = session }
            await prepareCurrentItem()
            if session.current == nil { await finishWorkflow() }
        } catch {
            workflowLog.error("workflow: \(error.message, privacy: .public)")
        }
    }

    /// Start → workflow and back (about 350 ms); Reduce Motion cross-fades.
    static var workflowMotion: Animation {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion ? .easeInOut(duration: 0.2) : .smooth(duration: 0.35)
    }

    /// Item → item (about 200 ms).
    static let itemMotion = Animation.snappy(duration: 0.2)

    /// Back to the start screen; the set stays open to resume today.
    func leaveWorkflow() {
        withAnimation(Self.workflowMotion) { workflowSession = nil }
        Task { await refreshWorkflowOverview() }
    }

    /// Load what the current item shows: its conversation and the text
    /// of the latest message from someone else.
    func prepareCurrentItem() async {
        guard let session = workflowSession, let item = session.current else { return }
        await prepare(item, in: session)
    }

    private func prepare(_ item: WorkflowItem, in session: WorkflowSession) async {
        guard let core, let threadID = threadID(of: item) else { return }
        if session.threads[threadID] == nil, let detail = try? await core.thread(threadID) {
            session.threads[threadID] = detail
            if let latest = Self.latestReceived(detail), let body = try? await core.renderedBody(latest.id),
               let html = body.html {
                session.bodies[threadID] = Self.readingText(ComposerAssistant.plainText(fromHTML: html))
            }
        }
    }

    /// Plain text for reading at content size: paragraphs apart, the
    /// quoted history (from "On … wrote:") left out.
    static func readingText(_ text: String) -> String {
        var lines: [String] = []
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("On "), trimmed.hasSuffix("wrote:") { break }
            if trimmed.hasPrefix(">") { continue }
            lines.append(trimmed)
        }
        // A greeting line and a sign-off stay tight; paragraphs get air.
        let paragraphs = lines.split(separator: "", omittingEmptySubsequences: true).map { $0.joined(separator: "\n") }
        let joined = paragraphs.count > 1 ? paragraphs.joined(separator: "\n\n") : lines.filter { !$0.isEmpty }.joined(separator: "\n\n")
        return joined.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The conversation an item is about.
    func threadID(of item: WorkflowItem) -> String? {
        if let id = item.threadId { return id }
        if let task = item.taskId { return tasks.tasks.first { $0.id == task }?.threadId }
        return nil
    }

    func task(of item: WorkflowItem) -> TaskItem? {
        item.taskId.flatMap { id in tasks.tasks.first { $0.id == id } }
    }

    /// The conversation's latest message from someone else.
    static func latestReceived(_ detail: ThreadDetail) -> MessageInfo? {
        detail.messages.last { !$0.isSentByMe && !$0.isDraft } ?? detail.messages.last { !$0.isDraft }
    }

    // MARK: Commands

    /// What the current item offers, in the decision bar's order.
    func workflowActions() -> [WorkflowAction] {
        guard let session = workflowSession, session.current != nil else { return [] }
        let kind = session.kind
        func a(_ c: WorkflowCommand, _ style: WorkflowButton.Style = .plain, _ title: String? = nil) -> WorkflowAction {
            WorkflowAction(command: c, title: title ?? c.title(in: kind), style: style)
        }
        if let writer = session.writer { return writingActions(writer, a) }
        switch kind {
        case .triage:
            return [a(.replyNow, .primary, "Reply"), a(.task), a(.done), a(.later), a(.skip, .quiet)]
        case .replies:
            return [a(.primary, .primary, "Reply"), a(.task), a(.done), a(.skip, .quiet)]
        case .tasks:
            let replies: Bool = {
                guard let item = session.current, let task = task(of: item) else { return false }
                return task.action != .noEmail
            }()
            return (replies ? [a(.primary, .primary, "Reply")] : []) + [a(.done, replies ? .plain : .primary), a(.later),
                                                                         a(.skip, .quiet)]
        default:
            return [a(.done, .primary), a(.skip, .quiet)]
        }
    }

    /// While Kaluta writes the item's answer: the step's own actions.
    private func writingActions(_ writer: WorkflowWriter,
                                _ a: (WorkflowCommand, WorkflowButton.Style, String?) -> WorkflowAction) -> [WorkflowAction] {
        switch writer.step {
        case .choose:
            let picks: [WorkflowCommand] = [.pick1, .pick2, .pick3]
            return picks.prefix(writer.answers.count).map { var x = a($0, .plain, nil); x.shown = false; return x }
                + [a(.tellKaluta, .plain, nil), a(.back, .quiet, "Cancel")]
        case .say:
            return [a(.primary, .primary, agent.isProviderReady ? "Write it" : "Write it yourself"), a(.back, .quiet, "Cancel")]
        case .final:
            let send = writer.taskID == nil ? [a(.send, .primary, "Send")]
                : [a(.send, .primary, "Send and finish the task"), a(.sendKeepTask, .plain, nil)]
            return send + [a(.back, .quiet, "Change what I said")]
        case .writing, .sending:
            return []
        }
    }

    /// A writing step's command.
    private func performWriting(_ command: WorkflowCommand, _ writer: WorkflowWriter, item: WorkflowItem,
                                session: WorkflowSession) async {
        switch (writer.step, command) {
        case (.choose, .pick1): writer.pick(0)
        case (.choose, .pick2): writer.pick(1)
        case (.choose, .pick3): writer.pick(2)
        case (.choose, .tellKaluta): writer.tellKaluta()
        case (.choose, .back), (.say, .back):
            session.writer = nil
        case (.say, .primary):
            await writer.write(model: self)
        case (.final, .back):
            writer.changeWhatISaid()
        case (.final, .send), (.final, .sendKeepTask):
            let keep = command == .sendKeepTask
            guard await writer.send(model: self, keepTask: keep) else { return }
            await decide(item, writer.taskID != nil && !keep ? "done" : "reply", session: session, notice: nil)
        default:
            break
        }
    }

    /// One implementation per command, whatever called it (spec §14.13).
    func performWorkflow(_ command: WorkflowCommand) async {
        guard let session = workflowSession else { return }
        switch command {
        case .back:
            leaveWorkflow()
            return
        case .whatsLeft:
            announce(session.whatsLeft(secondsPerItem: secondsPerItem(session.kind)))
            return
        case .whereAmI:
            announce(whereAmI(session))
            return
        default:
            break
        }
        guard let item = session.current, workflowActions().contains(where: { $0.command == command }) else { return }
        if let writer = session.writer {
            await performWriting(command, writer, item: item, session: session)
            return
        }
        switch (session.kind, command) {
        case (_, .skip):
            session.exit = .up
            if let core, let set = try? await core.skipWorkflowItem(set: session.workflowSet.id, key: item.key) {
                await show(set, in: session)
            }
        case (.triage, .done), (.replies, .done):
            await archiveCurrent(item, session: session)
        case (.triage, .later):
            await decide(item, "leave", session: session, notice: "Left in the Inbox")
        case (.triage, .replyNow), (.replies, .primary), (.tasks, .primary):
            startWriting(item, session: session)
        case (.triage, .task), (.replies, .task):
            if let id = threadID(of: item), let row = session.threads[id]?.thread {
                await openTaskDialog(for: row)
            }
        case (.tasks, .done):
            await finishTask(item, session: session)
        case (.tasks, .later):
            await moveTaskToTomorrow(item, session: session)
        default:
            await decide(item, "done", session: session, notice: nil)
        }
    }

    /// A task made from the task dialog for the current conversation.
    func workflowTaskMade(threadID: String) async {
        guard let session = workflowSession, let item = session.current, self.threadID(of: item) == threadID else { return }
        await decide(item, "task", session: session, notice: nil)
    }

    // MARK: Deciding

    /// Record a decision and move on; ⌘Z undoes it (and `undoMail`, the
    /// mail change it made), putting the item back in the set.
    private func decide(_ item: WorkflowItem, _ decision: String, session: WorkflowSession, notice: String?,
                        undoMail: (@MainActor () async -> Void)? = nil, redoMail: (@MainActor () async -> Void)? = nil) async {
        guard let core, let accountID = openAccountID else { return }
        let setID = session.workflowSet.id
        let millis = session.elapsedMillis
        session.exit = .of(decision)
        do {
            try await core.decideWorkflowItem(set: setID, key: item.key, decision: decision, millis: millis)
        } catch {
            workflowLog.error("workflow decision: \(error.message, privacy: .public)")
            return
        }
        undo.record(accountID: accountID, actionName: notice ?? "Decision", noticeText: notice ?? "",
                    showNotice: notice != nil,
                    undo: { [weak self] in
                        await undoMail?()
                        _ = try? await core.undoWorkflowDecision(set: setID, key: item.key)
                        await self?.reloadWorkflow(setID)
                    },
                    redo: { [weak self] in
                        await redoMail?()
                        try? await core.decideWorkflowItem(set: setID, key: item.key, decision: decision, millis: millis)
                        await self?.reloadWorkflow(setID)
                    })
        await reloadWorkflow(setID)
    }

    /// The set as stored now; the end once nothing is left.
    private func reloadWorkflow(_ setID: Int64) async {
        guard let session = workflowSession, session.workflowSet.id == setID, let core,
              let set = try? await core.startWorkflow(session.kind), set.id == setID else { return }
        session.score = nil
        await show(set, in: session)
        if session.current == nil { await finishWorkflow() }
    }

    /// The set after a change: the next item's conversation loaded first,
    /// then the decided item leaves and the next rises (spec §14.13).
    private func show(_ set: WorkflowSet, in session: WorkflowSession) async {
        if let next = set.items.first(where: { $0.decision == nil }) { await prepare(next, in: session) }
        withAnimation(Self.itemMotion) { session.replace(set) }
    }

    private func finishWorkflow() async {
        guard let session = workflowSession, let core else { return }
        if session.total == 0 {
            session.score = WorkflowScore(kind: session.kind, decided: 0, items: 0, millis: 0, counts: [])
            return
        }
        let score = try? await core.finishWorkflow(set: session.workflowSet.id)
        withAnimation(Self.itemMotion) { session.score = score }
        await refreshWorkflowOverview()
    }

    private func archiveCurrent(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let id = threadID(of: item) else { return }
        let token = try? await core.archive([id])
        let subject = session.threads[id]?.thread.subject ?? ""
        await decide(item, "archive", session: session, notice: subject.isEmpty ? "Archived" : "Archived “\(subject)”",
                     undoMail: { if let token { _ = try? await core.undo(token) } },
                     redoMail: { if let token { _ = try? await core.redo(token) } })
    }

    /// Reply: *What do you want to say?* under the email (spec §14.13).
    private func startWriting(_ item: WorkflowItem, session: WorkflowSession) {
        guard let id = threadID(of: item), let detail = session.threads[id],
              let message = Self.latestReceived(detail) else { return }
        let task = self.task(of: item)
        session.writer = WorkflowWriter(threadID: id, messageID: task?.messageId ?? message.id,
                                        replyAll: task?.action == .replyAll, taskID: task?.id,
                                        email: session.bodies[id] ?? message.snippet)
        if !agent.isProviderReady { Task { await agent.loadProviders() } }
    }

    private func finishTask(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let task = task(of: item) else { return }
        guard (try? await core.setTaskDone(task.id, true)) != nil else { return }
        await tasks.load()
        await decide(item, "done", session: session, notice: "Done: “\(task.title)”",
                     undoMail: { _ = try? await core.setTaskDone(task.id, false) },
                     redoMail: { _ = try? await core.setTaskDone(task.id, true) })
    }

    private func moveTaskToTomorrow(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let task = task(of: item),
              let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: .now) else { return }
        let edit = TaskEdit(title: task.title, notes: task.notes, category: task.category,
                            dueDay: DueDay.string(tomorrow), action: task.action)
        let before = TaskEdit(title: task.title, notes: task.notes, category: task.category,
                              dueDay: task.dueDay, action: task.action)
        guard (try? await core.updateTask(task.id, edit)) != nil else { return }
        await tasks.load()
        await decide(item, "later", session: session, notice: "Moved to tomorrow: “\(task.title)”",
                     undoMail: { _ = try? await core.updateTask(task.id, before) },
                     redoMail: { _ = try? await core.updateTask(task.id, edit) })
    }

    // MARK: Spoken answers

    private func secondsPerItem(_ kind: WorkflowKind) -> Int {
        guard let summary = workflowOverview?.workflows.first(where: { $0.kind == kind }), summary.count > 0 else {
            return kind == .replies || kind == .tasks ? 120 : 30
        }
        return Int(summary.estimateSecs / summary.count)
    }

    private func whereAmI(_ session: WorkflowSession) -> String {
        guard let item = session.current else { return "\(session.kind.title), finished." }
        if let task = task(of: item) { return "\(session.kind.title), \(session.progress): \(task.title)." }
        if let id = threadID(of: item), let detail = session.threads[id] {
            let from = Self.latestReceived(detail)?.from.map { $0.name ?? $0.email } ?? ""
            return "\(session.kind.title), \(session.progress): \(detail.thread.subject), from \(from)."
        }
        return "\(session.kind.title), \(session.progress)."
    }

    private func announce(_ text: String) {
        AccessibilityNotification.Announcement(text).post()
        undo.show(text, accountID: openAccountID ?? "")
    }
}
