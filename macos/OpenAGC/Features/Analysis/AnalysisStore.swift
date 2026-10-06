import Foundation
import Observation

/// What the Analysis section shows (spec §14.10): the proposals waiting,
/// the metrics, and the pairs behind the chosen proposal.
@MainActor
@Observable
final class AnalysisStore {
    /// The list's selection: `learning` (the learning runs' decisions) or a
    /// proposal's tag.
    static let learningTag = "learning"

    private(set) var queue: AnalysisQueue?
    private(set) var metrics: AnalysisMetrics?
    /// The pairs behind the chosen proposal.
    private(set) var pairs: [AnalysisPairInfo] = []
    private(set) var loaded = false
    private(set) var error: String?
    var selection: String? {
        didSet { if selection != oldValue { showsAllPairs = false; Task { await loadPairs() } } }
    }
    /// "Why?": every pair, not just the first few.
    var showsAllPairs = false
    var watchingExpanded = false

    @ObservationIgnored private let core: CoreClient?
    /// One read at a time; a load asked for meanwhile reads once more after
    /// it, and returns only when that read is applied.
    @ObservationIgnored private var reading: Task<Void, Never>?
    @ObservationIgnored private var readAgain = false

    init(core: CoreClient?) {
        self.core = core
    }

    static func tag(_ proposal: AnalysisProposalInfo) -> String { "p\(proposal.id)" }

    var proposals: [AnalysisProposalInfo] { queue?.guide ?? [] }
    var watching: [AnalysisProposalInfo] { queue?.watching ?? [] }
    var learningDecisions: Int { Int(queue?.learningDecisions ?? 0) }
    /// Everything waiting a decision.
    var waiting: Int { proposals.count + learningDecisions }
    var unseen: Bool { queue?.unseen ?? false }

    var selectedProposal: AnalysisProposalInfo? {
        (proposals + watching).first { Self.tag($0) == selection }
    }

    func load() async {
        if let reading {
            readAgain = true
            await reading.value
            return
        }
        let task = Task {
            repeat {
                readAgain = false
                await read()
            } while readAgain
        }
        reading = task
        await task.value
        reading = nil
    }

    private func read() async {
        guard let core else { return }
        do {
            let queue = try await core.analysisQueue()
            let metrics = try? await core.analysisMetrics()
            self.queue = queue
            self.metrics = metrics
            error = nil
            // Keep the selection while it is there; else the first thing waiting.
            let tags = [queue.learningDecisions > 0 ? Self.learningTag : nil].compactMap { $0 }
                + (queue.guide + queue.watching).map(Self.tag)
            if selection.map({ !tags.contains($0) }) ?? true {
                selection = tags.first
            } else {
                await loadPairs()
            }
        } catch {
            self.error = error.message
        }
        loaded = true
    }

    func loadPairs() async {
        guard let core, let proposal = selectedProposal else { pairs = []; return }
        pairs = (try? await core.analysisPairs(proposal.id)) ?? []
    }
}

extension AnalysisProposalInfo {
    /// "New guideline", "Change", "Narrow or widen", "Remove".
    var title: String {
        switch op {
        case .add: kind == .rule ? "New rule" : "New guideline"
        case .edit: "Change"
        case .rescope: "Change where it applies"
        case .remove: "Remove"
        }
    }

    var symbol: String {
        switch op {
        case .add: "plus.circle"
        case .edit: "pencil.circle"
        case .rescope: "scope"
        case .remove: "minus.circle"
        }
    }

    /// "Seen in 4 messages".
    var strength: String { support == 1 ? "Seen in 1 message" : "Seen in \(support) messages" }
}
