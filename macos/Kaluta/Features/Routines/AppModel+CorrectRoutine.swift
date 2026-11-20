import Foundation

/// Correct it once (spec §14.13 amendment 2026-11-18): moving a
/// conversation a routine filed back to the Inbox, from the mail window,
/// proposes a change to the routine as Weekly reading's *Back to the
/// Inbox* does, and the notice asks why in a line. The move is one
/// undoable change; the routine changes only once the proposal is
/// accepted in Teach Kaluta.
extension AppModel {
    func correctRoutine(threadID: String) async {
        guard let core, let accountID = openAccountID, let back = try? await core.backToInbox(threadID) else { return }
        let text = back.proposalId == nil ? "Moved 1 conversation to the Inbox"
            : "Moved to the Inbox. Kaluta will propose a change to the routine."
        undo.record(accountID: accountID, actionName: "Move to Inbox", noticeText: text,
                    undo: {
                        if let token = back.undo { _ = try? await core.undo(token) }
                        if let proposal = back.proposalId { try? await core.withdrawRoutineEvidence(proposal, threadID: threadID) }
                    },
                    redo: { if let token = back.undo { _ = try? await core.redo(token) } })
        if let proposal = back.proposalId, let notice = undo.notice {
            reasonRequest = ReasonRequest(noticeID: notice.id, proposalID: proposal, threadID: threadID)
        }
    }

    /// The reason typed in the notice, kept with the proposal.
    func saveRoutineReason(_ reason: String) async {
        guard let request = reasonRequest, let core else { return }
        reasonRequest = nil
        let text = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        try? await core.explainRoutineEvidence(request.proposalID, threadID: request.threadID, reason: text)
        undo.dismissNotice()
    }
}
