import AppKit
import Foundation

/// Kaluta's writing in a workflow (spec §14.13): the user says what to
/// say (or picks one of up to three answers), Kaluta writes it, and the
/// email that will be sent ends on an editable screen with quick changes
/// and Send. What the user told Kaluta goes with the draft as its
/// direction (ADR 0013), evidence for the daily review.
@MainActor
@Observable
final class WorkflowWriter {
    enum Step: Equatable {
        /// Up to three answers to pick from (1–3), or tell Kaluta.
        case choose
        /// *What do you want to say?*
        case say
        /// Kaluta asks before it writes (spec §14.9): one question at a time.
        case asking
        /// Kaluta is writing (or changing) the email.
        case writing
        /// The email that will be sent, editable.
        case final
        case sending
        /// Forward (F): whom to, and a note; the email goes below it.
        case forward
    }

    /// One of Kaluta's answers: a one-line label and the email.
    struct Answer: Equatable, Identifiable {
        let label: String
        let text: String
        var id: String { label }
    }

    /// The conversation and the message answered.
    let threadID: String
    let messageID: String
    let replyAll: Bool
    /// The task the reply answers, if any: Send finishes it or keeps it open.
    let taskID: Int64?
    /// Forwarding the message rather than answering it.
    let forwarding: Bool
    /// Whom a forward goes to, and the name or address being typed.
    private(set) var recipients: [AddressInfo] = []
    var recipientText = ""
    /// The email being answered, as read on screen.
    let email: String
    let questions: [String]
    let answers: [Answer]

    private(set) var step: Step
    /// *Write it myself* from the answers: what is typed is the email,
    /// Kaluta writes nothing.
    private(set) var ownWordsOnly = false
    /// The field: what to say, or (on the final screen) what to change.
    var field = ""
    /// What the user told Kaluta, in order: the first is what to say,
    /// later ones are changes.
    private(set) var directions: [String] = []
    /// The email that will be sent.
    var text = ""
    /// Kaluta's last text, and the guide version it followed.
    private(set) var aiText: String?
    private(set) var guideVersion: Int64?
    /// Why the last request did not work.
    private(set) var problem: String?
    /// What Kaluta asked before it would write (spec §14.9): facts it does
    /// not have, one question at a time, with answers to pick from where
    /// the thread offers them.
    private(set) var factQuestions: [ComposerAssistant.FactQuestion] = []
    /// The question on screen.
    private(set) var questionIndex = 0
    /// The answers so far, by question; "" for one skipped.
    private(set) var factAnswers: [String] = []
    /// The session Kaluta asked in, kept for the answers.
    private var chat: KalutaChat?
    /// Where the request that was asked about came from, to go back to.
    private var askedFrom: (step: Step, instruction: String) = (.say, "")

    /// The answers were written ahead with the writing guide and facts,
    /// one per option (spec §14.13), not the triage prompt's quick ones.
    let answersByWriter: Bool

    init(threadID: String, messageID: String, replyAll: Bool, taskID: Int64?, email: String,
         answers: [Answer] = [], answersByWriter: Bool = false, forwarding: Bool = false,
         attachmentsDirectory: URL? = nil) {
        files = DraftAttachments(directory: attachmentsDirectory)
        self.answersByWriter = answersByWriter
        self.threadID = threadID
        self.messageID = messageID
        self.replyAll = replyAll
        self.taskID = taskID
        self.email = email
        self.forwarding = forwarding
        questions = WritingText.questions(in: email)
        self.answers = Array(answers.prefix(3))
        step = forwarding ? .forward : answers.isEmpty ? .say : .choose
    }

    // MARK: Attachments

    /// Files to go with the email, copied so they are kept (spec §14.13).
    private(set) var attachments: [DraftAttachmentInfo] = []
    private let files: DraftAttachments
    /// The file chooser is open (⇧⌘A).
    var choosingFiles = false

    func attach(_ urls: [URL]) {
        attachments += files.copy(urls)
    }

    func removeAttachment(_ attachment: DraftAttachmentInfo) {
        attachments.removeAll { $0.path == attachment.path }
        files.remove(attachment)
    }

    // MARK: Forwarding

