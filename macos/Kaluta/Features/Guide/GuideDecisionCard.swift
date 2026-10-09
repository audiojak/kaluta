import Foundation

extension AppModel {
    /// Accept or reject a proposal; undoable.
    func decideGuide(_ entry: GuideEntry, accept: Bool) async {
        await applyGuideEdits([.decide(id: entry.id, status: accept ? .accepted : .rejected)],
                              reason: accept ? "accept" : "reject", actionName: accept ? "Accept Entry" : "Reject Entry",
                              notice: accept ? "Added to your writing guide" : "Left out of your writing guide")
    }

    /// Use a proposal in place of the accepted entry it contradicts: one
    /// change, undoable.
    func replaceGuideEntry(_ old: GuideEntry, with new: GuideEntry) async {
        await applyGuideEdits([.decide(id: new.id, status: .accepted), .decide(id: old.id, status: .rejected)],
                              reason: "replace", actionName: "Replace Entry", notice: "Replaced “\(old.statement)”")
    }
}
