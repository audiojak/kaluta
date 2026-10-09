import Foundation

/// The Writing Guide section (spec §14.9): showing it, learning, and
/// changing entries with Undo.
extension AppModel {
    /// The sidebar's Writing Guide entry, in place of a mailbox id.
    static let guideMailboxID = "@guide"

    var isGuide: Bool { selectedMailboxID == Self.guideMailboxID && threads.searchQuery == nil }

    /// Decisions waiting: proposals of finished runs not yet decided.
    var guideDecisionsWaiting: Int {
        guard let p = guideProgress else { return 0 }
        return Int(p.decisionsTotal) - Int(p.decisionsDone)
    }

    /// A learning run in progress (running or paused).
    var guideRunActive: Bool {
        guard let status = guideProgress?.run?.status else { return false }
        return status == .running || status == .paused
    }

    /// Apply edits as one change, on the account's undo stack (ADR 0006).
    @discardableResult
    func applyGuideEdits(_ edits: [GuideEdit], reason: String, actionName: String,
                         notice: String) async -> Result<GuideChange, CoreClientError> {
        guard let core, let accountID = openAccountID else {
            return .failure(CoreClientError(kind: .notFound, message: "No account is open"))
        }
        do {
            let change = try await core.applyGuideEdits(edits, reason: reason)
            undo.record(accountID: accountID, actionName: actionName, noticeText: notice,
                        undo: { try? await core.undoGuideChange(change.changeId) },
                        redo: { try? await core.redoGuideChange(change.changeId) })
            await guide.load()
            guideProgress = try? await core.guideProgress()
            return .success(change)
        } catch {
            return .failure(error)
        }
    }

    func deleteGuideEntry(_ entry: GuideEntry) async {
        await applyGuideEdits([.delete(id: entry.id)], reason: "delete", actionName: "Delete Entry",
                              notice: "Deleted “\(entry.statement)”")
    }
}
