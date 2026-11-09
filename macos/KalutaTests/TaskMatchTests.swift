import Foundation
import Testing
@testable import Kaluta

/// Finding the task an email belongs to, and adding it (spec §14.8,
/// amended 2026-11-09).
@MainActor
struct TaskMatchTests {
    @Test func aMatchSaysWhatItIsAndWhy() {
        let task = TaskItem(id: 1, threadId: "t1", messageId: nil, title: "Sign the agreement", notes: "",
                            category: "Review", dueDay: nil, action: .reply, done: false, fromAi: false, why: "",
                            createdAt: 0, completedAt: nil, subject: "", senderName: nil, senderEmail: nil,
                            linkedThreadIds: ["t2", "t3"])
        let match = TaskMatch(task: task, why: "Same subject and sender")
        #expect(TaskMatchesView.detail(match) == "Review · 3 emails already · Same subject and sender")
    }

    /// The dialog's email finds the task made for a like one, is added to
    /// it, and ⌘Z takes it off again.
    @Test func theDialogsEmailIsAddedToTheTaskItBelongsTo() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        model.undo.runsClock = false
        let core = try #require(model.core)
        let rows = try await core.threads(in: "INBOX", limit: 50).rows
        let words = { (s: String) in s.split { !$0.isLetter && !$0.isNumber }.filter { $0.count >= 3 } }
        let email = try #require(rows.first { words($0.subject).count >= 2 })
        let other = try #require(rows.first { $0.id != email.id })
        let made = try await core.createTasks([NewTask(threadId: other.id, messageId: nil,
                                                       title: "Deal with \(email.subject)", notes: "",
                                                       category: "Review", dueDay: nil, action: .reply, why: "",
                                                       fromAi: false)])
        let task = try #require(made.first)

        let draft = TaskDraft(threadID: email.id, subject: email.subject, sender: "", categories: ["Review"])
        await draft.matcher.run(threadID: email.id, core: core)
        let match = try #require(draft.matcher.matches.first { $0.task.id == task.id }, "\(draft.matcher.state)")

        model.taskDraft = draft
        await model.addDialogEmail(to: match, draft: draft)
        #expect(model.taskDraft == nil, "the dialog closes")
        #expect(draft.matcher.state == .idle, "the search is let go")
        let added = try #require(try await core.listTasks(includeDone: false).first { $0.id == task.id })
        #expect(added.linkedThreadIds == [email.id])
        model.undo.manager(for: try #require(model.openAccountID)).undo()
        try await Task.sleep(for: .milliseconds(300))
        let undone = try #require(try await core.listTasks(includeDone: false).first { $0.id == task.id })
        #expect(undone.linkedThreadIds.isEmpty)
    }
}
