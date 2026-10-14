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
            // Enter is Return, everywhere.
            (.enter, "\u{3}", [], .primary),
            (.enter, "\u{3}", .command, .send),
            (.enter, "\u{3}", [.command, .option], .sendKeepTask),
            (.delete, "", [], .reject),
            (.escape, "", [], .back),
            (KeyEquivalent("e"), "e", [], .done),
            (KeyEquivalent("E"), "E", .shift, .done),
            (KeyEquivalent("t"), "t", [], .task),
            (KeyEquivalent("l"), "l", [], .later),
            (KeyEquivalent("s"), "s", [], .skip),
            (KeyEquivalent("r"), "r", [], .replyNow),
            (KeyEquivalent("o"), "o", [], .openInFull),
            (KeyEquivalent("w"), "w", [], .edit),
            (KeyEquivalent("i"), "i", [], .backToInbox),
            (KeyEquivalent("c"), "c", [], .copyCode),
            (KeyEquivalent("2"), "2", [], .pick2),
            (KeyEquivalent("/"), "/", [], .tellKaluta),
            // ⌘E clears a reading bucket; another modifier makes it a menu key.
            (KeyEquivalent("e"), "e", .command, .clearBucket),
            (KeyEquivalent("e"), "e", [.command, .shift], nil),
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
        // In an item Return is Reply: a nudge is one.
        #expect(WorkflowCommand.replyNow.keyLabel == "Return")
        #expect(WorkflowCommand.replyNow.title(in: .triage) == "Reply")
        #expect(WorkflowCommand.replyNow.title(in: .waiting) == "Nudge")
        // Teach Kaluta: E accepts, W edits, O shows the evidence.
        #expect(WorkflowCommand.done.title(in: .teach) == "Accept")
        #expect(WorkflowCommand.edit.keyLabel == "W")
        #expect(WorkflowCommand.openInFull.title(in: .teach) == "Show the evidence")
    }

    /// Triage takes a suggestion with its own button's key, never Return
    /// unless it is a reply.
    @Test func aTriageSuggestionIsTakenByItsOwnKey() {
        func suggestion(_ kind: TriageKind) -> TriageSuggestion {
            TriageSuggestion(threadId: "t", kind: kind, seconds: nil, reason: "", answers: [], task: nil)
        }
        #expect(AppModel.triageCommand(suggestion(.quickReply)) == .replyNow)
        #expect(AppModel.triageCommand(suggestion(.task)) == .task)
        #expect(AppModel.triageCommand(suggestion(.archive)) == .done)
        #expect(AppModel.triageCommand(suggestion(.leave)) == .later)
        #expect(KeyEquivalent.enter.isReturn && KeyEquivalent.return.isReturn && !KeyEquivalent.space.isReturn)
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
        #expect(WritingText.starters(for: questions) == ["Go for Tuesday", "Push to Thursday"], "a choice gives its options")
        #expect(WritingText.starters(for: ["Do you want to meet them once more, or should I start on the offer?"])
            == ["Meet them once more", "Start on the offer"])
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

    @Test func aWaitSaysHowLongAndHowItEnded() {
        let now = Date(timeIntervalSince1970: 1_791_000_000)
        let since = Calendar.current.date(byAdding: .day, value: -9, to: now)!
        let wait = WaitingInfo(threadId: "t", who: "Hayden", since: Int64(since.timeIntervalSince1970 * 1000),
                               reason: "", nudges: [], state: "waiting")
        #expect(WaitingLine.waited(wait, now: now) == "9 days")
        let item = { (d: String?) in WorkflowItem(key: "thread:t", threadId: "t", taskId: nil, decision: d) }
        #expect(WorkflowRail.detail(wait, item(nil), now: now) == "9 days")
        #expect(WorkflowRail.detail(wait, item("answered"), now: now) == "Answered · off the list")
        #expect(WorkflowRail.detail(wait, item("nudged"), now: now) == "Nudged")
        #expect(WorkflowEndView.words("later", count: 2, kind: .waiting) == "to check again")
    }

    @Test func aReadingItemBelongsToItsDueBucket() {
        let items = ["a", "b", "c"].map { WorkflowItem(key: "thread:\($0)", threadId: $0, taskId: nil, decision: nil) }
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .reading, items: items, startedAt: 0, resumed: false))
        session.reading = ReadingReport(sortedThisWeek: 5, buckets: [
            ReadingBucket(id: "daily", title: "Daily", labelId: "L1", cadence: "daily", dueToday: true,
                          verb: "Act on what needs you", threadIds: ["a"], subjects: ["A"]),
            ReadingBucket(id: "news", title: "Newsletters", labelId: "L2", cadence: "weekly", dueToday: true,
                          verb: "Read, then clear", threadIds: ["b", "c"], subjects: ["B", "C"]),
            ReadingBucket(id: "events", title: "Events", labelId: "L3", cadence: "weekly", dueToday: false,
                          verb: "", threadIds: ["c"], subjects: ["C"]),
        ])
        #expect(session.bucket(of: items[0])?.id == "daily")
        #expect(session.bucket(of: items[2])?.id == "news", "a bucket not due today does not claim it")
        #expect(WorkflowRail.readingDetail(WorkflowItem(key: "thread:b", threadId: "b", taskId: nil, decision: "inbox"))
            == "Back in the Inbox")
        #expect(WorkflowEndView.words("kept", count: 1, kind: .reading) == "kept for later")
    }

    @Test func teachKalutaPutsRoutineChangesFirstGroupedByKind() throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        model.routineProposals = [
            RoutineProposalInfo(id: 7, statement: "Leave mail from news@example.com in the Inbox",
                                rule: "Mail from news@example.com: leave it in the Inbox",
                                evidence: [RoutineEvidence(threadId: "t", subject: "Issue 42", bucket: "2-Weekly", at: 0)]),
        ]
        model.enterReview(.teach)
        #expect(model.reviewMode == .teach)
        #expect(model.reviewItems.first?.tag == "routine:7" && model.reviewItems.first?.group == "Sorting routine")
        #expect(model.reviewSelection == "routine:7")
        #expect(model.reviewIsWaiting("routine:7"))
        #expect(ReviewQueueView.groups(model.reviewItems).first == "Sorting routine")
        #expect(AppModel.routineCaption(model.routineProposals[0]).contains("1 conversation you moved back"))
        model.leaveReview()
        #expect(model.reviewMode == nil && model.teachSelection == nil)
    }

    @Test func agentsItemsAreReadFromTheirKeys() throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        let items = ["action:42", "code:scout/in:fake1", "run:routine-1/9"].map {
            WorkflowItem(key: $0, threadId: nil, taskId: nil, decision: nil)
        }
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .agents, items: items, startedAt: 0, resumed: false))
        session.agentsWork = AgentsWork(codes: [
            AgentCode(accountId: "scout", mailbox: "scout@agents.example", messageId: "in:fake1",
                      from: "Example Journals", subject: "Your verification code", code: "482913",
                      receivedAt: 0, expiresAt: nil),
        ], sentOnTheirOwn: [AgentSent(accountId: "scout", mailbox: "scout@agents.example", count: 2)], since: 0)
        if case .elsewhere = model.agentItem(items[0], in: session) {} else { Issue.record("an unknown action is elsewhere") }
        if case let .code(code) = model.agentItem(items[1], in: session) { #expect(code.code == "482913") }
        else { Issue.record("the code") }
        if case let .run(routine, run) = model.agentItem(items[2], in: session) { #expect(routine == "routine-1" && run == 9) }
        else { Issue.record("the run") }
        #expect(AgentItemView.spaced("482913") == "482 913")
        #expect(model.workflowPill(session) == "Agent mailboxes sent 2 on their own")
    }

    @Test func anAgentsDraftIsEditedAboveItsQuote() {
        let text = "Hi Morgan,\nYes to two years.\nJo\nOn 2026-10-12, Morgan Okafor wrote:\nHi Jo,\nCan you?"
        let (reply, quote) = AppModel.splitQuote(text)
        #expect(reply == "Hi Morgan,\nYes to two years.\nJo")
        #expect(quote.hasPrefix("On 2026-10-12, Morgan Okafor wrote:"))
        #expect(AppModel.splitQuote("Just a note").quote.isEmpty)
    }

    @Test func turningOnTheOfferedWorkflowsOpensWhatsNext() throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        model.showMail()
        #expect(!model.workflowsOfferable, "no agent found yet: nothing to offer")
        model.finishTip(.workflows, accept: true)
        #expect(model.workflowsEnabled && model.home == .start)
        #expect(!model.workflowsOfferable, "on already")
    }

    @Test func anAccountsAgentsAreFoundWhenItOpens() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: true)
        // Nothing asked for an agent: the account's own store looks anyway,
        // so the agent bar does not stay on "Looking for Claude…".
        for _ in 0..<200 where model.agent.providers.isEmpty { try await Task.sleep(for: .milliseconds(20)) }
        #expect(!model.agent.providers.isEmpty)
    }

    @Test func useMyWordsSendsWhatWasTyped() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        let writer = WorkflowWriter(threadID: "t", messageID: "m", replyAll: false, taskID: nil, email: "Chat?")
        writer.field = "Happy to, next Tuesday at 10."
        await writer.write(model: model, asWritten: true)
        #expect(writer.step == .final && writer.text == "Happy to, next Tuesday at 10." && writer.aiText == nil)
        #expect(WorkflowCommand.ownWords.title(in: .triage) == "Use my words" && WorkflowCommand.ownWords.keyLabel == nil)
    }
}
