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
        /// Kaluta is writing (or changing) the email.
        case writing
        /// The email that will be sent, editable.
        case final
        case sending
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
    /// The email being answered, as read on screen.
    let email: String
    let questions: [String]
    let answers: [Answer]

    private(set) var step: Step
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
    /// Why the last request did not work, or what Kaluta needs to know.
    private(set) var problem: String?

    init(threadID: String, messageID: String, replyAll: Bool, taskID: Int64?, email: String,
         answers: [Answer] = []) {
        self.threadID = threadID
        self.messageID = messageID
        self.replyAll = replyAll
        self.taskID = taskID
        self.email = email
        questions = WritingText.questions(in: email)
        self.answers = Array(answers.prefix(3))
        step = answers.isEmpty ? .say : .choose
    }

    var starters: [String] { WritingText.starters(for: questions) }

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
        step = .say
    }

    /// Back from the final email to what the user said, to change it.
    func changeWhatISaid() {
        guard step == .final else { return }
        field = directions.first.map { $0.hasPrefix("Chose: ") ? "" : $0 } ?? ""
        directions = []
        step = answers.isEmpty ? .say : .choose
    }

    /// Return on *What do you want to say?*: Kaluta writes it, or, with no
    /// agent ready, the user's words become the email (*Write it yourself*).
    func write(model: AppModel) async {
        let said = field.trimmingCharacters(in: .whitespacesAndNewlines)
        guard step == .say, !said.isEmpty else { return }
        directions = [said]
        field = ""
        guard model.agent.isProviderReady else {
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

    private func ask(model: AppModel, instruction: String, draft: String) async {
        guard let core = model.core else { return }
        let back: Step = draft.isEmpty ? .say : .final
        step = .writing
        problem = nil
        do {
            guard let detail = try await core.thread(threadID) else {
                throw CoreClientError(kind: .notFound, message: "This conversation is no longer here.")
            }
            let latest = AppModel.latestReceived(detail)
            let to = latest?.from.map { [$0.email] } ?? []
            let guide = try? await core.guideForMessage(recipients: to, messageType: "reply", audiences: nil)
            guideVersion = (guide?.text.isEmpty ?? true) ? nil : guide?.version
            let prompt = ComposerAssistant.prompt(
                instruction: instruction, from: (try? await core.accountAddress()) ?? "", to: to,
                subject: Self.replySubject(detail.thread.subject), draft: draft, original: email,
                guide: guide?.text ?? "")
            let answer = ComposerAssistant.cleaned(try await model.askKaluta(prompt, about: [threadID]))
            if let asked = ComposerAssistant.questions(in: answer), !asked.isEmpty {
                // Facts Kaluta does not have: the user says them too.
                problem = "Kaluta needs to know: " + asked.map { $0.question }.joined(separator: " ")
                field = instruction
                directions.removeLast()
                step = back
                return
            }
            guard !answer.isEmpty else { throw CoreClientError(kind: .agent, message: "Kaluta did not write anything.") }
            text = answer
            aiText = answer
            step = .final
        } catch {
            problem = (error as? CoreClientError)?.message ?? error.localizedDescription
            field = instruction
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
            let id = try await core.saveDraft(draft)
            if let aiText {
                if let guideVersion { try? await core.setDraftGuideVersion(id, guideVersion) }
                try? await core.recordWritingHelp(draftID: id, agent: model.agent.providerID,
                                                  instruction: directions.joined(separator: "\n"), text: aiText,
                                                  guideVersion: guideVersion, audiences: [])
            }
            let held = try await core.sendDraft(id)
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
        guard let core else { throw CoreClientError(kind: .internalError, message: "The core is not running.") }
        let session = try await core.startReadOnlyAgentSession(provider: agent.providerID, selection: threads)
        defer {
            agentSinks[session] = nil
            Task { try? await core.closeAgentSession(session) }
        }
        let turn = KalutaTurn()
        return try await withCheckedThrowingContinuation { continuation in
            turn.continuation = continuation
            agentSinks[session] = { [weak self] events in
                for event in events {
                    switch event {
                    case let .textDelta(text):
                        turn.reply += text
                    case let .actionProposed(actionID, _, _, _):
                        try? self?.core?.resolveAgentAction(actionID, approve: false)
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
