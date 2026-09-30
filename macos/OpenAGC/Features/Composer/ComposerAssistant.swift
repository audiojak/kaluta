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
        guard !text.isEmpty, state != .working, let core = model.core else { return }
        self.store = store
        self.model = model
        state = .working
        reply = ""
        do {
            // The account's writing guide for these recipients and this kind
            // of message (spec §14.9); the user's own draft stays part of
            // the prompt.
            guide = try? await core.guideForMessage(recipients: (store.to + store.cc).map(\.email),
                                                    messageType: Self.messageType(store), audiences: nil)
            let session = try await core.startReadOnlyAgentSession(provider: model.agent.providerID,
                                                                   selection: store.threadID.map { [$0] } ?? [])
            sessionID = session
            model.agentSinks[session] = { [weak self] events in await self?.ingest(events) }
            let prompt = Self.prompt(instruction: text, from: store.from, to: store.to.map(\.email),
                                     subject: store.subject, draft: store.body.string, original: original,
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

    func undo() {
        guard let previousBody, let store else { return }
        store.body = previousBody
        self.previousBody = nil
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
                apply()
            case let .turnFailed(message):
                fail(message)
            default:
                break
            }
        }
    }

    private func apply() {
        let text = Self.cleaned(reply)
        guard state == .working, let store else { return }
        guard !text.isEmpty else { return fail("The agent did not write anything. Try asking another way.") }
        previousBody = store.body
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
