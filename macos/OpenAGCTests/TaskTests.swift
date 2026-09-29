import Foundation
import Testing
@testable import OpenAGC

@MainActor
struct TaskSuggesterTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        return model
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
    private struct Timeout: Error {}

    @Test func claudeSuggestsATaskForEachEmailAndChangesNothing() async throws {
        let model = try await demo()
        #expect(TaskSuggester.canAsk(model), "the fake agent is ready")
        let ids = Array(model.threads.rows.prefix(3).map(\.id))
        let inbox = model.threads.rows.map(\.id)
        let suggester = TaskSuggester()
        let now = try #require(DueDay.date("2026-09-28"))
        await suggester.run(threadIDs: ids, model: model, now: now)
        try await waitUntil { suggester.state != .working }
        let found = suggester.suggestions
        #expect(found.map(\.threadId) == ids)
        #expect(found[0].category == "Reply" && found[0].action == .reply)
        #expect(found[0].dueDay == "2026-09-28", "the prompt carried today's date")
        #expect(found[1].dueDay == nil && found[1].action == .noEmail)
        #expect(found[0].title.hasPrefix("Reply about "))
        #expect(model.agentSinks.isEmpty, "the session is closed and forgotten")
        #expect(model.agent.entries.isEmpty, "suggestions stay out of the agent panel")
        #expect(model.threads.rows.map(\.id) == inbox, "no mail changed")
        #expect(try await model.core!.listTasks().isEmpty, "nothing is a task until accepted")
    }

    @Test func cancellingWhileStartingLeavesNoSessionBehind() async throws {
        let model = try await demo()
        let ids = Array(model.threads.rows.prefix(2).map(\.id))
        let suggester = TaskSuggester()
        let running = Task { await suggester.run(threadIDs: ids, model: model) }
        await Task.yield()
        suggester.cancel()
        await running.value
        try await Task.sleep(for: .milliseconds(300))
        #expect(suggester.state == .idle)
        #expect(model.agentSinks.isEmpty)
    }

    @Test func aBulkRequestAsksAboutAtMostFiftyThreads() async throws {
        let model = try await demo()
        model.selectedThreadIDs = Set(model.threads.rows.map(\.id))
        #expect(model.bulkTaskTargets.count == min(AppModel.bulkTaskLimit, model.threads.rows.count))
    }

    @Test func acceptedTasksAreStoredLabelledAndAnnounced() async throws {
        let model = try await demo()
        let core = try #require(model.core)
        let row = try #require(model.threads.rows.first)
        let revision = model.tasksRevision
        let made = try await core.createTasks([
            NewTask(threadId: row.id, messageId: nil, title: "Send the figures", notes: "", category: "Reply",
                    dueDay: "2026-10-01", action: .reply, why: "They asked.", fromAi: true),
        ])
        #expect(made.count == 1 && made[0].subject == row.subject)
        #expect(try await core.threadsWithOpenTasks([row.id]) == [row.id])
        let label = try #require(try await core.taskLabelID())
        #expect(try await core.labels().contains { $0.id == label && $0.name == "Task" })
        try await waitUntil { model.tasksRevision > revision }

        let done = try await core.setTaskDone(made[0].id, true)
        #expect(done.done)
        #expect(try await core.threadsWithOpenTasks([row.id]).isEmpty)
        let deleted = try await core.deleteTask(made[0].id)
        #expect(try await core.listTasks(includeDone: true).isEmpty)
        _ = try await core.restoreTask(deleted)
        #expect(try await core.listTasks(includeDone: true).map(\.id) == [made[0].id])
        #expect(try await core.taskCategories().first == "Reply")
    }
}

@MainActor
struct TaskDialogTests {
    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
    private struct Timeout: Error {}

    @Test func claudesGuessFillsTheDialogAndAddingItCanBeUndone() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        let row = try #require(model.threads.rows.first)
        model.selectedThreadID = row.id
        let now = try #require(DueDay.date("2026-09-28"))

