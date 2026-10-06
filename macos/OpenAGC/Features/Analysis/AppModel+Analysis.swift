import Foundation

/// The Analysis section (spec §14.10): opening it, Run Now, and deciding
/// proposals with Undo.
extension AppModel {
    /// The sidebar's Analysis entry, in place of a mailbox id.
    static let analysisMailboxID = "@analysis"

    var isAnalysis: Bool { selectedMailboxID == Self.analysisMailboxID && threads.searchQuery == nil }

    /// Analysis shows once the account has finished a learning run.
    var showsAnalysis: Bool { analysisProgress?.available == true }

    /// A daily review in progress (running or paused).
    var analysisRunActive: Bool {
        guard let status = analysisProgress?.run?.status else { return false }
        return status == .running || status == .paused
    }

    /// Open Analysis, on the learning runs' decisions if asked.
    func openAnalysis(learning: Bool = false) {
        guidePrompt = nil
        analysis.showsFacts = false
        selectedMailboxID = Self.analysisMailboxID
        if learning { analysis.selection = AnalysisStore.learningTag }
    }

    /// Looking at Analysis: read it, and clear the dot.
    func analysisShown() async {
        guard let core else { return }
        await analysis.load()
        if analysis.unseen {
            try? await core.analysisSeen()
            await analysis.load()
            await refreshAnalysisDots()
        }
    }

    /// Proposals changed (a review, a decision, an undo).
    func analysisChanged() async {
        // Seen only when the user can see it: not with the app behind.
        if isAnalysis, notifier.isAppActive() { await analysisShown() } else { await analysis.load() }
        await refreshAnalysisDots()
    }

    func refreshAnalysisDots() async {
        guard let core else { return }
        let unseen = Set(await core.accountsWithUnseenAnalysis())
        if unseen != unseenAnalysisAccounts { unseenAnalysisAccounts = unseen }
    }

    /// A review finished in some account: the notification, if wanted.
    func analysisReviewed(accountID: String?) async {
        await refreshAnalysisDots()
        guard let accountID else { return }
        // Another account's count is not loaded: no number then.
        let count: Int? = accountID == openAccountID ? analysis.proposals.filter(\.unseen).count
            + analysis.factProposals.filter(\.unseen).count : nil
        guard count.map({ $0 > 0 }) ?? unseenAnalysisAccounts.contains(accountID) else { return }
        notifier.announceAnalysis(proposals: count, accountID: accountID, today: Self.localDay(Date()))
    }

    /// The local calendar day, YYYY-MM-DD.
    static func localDay(_ date: Date) -> String {
        let c = Calendar.current.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }

    func runAnalysisNow() async {
        guard let core else { return }
        do {
            _ = try await core.startAnalysisRun(agent: agent.providerID)
        } catch {
            analysisError = error.message
        }
        analysisProgress = try? await core.analysisProgress()
    }

    func pauseAnalysis() async {
        try? await core?.pauseAnalysisRun()
        analysisProgress = try? await core?.analysisProgress()
    }

    func resumeAnalysis() async {
        guard let core else { return }
        do {
            _ = try await core.resumeAnalysisRun()
        } catch {
            analysisError = error.message
        }
        analysisProgress = try? await core.analysisProgress()
    }

    /// Accept or reject proposals: one change on the account's undo stack.
    func decideAnalysis(_ proposals: [AnalysisProposalInfo], accept: Bool) async {
        guard let core, !proposals.isEmpty else { return }
        let name = proposals.count == 1 ? "Proposal" : "Proposals"
        let notice = accept
            ? (proposals.count == 1 ? "Your writing guide is changed" : "\(proposals.count) changes made to your writing guide")
            : (proposals.count == 1 ? "Left out; it will not be proposed again" : "\(proposals.count) proposals left out")
        await recordAnalysis(accept ? "Accept \(name)" : "Reject \(name)", notice: notice) { () async throws(CoreClientError) -> GuideChange in
            try await core.decideAnalysisProposals(proposals.map(\.id), accept: accept)
        }
    }

    /// Accept a proposal as the user edited it.
    func acceptAnalysis(_ proposal: AnalysisProposalInfo, as fields: GuideEntryFields) async {
        guard let core else { return }
        await recordAnalysis("Accept Proposal", notice: "Your writing guide is changed") { () async throws(CoreClientError) -> GuideChange in
            try await core.acceptAnalysisProposal(proposal.id, as: fields)
        }
    }

    /// Leave one message's edits out of every proposal.
    func ignoreAnalysisPair(_ pair: AnalysisPairInfo) async {
        guard let core else { return }
        await recordAnalysis("Ignore Message", notice: "That message’s edits no longer count") { () async throws(CoreClientError) -> GuideChange in
            try await core.ignoreAnalysisPair(pair.compositionId)
        }
    }

    private func recordAnalysis(_ actionName: String, notice: String,
                                _ change: () async throws(CoreClientError) -> GuideChange) async {
        guard let core, let accountID = openAccountID else { return }
        do {
            let made = try await change()
            undo.record(accountID: accountID, actionName: actionName, noticeText: notice,
                        undo: { try? await core.undoGuideChange(made.changeId) },
                        redo: { try? await core.redoGuideChange(made.changeId) })
            analysisError = nil
        } catch {
            analysisError = error.message
        }
        await guide.load()
        guideProgress = try? await core.guideProgress()
        await analysisChanged()
    }
}