    /// An address as typed: "Ann Lee <ann@example.com>" or "ann@example.com".
    static func address(from text: String) -> AddressInfo? {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        var name: String?
        var email = text
        if let open = text.lastIndex(of: "<"), text.hasSuffix(">") {
            name = String(text[..<open]).trimmingCharacters(in: .whitespaces)
            email = String(text[text.index(after: open)..<text.index(before: text.endIndex)])
        }
        email = email.trimmingCharacters(in: .whitespaces)
        guard let at = email.firstIndex(of: "@"), at != email.startIndex,
              email[email.index(after: at)...].contains("."), !email.contains(" "), !email.hasSuffix(".")
        else { return nil }
        return AddressInfo(name: name?.isEmpty == false ? name : nil, email: email)
    }

    /// Add one to whom the forward goes (once per address).
    func addRecipient(_ address: AddressInfo) {
        guard !recipients.contains(where: { $0.email.lowercased() == address.email.lowercased() }) else { return }
        recipients.append(address)
        problem = nil
    }

    func removeRecipient(_ email: String) {
        recipients.removeAll { $0.email.lowercased() == email.lowercased() }
    }

    /// What was typed, as a recipient; false when it is not an address.
    func takeTypedRecipient() -> Bool {
        guard let address = Self.address(from: recipientText) else { return !recipientText.trimmingCharacters(in: .whitespaces).isEmpty ? false : true }
        addRecipient(address)
        recipientText = ""
        return true
    }

    /// Send the forward: the note, then the email as it was sent, to the
    /// recipients. Whether it went; a held send (Undo Send) says so.
    private(set) var sentHeld = false
    func sendForward(model: AppModel) async -> Bool {
        guard forwarding, step == .forward, let core = model.core, let account = model.openAccountID else { return false }
        guard takeTypedRecipient() else {
            problem = "“\(recipientText.trimmingCharacters(in: .whitespaces))” is not an email address."
            return false
        }
        guard !recipients.isEmpty else {
            problem = "Who should it go to?"
            return false
        }
        step = .sending
        do {
            var draft = try await core.forwardDraft(of: messageID)
            draft.to = recipients
            let note = field.trimmingCharacters(in: .whitespacesAndNewlines)
            draft.bodyHtml = note.isEmpty ? ""
                : ComposerHTML.html(from: NSAttributedString(string: note, attributes: [.font: ComposerHTML.bodyFont]))
            let id = try await core.saveDraft(draft)
            let held = try await core.sendDraft(id)
            sentHeld = held
            await model.messageSent(heldDraftID: held ? id : nil, taskID: nil, finishTask: false, accountID: account)
            return true
        } catch {
            problem = (error as? CoreClientError)?.message ?? error.localizedDescription
            step = .forward
            return false
        }
    }

    /// Short answers to the email's questions (Yes, No; its choices): 1–3
    /// say one, and Kaluta writes the reply from it.
    var starters: [String] { Array(WritingText.starters(for: questions).prefix(3)) }

    /// Say that answer: Kaluta writes the reply from it, asking first if
    /// it must (spec §14.9); words already typed follow it. With no agent
    /// ready the answer seeds the field, the words being the email.
    @discardableResult
    func pickStarter(_ index: Int, model: AppModel) async -> Bool {
        guard step == .say, !ownWordsOnly, starters.indices.contains(index) else { return false }
        let typed = field.trimmingCharacters(in: .whitespacesAndNewlines)
        guard model.agent.isProviderReady else {
            addStarter(starters[index])
            return true
        }
        field = typed.isEmpty ? starters[index] : "\(starters[index]), \(typed)"
        await write(model: model)
        return true
    }

    /// What Kaluta is told when the user says nothing: the review reads it
    /// as no direction at all (spec §14.13).
    static let firstPassInstruction = "Write a reply"