        await model.openTaskDialog(now: now)
        let draft = try #require(model.taskDraft)
        try await waitUntil { draft.suggester.state != .working }
        #expect(draft.title.hasPrefix("Reply about "))
        #expect(draft.category == "Reply" && draft.action == .reply)
        #expect(draft.hasDue && DueDay.string(draft.due) == "2026-09-28")
        #expect(draft.categories.count == 7)

        draft.title = "Send Emerson the plan"
        draft.category = "Follow Up"
        await model.acceptTask(draft)
        #expect(model.taskDraft == nil, "the dialog closes")
        let tasks = try await model.core!.listTasks()
        #expect(tasks.map(\.title) == ["Send Emerson the plan"])
        #expect(tasks[0].category == "Follow Up" && tasks[0].dueDay == "2026-09-28")
        #expect(tasks[0].fromAi == false, "the user rewrote Claude's title")
        #expect(model.undo.notice?.text == "Added a task: “Send Emerson the plan”")
        let account = try #require(model.openAccountID)
        #expect(model.undo.undoTitle(in: account) == "Undo Add Task")

        model.undo.undo(in: account)
        try await waitUntil { model.undo.canRedo(in: account) }
        for _ in 0..<50 where try await !model.core!.listTasks().isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await model.core!.listTasks().isEmpty)
        model.undo.redo(in: account)
        for _ in 0..<50 where try await model.core!.listTasks().isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await model.core!.listTasks().map(\.id) == tasks.map(\.id))
    }

    @Test func aTitleTypedWhileClaudeThinksIsKept() {
        let draft = TaskDraft(threadID: "t1", subject: "Lunch", sender: "Ann", categories: ["Reply", "Decide"])
        draft.title = "My own"
        draft.apply(TaskSuggestion(threadId: "t1", title: "Claude's", category: "Decide", dueDay: nil,
                                   action: .noEmail, why: "Because"))
        #expect(draft.title == "My own")
        #expect(draft.category == "Decide" && !draft.hasDue && draft.action == .noEmail)
        #expect(draft.newTask.fromAi == false && draft.newTask.why == "Because")
        let fresh = TaskDraft(threadID: "t1", subject: "Lunch", sender: "Ann", categories: ["Reply"])
        fresh.apply(TaskSuggestion(threadId: "t1", title: "Claude's", category: "Reply", dueDay: "2026-10-01",
                                   action: .reply, why: ""))
        #expect(fresh.title == "Claude's" && fresh.newTask.fromAi && fresh.newTask.dueDay == "2026-10-01")
        #expect(!TaskDraft(threadID: "t", subject: "", sender: "", categories: []).canAdd)
    }
}

