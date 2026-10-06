import AppKit
import Foundation
import Observation

/// Writing help in the composer: the user says what they want ("write a
/// reply saying yes", "make it shorter") and the agent's answer becomes
/// the message body, with Undo. It runs in a session of its own that can
/// see only the thread being answered (none for a new message), and
/// anything it proposes that would change mail is refused: it writes
/// text, nothing else.
@MainActor
@Observable
final class ComposerAssistant {
    enum State: Equatable {
        case idle, working, done, failed(String)
        /// The agent needs facts it does not have before it drafts.
        case asking([FactQuestion])
    }

    /// A fact the agent asked for: the question, and a short name for the
    /// fact when it is kept in the writing guide ("What my company does").
    struct FactQuestion: Equatable, Identifiable {
        let question: String
        /// The fact's label ("What the company does").
        let fact: String
        /// Where it goes in Facts (spec §14.11): a category key.
        var category = "other"
        var id: String { question }
    }

    private(set) var state: State = .idle
    var instruction = ""
    /// The body before the agent's text replaced it, for Undo.
    private(set) var previousBody: NSAttributedString?
    /// The writing guide the last draft followed (spec §14.9), if any.
    private(set) var guide: GuideRendered?

    /// What the last draft still breaks in the guide, after one rewrite.
    private(set) var checkFailures: [String] = []
    /// The audiences the last draft was written for (spec §14.9); empty
    /// when the recipients are in none.
    private(set) var writtenFor: [String] = []
    /// Audiences the user can switch the draft to: the confirmed groups.
    private(set) var audienceChoices: [String] = []
    /// Drafts written this session, by audience key, for switching back.
    @ObservationIgnored private var draftsByAudience: [String: String] = [:]
    /// The request the drafts answer, and the user's own text they started
    /// from: a new audience gets a new draft from the same two.
    @ObservationIgnored private var lastInstruction: String?
    @ObservationIgnored private var original = ""
    /// The body when the request was made: every audience's draft starts
    /// from it.
    @ObservationIgnored private var startText = ""
    /// The audience key of the draft showing, so edits to it are kept when
    /// switching away and back.
    @ObservationIgnored private var shownKey: String?
    /// The audiences asked for on this run (nil: the recipients' own).
    @ObservationIgnored private var requestedAudiences: [String]?
    @ObservationIgnored private var rewrote = false
    /// The agent may still ask for facts on this turn (the first of a
    /// request); facts the user gave for this request, for later drafts.
    @ObservationIgnored private var mayAsk = false
    @ObservationIgnored private var givenFacts: [String] = []

    /// The last draft followed a writing guide with something in it.
    var followsGuide: Bool { !(guide?.text.isEmpty ?? true) }

    @ObservationIgnored private var sessionID: String?
    @ObservationIgnored private var reply = ""
    @ObservationIgnored private weak var store: ComposerStore?
    @ObservationIgnored private weak var model: AppModel?

    /// Quick requests, offered in a menu; choosing one puts it in the
    /// field to send or change.
    static func suggestions(replying: Bool) -> [String] {
        (replying ? ["Write a reply", "Say yes, and suggest a time to talk", "Decline politely"] : ["Write this message"])
            + ["Make it shorter", "Make it friendlier", "Make it more formal", "Fix spelling and grammar"]
    }

    func run(store: ComposerStore, model: AppModel, original: String) async {
        let text = instruction.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, state != .working else { return }
        // A new request starts over: its drafts are its own.
        draftsByAudience = [:]
        lastInstruction = text
        self.original = original
        startText = store.body.string
        shownKey = nil
        givenFacts = []
        await write(text, base: startText, store: store, model: model, audiences: nil)
    }

    /// Write a new draft of the same request for other audiences, from the
    /// user's own text (spec §14.9, drafting with an audience); `nil` is the
    /// recipients' own. A draft already written for them comes back at
    /// once. A draft the agent wrote on its own (review) is rewritten for
    /// the audience.
    func switchAudience(to audiences: [String]?, store: ComposerStore, model: AppModel) async {
        guard state != .working else { return }
        let key = Self.key(audiences)
        keepEdits(store)
        if let cached = draftsByAudience[key] {
            previousBody = store.body
            store.body = NSAttributedString(string: cached, attributes: [.font: ComposerHTML.bodyFont])
            writtenFor = audiences ?? guide?.audiences ?? []
            shownKey = key
            record(cached, store: store)
            return
        }
        let request = lastInstruction ?? "Rewrite this message for \((audiences ?? []).joined(separator: ", ")), keeping what it says"
        // A draft the agent wrote on its own has no request: rewrite what shows.
        let base = lastInstruction == nil ? store.body.string : startText
        await write(request, base: base, store: store, model: model, audiences: audiences)
    }