    /// ⌘Return on the words: Kaluta writes the reply it finds likely, with
    /// nothing said; it asks first if it must (spec §14.9).
    func firstPass(model: AppModel) async {
        guard step == .say, !ownWordsOnly, model.agent.isProviderReady else { return }
        // Words typed are what to say: they are not thrown away.
        guard field.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return await write(model: model) }
        directions = [Self.firstPassInstruction]
        field = ""
        await ask(model: model, instruction: Self.firstPassInstruction, draft: "")
    }

    /// The words to put back in the field: none for a first pass.
    private static func words(_ instruction: String) -> String {
        instruction == firstPassInstruction ? "" : instruction
    }

    /// What the user said (or chose), shown above the final email.
    var said: String { directions.first ?? "" }

    func addStarter(_ starter: String) {
        let trimmed = field.trimmingCharacters(in: .whitespaces)
        field = trimmed.isEmpty ? starter + ", " : trimmed + " " + starter
    }

    /// 1–3: the answer, on the editable screen.
    func pick(_ index: Int) {
        guard step == .choose, answers.indices.contains(index) else { return }
        let answer = answers[index]
        directions = ["Chose: \(answer.label)"]
        text = answer.text
        aiText = answer.text
        step = .final
    }

    /// From the answers to *Tell Kaluta what to say…*.
    func tellKaluta() {
        guard step == .choose else { return }
        ownWordsOnly = false
        step = .say
    }

    /// From the answers to *Write it myself* (⌘Return): a field whose
    /// words are the email.
    func writeMyself() {
        guard step == .choose else { return }
        ownWordsOnly = true
        step = .say
    }

    /// Esc on *What do you want to say?*: back to the answers, when there
    /// are any. False: nothing to go back to (the writer closes).
    func backToAnswers() -> Bool {
        guard step == .say, !answers.isEmpty else { return false }
        field = ""
        ownWordsOnly = false
        step = .choose
        return true
    }

    /// Back from the final email to what the user said, to change it.
    func changeWhatISaid() {
        guard step == .final else { return }
        field = directions.first.map { $0.hasPrefix("Chose: ") ? "" : Self.words($0) } ?? ""
        directions = []
        step = answers.isEmpty || ownWordsOnly ? .say : .choose
    }

    /// Return on *What do you want to say?*: Kaluta writes it, or, with no
    /// agent ready, the user's words become the email (*Write it yourself*).
    func write(model: AppModel, asWritten: Bool = false) async {
        let said = field.trimmingCharacters(in: .whitespacesAndNewlines)
        guard step == .say, !said.isEmpty else { return }
        directions = [said]
        field = ""
        // Use my words, Write it myself, or no agent ready: what the user
        // typed is the email.
        guard model.agent.isProviderReady, !asWritten, !ownWordsOnly else {
            text = said
            aiText = nil
            step = .final
            return
        }
        await ask(model: model, instruction: said, draft: "")
    }

    /// A quick change (Shorter, Warmer) or what the user typed.
    func revise(_ request: String, model: AppModel) async {
        let request = request.trimmingCharacters(in: .whitespacesAndNewlines)
        guard step == .final, !request.isEmpty, model.agent.isProviderReady else { return }
        directions.append(request)
        field = ""
        await ask(model: model, instruction: request, draft: text)
    }

    // MARK: Kaluta's questions

    /// The question on screen, while Kaluta asks.
    var question: ComposerAssistant.FactQuestion? {
        factQuestions.indices.contains(questionIndex) ? factQuestions[questionIndex] : nil
    }

    var onLastQuestion: Bool { questionIndex >= factQuestions.count - 1 }

    /// Kaluta asked instead of writing: its questions, from the first,
    /// with the session they were asked in (kept for the answers).
    func beginAsking(_ questions: [ComposerAssistant.FactQuestion], chat: KalutaChat? = nil) {
        guard !questions.isEmpty else { return }
        self.chat?.close()
        self.chat = chat
        factQuestions = questions
        factAnswers = Array(repeating: "", count: questions.count)
        questionIndex = 0
        field = ""
        step = .asking
    }

    /// Return: the field answers the question (nothing typed skips it),
    /// and the next one comes up. True once every question has been
    /// answered: Kaluta writes.
    func answerQuestion() -> Bool {
        guard step == .asking, question != nil else { return false }
        factAnswers[questionIndex] = field.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !onLastQuestion else { return true }
        questionIndex += 1
        field = factAnswers[questionIndex]
        return false
    }

    /// 1–3: one of the answers offered, as the answer.
    func pickChoice(_ index: Int) -> Bool {
        guard let question, question.choices.indices.contains(index) else { return false }
        field = question.choices[index]
        return answerQuestion()
    }

    /// ⇧Return: no answer; Kaluta leaves a [blank] for it.
    func skipQuestion() -> Bool {
        field = ""
        return answerQuestion()
    }

    /// Esc: the question before; from the first, back to the words, the
    /// questions dropped.
    func backFromAsking() {
        guard step == .asking else { return }
        if questionIndex > 0 {
            factAnswers[questionIndex] = field.trimmingCharacters(in: .whitespacesAndNewlines)
            questionIndex -= 1
            field = factAnswers[questionIndex]
            return
        }
        chat?.close()
        chat = nil
        factQuestions = []
        // The request stays with the words, not as a direction.
        _ = directions.popLast()
        field = Self.words(askedFrom.instruction)
        step = askedFrom.step
    }

    /// Dropped: Kaluta's session goes, and the copies of files not sent.
    func close() {
        chat?.close()
        chat = nil
        attachments.forEach(files.remove)
        attachments = []
    }

    /// After the last question: the answers go to Kaluta in the session
    /// it asked in, and to Facts as proposals (spec §14.9), never added
    /// outright. Each is recorded with the draft's directions.
    func writeWithAnswers(model: AppModel) async {
        guard step == .asking, let chat else { return }
        let given = zip(factQuestions, factAnswers).filter { !$1.isEmpty }.map { ($0, $1) }
        step = .writing
        factQuestions = []
        field = ""
        if !given.isEmpty, let core = model.core {
            _ = try? await core.proposeWritingHelpFacts(given.map { q, a in
                WritingHelpAnswer(category: q.category, label: q.fact, value: a, messageId: messageID)
            })
        }
        self.chat = nil
        defer { chat.close() }
        do {
            let answer = ComposerAssistant.cleaned(
                try await chat.ask(ComposerAssistant.answersPrompt(given.map { ($0.question, $1) })))
            if let more = ComposerAssistant.questions(in: answer), !more.isEmpty {
                throw CoreClientError(kind: .agent, message: "Kaluta still asks: " + more.map(\.question).joined(separator: " "))
            }
            guard !answer.isEmpty else { throw CoreClientError(kind: .agent, message: "Kaluta did not write anything.") }
            directions.append(contentsOf: given.map { "\($0.question) \($1)" })
            text = answer
            aiText = answer
            step = .final
        } catch {
            problem = (error as? CoreClientError)?.message ?? error.localizedDescription
            _ = directions.popLast()
            field = askedFrom.instruction
            step = askedFrom.step
        }
    }

    private func ask(model: AppModel, instruction: String, draft: String) async {
        guard let core = model.core else { return }
        let back: Step = draft.isEmpty ? .say : .final
        step = .writing
        problem = nil
        var chat: KalutaChat?
        do {
            guard let detail = try await core.thread(threadID) else {
                throw CoreClientError(kind: .notFound, message: "This conversation is no longer here.")
            }
            let latest = AppModel.latestReceived(detail)
            // A follow-up to the user's own message goes to its recipients.
            let to = latest.map { $0.isSentByMe ? $0.to.map(\.email) : ($0.from.map { [$0.email] } ?? []) } ?? []
            let guide = try? await core.guideForMessage(recipients: to, messageType: "reply", audiences: nil)
            guideVersion = (guide?.text.isEmpty ?? true) ? nil : guide?.version
            let prompt = ComposerAssistant.prompt(
                instruction: instruction, from: (try? await core.accountAddress()) ?? "", to: to,
                subject: Self.replySubject(detail.thread.subject), draft: draft, original: email,
                guide: guide?.text ?? "")
            let opened = try await KalutaChat.open(model: model, about: [threadID])
            chat = opened
            let answer = ComposerAssistant.cleaned(try await opened.ask(prompt))
            if let questions = ComposerAssistant.questions(in: answer), !questions.isEmpty {
                // Facts Kaluta does not have: it asks, one question at a
                // time, in the session it asked in. The request stays the
                // first direction; the answers join it once it writes.
                askedFrom = (back, instruction)
                beginAsking(questions, chat: opened)
                return
            }
            opened.close()
            guard !answer.isEmpty else { throw CoreClientError(kind: .agent, message: "Kaluta did not write anything.") }
            text = answer
            aiText = answer
            step = .final
        } catch {
            chat?.close()
            problem = (error as? CoreClientError)?.message ?? error.localizedDescription
            field = Self.words(instruction)
            directions.removeLast()
            step = back
        }
    }

    /// Send the email as a reply; with a task, `keepTask` leaves it open,
    /// otherwise sending finishes it (spec §14.13). What Kaluta wrote is
    /// recorded with the user's directions. Whether it went.
    func send(model: AppModel, keepTask: Bool = false) async -> Bool {
        let body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard step == .final, !body.isEmpty, let core = model.core, let account = model.openAccountID else { return false }
        step = .sending
        do {
            var draft = try await core.replyDraft(to: messageID, all: replyAll)
            draft.bodyHtml = ComposerHTML.html(from: NSAttributedString(string: body,
                                                                         attributes: [.font: ComposerHTML.bodyFont]))
            draft.attachments = attachments
            let id = try await core.saveDraft(draft)
            if let aiText {
                if let guideVersion { try? await core.setDraftGuideVersion(id, guideVersion) }
                try? await core.recordWritingHelp(draftID: id, agent: model.agent.providerID,
                                                  instruction: directions.joined(separator: "\n"), text: aiText,
                                                  guideVersion: guideVersion, audiences: [])
            }
            let held = try await core.sendDraft(id)
            // The draft has the copies now: they go with it, held or sent.
            attachments = []
            await model.messageSent(heldDraftID: held ? id : nil, taskID: taskID, finishTask: taskID != nil && !keepTask,
                                    accountID: account)
            return true
        } catch {
            problem = (error as? CoreClientError)?.message ?? error.localizedDescription
            step = .final
            return false
        }
    }

    static func replySubject(_ subject: String) -> String {
        subject.lowercased().hasPrefix("re:") ? subject : "Re: \(subject)"
    }
}