@MainActor
struct TaskListTests {
    private func demoWithTasks() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        model.undo.runsClock = false
        await model.seedDemoTasks()
        model.selectedMailboxID = AppModel.tasksMailboxID
        await model.tasks.load()
        return model
    }

    private func settle(_ model: AppModel) async throws {
        try await Task.sleep(for: .milliseconds(150))
        await model.tasks.load()
    }

    @Test func tasksGroupByWhenTheyAreDueAndShowTheirEmail() async throws {
        let model = try await demoWithTasks()
        #expect(model.isTaskList)
        #expect(model.listMailboxID == nil, "no mailbox is listed behind the tasks")
        let sections = model.tasks.sections()
        #expect(sections.map(\.group) == [.overdue, .today, .thisWeek, .later])
        #expect(sections.map { $0.tasks.count } == [1, 2, 1, 1])
        #expect(model.tasks.dueCount == 3, "the sidebar badge: overdue and today")
        #expect(model.tasks.openCount == 5)

        let first = try #require(sections.first?.tasks.first)
        model.selectTask(first.id)
        #expect(model.selectedThreadID == first.threadId, "the reader shows the task's email")

        model.tasks.showsDone = true
        try await settle(model)
        #expect(model.tasks.tasks.map(\.title) == ["Forward the invoice to accounts"])
    }

    @Test func doneDeleteAndCategoryAreUndoable() async throws {
        let model = try await demoWithTasks()
        let account = try #require(model.openAccountID)
        let first = try #require(model.tasks.sections().first?.tasks.first)
        model.selectTask(first.id)

        await model.toggleSelectedTaskDone()
        #expect(!model.tasks.tasks.contains { $0.id == first.id })
        #expect(model.tasks.selectedID != nil && model.tasks.selectedID != first.id, "the next task is selected")
        #expect(model.undo.undoTitle(in: account) == "Undo Complete Task")
        model.undo.undo(in: account)
        try await settle(model)
        #expect(model.tasks.tasks.contains { $0.id == first.id })

        model.selectTask(first.id)
        await model.setSelectedTaskCategory("Admin")
        #expect(model.tasks.selected?.category == "Admin")
        model.undo.undo(in: account)
        try await settle(model)
        #expect(model.tasks.tasks.first { $0.id == first.id }?.category == "Reply")

        model.selectTask(first.id)
        await model.deleteSelectedTask()
        #expect(!model.tasks.tasks.contains { $0.id == first.id })
        model.undo.undo(in: account)
        try await settle(model)
        #expect(model.tasks.tasks.first { $0.id == first.id }?.title == first.title)
    }

    @Test func editingChangesTheTaskAndUndoPutsItBack() async throws {
        let model = try await demoWithTasks()
        let account = try #require(model.openAccountID)
        let task = try #require(model.tasks.sections().first?.tasks.first)
        model.selectTask(task.id)
        await model.editSelectedTask()
        let draft = try #require(model.taskDraft)
        #expect(draft.editing?.id == task.id && draft.title == task.title && draft.hasDue)
        draft.title = "Send the plan tomorrow"
        draft.hasDue = false
        await model.acceptTask(draft)
        #expect(model.taskDraft == nil)
        let changed = try #require(model.tasks.tasks.first { $0.id == task.id })
        #expect(changed.title == "Send the plan tomorrow" && changed.dueDay == nil)
        model.undo.undo(in: account)
        try await settle(model)
        #expect(model.tasks.tasks.first { $0.id == task.id }?.title == task.title)
    }

    @Test func sendingATasksReplyAsksWhetherItIsDone() async throws {
        let model = try await demoWithTasks()
        let account = try #require(model.openAccountID)
        var opened: [ComposeRequest] = []
        model.openComposer = { opened.append($0) }
        let task = try #require(model.tasks.sections().first?.tasks.first)
        model.selectTask(task.id)
        await model.reader.show(threadID: task.threadId)
        model.reply(all: false)
        #expect(opened.first?.taskID == task.id, "the reply carries its task")

        await model.messageSent(heldDraftID: nil, taskID: task.id, accountID: account)
        #expect(model.taskDoneQuestion?.task.id == task.id, "sending asks whether the task is done")
        await model.answerTaskDone(false)
        #expect(model.taskDoneQuestion == nil)
        try await settle(model)
        #expect(model.tasks.tasks.contains { $0.id == task.id }, "No keeps it open")

        await model.messageSent(heldDraftID: nil, taskID: task.id, accountID: account)
        await model.answerTaskDone(true)
        try await settle(model)
        #expect(!model.tasks.tasks.contains { $0.id == task.id }, "Yes finishes it")
        #expect(model.undo.notice?.text.hasPrefix("Task done") == true)
        model.undo.undo(in: account)
        try await settle(model)
        #expect(model.tasks.tasks.contains { $0.id == task.id }, "undo opens it again")

        // Outside the task list, a reply answers no task.
        model.selectedMailboxID = "INBOX"
        model.selectedThreadID = task.threadId
        await model.reader.show(threadID: task.threadId)
        model.reply(all: false)
        #expect(opened.last?.taskID == nil)
    }
}

