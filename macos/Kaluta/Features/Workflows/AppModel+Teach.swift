import SwiftUI

/// Teach Kaluta as a workflow (spec §14.13): one proposal at a time, the
/// change it makes large and its evidence on O, Accept (E), Edit… (W),
/// Reject (⌫) and Skip (S). Each decision and its change to the guide,
/// the facts or the routine are one ⌘Z.
extension AppModel {
    /// A Teach Kaluta item as the stores hold it.
    enum TeachItem {
        case routine(RoutineProposalInfo)
        case rule(GuideEntry)
        case proposal(AnalysisProposalInfo)
        case fact(AnalysisFactProposalInfo)
    }

    /// The item behind a set's key (`routine:`, `rule:`, `proposal:` or
    /// `fact:` and its id); nil once it no longer waits.
    func teachItem(_ item: WorkflowItem) -> TeachItem? {
        let parts = item.key.split(separator: ":", maxSplits: 1)
        guard parts.count == 2, let id = Int64(parts[1]) else { return nil }
        switch parts[0] {
        case "routine": return routineProposals.first { $0.id == id }.map(TeachItem.routine)
        case "rule": return guide.decisions.first { $0.id == id }.map(TeachItem.rule)
        case "proposal": return analysis.proposals.first { $0.id == id }.map(TeachItem.proposal)
        case "fact": return analysis.factProposals.first { $0.id == id }.map(TeachItem.fact)
        default: return nil
        }
    }

    /// The rail's line for each item still waiting.
    func noteTeachLines(_ session: WorkflowSession) {
        for item in session.items {
            guard let t = teachItem(item) else { continue }
            session.teachLines[item.key] = Self.teachLine(t)
        }
    }

    static func teachLine(_ t: TeachItem) -> TeachLine {
        switch t {
        case let .routine(p): TeachLine(title: p.statement, kind: "Routine change")
        case let .rule(entry): TeachLine(title: entry.statement, kind: entry.kind.title)
        case let .proposal(p): TeachLine(title: p.statement, kind: p.title)
        case let .fact(p): TeachLine(title: p.headline, kind: "Fact")
        }
    }

    /// Everything Teach Kaluta shows, as the stores hold it now.
    func loadTeach() async {
        await analysis.load()
        await guide.load()
        await loadRoutineProposals()
    }

    /// The item's evidence, when it has some to show on O.
    static func hasEvidence(_ t: TeachItem) -> Bool {
        switch t {
        case let .routine(p): !p.evidence.isEmpty
        case let .rule(entry): entry.evidence.contains { !$0.contradicts }
        case let .proposal(p): p.support > 0
        case let .fact(p): !p.quote.isEmpty
        }
    }

    /// What the current item offers: Accept, Edit… where the item can be
    /// edited, Reject, the evidence and Skip.
    func teachActions(_ session: WorkflowSession,
                      _ a: (WorkflowCommand, WorkflowButton.Style, String?) -> WorkflowAction) -> [WorkflowAction] {
        guard let item = session.current, let t = teachItem(item) else {
            // Decided on its page meanwhile: on to the next.
            return [a(.done, .primary, "Next")]
        }
        var actions = [a(.done, .primary, "Accept")]
        switch t {
        case .rule: actions.append(a(.edit, .plain, nil))
        case let .proposal(p) where p.op != .remove: actions.append(a(.edit, .plain, nil))
        default: break
        }
        actions.append(a(.reject, .plain, "Reject"))
        if Self.hasEvidence(t) {
            actions.append(a(.openInFull, .plain, session.showsInFull ? "Hide the evidence" : nil))
        }
        return actions + [a(.skip, .quiet, nil)]
    }

    /// A proposal's AI drafts and what was sent, for its evidence.
    func prepareTeach(_ item: WorkflowItem) async {
        guard case let .proposal(p) = teachItem(item) else { return }
        analysis.selection = AnalysisStore.tag(p)
        analysis.showsAllPairs = false
        await analysis.loadPairs()
    }