    /// The audiences a draft can be switched to (the confirmed groups).
    func loadAudiences(_ core: CoreClient?) async {
        guard let core, audienceChoices.isEmpty else { return }
        audienceChoices = ((try? await core.audienceGroups()) ?? []).filter { $0.status == .confirmed }.map(\.name)
    }

    static func key(_ audiences: [String]?) -> String { audiences.map { $0.sorted().joined(separator: "|") } ?? "" }

    /// The user's edits to the draft showing stay with its audience.
    private func keepEdits(_ store: ComposerStore) {
        if let shownKey, draftsByAudience[shownKey] != nil { draftsByAudience[shownKey] = store.body.string }
    }

    private func write(_ text: String, base: String, store: ComposerStore, model: AppModel, audiences: [String]?) async {
        guard let core = model.core else { return }
        self.store = store
        self.model = model
        requestedAudiences = audiences
        state = .working
        reply = ""
        rewrote = false
        mayAsk = true
        checkFailures = []
        if audienceChoices.isEmpty {
            audienceChoices = ((try? await core.audienceGroups()) ?? []).filter { $0.status == .confirmed }.map(\.name)
        }
        do {
            // The account's writing guide for these recipients and this kind
            // of message (spec §14.9); the user's own draft stays part of
            // the prompt.
            guide = try? await core.guideForMessage(recipients: (store.to + store.cc).map(\.email),
                                                    messageType: Self.messageType(store), audiences: audiences)
            let session = try await core.startReadOnlyAgentSession(provider: model.agent.providerID,
                                                                   selection: store.threadID.map { [$0] } ?? [])
            sessionID = session
            model.agentSinks[session] = { [weak self] events in await self?.ingest(events) }
            let prompt = Self.prompt(instruction: text, from: store.from, to: store.to.map(\.email),
                                     subject: store.subject, draft: base, original: original,
                                     guide: guide?.text ?? "", facts: givenFacts)
            try await core.sendAgentPrompt(session, prompt)
        } catch let error as CoreClientError {
            fail(error.message)
        } catch {
            fail(error.localizedDescription)
        }
    }

    func cancel() {
        if case .asking = state {
            finish()
            state = .idle
            return
        }
        guard state == .working else { return }
        if let core = model?.core, let sessionID { Task { try? await core.cancelAgentTurn(sessionID) } }
        finish()
        state = .idle
    }

    /// Back to what the body was before the agent's last text replaced it.
    func undo() {
        guard let previousBody, let store else { return }
        store.body = previousBody
        self.previousBody = nil
        // The drafts stay for switching; which one shows is no longer known.
        shownKey = nil
        writtenFor = []
        state = .idle
    }

    func ingest(_ events: [AgentEventInfo]) async {
        for event in events {
            switch event {
            case let .textDelta(text):
                reply += text
            case let .actionProposed(actionID, _, _, _):
                // Writing help changes no mail: refuse anything that would.
                try? model?.core?.resolveAgentAction(actionID, approve: false)
            case .turnCompleted:
                await apply()
            case let .turnFailed(message):
                fail(message)
            default:
                break
            }
        }
    }

