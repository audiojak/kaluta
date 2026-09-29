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