    func performTeach(_ command: WorkflowCommand, item: WorkflowItem, session: WorkflowSession) async {
        let t = teachItem(item)
        switch command {
        case .done:
            if t == nil {
                await decideWorkflowItem(item, "elsewhere", session: session, notice: nil)
            } else {
                await decideTeach(item, accept: true, session: session)
            }
        case .reject:
            await decideTeach(item, accept: false, session: session)
        case .edit:
            switch t {
            case let .rule(entry): guideSheet = .edit(entry, category: entry.category)
            case let .proposal(p): guideSheet = .proposal(p)
            default: break
            }
        case .openInFull:
            withAnimation(Self.itemMotion) { session.showsInFull.toggle() }
        default:
            break
        }
    }

    /// An edit sheet closed: what it decided leaves the set.
    func teachEdited() async {
        guard let session = workflowSession, session.kind == .teach else { return }
        await teachChanged()
        await refreshWorkflow()
    }

    /// Accept or reject: the change, then the decision, with one ⌘Z for
    /// both. A routine change with no routine to change says why.
    private func decideTeach(_ item: WorkflowItem, accept: Bool, session: WorkflowSession) async {
        guard let core, let accountID = openAccountID, let t = teachItem(item) else { return }
        let undoStep: @MainActor () async -> Void
        let redoStep: @MainActor () async -> Void
        let notice: String
        do {
            switch t {
            case let .routine(p):
                try await core.decideRoutineProposal(p.id, accept: accept)
                undoStep = { try? await core.reopenRoutineProposal(p.id) }
                redoStep = { try? await core.decideRoutineProposal(p.id, accept: accept) }
                notice = accept ? "The routine now leaves this mail alone" : "Left the routine as it is"
            case let .rule(entry):
                var edits: [GuideEdit] = [.decide(id: entry.id, status: accept ? .accepted : .rejected)]
                // One that goes against an accepted entry replaces it.
                if accept, let old = entry.contradictionOf.flatMap({ id in guide.entries.first { $0.id == id } }),
                   old.status == .accepted {
                    edits.append(.decide(id: old.id, status: .rejected))
                }
                let change = try await core.applyGuideEdits(edits, reason: !accept ? "reject"
                                                            : edits.count > 1 ? "replace" : "accept")
                undoStep = { try? await core.undoGuideChange(change.changeId) }
                redoStep = { try? await core.redoGuideChange(change.changeId) }
                notice = accept ? "Added to your writing guide" : "Left out of your writing guide"
            case let .proposal(p):
                let change = try await core.decideAnalysisProposals([p.id], accept: accept)
                undoStep = { try? await core.undoGuideChange(change.changeId) }
                redoStep = { try? await core.redoGuideChange(change.changeId) }
                notice = accept ? "Your writing guide is changed" : "Left out; it will not be proposed again"
            case let .fact(p):
                let change = try await core.decideFactProposals([p.id], accept: accept,
                                                                uses: [p.id: analysis.use(of: p)])
                undoStep = { try? await core.undoFactChange(change.changeId) }
                redoStep = { try? await core.redoFactChange(change.changeId) }
                notice = accept ? "Added to your facts" : "Left out; it will not be proposed again"
            }
        } catch {
            undo.show(error.message, accountID: accountID, offersUndo: false)
            return
        }
        await teachChanged()
        await decideWorkflowItem(item, accept ? "accepted" : "rejected", session: session, notice: notice,
                                 undoMail: { [weak self] in await undoStep(); await self?.teachChanged() },
                                 redoMail: { [weak self] in await redoStep(); await self?.teachChanged() })
    }

    /// The guide, the facts and the routine changes as they are now.
    func teachChanged() async {
        await guide.load()
        guideProgress = try? await core?.guideProgress()
        await analysisChanged()
        await facts.load()
        await loadRoutineProposals()
    }
}
