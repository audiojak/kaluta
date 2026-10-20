import SwiftUI

/// What the window shows when no review is open (spec §14.13).
enum WorkflowHome: String {
    /// The start screen, *What's next*: home.
    case start
    /// The mail view (§14.3), one key away.
    case mail
}

/// Which way a decided item leaves (spec §14.13): archive left, reply
/// right, task down; leaving it, putting it off or skipping it, up.
enum WorkflowExit {
    case left, right, down, up

    static func of(_ decision: String) -> WorkflowExit {
        switch decision {
        case "archive", "done": .left
        case "reply": .right
        case "task": .down
        default: .up
        }
    }

    /// How far it travels as it fades.
    var offset: CGSize {
        switch self {
        case .left: CGSize(width: -Self.distance, height: 0)
        case .right: CGSize(width: Self.distance, height: 0)
        case .down: CGSize(width: 0, height: Self.distance)
        case .up: CGSize(width: 0, height: -Self.distance)
        }
    }

    static let distance: CGFloat = 140
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
    /// That message as sanitized HTML, shown with its own formatting.
    var rendered: [String: RenderedBody] = [:]
    /// Conversations whose remote images the user loaded here.
    var imagesLoaded: Set<String> = []
    /// Each email's inline images, by thread id.
    var inlineImages: [String: [String: InlineImage]] = [:]
    /// Senders the user has written to (their images may load by themselves).
    var knownSenders: Set<String> = []
    /// The end's score, once the set is finished.
    var score: WorkflowScore?
    private(set) var itemShownAt = Date()
    /// Which way the item on screen leaves when it is decided.
    var exit = WorkflowExit.left
    /// Kaluta's writing for the item on screen, once the user answers it.
    var writer: WorkflowWriter?
    /// A task being made on the item's screen (Triage).
    var taskForm: WorkflowTaskForm?
    /// Every task in a Tasks set, finished ones too, by id: the rail.
    var taskInfo: [Int64: TaskItem] = [:]
    /// Weekly reading: the report, Kaluta's summaries by thread id, whether
    /// the report is showing (before the first bucket), and whether the
    /// item shows in full rather than summarised.
    var reading: ReadingReport?
    var summaries: [String: ReadingSummary] = [:]
    var showsReport = false
    var showsInFull = false
    /// The conversation's other messages are open on the card (C), and
    /// which of them are open in full, by message id.
    var showsConversation = false {
        // Closed: replies answer the email on the card again.
        didSet { if !showsConversation { answeringMessageID = nil } }
    }
    var openMessages: Set<String> = []
    /// The conversation's message last opened on the card: what Reply (R,
    /// Return), Reply all (A) and Forward (F) answer. Nil: the card's email.
    var answeringMessageID: String?
    /// The key list (?) is open.
    var showsKeys = false
    /// The conversation's line has scrolled above the top: it pins there.
    var conversationLinePassed = false
    /// Those messages as sanitized HTML, and their inline images, by
    /// message id.
    var messageBodies: [String: RenderedBody] = [:]
    var messageImages: [String: [String: InlineImage]] = [:]
    /// Agents: codes and what agent mailboxes sent, and the editable text
    /// of each proposed send, by item key.
    var agentsWork: AgentsWork?
    var agentDrafts: [String: String] = [:]
    /// Each draft's reply as the agent wrote it and the quoted original
    /// below it, which the edit leaves as it was.
    var agentDraftReplies: [String: String] = [:]
    var agentDraftQuotes: [String: String] = [:]
    /// The morning run's record, by item key.
    var agentRuns: [String: RoutineRunInfo] = [:]
    /// Each conversation's wait (Waiting on others), by thread id.
    var waits: [String: WaitingInfo] = [:]
    /// *Check again* is choosing its day.
    var choosingCheckAgain = false
    /// Kaluta's triage suggestions, by thread id, as each item is shown.
    var suggestions: [String: TriageSuggestion] = [:]
    /// What the user sent the sender since, in another conversation (Triage,
    /// Replies): they may have answered there. By thread id.
    var writtenSince: [String: WrittenSince] = [:]
    /// Teach Kaluta's rail: each item's line and kind, by key, kept from
    /// the start (a decided proposal leaves its store).
    var teachLines: [String: TeachLine] = [:]

    init(set: WorkflowSet) {
        kind = set.kind
        self.workflowSet = set
    }

    func replace(_ set: WorkflowSet) {
        if set.items.first(where: { $0.decision == nil })?.key != current?.key {
            writer = nil
            taskForm = nil
            choosingCheckAgain = false
            showsInFull = false
            showsConversation = false
            openMessages = []
            answeringMessageID = nil
            conversationLinePassed = false
        }
        self.workflowSet = set
        itemShownAt = .now
    }

    var items: [WorkflowItem] { workflowSet.items }
    var current: WorkflowItem? { workflowSet.items.first { $0.decision == nil } }
    var decided: Int { workflowSet.items.count { $0.decision != nil } }
    var total: Int { workflowSet.items.count }
    var isFinished: Bool { score != nil }

    /// A filed conversation's subject, from the reading report.
    func readingSubject(_ threadID: String) -> String? {
        for b in reading?.buckets ?? [] {
            if let i = b.threadIds.firstIndex(of: threadID), i < b.subjects.count { return b.subjects[i] }
        }
        return nil
    }

    /// A field has the keyboard's work: what to say, the email to send, or
    /// a task being made. The workflow's own keys stand aside meanwhile.
    var isTyping: Bool {
        if taskForm != nil { return true }
        guard let step = writer?.step else { return false }
        return step == .say || step == .final
    }

    /// The reading bucket an item is in.
    func bucket(of item: WorkflowItem) -> ReadingBucket? {
        guard let id = item.threadId else { return nil }
        return reading?.buckets.first { $0.dueToday && $0.threadIds.contains(id) }
    }

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

/// A Teach Kaluta item on the rail: what it says and what kind it is.
struct TeachLine: Equatable {
    let title: String
    let kind: String
}