    private func apply() async {
        let text = Self.cleaned(reply)
        guard state == .working, let store else { return }
        // Facts it needs first: ask the user, keeping the session for the answers.
        if mayAsk, let questions = Self.questions(in: text), !questions.isEmpty {
            mayAsk = false
            state = .asking(questions)
            return
        }
        mayAsk = false
        guard !text.isEmpty else { return fail("The agent did not write anything. Try asking another way.") }
        let rewriting = sessionID
        // Checks run on what the agent wrote, never on what the user typed
        // (spec §14.9). A failing draft goes back once for a rewrite.
        var failures: [String] = []
        if followsGuide, let core = model?.core {
            failures = ((try? await core.checkGuideDraft(text, recipients: (store.to + store.cc).map(\.email),
                                                         messageType: Self.messageType(store), audiences: guide?.audiences))
                ?? []).map(\.message)
            // Cancelled while checking: nothing is applied.
            guard state == .working, sessionID == rewriting else { return }
            if !failures.isEmpty, !rewrote, let sessionID {
                rewrote = true
                reply = ""
                let again = Self.rewritePrompt(draft: text, failures: failures)
                if (try? await core.sendAgentPrompt(sessionID, again)) != nil { return }
            }
        }
        checkFailures = failures
        keepEdits(store)
        previousBody = store.body
        shownKey = Self.key(requestedAudiences)
        draftsByAudience[Self.key(requestedAudiences)] = text
        writtenFor = requestedAudiences ?? guide?.audiences ?? []
        store.body = NSAttributedString(string: text, attributes: [.font: ComposerHTML.bodyFont])
        instruction = ""
        state = .done
        record(text, store: store)
        finish()
    }

    /// Save the draft with the agent's text and keep what it wrote (spec
    /// §14.10); the draft records the guide version it was written under.
    private func record(_ text: String, store: ComposerStore) {
        guard let core = model?.core, let agent = model?.agent.providerID else { return }
        let version = followsGuide ? guide?.version : nil
        let instruction = lastInstruction ?? ""
        let audiences = writtenFor
        Task { [weak store] in
            guard let store else { return }
            await store.save()
            guard store.draftID != 0 else { return }
            if let version { try? await core.setDraftGuideVersion(store.draftID, version) }
            try? await core.recordWritingHelp(draftID: store.draftID, agent: agent, instruction: instruction,
                                              text: text, guideVersion: version, audiences: audiences)
        }
    }

