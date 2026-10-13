import SwiftUI

/// Every action a workflow offers (spec §14.13, *One command layer*). Each
/// is named once and has one implementation, which its key, its button,
/// VoiceOver's actions and, next, voice all call. The same verb has the
/// same key in every workflow.
enum WorkflowCommand: String, CaseIterable, Identifiable {
    /// Return (or Enter) outside an item: go on (start reading, make the
    /// task, write it). In an item Return is always Reply.
    case primary
    /// ⌘Return: send the email on screen.
    case send
    /// ⌥⌘Return: send and keep the conversation's task open.
    case sendKeepTask
    /// E: archive, done, let it go, read and next.
    case done
    /// T: make a task.
    case task
    /// L: later (leave in the Inbox, tomorrow, check again, keep).
    case later
    /// S: skip for now; the item moves to the end of the set.
    case skip
    /// Return (or Enter) and R in an item: reply, wherever there is one
    /// (a nudge is a reply). Return never does anything else in an item.
    case replyNow
    /// O: open in full, or back to the summary (Weekly reading).
    case openInFull
    /// I: back to the Inbox (Weekly reading).
    case backToInbox
    /// ⌘E: clear the bucket (Weekly reading): the rest of it read.
    case clearBucket
    /// C: copy the code (Agents).
    case copyCode
    /// ⌫: reject, don't send.
    case reject
    /// 1–3: pick one of Kaluta's answers.
    case pick1, pick2, pick3
    /// /: tell Kaluta what to say.
    case tellKaluta
    /// The words typed are the email, as written: no drafting.
    case ownWords
    /// Esc: back one step; from an item, back to Mail.
    case back
    /// Answers aloud and on screen what is left of the set.
    case whatsLeft
    /// Answers which workflow and item this is.
    case whereAmI

    var id: String { rawValue }

    /// The key as drawn on its control; nil for commands with no key.
    var keyLabel: String? {
        switch self {
        case .primary: "Return"
        case .send: "⌘Return"
        case .sendKeepTask: "⌥⌘Return"
        case .done: "E"
        case .task: "T"
        case .later: "L"
        case .skip: "S"
        case .replyNow: "Return"
        case .openInFull: "O"
        case .backToInbox: "I"
        case .clearBucket: "⌘E"
        case .copyCode: "C"
        case .reject: "⌫"
        case .pick1: "1"
        case .pick2: "2"
        case .pick3: "3"
        case .tellKaluta: "/"
        case .back: "Esc"
        case .whatsLeft, .whereAmI, .ownWords: nil
        }
    }

    /// The single-character keys, by character.
    private static let characters: [Character: WorkflowCommand] = [
        "e": .done, "t": .task, "l": .later, "s": .skip, "r": .replyNow, "o": .openInFull, "i": .backToInbox,
        "c": .copyCode,
        "1": .pick1, "2": .pick2, "3": .pick3, "/": .tellKaluta,
    ]

    /// The command a key press means, if any. Single keys count only with
    /// no modifier held (as in the mail list, design system *Keys*).
    static func command(key: KeyEquivalent, characters: String, modifiers: EventModifiers) -> WorkflowCommand? {
        switch key {
        case .return, .enter:
            if modifiers.contains(.command) { return modifiers.contains(.option) ? .sendKeepTask : .send }
            return modifiers.isEmpty ? .primary : nil
        case .delete, .deleteForward: return modifiers.isEmpty ? .reject : nil
        case .escape: return .back
        default: break
        }
        if modifiers == .command, characters.lowercased() == "e" { return .clearBucket }
        guard modifiers.isDisjoint(with: [.command, .control, .option]),
              let c = characters.lowercased().first, characters.count == 1 else { return nil }
        return Self.characters[c]
    }

    /// The command's name in `kind`: the label on its control, and what
    /// VoiceOver and voice call it. Workflows that name the primary action
    /// by the item (Reply, Make the task) pass their own title.
    func title(in kind: WorkflowKind) -> String {
        switch self {
        case .primary: "Continue"
        case .send: "Send"
        case .sendKeepTask: "Send, keep the task open"
        case .done:
            switch kind {
            case .triage: "Archive"
            case .replies: "No reply needed"
            case .tasks, .agents: "Done"
            case .waiting: "Let it go"
            case .reading: "Read, next"
            case .teach: "Accept"
            }
        case .task: "Make a task"
        case .later:
            switch kind {
            case .triage: "Leave in Inbox"
            case .tasks: "Tomorrow"
            case .waiting: "Check again later"
            case .reading: "Keep for later"
            default: "Later"
            }
        case .skip: "Skip for now"
        case .replyNow: kind == .waiting ? "Nudge" : "Reply"
        case .openInFull: "Open in full"
        case .backToInbox: "Back to the Inbox"
        case .clearBucket: "Clear the bucket"
        case .copyCode: "Copy the code"
        case .reject: kind == .agents ? "Don't send" : "Reject"
        case .pick1: "First answer"
        case .pick2: "Second answer"
        case .pick3: "Third answer"
        case .tellKaluta: "Tell Kaluta what to say"
        case .ownWords: "Use my words"
        case .back: "Back"
        case .whatsLeft: "What's left?"
        case .whereAmI: "Where am I?"
        }
    }
}

/// A command as one item offers it: its title for this item, and how its
/// control looks in the decision bar.
struct WorkflowAction: Identifiable, Equatable {
    let command: WorkflowCommand
    let title: String
    var style: WorkflowButton.Style = .plain
    /// Shown as a control; hidden ones still answer their key and VoiceOver.
    var shown = true
    /// Kaluta suggests this one (Triage): marked on its control.
    var suggested = false

    var id: String { command.id }
}

extension KeyEquivalent {
    /// The Enter key (the keypad's, or fn-Return): wherever Return acts,
    /// Enter acts the same.
    static let enter = KeyEquivalent("\u{3}")

    /// Return or Enter.
    var isReturn: Bool { self == .return || self == .enter }
}
