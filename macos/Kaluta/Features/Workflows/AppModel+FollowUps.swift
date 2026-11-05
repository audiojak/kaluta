import AppKit
import SwiftUI

/// Follow-ups (spec §14.14): meetings from Granola that deserve a
/// follow-up email. 1–3 open one of Kaluta's drafts in a composer to edit
/// and send; E is no follow-up; L brings it back tomorrow.
extension AppModel {
    /// Every meeting of the set, for the item and the rail.
    func loadMeetings(_ session: WorkflowSession) async {
        guard let core else { return }
        for item in session.items {
            guard let id = item.meetingID, session.meetings[id] == nil,
                  let meeting = try? await core.meetingFollowUp(id) else { continue }
            session.meetings[id] = meeting
        }
    }

    func followUpActions(_ session: WorkflowSession,
                         _ a: (WorkflowCommand, WorkflowButton.Style, String?) -> WorkflowAction) -> [WorkflowAction] {
        let options = session.current?.meetingID.flatMap { session.meetings[$0] }?.options ?? []
        let picks = options.prefix(3).enumerated().map { index, option in
            a(FollowUpItemView.pick(index), index == 0 ? .primary : .plain, option.label)
        }
        var granola = a(.openInFull, .quiet, "Open in Granola")
        granola.shown = session.current?.meetingID.flatMap { session.meetings[$0] }?.url != nil
        return picks + [a(.done, .plain, "No follow-up"), a(.later, .plain, "Tomorrow"), a(.skip, .quiet, nil)]
            + (granola.shown ? [granola] : [])
    }

    func performFollowUp(_ command: WorkflowCommand, item: WorkflowItem, session: WorkflowSession) async {
        let meeting = item.meetingID.flatMap { session.meetings[$0] }
        switch command {
        case .pick1, .pick2, .pick3:
            let index = command == .pick1 ? 0 : command == .pick2 ? 1 : 2
            guard let option = meeting?.options[safe: index] else { return }
            compose(.prefilled(to: option.to, cc: [], subject: option.subject, body: option.body))
            await decideWorkflowItem(item, "sent", session: session, notice: "Opened “\(option.subject)” to send")
        case .done:
            await decideWorkflowItem(item, "none", session: session, notice: "No follow-up")
        case .later:
            await decideWorkflowItem(item, "later", session: session, notice: "Back tomorrow")
        case .openInFull:
            if let url = meeting?.url.flatMap(URL.init(string:)), url.scheme == "https" {
                NSWorkspace.shared.open(url)
            }
        default:
            break
        }
    }
}

extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
