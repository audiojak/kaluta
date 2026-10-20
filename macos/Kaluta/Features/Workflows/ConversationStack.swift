import SwiftUI

/// The rest of a workflow email's conversation (spec §14.13): one quiet
/// line on the card, "3 earlier messages · Paul Allen, you", that C or a
/// click opens into a row per message (sender, date, its first line, its
/// files). A row opens that message in full on the card; Esc closes the
/// stack. O still opens the whole conversation in its own window.
struct ConversationStack: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let threadID: String
    /// Oldest first, without the email on the card.
    let messages: [MessageInfo]
    /// All of them came before the email on the card.
    let earlierOnly: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Space.s) {
            ConversationLine(session: session, messages: messages, earlierOnly: earlierOnly)
                // Scrolled past while open: the line pins under the header.
                .onGeometryChange(for: Bool.self) { proxy in
                    proxy.frame(in: .named(ConversationLine.scrollSpace)).maxY < 0
                } action: { passed in
                    session.conversationLinePassed = passed
                }
            if session.showsConversation {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(messages, id: \.id) { message in
                        row(message)
                        InsetRule(inset: 0)
                    }
                }
                .transition(.opacity)
            }
        }
        .font(TypeRole.workflowContext)
    }

    private func row(_ message: MessageInfo) -> some View {
        let open = session.openMessages.contains(message.id)
        // The one Reply answers: raised and marked.
        let current = open && session.answeringMessageID == message.id
        let tint = Tone.workflow(session.kind)
        let files = message.attachments.filter { !$0.isInline }
        return VStack(alignment: .leading, spacing: Space.s) {
            Button { Task { await model.toggleConversationMessage(message, session: session) } } label: {
                VStack(alignment: .leading, spacing: Space.xs) {
                    HStack(alignment: .firstTextBaseline, spacing: Space.m) {
                        Text(Self.sender(message, own: model.ownAddresses, capitalized: true))
                            .fontWeight(.semibold)
                        Text(Self.when(message))
                            .foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                    }
                    if !open {
                        Text(message.snippet)
                            .lineLimit(1)
                            .foregroundStyle(.secondary)
                        if !files.isEmpty {
                            Label(files.map(\.filename).joined(separator: ", "), systemImage: "paperclip")
                                .lineLimit(1)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .hoverHelp(open ? "Close this message" : "Open this message")
            // The highlight says it on screen; VoiceOver hears it.
            .accessibilityAddTraits(current ? .isSelected : [])
            .accessibilityHint(open ? "Closes this message" : "Opens this message")
            if open {
                if let html = session.messageBodies[message.id]?.html {
                    WorkflowEmailBody(html: html,
                                      allowRemote: model.workflowImagesAllowed(threadID, session: session, message: message),
                                      inlineImages: session.messageImages[message.id] ?? [:])
                } else {
                    // Until its body is read (or when it has none yet).
                    Text(message.snippet)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
                if !files.isEmpty {
                    AttachmentStrip(attachments: files, inset: 0)
                }
            }
        }
        .padding(.vertical, Space.m)
        // Into the card's margin, so the text does not move when selected.
        .background {
            if current {
                RoundedRectangle(cornerRadius: Radius.card)
                    .fill(tint.opacity(0.06))
                    .overlay(alignment: .leading) {
                        Capsule().fill(tint).frame(width: Self.bar).padding(.vertical, Space.m)
                    }
                    .padding(.horizontal, -Space.l)
            }
        }
        // Where P and N bring the message: its header, a fifth of the way down.
        .background(alignment: .top) {
            Color.clear.frame(height: 1).id(Self.rowID(message.id))
        }
        .animation(AppModel.itemMotion, value: current)
    }

    static let bar: CGFloat = 3

    /// A row's scroll target: P and N bring it into view.
    static func rowID(_ messageID: String) -> String { "conversation-\(messageID)" }

    /// "3 earlier messages · Paul Allen, Paul Sevilla, you"; "2 other
    /// messages · …" when some came after the email on the card. Up to
    /// three senders, in the order they first wrote, then "+ 2".
    static func summary(_ messages: [MessageInfo], earlierOnly: Bool, own: Set<String>) -> String {
        let head = head(messages, earlierOnly: earlierOnly)
        return names(messages, own: own).map { "\(head) · \($0)" } ?? head
    }

    /// "3 earlier messages"; "2 other messages" when some came after.
    static func head(_ messages: [MessageInfo], earlierOnly: Bool) -> String {
        let noun = messages.count == 1 ? "message" : "messages"
        return "\(messages.count) \(earlierOnly ? "earlier" : "other") \(noun)"
    }

    /// "Paul Allen, Paul Sevilla, you", then "+ 2"; nil with no senders.
    static func names(_ messages: [MessageInfo], own: Set<String>) -> String? {
        var names: [String] = []
        for message in messages {
            let name = sender(message, own: own, capitalized: false)
            if !names.contains(name) { names.append(name) }
        }
        guard !names.isEmpty else { return nil }
        return names.prefix(3).joined(separator: ", ") + (names.count > 3 ? " + \(names.count - 3)" : "")
    }

    /// The sender's name, or "you" for the user.
    static func sender(_ message: MessageInfo, own: Set<String>, capitalized: Bool) -> String {
        if message.isSentByMe || message.from.map({ own.contains($0.email.lowercased()) }) == true {
            return capitalized ? "You" : "you"
        }
        guard let from = message.from else { return "Unknown sender" }
        return from.name?.isEmpty == false ? from.name! : from.email
    }

    static func when(_ message: MessageInfo) -> String {
        Date(timeIntervalSince1970: TimeInterval(message.date) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}

/// The conversation's line: "3 earlier messages · 2 of 3 · names" and its
/// keys as links (Show C, Previous P, Next N). On the card, and pinned
/// under the header once scrolled past while the conversation is open.
struct ConversationLine: View {
    @Environment(AppModel.self) private var model
    let session: WorkflowSession
    let messages: [MessageInfo]
    let earlierOnly: Bool

    /// The workflow's scroll view, for telling when the line has scrolled away.
    static let scrollSpace = "workflowScroll"

    var body: some View {
            HStack(alignment: .firstTextBaseline, spacing: Space.l) {
                // What the line is about, then who wrote, quieter; the
                // names give way first when room runs short.
                Button(action: toggle) {
                    HStack(alignment: .firstTextBaseline, spacing: Space.s) {
                        Image(systemName: session.showsConversation ? "chevron.down" : "chevron.right")
                            .font(TypeRole.workflowKey)
                            .foregroundStyle(.secondary)
                            .frame(width: Space.l)
                        Text(ConversationStack.head(messages, earlierOnly: earlierOnly))
                            .fontWeight(.semibold)
                            .foregroundStyle(.primary)
                            .fixedSize()
                        // Where P and N are: "2 of 3".
                        if let at = messages.firstIndex(where: { $0.id == session.answeringMessageID }) {
                            Text("· \(at + 1) of \(messages.count)")
                                .foregroundStyle(.primary)
                                .fixedSize()
                        }
                        if let names = ConversationStack.names(messages, own: model.ownAddresses) {
                            Text("· \(names)")
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hoverHelp(session.showsConversation ? "Hide the conversation (C)" : "Show the rest of the conversation (C)")
                .accessibilityLabel(ConversationStack.summary(messages, earlierOnly: earlierOnly, own: model.ownAddresses))
                Spacer(minLength: Space.m)
                // The keys, as the email's own links below show theirs.
                link(session.showsConversation ? "Hide" : "Show", key: "C",
                     help: session.showsConversation ? "Hide the conversation (C)" : "Show the rest of the conversation (C)",
                     action: toggle)
                // Message by message, as in Gmail.
                link("Previous", key: "P", help: "Open the message before (P)") {
                    Task { await model.performWorkflow(.previousMessage) }
                }
                link("Next", key: "N", help: "Open the message after, or back to this email (N)") {
                    Task { await model.performWorkflow(.nextMessage) }
                }
            }
        .font(TypeRole.workflowContext)
    }

    private func toggle() {
        // A click works while writing too, where C is typing.
        withAnimation(AppModel.itemMotion) { session.showsConversation.toggle() }
    }

    /// A key's link: what it does, and the key, as *Open the email O*.
    private func link(_ label: String, key: String, help: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: Space.s) {
                Text(label)
                Keycap(key: key)
            }
        }
        .buttonStyle(.link)
        .fixedSize()
        .hoverHelp(help)
    }
}
