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
            HStack(spacing: Space.l) {
                Button {
                    // A click works while writing too, where C is typing.
                    withAnimation(AppModel.itemMotion) { session.showsConversation.toggle() }
                } label: {
                    HStack(spacing: Space.s) {
                        Image(systemName: session.showsConversation ? "chevron.down" : "chevron.right")
                            .font(TypeRole.workflowKey)
                            .frame(width: Space.l)
                        Text(Self.summary(messages, earlierOnly: earlierOnly, own: model.ownAddresses))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .layoutPriority(-1)
                        hint(session.showsConversation ? "Hide" : "Show", key: "C")
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .hoverHelp(session.showsConversation ? "Hide the conversation (C)" : "Show the rest of the conversation (C)")
                // Message by message, as in Gmail.
                Button { Task { await model.performWorkflow(.previousMessage) } } label: { hint("Previous", key: "P") }
                    .buttonStyle(.plain)
                    .hoverHelp("Open the message before (P)")
                Button { Task { await model.performWorkflow(.nextMessage) } } label: { hint("Next", key: "N") }
                    .buttonStyle(.plain)
                    .hoverHelp("Open the message after, or back to this email (N)")
            }
            .foregroundStyle(.secondary)
            if session.showsConversation {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(messages, id: \.id) { message in
                        row(message)
                            .id(Self.rowID(message.id))
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
                actions(message)
            }
        }
        .padding(.vertical, Space.m)
    }

    /// A word for what a key does, and the key.
    private func hint(_ label: String, key: String) -> some View {
        HStack(spacing: Space.xs) {
            Text(label)
            Keycap(key: key)
        }
        .fixedSize()
    }

    /// The message's own Reply, Reply all and Forward; on the one being
    /// answered, with their keys.
    private func actions(_ message: MessageInfo) -> some View {
        let current = session.answeringMessageID == message.id
        return HStack(spacing: Space.m) {
            action("Reply", key: "R", current: current) { answer(message, .replyNow) }
            if ReaderStore.offersReplyAll(message, own: model.ownAddresses) {
                action("Reply all", key: "A", current: current) { answer(message, .replyAll) }
            }
            action("Forward", key: "F", current: current) { answer(message, .forward) }
        }
        .padding(.top, Space.xs)
    }

    private func action(_ title: String, key: String, current: Bool, perform: @escaping () -> Void) -> some View {
        Button(action: perform) {
            HStack(spacing: Space.s) {
                Text(title)
                if current { Keycap(key: key) }
            }
        }
        .buttonStyle(.bordered)
        .hoverHelp(current ? "\(title) to this message (\(key))" : "\(title) to this message")
    }

    /// This message, then the command, as its key would.
    private func answer(_ message: MessageInfo, _ command: WorkflowCommand) {
        session.answeringMessageID = message.id
        Task { await model.performWorkflow(command) }
    }

    /// A row's scroll target: P and N bring it into view.
    static func rowID(_ messageID: String) -> String { "conversation-\(messageID)" }

    /// "3 earlier messages · Paul Allen, Paul Sevilla, you"; "2 other
    /// messages · …" when some came after the email on the card. Up to
    /// three senders, in the order they first wrote, then "+ 2".
    static func summary(_ messages: [MessageInfo], earlierOnly: Bool, own: Set<String>) -> String {
        let noun = messages.count == 1 ? "message" : "messages"
        let head = "\(messages.count) \(earlierOnly ? "earlier" : "other") \(noun)"
        var names: [String] = []
        for message in messages {
            let name = sender(message, own: own, capitalized: false)
            if !names.contains(name) { names.append(name) }
        }
        guard !names.isEmpty else { return head }
        let shown = names.prefix(3).joined(separator: ", ")
        return "\(head) · \(shown)\(names.count > 3 ? " + \(names.count - 3)" : "")"
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
