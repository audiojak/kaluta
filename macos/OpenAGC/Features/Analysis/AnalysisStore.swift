import Foundation
import Observation

/// The proposals waiting (spec §14.10), shown in the Writing Guide (rules)
/// and Facts: the review's metrics, and the pairs behind the chosen
/// proposal.
@MainActor
@Observable
final class AnalysisStore {
    private(set) var queue: AnalysisQueue?
    private(set) var metrics: AnalysisMetrics?
    /// The pairs behind the chosen proposal.
    private(set) var pairs: [AnalysisPairInfo] = []
    private(set) var loaded = false
    private(set) var error: String?
    /// The proposed rule chosen in the Writing Guide: a learning
    /// decision's or a review proposal's tag; nil when a category is.
    var selection: String? {
        didSet {
            if selection != oldValue {
                showsAllPairs = false
                pairs = []
                Task { await loadPairs() }
            }
        }
    }
    /// "Why?": every pair, not just the first few.
    var showsAllPairs = false
    var watchingExpanded = false
    /// The review flow fills the Writing Guide's detail (spec §14.10),
    /// or Facts' detail, instead of the chosen category or fact.
    var reviewingRules = false
    var reviewingFacts = false
    /// How freely drafts may use a proposed fact once accepted, where the
    /// user changed it from the proposal's default.
    var factUses: [Int64: FactUse] = [:]

    /// The use a proposed fact is accepted with.
    func use(of proposal: AnalysisFactProposalInfo) -> FactUse { factUses[proposal.id] ?? proposal.use }

    @ObservationIgnored private let core: CoreClient?
    @ObservationIgnored private let reads = SerialReads()

    init(core: CoreClient?) {
        self.core = core
    }

    static func tag(_ proposal: AnalysisProposalInfo) -> String { "p\(proposal.id)" }
    /// A proposed rule's tag, not a category's (categories are "A1"...).
    static func isProposalTag(_ tag: String) -> Bool {
        guard let first = tag.first, first == "d" || first == "p" else { return false }
        return !tag.dropFirst().isEmpty && tag.dropFirst().allSatisfy(\.isNumber)
    }

    /// A proposed fact's tag, not a fact's ("a12", "g3").
    static func isFactProposalTag(_ tag: String) -> Bool {
        tag.first == "f" && !tag.dropFirst().isEmpty && tag.dropFirst().allSatisfy(\.isNumber)
    }

    /// A learning decision (a proposed guide entry).
    static func tag(_ decision: GuideEntry) -> String { "d\(decision.id)" }
    static func tag(_ proposal: AnalysisFactProposalInfo) -> String { "f\(proposal.id)" }

    func factProposal(tagged tag: String?) -> AnalysisFactProposalInfo? { factProposals.first { Self.tag($0) == tag } }

    var proposals: [AnalysisProposalInfo] { queue?.guide ?? [] }
    var factProposals: [AnalysisFactProposalInfo] { queue?.facts ?? [] }
    var watching: [AnalysisProposalInfo] { queue?.watching ?? [] }
    var learningDecisions: Int { Int(queue?.learningDecisions ?? 0) }
    /// Everything waiting a decision.
    var waiting: Int { proposals.count + factProposals.count + learningDecisions }
    var unseen: Bool { queue?.unseen ?? false }
    var unseenRules: Bool { queue?.unseenRules ?? false }
    var unseenFacts: Bool { queue?.unseenFacts ?? false }
    /// Proposed rules waiting: the learning decisions and the review's.
    var rulesWaiting: Int { learningDecisions + proposals.count }

    var selectedProposal: AnalysisProposalInfo? {
        (proposals + watching).first { Self.tag($0) == selection }
    }

    func load() async {
        await reads.run { [weak self] in await self?.read() }
    }

    private func read() async {
        guard let core else { return }
        do {
            let queue = try await core.analysisQueue()
            let metrics = try? await core.analysisMetrics()
            self.queue = queue
            self.metrics = metrics
            error = nil
            // A review proposal decided or gone: the list picks the next.
            let tags = (queue.guide + queue.watching).map(Self.tag)
            if let selection, selection.hasPrefix("p"), !tags.contains(selection) {
                self.selection = nil
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
        let read = (try? await core.analysisPairs(proposal.id)) ?? []
        // Chosen something else meanwhile: that one's pairs are coming.
        guard selectedProposal?.id == proposal.id else { return }
        pairs = read
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