    /// The user's answers to the agent's questions (empty: skipped), sent
    /// in the same session; with `save`, each answer is kept as a fact in the
    /// writing guide (undoable) so later drafts have it.
    func answer(_ answers: [String: String], save: Bool) async {
        guard case let .asking(questions) = state, let sessionID, let core = model?.core else { return }
        let given = questions.compactMap { q -> (FactQuestion, String)? in
            let a = (answers[q.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return a.isEmpty ? nil : (q, a)
        }
        if save, !given.isEmpty, let model {
            // Kept in Facts (spec §14.11), where later drafts find them.
            await model.facts.load()
            let known = Set(model.facts.categories.map(\.key))
            let edits: [FactEdit] = given.map { q, a in
                let category = known.contains(q.category) ? q.category : "other"
                // A fact by that label already: the answer replaces its value.
                if let old = model.facts.facts.first(where: {
                    $0.scope == .account && $0.status == .accepted && $0.category == category
                        && $0.label.lowercased() == q.fact.lowercased()
                }) {
                    return .update(id: old.id, fields: FactFields(category: category, label: old.label, value: a,
                                                                  use: old.use, asOf: old.asOf))
                }
                return .add(fields: FactFields(category: category, label: q.fact, value: a, use: .free, asOf: nil),
                            status: .accepted, source: .writingHelp)
            }
            await model.applyFactEdits(edits, actionName: given.count == 1 ? "Add Fact" : "Add Facts",
                                       notice: given.count == 1 ? "Added a fact" : "Added \(given.count) facts")
        }
        givenFacts += given.map { "\($0.0.fact): \($0.1)" }
        state = .working
        reply = ""
        do {
            try await core.sendAgentPrompt(sessionID, Self.answersPrompt(given.map { ($0.0.question, $0.1) }))
        } catch let error as CoreClientError {
            fail(error.message)
        } catch {
            fail(error.localizedDescription)
        }
    }

    /// The agent's questions, when its answer is them rather than a draft:
    /// a JSON object with a list of questions. Pure.
    static func questions(in text: String) -> [FactQuestion]? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{"), let data = trimmed.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let list = object["questions"] as? [Any] else { return nil }
        let questions = list.prefix(5).compactMap { item -> FactQuestion? in
            guard let item = item as? [String: Any],
                  let question = (item["question"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !question.isEmpty else { return nil }
            let label = ((item["label"] ?? item["fact"]) as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let category = (item["category"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
            return FactQuestion(question: question, fact: label.isEmpty ? question : label,
                                category: category.isEmpty ? "other" : category)
        }
        return questions
    }

    /// The second turn: the answers, then the draft.
    static func answersPrompt(_ answers: [(question: String, answer: String)]) -> String {
        let given = answers.isEmpty ? "The user chose not to answer." : answers.map { "- \($0.question) \($0.answer)" }
            .joined(separator: "\n")
        return """
        The user's answers:
        \(given)

        Now write the message. Use only facts the user gave, the thread or the writing guide; leave a [bracket] \
        for anything still not known. Answer with only the text of the message body.
        """
    }

    private func fail(_ message: String) {
        state = .failed(message)
        finish()
    }

    private func finish() {
        if let sessionID {
            model?.agentSinks[sessionID] = nil
            if let core = model?.core { Task { try? await core.closeAgentSession(sessionID) } }
        }
        sessionID = nil
    }

    /// What the agent is asked: the user's request, the message so far, and
    /// the message being answered, with the rules for its answer.
    /// `new`, `reply` or `forward`, as the guide scopes messages.
    static func messageType(_ store: ComposerStore) -> String {
        let subject = store.subject.trimmingCharacters(in: .whitespaces).lowercased()
        if subject.hasPrefix("fwd:") || subject.hasPrefix("fw:") { return "forward" }
        return store.inReplyTo == nil ? "new" : "reply"
    }

    /// Asked once when a draft breaks the guide's checks.
    static func rewritePrompt(draft: String, failures: [String]) -> String {
        """
        Your draft breaks the user's writing guide:
        \(failures.map { "- \($0)" }.joined(separator: "\n"))
        Rewrite it so it follows the guide, changing as little else as you can. Answer with only the text of \
        the message body.

        Your draft:
        <<<
        \(draft)
        >>>
        """
    }

    static func prompt(instruction: String, from: String, to: [String], subject: String, draft: String,
                       original: String, guide: String = "", facts: [String] = []) -> String {
        var parts = [
            "You are helping write an email in OpenAGC's composer. The user asks: \(instruction)",
            """
            Answer with only the text of the message body, ready to send: no subject line, no notes about \
            what you did, no Markdown, and do not repeat the quoted original. Do not create, change, send \
            or delete any mail, drafts or labels; you may read the thread for context.
            """,
            """
            Never invent facts. If the request needs facts you do not have (about the user, their company, \
            figures, dates, names) and they are not in the thread, the writing guide or below, do not write \
            the message yet: answer with only a JSON object, {"questions": [{"question": "What does your \
            company do?", "category": "work", "label": "What the company does"}]}, with at most five \
            questions; the category is one of identity, contact, availability, people, work, preferences \
            or other. The user answers, then you write it.
            """,
            "From: \(from)\nTo: \(to.isEmpty ? "(nobody yet)" : to.joined(separator: ", "))\nSubject: \(subject)",
            "The message so far:\n<<<\n\(draft.trimmingCharacters(in: .whitespacesAndNewlines))\n>>>",
        ]
        if !facts.isEmpty { parts.append("Facts the user gave for this message:\n" + facts.map { "- \($0)" }.joined(separator: "\n")) }
        if !guide.isEmpty { parts.append(guide) }
        let original = original.trimmingCharacters(in: .whitespacesAndNewlines)
        if !original.isEmpty {
            parts.append("The message being answered or forwarded:\n<<<\n\(String(original.prefix(12_000)))\n>>>")
        }
        return parts.joined(separator: "\n\n")
    }

    /// The answer without surrounding space or a code fence around it.
    static func cleaned(_ text: String) -> String {
        var s = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.hasPrefix("```"), s.hasSuffix("```"), s.count >= 6 {
            s = String(s.dropFirst(3).dropLast(3))
            if let newline = s.firstIndex(of: "\n"), !s[..<newline].contains(" ") { s = String(s[s.index(after: newline)...]) }
            s = s.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return s
    }

    /// The quoted original as plain text, for the prompt.
    static func plainText(fromHTML html: String) -> String {
        guard !html.isEmpty, let data = html.data(using: .utf8),
              let parsed = try? NSAttributedString(data: data, options: [
                  .documentType: NSAttributedString.DocumentType.html,
                  .characterEncoding: String.Encoding.utf8.rawValue,
              ], documentAttributes: nil)
        else { return "" }
        return parsed.string
    }
}
