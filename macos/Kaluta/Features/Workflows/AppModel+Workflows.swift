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
            await enterTeach()
            return

        default:
            break
        }
        guard let core else { return }
        do {
            let set = try await core.startWorkflow(kind)
            let session = WorkflowSession(set: set)
            // Tasks shows them; Triage says which conversations have one.
            if kind == .tasks || kind == .triage { await tasks.load() }
            // The card grows into the window (spec §14.13); the first
            // item loads behind it.
            if kind == .tasks { await loadTaskInfo(session) }
            if kind == .waiting { await loadWaits(session) }
            if kind == .agents { await loadAgentsWork(session) }
            if kind == .reading {
                await loadReading(session)
                // The report first, unless the set was already begun today.
                session.showsReport = session.decided == 0
            }
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
        if workflowSession?.kind == .agents { Task { try? await core?.markAgentsLooked() } }
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
        if session.kind == .triage, session.suggestions[threadID] == nil,
           let suggestion = try? await core.triageSuggestion(threadID) {
            session.suggestions[threadID] = suggestion
        }
        if session.threads[threadID] == nil, let detail = try? await core.thread(threadID) {
            session.threads[threadID] = detail
            if let latest = Self.shownMessage(detail, session.kind), let body = try? await core.renderedBody(latest.id),
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
        item.taskId.flatMap { id in tasks.tasks.first { $0.id == id } ?? workflowSession?.taskInfo[id] }
    }

    /// The set's tasks, finished ones too, for the rail.
    func loadTaskInfo(_ session: WorkflowSession) async {
        guard let core, let all = try? await core.listTasks(includeDone: true) else { return }
        let ids = Set(session.items.compactMap(\.taskId))
        session.taskInfo = Dictionary(all.filter { ids.contains($0.id) }.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
    }

    /// The message an item shows: in Waiting on others the user's own (what
    /// they asked), elsewhere the latest from someone else.
    static func shownMessage(_ detail: ThreadDetail, _ kind: WorkflowKind) -> MessageInfo? {
        kind == .waiting ? detail.messages.last { !$0.isDraft } : latestReceived(detail)
    }

    /// The Agents set's codes, and the text of each proposed send.
    func loadAgentsWork(_ session: WorkflowSession) async {
        guard let core else { return }
        session.agentsWork = try? await core.agentsWork()
        for item in session.items where item.key.hasPrefix("action:") && session.agentDrafts[item.key] == nil {
            guard let id = Int64(item.key.dropFirst("action:".count)),
                  let info = agent.proposalInfo(id), let draftID = info.draftID,
                  let draft = try? await core.draft(draftID) else { continue }
            let (reply, quote) = Self.splitQuote(ComposerAssistant.plainText(fromHTML: draft.bodyHtml))
            session.agentDrafts[item.key] = reply
            session.agentDraftReplies[item.key] = reply
            session.agentDraftQuotes[item.key] = quote
        }
        for item in session.items where item.key.hasPrefix("run:") {
            let parts = item.key.dropFirst("run:".count).split(separator: "/")
            guard parts.count == 2, let run = Int64(parts[1]),
                  let runs = try? await core.routineRuns(String(parts[0]), limit: 5) else { continue }
            session.agentRuns[item.key] = runs.first { $0.runId == run }
        }
    }

    /// A draft's text split at the quoted original ("On … wrote:", or the
    /// first "> " line): the reply above to edit, the quote below to keep.
    static func splitQuote(_ text: String) -> (reply: String, quote: String) {
        let lines = text.components(separatedBy: .newlines)
        let at = lines.firstIndex { line in
            let t = line.trimmingCharacters(in: .whitespaces)
            return (t.hasPrefix("On ") && t.hasSuffix("wrote:")) || t.hasPrefix(">")
        }
        guard let at else { return (text.trimmingCharacters(in: .whitespacesAndNewlines), "") }
        return (lines[..<at].joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines),
                lines[at...].joined(separator: "\n"))
    }

    /// What an Agents item is.
    enum AgentItem {
        case action(id: Int64, tool: String, summary: String, draftID: Int64?, request: String?)
        /// A proposal another account's panel holds.
        case elsewhere
        case code(AgentCode)
        case run(routine: String, run: Int64)
    }

    func agentItem(_ item: WorkflowItem, in session: WorkflowSession) -> AgentItem? {
        let key = item.key
        if key.hasPrefix("action:"), let id = Int64(key.dropFirst("action:".count)) {
            guard let info = agent.proposalInfo(id) else { return .elsewhere }
            return .action(id: id, tool: info.tool, summary: info.summary, draftID: info.draftID, request: info.request)
        }
        if key.hasPrefix("code:") {
            let rest = key.dropFirst("code:".count)
            return session.agentsWork?.codes.first { "\($0.accountId)/\($0.messageId)" == rest }.map { .code($0) }
        }
        if key.hasPrefix("run:") {
            let parts = key.dropFirst("run:".count).split(separator: "/")
            if parts.count == 2, let run = Int64(parts[1]) { return .run(routine: String(parts[0]), run: run) }
        }
        return nil
    }

    /// Weekly reading's report and Kaluta's summaries of the set.
    func loadReading(_ session: WorkflowSession) async {
        guard let core else { return }
        session.reading = try? await core.readingReport()
        let ids = session.items.compactMap(\.threadId)
        for s in (try? await core.readingSummaries(ids)) ?? [] { session.summaries[s.threadId] = s }
    }

    /// Every wait in a Waiting set, for the rail and the items.
    func loadWaits(_ session: WorkflowSession) async {
        guard let core else { return }
        for id in session.items.compactMap(\.threadId) {
            if let info = try? await core.waitingInfo(id) { session.waits[id] = info }
        }
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
        if session.kind == .reading, session.showsReport {
            return [a(.primary, .primary, "Start reading")]
        }
        if session.choosingCheckAgain {
            return [a(.pick1, .plain, "Tomorrow"), a(.pick2, .plain, "In 3 days"), a(.pick3, .plain, "Next week"),
                    a(.back, .quiet, "Cancel")]
        }
        if let form = session.taskForm {
            return [a(.primary, .primary, form.isExisting ? "Save the task" : "Make the task"), a(.back, .quiet, "Cancel")]
        }
        switch kind {
        case .triage:
            return triageActions(session, a)
        case .replies:
            return [a(.primary, .primary, "Reply"), a(.task), a(.done), a(.skip, .quiet)]
        case .tasks:
            let replies: Bool = {
                guard let item = session.current, let task = task(of: item) else { return false }
                return task.action != .noEmail
            }()
            return (replies ? [a(.primary, .primary, "Reply")] : []) + [a(.done, replies ? .plain : .primary), a(.later),
                                                                         a(.skip, .quiet)]
        case .waiting:
            return [a(.primary, .primary, "Nudge"), a(.done), a(.later, .plain, "Check again"), a(.skip, .quiet)]
        case .agents:
            switch session.current.flatMap({ agentItem($0, in: session) }) {
            case let .action(_, tool, _, draftID, _) where AgentStore.isSend(tool) && draftID != nil:
                return [a(.send, .primary, "Send"), a(.reject, .plain, "Don't send"), a(.later, .plain, "Save as a draft"),
                        a(.skip, .quiet)]
            case .action:
                return [a(.primary, .primary, "Approve"), a(.reject, .plain, "Don't"), a(.skip, .quiet)]
            case .code:
                return [a(.copyCode, .primary), a(.done, .plain, "Done"), a(.skip, .quiet)]
            case .run:
                return [a(.done, .primary, "Keep"), a(.reject, .plain, "Undo the run"), a(.skip, .quiet)]
            case .elsewhere, nil:
                return [a(.done, .primary, "Done"), a(.skip, .quiet)]
            }
        case .reading:
            // T and S answer their keys without a button: the bar fits.
            var task = a(.task)
            task.shown = false
            var skip = a(.skip, .quiet)
            skip.shown = false
            return [a(.done, .primary), a(.primary, .plain, session.showsInFull ? "Show the summary" : "Open in full"),
                    a(.later), a(.backToInbox), a(.clearBucket, .quiet), task, skip]
        default:
            return [a(.done, .primary), a(.skip, .quiet)]
        }
    }

    /// Triage (spec §14.13): Return takes Kaluta's suggestion; R, T, E, L
    /// and S are always there (the one Return already takes is not shown
    /// twice, its key still works).
    private func triageActions(_ session: WorkflowSession,
                               _ a: (WorkflowCommand, WorkflowButton.Style, String?) -> WorkflowAction) -> [WorkflowAction] {
        let suggested = session.current.flatMap(threadID(of:)).flatMap { session.suggestions[$0] }
        let taken: WorkflowCommand? = switch suggested?.kind {
        case .quickReply: .replyNow
        case .task: .task
        case .archive: .done
        case .leave: .later
        case nil: nil
        }
        let primary = taken.map { a(.primary, .primary, $0 == .replyNow ? "Reply" : $0.title(in: .triage)) }
        let rest = [a(.replyNow, .plain, "Reply"), a(.task, .plain, nil), a(.done, .plain, nil), a(.later, .plain, nil)]
            .map { action -> WorkflowAction in
                var action = action
                if action.command == taken { action.shown = false }
                // With no suggestion, Reply leads.
                if taken == nil, action.command == .replyNow { action.style = .primary }
                return action
            }
        return (primary.map { [$0] } ?? []) + rest + [a(.skip, .quiet, nil)]
    }

    /// While Kaluta writes the item's answer: the step's own actions.
    private func writingActions(_ writer: WorkflowWriter,
                                _ a: (WorkflowCommand, WorkflowButton.Style, String?) -> WorkflowAction) -> [WorkflowAction] {
        switch writer.step {
        case .choose:
            let picks: [WorkflowCommand] = [.pick1, .pick2, .pick3]
            return picks.prefix(writer.answers.count).map { var x = a($0, .plain, nil); x.shown = false; return x }
                // The answers' own row says "None of these? Tell Kaluta…".
                + [{ var x = a(.tellKaluta, .plain, nil); x.shown = false; return x }(), a(.back, .quiet, "Cancel")]
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
            if session.kind == .waiting {
                // The nudge went: the wait starts again.
                try? await core?.restartWaiting(writer.threadID)
                await decide(item, "nudged", session: session, notice: nil)
                return
            }
            let finished = writer.taskID != nil && !keep
            // Replies: send and archive. Tasks: a finished task's email goes
            // out of the Inbox with it (spec §14.13).
            let archives = session.kind == .replies || (session.kind == .tasks && finished)
            let token = archives ? try? await core?.archive([writer.threadID]) : nil
            await decide(item, finished ? "done" : "reply", session: session, notice: nil,
                         undoMail: { [core] in if let token { _ = try? await core?.undo(token) } },
                         redoMail: { [core] in if let token { _ = try? await core?.redo(token) } })
        default:
            break
        }
    }

    /// One implementation per command, whatever called it (spec §14.13).
    func performWorkflow(_ command: WorkflowCommand) async {
        guard let session = workflowSession else { return }
        switch command {
        case .back where session.writer == nil && session.taskForm == nil && !session.choosingCheckAgain:
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
        if session.kind == .reading, session.showsReport {
            withAnimation(Self.itemMotion) { session.showsReport = false }
            return
        }
        if session.choosingCheckAgain {
            session.choosingCheckAgain = false
            let days: Int? = switch command {
            case .pick1: 1
            case .pick2: 3
            case .pick3: 7
            default: nil
            }
            if let days { await checkAgain(item, inDays: days, session: session) }
            return
        }
        if let form = session.taskForm {
            if command == .back { session.taskForm = nil } else { await makeTask(form, item: item, session: session) }
            return
        }
        switch (session.kind, command) {
        case (.triage, .primary):
            // Kaluta's suggestion, as its own key would.
            let suggested = threadID(of: item).flatMap { session.suggestions[$0] }
            switch suggested?.kind {
            case .quickReply: startWriting(item, session: session)
            case .task: await openTaskForm(item, session: session)
            case .archive: await archiveCurrent(item, session: session)
            case .leave: await decide(item, "leave", session: session, notice: "Left in the Inbox")
            case nil: startWriting(item, session: session)
            }
        case (.triage, .task):
            await openTaskForm(item, session: session)
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
        case (.replies, .task):
            if let id = threadID(of: item), let row = session.threads[id]?.thread {
                await openTaskDialog(for: row)
            }
        case (.tasks, .done):
            await finishTask(item, session: session)
        case (.tasks, .later):
            await moveTaskToTomorrow(item, session: session)
        case (.agents, _):
            await performAgents(command, item: item, session: session)
        case (.reading, .primary):
            session.showsInFull.toggle()
        case (.reading, .done):
            await markRead([item], session: session, notice: nil)
        case (.reading, .later):
            await decide(item, "kept", session: session, notice: "Kept for later")
        case (.reading, .backToInbox):
            await sendBackToInbox(item, session: session)
        case (.reading, .task):
            await openTaskForm(item, session: session)
        case (.reading, .clearBucket):
            let bucket = session.bucket(of: item)
            let rest = session.items.filter { $0.decision == nil && session.bucket(of: $0)?.id == bucket?.id }
            await markRead(rest, session: session, notice: "Cleared \(bucket?.title ?? "the bucket")")
        case (.waiting, .primary):
            startWriting(item, session: session)
        case (.waiting, .done):
            await letGo(item, session: session)
        case (.waiting, .later):
            session.choosingCheckAgain = true
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
        if session.kind == .tasks { await loadTaskInfo(session) }
        if session.kind == .waiting { await loadWaits(session) }
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
              let message = Self.shownMessage(detail, session.kind) else { return }
        // The item's task, else the conversation's: Send offers to finish it.
        let task = session.kind == .waiting ? nil : self.task(of: item) ?? openTask(onThread: id)
        // Kaluta's answers: a quick reply's (Triage), or the follow-ups
        // written ahead (Waiting).
        let written = session.kind == .waiting ? session.waits[id]?.nudges : session.suggestions[id]?.answers
        let answers = written?.map { WorkflowWriter.Answer(label: $0.label, text: $0.text) } ?? []
        session.writer = WorkflowWriter(threadID: id, messageID: task?.messageId ?? message.id,
                                        replyAll: task?.action == .replyAll, taskID: task?.id,
                                        email: session.bodies[id] ?? message.snippet, answers: answers)
        if !agent.isProviderReady { Task { await agent.loadProviders() } }
    }

    /// An Agents item's command (spec §14.13).
    private func performAgents(_ command: WorkflowCommand, item: WorkflowItem, session: WorkflowSession) async {
        guard let core else { return }
        switch (agentItem(item, in: session), command) {
        case let (.action(id, _, _, draftID?, _), .send):
            // The email as edited here is what goes (spec §10.4).
            // Only an edited reply is saved; the quote below it goes as it was.
            if let text = session.agentDrafts[item.key], text != session.agentDraftReplies[item.key],
               var draft = try? await core.draft(draftID) {
                let quote = session.agentDraftQuotes[item.key] ?? ""
                let whole = quote.isEmpty ? text : text + "\n\n" + quote
                draft.bodyHtml = ComposerHTML.html(from: NSAttributedString(string: whole,
                                                                             attributes: [.font: ComposerHTML.bodyFont]))
                _ = try? await core.saveDraft(draft)
            }
            agent.resolve(id, approve: true)
            await decide(item, "sent", session: session, notice: nil)
        case let (.action(id, _, _, _, _), .primary):
            agent.resolve(id, approve: true)
            await decide(item, "approved", session: session, notice: nil)
        case let (.action(id, _, _, _, _), .reject):
            agent.resolve(id, approve: false)
            await decide(item, "declined", session: session, notice: "Not sent")
        case let (.action(id, _, _, _, _), .later):
            // Declined, the draft stays in Drafts.
            agent.resolve(id, approve: false)
            await decide(item, "drafted", session: session, notice: "Kept in Drafts")
        case let (.code(code), .copyCode):
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(code.code, forType: .string)
            await decide(item, "copied", session: session, notice: "Copied \(code.code)")
        case let (.run(_, run), .reject):
            guard (try? await core.undoRoutineRun(run)) != nil else { return }
            await decide(item, "undone", session: session, notice: "This morning's run is undone")
        case (.run, .done):
            await decide(item, "kept", session: session, notice: nil)
        default:
            await decide(item, "done", session: session, notice: nil)
        }
    }

    /// Read, next (E) and Clear the bucket (⌘E): the mail is marked read
    /// and keeps the routine's label (spec §14.13); ⌘Z undoes all of it.
    private func markRead(_ items: [WorkflowItem], session: WorkflowSession, notice: String?) async {
        guard let core, let accountID = openAccountID, let first = items.first else { return }
        let ids = items.compactMap(\.threadId)
        let token = try? await core.setRead(ids, true)
        let setID = session.workflowSet.id
        let millis = session.elapsedMillis
        session.exit = .left
        for item in items {
            try? await core.decideWorkflowItem(set: setID, key: item.key, decision: "read",
                                               millis: item.key == first.key ? millis : 0)
        }
        undo.record(accountID: accountID, actionName: notice ?? "Read", noticeText: notice ?? "",
                    showNotice: notice != nil,
                    undo: { [weak self] in
                        if let token { _ = try? await core.undo(token) }
                        for item in items { _ = try? await core.undoWorkflowDecision(set: setID, key: item.key) }
                        await self?.reloadWorkflow(setID)
                    },
                    redo: { [weak self] in
                        if let token { _ = try? await core.redo(token) }
                        for item in items {
                            try? await core.decideWorkflowItem(set: setID, key: item.key, decision: "read", millis: 0)
                        }
                        await self?.reloadWorkflow(setID)
                    })
        await reloadWorkflow(setID)
    }

    /// Back to the Inbox (I): out of the bucket, and Kaluta proposes leaving
    /// such mail alone, to accept in Teach Kaluta (spec §14.13).
    private func sendBackToInbox(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let id = threadID(of: item), let back = try? await core.backToInbox(id) else { return }
        let notice = back.proposalId == nil ? "Back in the Inbox"
            : "Back in the Inbox. Kaluta proposes a change to the routine in Teach Kaluta"
        await decide(item, "inbox", session: session, notice: notice,
                     undoMail: {
                         if let token = back.undo { _ = try? await core.undo(token) }
                         if let proposal = back.proposalId { try? await core.withdrawRoutineEvidence(proposal, threadID: id) }
                     },
                     redoMail: { if let token = back.undo { _ = try? await core.redo(token) } })
    }

    /// *Let it go* (E): the wait ends and Kaluta's label comes off; ⌘Z
    /// puts both back.
    private func letGo(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let id = threadID(of: item), (try? await core.letGoWaiting(id)) != nil else { return }
        let who = session.waits[id]?.who ?? ""
        await decide(item, "let_go", session: session, notice: who.isEmpty ? "Let it go" : "No longer waiting on \(who)",
                     undoMail: { try? await core.resumeWaiting(id) },
                     redoMail: { try? await core.letGoWaiting(id) })
    }

    /// *Check again* (L): hidden until the day chosen.
    private func checkAgain(_ item: WorkflowItem, inDays days: Int, session: WorkflowSession) async {
        guard let core, let id = threadID(of: item),
              let day = Calendar.current.date(byAdding: .day, value: days, to: Calendar.current.startOfDay(for: .now)),
              (try? await core.checkWaitingAgain(id, until: day)) != nil else { return }
        let when = days == 1 ? "tomorrow" : day.formatted(.dateTime.weekday(.wide).day().month(.abbreviated))
        await decide(item, "later", session: session, notice: "Check again \(when)",
                     undoMail: { try? await core.checkWaitingAgain(id, until: nil) },
                     redoMail: { try? await core.checkWaitingAgain(id, until: day) })
    }

    /// The conversation's open task, if it has one.
    func openTask(onThread threadID: String) -> TaskItem? {
        tasks.tasks.first { $0.threadId == threadID && !$0.done }
    }

    /// T in Triage: the task on the item's screen, filled in from Kaluta's
    /// suggestion; a conversation that has one shows it rather than making
    /// a second (spec §14.13).
    private func openTaskForm(_ item: WorkflowItem, session: WorkflowSession) async {
        guard let core, let id = threadID(of: item), let detail = session.threads[id] else { return }
        let categories = (try? await core.taskCategories()) ?? []
        if let existing = openTask(onThread: id) {
            session.taskForm = WorkflowTaskForm(draft: TaskDraft(editing: existing, categories: categories))
            return
        }
        let draft = TaskDraft(threadID: id, subject: detail.thread.subject.isEmpty ? "(no subject)" : detail.thread.subject,
                              sender: Self.latestReceived(detail)?.from.map { $0.name ?? $0.email } ?? "",
                              categories: categories)
        session.taskForm = WorkflowTaskForm(draft: draft)
        if let suggested = session.suggestions[id]?.task {
            draft.apply(suggested)
        } else if TaskSuggester.canAsk(self) {
            await draft.suggester.run(threadIDs: [id], model: self)
        }
    }

    /// Return on the task: make it (or save the conversation's), archive
    /// the email if ticked, and move on; ⌘Z takes all of it back.
    private func makeTask(_ form: WorkflowTaskForm, item: WorkflowItem, session: WorkflowSession) async {
        let draft = form.draft
        guard draft.canAdd, let core else { return }
        var made: [TaskItem] = []
        if let task = draft.editing {
            if let error = await update(task, to: draft.edit, actionName: "Edit Task", notice: "Changed “\(draft.title)”") {
                draft.error = error
                return
            }
        } else {
            do {
                made = try await core.createTasks([draft.newTask])
            } catch {
                draft.error = error.message
                return
            }
        }
        draft.suggester.cancel()
        await tasks.load()
        let token = form.archiveEmail ? try? await core.archive([draft.threadID]) : nil
        let title = draft.title.trimmingCharacters(in: .whitespacesAndNewlines)
        await decide(item, "task", session: session, notice: made.isEmpty ? "Saved “\(title)”" : "Task: “\(title)”",
                     undoMail: {
                         if let token { _ = try? await core.undo(token) }
                         for t in made { _ = try? await core.deleteTask(t.id) }
                     },
                     redoMail: {
                         for t in made { _ = try? await core.restoreTask(t) }
                         if let token { _ = try? await core.redo(token) }
                     })
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
