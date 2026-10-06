import Foundation

/// Proposals (spec §14.10): proposed rules wait in the Writing Guide and
/// proposed facts in Facts. Run Now, and deciding proposals with Undo.
extension AppModel {
    /// The daily review runs once the account has finished a learning run.
    var reviewsAvailable: Bool { analysisProgress?.available == true }

    /// A daily review in progress (running or paused).
    var analysisRunActive: Bool {
        guard let status = analysisProgress?.run?.status else { return false }
        return status == .running || status == .paused
    }

    /// Open the Writing Guide's review flow on its first proposed rule (the
    /// learning decisions first when asked).
    func openProposedRules(learning: Bool = false) {
        guidePrompt = nil
        selectedMailboxID = Self.guideMailboxID
        analysis.reviewingRules = true
        let first = learning ? guide.decisions.first.map(AnalysisStore.tag) : nil
        analysis.selection = first ?? proposedRuleTags.first
    }

    /// The Writing Guide's proposed rules, in list order.
    var proposedRuleTags: [String] {
        guide.decisions.map(AnalysisStore.tag) + analysis.proposals.map(AnalysisStore.tag)
    }

    /// The learning decision chosen in the Writing Guide.
    var selectedDecision: GuideEntry? {
        guard let tag = analysis.selection else { return nil }
        return guide.decisions.first { AnalysisStore.tag($0) == tag }
    }

    /// After deciding the chosen rule, the next one waiting (else the one
    /// before), so the user can go down the list.
    private func selectNextProposedRule(after tag: String?, in before: [String]) {
        guard let tag, let at = before.firstIndex(of: tag) else { return }
        let left = proposedRuleTags
        analysis.selection = before[(at + 1)...].first { left.contains($0) }
            ?? before[..<at].last { left.contains($0) }
    }

    /// Accept a learning decision; one that goes against an accepted entry
    /// replaces it, as its default button says.
    func acceptDecision(_ entry: GuideEntry) async {
        let before = proposedRuleTags
        if let old = entry.contradictionOf.flatMap({ id in guide.entries.first { $0.id == id } }), old.status == .accepted {
            await replaceGuideEntry(old, with: entry)
        } else {
            await decideGuide(entry, accept: true)
        }
        await analysis.load()
        selectNextProposedRule(after: AnalysisStore.tag(entry), in: before)
    }

    func decideProposedRule(reject entry: GuideEntry) async {
        let before = proposedRuleTags
        await decideGuide(entry, accept: false)
        await analysis.load()
        selectNextProposedRule(after: AnalysisStore.tag(entry), in: before)
    }

    func decideProposedRule(_ proposal: AnalysisProposalInfo, accept: Bool) async {
        let before = proposedRuleTags
        await decideAnalysis([proposal], accept: accept)
        selectNextProposedRule(after: AnalysisStore.tag(proposal), in: before)
    }

    /// Accept All on the Proposed section: every proposed rule that goes
    /// against none of the user's (those wait for a decision of their own).
    /// The learning decisions and the review's proposals are one change
    /// each.
    func acceptAllProposedRules() async {
        let decisions = guide.decisions.filter { $0.contradictionOf == nil }
        if !decisions.isEmpty {
            await applyGuideEdits(decisions.map { .decide(id: $0.id, status: .accepted) }, reason: "accept",
                                  actionName: "Accept All",
                                  notice: decisions.count == 1 ? "Added to your writing guide"
                                      : "\(decisions.count) added to your writing guide")
        }
        let proposals = analysis.proposals.filter { $0.contradicts == nil }
        if !proposals.isEmpty { await decideAnalysis(proposals, accept: true) }
        await analysis.load()
        analysis.selection = proposedRuleTags.first
    }

    /// Looking at a page's proposals: read them, and clear its dot.
    func proposalsShown(_ page: ProposalPage) async {
        guard let core else { return }
        await analysis.load()
        let unseen = page == .rules ? analysis.unseenRules : analysis.unseenFacts
        if unseen {
            try? await core.analysisSeen(page)
            await analysis.load()
            await refreshAnalysisDots()
        }
    }

    /// Proposals changed (a review, a decision, an undo).
    func analysisChanged() async {
        // Seen only when the user can see it: not with the app behind.
        if notifier.isAppActive(), let page = shownProposalPage { await proposalsShown(page) } else { await analysis.load() }
        await refreshAnalysisDots()
    }

    /// The page showing proposals, if one is in front.
    var shownProposalPage: ProposalPage? { isGuide ? .rules : isFacts ? .facts : nil }

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