@MainActor
struct BulkTaskTests {
    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
    private struct Timeout: Error {}

    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        return model
    }

    @Test func theLatestTwentyGetOneRequestAndThreadsWithTasksStartUnchecked() async throws {
        let model = try await demo()
        let first = try #require(model.threads.rows.first)
        _ = try await model.core!.createTasks([
            NewTask(threadId: first.id, messageId: nil, title: "Already", notes: "", category: "Reply", dueDay: nil,
                    action: .reply, why: "", fromAi: false),
        ])
        await model.openBulkTasks()
        let draft = try #require(model.bulkTasks)
        #expect(draft.rows.count == min(20, model.threads.rows.count))
        #expect(draft.rows.map(\.threadID) == model.threads.rows.prefix(20).map(\.id))
        try await waitUntil { draft.suggester.state != .working }
        #expect(draft.rows.allSatisfy { !$0.title.isEmpty }, "one answer filled every row")
        #expect(draft.rows[0].hasTask && !draft.rows[0].included)
        #expect(draft.ready.count == draft.rows.count - 1)

        draft.rows[1].included = false
        let expected = draft.ready.count
        await model.acceptBulkTasks(draft)
        #expect(model.bulkTasks == nil)
        #expect(try await model.core!.listTasks().count == expected + 1)
        #expect(model.undo.notice?.text == "Added \(expected) tasks")

        let account = try #require(model.openAccountID)
        model.undo.undo(in: account)
        for _ in 0..<100 where try await model.core!.listTasks().count > 1 { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await model.core!.listTasks().map(\.title) == ["Already"], "one undo removes them all")
    }

    @Test func highlightedThreadsAreAskedAbout() async throws {
        let model = try await demo()
        let picked = Set(model.threads.rows.dropFirst(2).prefix(3).map(\.id))
        model.selectedThreadIDs = picked
        #expect(Set(model.bulkTaskTargets.map(\.id)) == picked)
        model.selectedThreadIDs = []
        #expect(model.bulkTaskTargets.count == min(20, model.threads.rows.count))
    }
}

@MainActor
struct HideTasksTests {
    @Test func theInboxCanHideEmailsWithTasksPerAccount() async throws {
        let defaults = CoreClient.appDefaults()
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()), defaults: defaults)
        await model.start(openDemo: true)
        model.showCategories = false
        let row = try #require(model.threads.rows.first)
        _ = try await model.core!.createTasks([
            NewTask(threadId: row.id, messageId: nil, title: "Do it", notes: "", category: "Reply", dueDay: nil,
                    action: .reply, why: "", fromAi: false),
        ])
        for _ in 0..<100 where model.taskLabelID == nil { try await Task.sleep(for: .milliseconds(20)) }
        let label = try #require(model.taskLabelID)
        #expect(model.listMailboxID?.contains("!") == false, "off by default")

        model.inboxHidesTasks = true
        #expect(model.listMailboxID?.hasSuffix("!" + label) == true)
        for _ in 0..<100 where model.threads.rows.contains(where: { $0.id == row.id }) {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(!model.threads.rows.contains { $0.id == row.id }, "the thread with a task left the Inbox")
        let account = try #require(model.openAccountID)
        #expect(defaults.bool(forKey: AppModel.hideTasksKey(account)), "remembered per account")

        // Search ignores it, as it ignores the tabs.
        model.searchText = "a"
        #expect(model.filteredSearch.contains("!") == false)
        model.searchText = ""

        model.inboxHidesTasks = false
        for _ in 0..<100 where !model.threads.rows.contains(where: { $0.id == row.id }) {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.threads.rows.contains { $0.id == row.id })
    }
}

@MainActor
struct TaskSettingsTests {
    @Test func categoriesAreAddedMovedRemovedAndReset() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        let model = AppModel(core: core)
        await model.start(openDemo: true)
        let editor = TaskCategoryEditor()
        await editor.load(core: core)
        #expect(editor.names.count == 7)
        editor.newName = " Call back "
        editor.add()
        editor.newName = "reply"
        #expect(!editor.canAdd, "names are unique in any case")
        editor.move(from: IndexSet(integer: 7), to: 0)
        editor.remove("Admin")
        await editor.save(editor.names, core: core)
        let stored = try await core.taskCategories()
        #expect(stored.first == "Call back" && !stored.contains("Admin") && stored.count == 7)
        editor.reset()
        for _ in 0..<50 where editor.names.first != "Reply" { try await Task.sleep(for: .milliseconds(20)) }
        #expect(try await core.taskCategories().count == 7)
        #expect(editor.names.first == "Reply")
    }
}
