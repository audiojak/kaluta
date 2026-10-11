import Foundation

/// What the window shows when no review is open (spec §14.13).
enum WorkflowHome: String {
    /// The start screen, *What's next*: home.
    case start
    /// The mail view (§14.3), one key away.
    case mail
}

/// A workflow being worked through: its set, fixed when it started, what is
/// decided, and what the current item needs on screen.
@MainActor @Observable
final class WorkflowSession {
    let kind: WorkflowKind
    private(set) var workflowSet: WorkflowSet
    /// Conversations shown so far, by thread id.
    var threads: [String: ThreadDetail] = [:]
    /// The text of each conversation's latest message from someone else.
    var bodies: [String: String] = [:]
    /// The end's score, once the set is finished.
    var score: WorkflowScore?
    private(set) var itemShownAt = Date()

    init(set: WorkflowSet) {
        kind = set.kind
        self.workflowSet = set
    }

    func replace(_ set: WorkflowSet) {
        self.workflowSet = set
        itemShownAt = .now
    }

    var items: [WorkflowItem] { workflowSet.items }
    var current: WorkflowItem? { workflowSet.items.first { $0.decision == nil } }
    var decided: Int { workflowSet.items.count { $0.decision != nil } }
    var total: Int { workflowSet.items.count }
    var isFinished: Bool { score != nil }

    /// "4 of 12": the item on screen, out of the set.
    var progress: String {
        guard total > 0 else { return "" }
        if isFinished { return "\(total) of \(total)" }
        return "\(min(decided + 1, total)) of \(total)"
    }

    /// Time on the current item, for the user's pace.
    var elapsedMillis: Int64 { Int64(Date().timeIntervalSince(itemShownAt) * 1000) }

    /// *What's left?*: the count and an estimate at `secondsPerItem`.
    func whatsLeft(secondsPerItem: Int) -> String {
        let left = total - decided
        guard left > 0 else { return "Nothing left in this set." }
        let minutes = max(1, (left * secondsPerItem + 30) / 60)
        return "\(left) of \(total) left, about \(minutes) minute\(minutes == 1 ? "" : "s")."
    }
}
