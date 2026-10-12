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

    @Test func aDecidedItemLeavesByItsVerb() {
        #expect(WorkflowExit.of("archive") == .left && WorkflowExit.of("done") == .left)
        #expect(WorkflowExit.of("reply") == .right)
        #expect(WorkflowExit.of("task") == .down)
        #expect(WorkflowExit.of("leave") == .up && WorkflowExit.of("later") == .up)
        #expect(WorkflowExit.left.offset.width < 0 && WorkflowExit.down.offset.height > 0)
    }

    @Test func theChordIsStrummedAndFades() {
        let samples = WorkflowSound.chord()
        let rate = WorkflowSound.sampleRate
        #expect(samples.count == Int(WorkflowSound.seconds * rate))
        #expect(abs((samples.map(abs).max() ?? 0) - WorkflowSound.peak) < 0.001, "scaled to its peak")
        #expect(samples.last == 0, "it ends in silence")
        // The strum starts at once.
        let step = Int(WorkflowSound.strum / 3 * rate)
        let first = samples[0..<step]
        #expect(first.contains { $0 != 0 })
        // Loud at the start, quiet after a second.
        let early = samples[0..<Int(0.2 * rate)].map(abs).max() ?? 0
        let late = samples[Int(1.0 * rate)..<Int(1.1 * rate)].map(abs).max() ?? 0
        #expect(late < early / 4)
    }

    @Test func theReviewMarksTheWordsOfWhatTheUserToldKaluta() {
        let pair = AnalysisPairInfo(compositionId: 1, createdAt: 0, agent: nil, kind: "reply", subject: "Logs", to: [],
                                    instruction: "Keep it short, no exclamation marks", aiText: nil, sentText: nil,
                                    sentQuote: "", aiQuote: "", askedQuote: "no exclamation marks")
        let told = AnalysisPairCard.told(pair)
        #expect(String(told.characters) == "You told Kaluta: Keep it short, no exclamation marks")
        let marked = told.runs.filter { $0.inlinePresentationIntent == .stronglyEmphasized }
        #expect(marked.map { String(told[$0.range].characters) } == ["no exclamation marks"])
    }

    @Test func anEmailsQuestionsGiveStartersForTheAnswer() {
        let email = "Hi Jo,\n\nQA signed off this morning. Are we go for Tuesday, or do you want to push to Thursday?\n\nAvery"
        let questions = WritingText.questions(in: email)
        #expect(questions == ["Are we go for Tuesday, or do you want to push to Thursday?"])
        #expect(WritingText.starters(for: questions) == ["Tuesday", "Thursday"], "a choice gives its options")
        #expect(WritingText.starters(for: ["Can you do Thursday at 2pm for the board prep with Avery?"]) == ["Yes", "No"])
        #expect(WritingText.starters(for: ["What time works?"]).isEmpty)
        let marked = WritingText.highlighted(email, questions: questions, tint: .yellow)
        #expect(marked.runs.filter { $0.backgroundColor != nil }.count == 1)
    }

    @Test func theWriterPicksAnAnswerOrTakesWhatTheUserSays() async throws {
        let answers = [WorkflowWriter.Answer(label: "Yes, Thursday", text: "Thursday works."),
                       WorkflowWriter.Answer(label: "Friday instead", text: "Could we do Friday?")]
        let writer = WorkflowWriter(threadID: "t", messageID: "m", replyAll: false, taskID: nil,
                                    email: "Can you do Thursday?", answers: answers)
        #expect(writer.step == .choose)
        writer.pick(1)
        #expect(writer.step == .final && writer.text == "Could we do Friday?" && writer.said == "Chose: Friday instead")
        writer.changeWhatISaid()
        #expect(writer.step == .choose && writer.field.isEmpty)
        writer.tellKaluta()
        #expect(writer.step == .say)

        // No agent ready: the user's words are the email (Write it yourself).
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        writer.field = "Thursday is fine, see you then."
        await writer.write(model: model)
        #expect(writer.step == .final && writer.text == "Thursday is fine, see you then." && writer.aiText == nil)
        #expect(writer.said == "Thursday is fine, see you then.")
        writer.changeWhatISaid()
        #expect(writer.field == "Thursday is fine, see you then.", "Change brings back what was said")
    }

    @Test func theWritingScreenReadsAsTheUserDidIt() {
        #expect(WorkflowWriteView.preview("Hi Sam,\n\nYes, that works.\n\nJo") == "Hi Sam, Yes, that works. Jo")
        #expect(WorkflowWriteView.saidLine("Chose: Yes") == "You chose: Yes")
        #expect(WorkflowWriteView.saidLine("Tuesday works") == "You said: Tuesday works")
        #expect(WorkflowWriteView.askTitle(.tasks) == "What did you decide?")
    }

    @Test func triageSaysWhatKalutaSuggests() {
        let reply = TriageSuggestion(threadId: "t", kind: .quickReply, seconds: 18, reason: "", answers: [], task: nil)
        #expect(TriageSuggestionLine.verb(reply) == "Quick reply · about 20 seconds")
        let task = TriageSuggestion(threadId: "t", kind: .task, seconds: nil, reason: "", answers: [], task: nil)
        #expect(TriageSuggestionLine.verb(task) == "Make a task")
    }
}
