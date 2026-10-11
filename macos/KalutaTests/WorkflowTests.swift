import SwiftUI
import Testing
@testable import Kaluta

/// Workflows (spec §14.13): one command layer, keys that mean the same in
/// every workflow, and the session's counts.
@MainActor
struct WorkflowTests {
    @Test func everyKeyMeansOneCommand() {
        let keys: [(KeyEquivalent, String, EventModifiers, WorkflowCommand?)] = [
            (.return, "\r", [], .primary),
            (.return, "\r", .command, .send),
            (.return, "\r", [.command, .option], .sendKeepTask),
            (.delete, "", [], .reject),
            (.escape, "", [], .back),
            (KeyEquivalent("e"), "e", [], .done),
            (KeyEquivalent("E"), "E", .shift, .done),
            (KeyEquivalent("t"), "t", [], .task),
            (KeyEquivalent("l"), "l", [], .later),
            (KeyEquivalent("s"), "s", [], .skip),
            (KeyEquivalent("r"), "r", [], .replyNow),
            (KeyEquivalent("i"), "i", [], .backToInbox),
            (KeyEquivalent("c"), "c", [], .copyCode),
            (KeyEquivalent("2"), "2", [], .pick2),
            (KeyEquivalent("/"), "/", [], .tellKaluta),
            // A modifier makes it a menu key, not a workflow's.
            (KeyEquivalent("e"), "e", .command, nil),
            (KeyEquivalent("x"), "x", [], nil),
        ]
        for (key, characters, modifiers, expected) in keys {
            #expect(WorkflowCommand.command(key: key, characters: characters, modifiers: modifiers) == expected,
                    "\(characters) \(modifiers)")
        }
    }

    @Test func everyCommandHasASpokenNameInEveryWorkflow() {
        for kind in WorkflowKind.all {
            let names = WorkflowCommand.allCases.map { $0.title(in: kind) }
            #expect(names.allSatisfy { !$0.isEmpty }, "\(kind)")
            #expect(Set(names).count == names.count, "names are unique within \(kind): \(names)")
        }
        // The same key, the workflow's own word.
        #expect(WorkflowCommand.done.title(in: .triage) == "Archive")
        #expect(WorkflowCommand.done.title(in: .waiting) == "Let it go")
        #expect(WorkflowCommand.later.title(in: .tasks) == "Tomorrow")
        #expect(WorkflowCommand.done.keyLabel == "E")
    }

    @Test func theStartScreenOrdersTheSevenWithTheirKeys() {
        #expect(WorkflowKind.all.map(\.number) == Array(1...7))
        #expect(WorkflowKind.all.first == .triage && WorkflowKind.all.last == .teach)
    }

    @Test func aSessionCountsWhatIsLeft() {
        let items = ["thread:a", "thread:b", "thread:c"].enumerated().map { i, key in
            WorkflowItem(key: key, threadId: String(key.dropFirst(7)), taskId: nil, decision: i == 0 ? "archive" : nil)
        }
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .triage, items: items, startedAt: 0, resumed: false))
        #expect(session.current?.key == "thread:b")
        #expect(session.progress == "2 of 3")
        #expect(session.whatsLeft(secondsPerItem: 30) == "2 of 3 left, about 1 minute.")
        session.score = WorkflowScore(kind: .triage, decided: 3, items: 3, millis: 9_000, counts: [])
        #expect(session.progress == "3 of 3")
    }

    @Test func anEmailReadsAsParagraphsWithoutItsQuotedHistory() {
        let text = "Hi Jo,\nCan you do Thursday?\nSam\nOn an earlier day, Jo wrote:\n> Hello"
        #expect(AppModel.readingText(text) == "Hi Jo,\n\nCan you do Thursday?\n\nSam")
        let spaced = "Hi Jo,\n\nFirst line\nsecond line.\n\nSam"
        #expect(AppModel.readingText(spaced) == "Hi Jo,\n\nFirst line\nsecond line.\n\nSam")
    }
}