extension AppModel {
    /// Ask Kaluta one prompt in a read-only session that sees only
    /// `threads`, and wait for the whole answer. Anything it proposes that
    /// would change mail is refused: it writes text, nothing else.
    func askKaluta(_ prompt: String, about threads: [String]) async throws -> String {
        let chat = try await KalutaChat.open(model: self, about: threads)
        defer { chat.close() }
        return try await chat.ask(prompt)
    }
}

/// A conversation with Kaluta in a read-only session that sees only some
/// threads: ask, then ask again with the answer in mind (its questions
/// answered), and close when done. Anything it proposes that would change
/// mail is refused: it writes text, nothing else.
@MainActor
final class KalutaChat {
    private weak var model: AppModel?
    private let core: CoreClient
    let session: String
    private var closed = false

    private init(model: AppModel, core: CoreClient, session: String) {
        self.model = model
        self.core = core
        self.session = session
    }

    static func open(model: AppModel, about threads: [String]) async throws -> KalutaChat {
        guard let core = model.core else { throw CoreClientError(kind: .internalError, message: "The core is not running.") }
        let session = try await core.startReadOnlyAgentSession(provider: model.agent.providerID, selection: threads)
        return KalutaChat(model: model, core: core, session: session)
    }

    /// One turn: the prompt, and the whole answer.
    func ask(_ prompt: String) async throws -> String {
        guard !closed, let model else { throw CoreClientError(kind: .internalError, message: "Kaluta's session is closed.") }
        let turn = KalutaTurn()
        let core = core
        let session = session
        defer { model.agentSinks[session] = nil }
        return try await withCheckedThrowingContinuation { continuation in
            turn.continuation = continuation
            model.agentSinks[session] = { [weak core] events in
                for event in events {
                    switch event {
                    case let .textDelta(text):
                        turn.reply += text
                    case let .actionProposed(actionID, _, _, _):
                        try? core?.resolveAgentAction(actionID, approve: false)
                    case .turnCompleted:
                        turn.finish(.success(turn.reply))
                    case let .turnFailed(message):
                        turn.finish(.failure(CoreClientError(kind: .agent, message: message)))
                    default:
                        break
                    }
                }
            }
            Task {
                do {
                    try await core.sendAgentPrompt(session, prompt)
                } catch {
                    turn.finish(.failure(error))
                }
            }
        }
    }

    func close() {
        guard !closed else { return }
        closed = true
        model?.agentSinks[session] = nil
        let core = core
        let session = session
        Task { try? await core.closeAgentSession(session) }
    }
}

/// One answer being waited for: the text so far, resumed once.
@MainActor
private final class KalutaTurn {
    var reply = ""
    var continuation: CheckedContinuation<String, Error>?

    func finish(_ result: Result<String, Error>) {
        continuation?.resume(with: result)
        continuation = nil
    }
}
