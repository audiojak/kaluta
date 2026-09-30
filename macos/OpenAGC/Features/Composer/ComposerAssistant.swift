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
                                     guide: guide?.text ?? "")
            try await core.sendAgentPrompt(session, prompt)
        } catch let error as CoreClientError {
            fail(error.message)
        } catch {
            fail(error.localizedDescription)
        }
    }

    func cancel() {
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
        // The draft records the guide version it was written under.
        if let version = guide?.version, !(guide?.text.isEmpty ?? true), let core = model?.core {
            Task { [weak store] in
                guard let store else { return }
                await store.save()
                if store.draftID != 0 { try? await core.setDraftGuideVersion(store.draftID, version) }
            }
        }
        finish()
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
                       original: String, guide: String = "") -> String {
        var parts = [
            "You are helping write an email in OpenAGC's composer. The user asks: \(instruction)",
            """
            Answer with only the text of the message body, ready to send: no subject line, no notes about \
            what you did, no Markdown, and do not repeat the quoted original. Do not create, change, send \
            or delete any mail, drafts or labels; you may read the thread for context.
            """,
            "From: \(from)\nTo: \(to.isEmpty ? "(nobody yet)" : to.joined(separator: ", "))\nSubject: \(subject)",
            "The message so far:\n<<<\n\(draft.trimmingCharacters(in: .whitespacesAndNewlines))\n>>>",
        ]
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
