import AppKit
import Observation
import SwiftUI

/// Rewrite in place (⌘J; spec §14.5 amendment 2026-11-18): Kaluta rewrites
/// only the selected text (the paragraph, with nothing selected) and it
/// takes the selection's place; the rest of the message is untouched.
/// The change is the text view's own, so ⌘Z takes it back. Like writing
/// help it runs in a read-only session that can change no mail.
@MainActor
@Observable
final class SelectionRewriter {
    enum State: Equatable {
        case idle, working, failed(String)
    }

    /// What can be asked, in the popover's order.
    static let requests = ["Improve the writing", "Fix spelling and grammar", "Make it shorter", "Make it longer"]

    private(set) var state: State = .idle
    @ObservationIgnored private var sessionID: String?
    @ObservationIgnored private var reply = ""
    @ObservationIgnored private weak var textView: NSTextView?
    @ObservationIgnored private var range = NSRange()
    @ObservationIgnored private var original = ""
    @ObservationIgnored private var request = ""
    @ObservationIgnored private weak var model: AppModel?
    @ObservationIgnored private weak var store: ComposerStore?

    /// What ⌘J acts on: the selection, else the paragraph the cursor is in
    /// (without its line break).
    static func target(in text: NSString, selection: NSRange) -> NSRange {
        if selection.length > 0 { return selection }
        let paragraph = text.paragraphRange(for: NSRange(location: min(selection.location, text.length), length: 0))
        var end = NSMaxRange(paragraph)
        while end > paragraph.location, [0x0A, 0x0D, 0x2028, 0x2029].contains(text.character(at: end - 1)) { end -= 1 }
        return NSRange(location: paragraph.location, length: end - paragraph.location)
    }

    /// Shows the requests at the selection (⌘J).
    func present(in textView: NSTextView, store: ComposerStore, model: AppModel) {
        guard state != .working else { return }
        let target = Self.target(in: textView.string as NSString, selection: textView.selectedRange())
        guard target.length > 0, let layout = textView.layoutManager, let container = textView.textContainer else { return }
        let glyphs = layout.glyphRange(forCharacterRange: target, actualCharacterRange: nil)
        var rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
        rect.origin.x += textView.textContainerOrigin.x
        rect.origin.y += textView.textContainerOrigin.y
        let popover = NSPopover()
        popover.behavior = .transient
        let menu = RewriteMenuView { [weak self, weak textView] request in
            popover.performClose(nil)
            guard let self, let textView else { return }
            Task { await self.rewrite(target, in: textView, request: request, store: store, model: model) }
        }
        popover.contentViewController = NSHostingController(rootView: menu)
        popover.show(relativeTo: rect, of: textView, preferredEdge: .maxY)
    }

    func rewrite(_ range: NSRange, in textView: NSTextView, request: String, store: ComposerStore, model: AppModel) async {
        guard let core = model.core, state != .working, NSMaxRange(range) <= (textView.string as NSString).length else { return }
        self.textView = textView
        self.range = range
        self.model = model
        self.store = store
        self.request = request
        original = (textView.string as NSString).substring(with: range)
        reply = ""
        state = .working
        do {
            let guide = try? await core.guideForMessage(recipients: (store.to + store.cc).map(\.email),
                                                        messageType: ComposerAssistant.messageType(store), audiences: nil)
            let session = try await core.startReadOnlyAgentSession(provider: model.agent.providerID,
                                                                   selection: store.threadID.map { [$0] } ?? [])
            sessionID = session
            model.agentSinks[session] = { [weak self] events in await self?.ingest(events) }
            try await core.sendAgentPrompt(session, Self.prompt(request: request, selection: original,
                                                                message: textView.string, guide: guide?.text ?? ""))
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

    func ingest(_ events: [AgentEventInfo]) async {
        for event in events {
            switch event {
            case let .textDelta(text):
                reply += text
            case let .actionProposed(actionID, _, _, _):
                // Rewriting changes no mail: refuse anything that would.
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

    /// The rewrite in place of the selection, if the selection is still as
    /// it was: the user may have typed meanwhile.
    private func apply() {
        defer { finish() }
        guard state == .working, let textView else { return }
        let text = ComposerAssistant.cleaned(reply)
        guard !text.isEmpty else { return fail("Kaluta did not write anything. Try again.") }
        let now = textView.string as NSString
        guard NSMaxRange(range) <= now.length, now.substring(with: range) == original else {
            return fail("The text changed while Kaluta was writing, so it was left as it is.")
        }
        // In the selection's own style: the rest of the message keeps its.
        let attributes = textView.textStorage?.attributes(at: range.location, effectiveRange: nil)
            ?? [.font: ComposerHTML.bodyFont, .foregroundColor: NSColor.textColor]
        textView.insertText(NSAttributedString(string: text, attributes: attributes), replacementRange: range)
        textView.setSelectedRange(NSRange(location: range.location, length: (text as NSString).length))
        state = .idle
        record(textView.string)
    }

    /// Kept as writing help (spec §14.10, ADR 0013): the message as Kaluta
    /// left it.
    private func record(_ body: String) {
        guard let core = model?.core, let agent = model?.agent.providerID, let store else { return }
        let request = request
        Task { [weak store] in
            guard let store else { return }
            await store.save()
            guard store.draftID != 0 else { return }
            try? await core.recordWritingHelp(draftID: store.draftID, agent: agent, instruction: request,
                                              text: body, guideVersion: nil, audiences: [])
        }
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

    static func prompt(request: String, selection: String, message: String, guide: String) -> String {
        var parts = [
            "You are helping edit part of an email in Kaluta's composer. The user asks: \(request)",
            """
            Rewrite only the passage below, keeping its meaning, facts, names and figures, and the language \
            it is in. Answer with only the rewritten passage: no quotation marks, no notes, no Markdown, \
            nothing from the rest of the message. Do not create, change, send or delete any mail, drafts or \
            labels.
            """,
            "The passage to rewrite:\n<<<\n\(selection)\n>>>",
            "The whole message, for context only:\n<<<\n\(String(message.prefix(12_000)))\n>>>",
        ]
        if !guide.isEmpty { parts.append(guide) }
        return parts.joined(separator: "\n\n")
    }
}

/// The ⌘J popover: what to do with the selection, or a request of one's own.
struct RewriteMenuView: View {
    let choose: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            ForEach(Array(SelectionRewriter.requests.enumerated()), id: \.offset) { index, request in
                Button { choose(request) } label: {
                    HStack {
                        Text(request)
                        Spacer(minLength: Space.m)
                        Keycap(key: "\(index + 1)")
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .command)
                .hoverHelp("\(request) (⌘\(index + 1))")
            }
            TextField("Or say how to change it", text: $text)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit {
                    let request = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    if !request.isEmpty { choose(request) }
                }
            KeyHints(hints: [KeyHint(keys: "⌘1–4", does: "Choose"), KeyHint(keys: "Return", does: "Ask"),
                             KeyHint(keys: "Esc", does: "Close")])
        }
        .padding(Space.m)
        .frame(width: 300)
        .onAppear { focused = true }
    }
}
