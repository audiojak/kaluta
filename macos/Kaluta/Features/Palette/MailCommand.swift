import AppKit
import SwiftUI

/// Something the mail window can be told to do: one definition for the
/// command palette (⌘K) and the list's single keys, so both name it and
/// run it the same way (design principle 15).
struct MailCommand: Identifiable {
    let id: String
    let title: String
    /// Its key as the palette shows it ("E", "⇧⌘R"), if it has one.
    var keys: String?
    /// The single key that runs it in the thread list, if any.
    var listKey: String?
    /// Why it cannot run now; the palette greys it out and says so.
    var unavailable: String?
    let run: @MainActor () -> Void
}

extension AppModel {
    /// Every command, in the order the palette lists them with nothing
    /// typed: acting on the selected mail first, then going places.
    var mailCommands: [MailCommand] {
        let none = actionTargets.isEmpty ? "Select a conversation first" : nil
        let noReply = isArchive ? Self.cannotSendReason : replyTargetMessageID == nil ? "Open a conversation first" : nil
        let noSend = isArchive ? Self.cannotSendReason : nil
        var commands: [MailCommand] = [
            MailCommand(id: "reply", title: "Reply", keys: "R", listKey: "r", unavailable: noReply) { self.reply(all: false) },
            MailCommand(id: "reply-all", title: "Reply All", keys: "A", listKey: "a", unavailable: noReply) {
                self.reply(all: true)
            },
            MailCommand(id: "forward", title: "Forward", keys: "F", listKey: "f", unavailable: noReply) { self.forward() },
            MailCommand(id: "archive", title: "Archive", keys: "E", listKey: "e", unavailable: none) {
                self.archiveSelection()
            },
            MailCommand(id: "trash", title: "Move to Trash", keys: "#", listKey: "#", unavailable: none) {
                self.trashSelection()
            },
            MailCommand(id: "read", title: "Mark as Read or Unread", keys: "U", listKey: "u", unavailable: none) {
                self.toggleReadSelection()
            },
            MailCommand(id: "star", title: "Star or Unstar", keys: "S", listKey: "s", unavailable: none) {
                self.toggleStarSelection()
            },
            MailCommand(id: "junk", title: isSpamMailbox ? "Not Junk" : "Mark as Junk", keys: "!", listKey: "!",
                        unavailable: none ?? (canJunk ? nil : "Not for mail you sent")) { self.toggleJunkSelection() },
            MailCommand(id: "inbox", title: "Move to Inbox", unavailable: none) { self.moveSelectionToInbox() },
            MailCommand(id: "watch",
                        title: !actionTargets.isEmpty && actionTargets.allSatisfy(watched.contains)
                            ? "Stop Telling Me About Replies" : "Tell Me When They Reply",
                        unavailable: none) { Task { await self.toggleWatchSelection() } },
            MailCommand(id: "task", title: "New Task from Email…", keys: "T", listKey: "t", unavailable: none) {
                Task { await self.openTaskDialog() }
            },
            MailCommand(id: "tasks", title: "Create Tasks…", keys: "⇧T", listKey: "T") {
                Task { await self.openBulkTasks() }
            },
            MailCommand(id: "new", title: "New Message", keys: "C", listKey: "c", unavailable: noSend) {
                self.compose(.new(to: nil))
            },
            MailCommand(id: "search", title: "Search Mail", keys: "/", listKey: "/") { self.focusSearch() },
            MailCommand(id: "check", title: "Check for New Mail", keys: "⇧⌘N") { self.core?.syncNow() },
            MailCommand(id: "undo", title: undo.undoTitle(in: openAccountID), keys: "⌘Z",
                        unavailable: undo.canUndo(in: openAccountID) ? nil : "Nothing to undo") { self.undoMailAction() },
            MailCommand(id: "agent", title: agent.isPresented ? "Hide Agent" : "Show Agent", keys: "⌥⌘I") {
                self.agent.isPresented.toggle()
            },
        ]
        commands += WorkflowKind.all.map { kind in
            MailCommand(id: "workflow-\(kind)", title: "Start \(kind.title)") { Task { await self.beginWorkflow(kind) } }
        }
        commands += snippetCommands(paletteSnippets)
        commands += (mailboxes.systemMailboxes + mailboxes.labels).map { mailbox in
            MailCommand(id: "go-\(mailbox.id)", title: "Go to \(mailbox.name)") { self.selectedMailboxID = mailbox.id }
        }
        return commands
    }

    /// The command a single key in the thread list runs, if any.
    func listCommand(for key: String) -> MailCommand? {
        mailCommands.first { $0.listKey == key }
    }

    /// ⌘K: the command palette, in the mail window (brought forward).
    func showPalette() {
        if let main = NSApp.windows.first(where: { $0.identifier?.rawValue.hasPrefix("main") == true }),
           NSApp.keyWindow !== main {
            main.makeKeyAndOrderFront(nil)
        }
        paletteShown = true
        Task { paletteSnippets = (try? await core?.snippets()) ?? [] }
    }

    /// The palette's last row: what was typed, asked of Kaluta.
    func askFromPalette(_ text: String) {
        agent.isPresented = true
        Task { await askAgent(text) }
    }
}

/// What the palette lists for what is typed: commands whose names match,
/// best first, then "Ask Kaluta" with the text.
struct PaletteModel {
    enum Row: Identifiable {
        case command(MailCommand)
        case ask(String)

        var id: String {
            switch self {
            case let .command(command): command.id
            case .ask: "ask"
            }
        }

        var title: String {
            switch self {
            case let .command(command): command.title
            case let .ask(text): "Ask Kaluta: \(text)"
            }
        }

        var unavailable: String? {
            if case let .command(command) = self { command.unavailable } else { nil }
        }
    }

    let rows: [Row]

    init(commands: [MailCommand], query: String) {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else {
            rows = commands.map(Row.command)
            return
        }
        let ranked = commands.enumerated().compactMap { index, command in
            Self.rank(command.title, query).map { (rank: $0, index: index, command: command) }
        }
        .sorted { ($0.rank, $0.index) < ($1.rank, $1.index) }
        rows = ranked.map { Row.command($0.command) } + [.ask(query)]
    }

    /// How well a name matches what is typed: 0 it starts with it, 1 one
    /// of its words does, 2 it contains it; nil it does not match.
    static func rank(_ title: String, _ query: String) -> Int? {
        let title = title.lowercased(), query = query.lowercased()
        if title.hasPrefix(query) { return 0 }
        if title.split(separator: " ").contains(where: { $0.hasPrefix(query) }) { return 1 }
        return title.contains(query) ? 2 : nil
    }

    /// The first row Return can take: the best command that can run, else
    /// asking Kaluta.
    var firstRunnable: Int {
        rows.firstIndex { $0.unavailable == nil } ?? 0
    }

    /// The row ↑ or ↓ moves to from `index`, staying inside the list.
    func moved(from index: Int, by delta: Int) -> Int {
        rows.isEmpty ? 0 : min(max(index + delta, 0), rows.count - 1)
    }
}
