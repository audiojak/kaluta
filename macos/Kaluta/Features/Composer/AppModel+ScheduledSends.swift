import AppKit
import SwiftUI

/// Send later (spec §14.5 amendment 2026-11-18, ADR 0022). A scheduled
/// draft stays a draft; the app sends it when its time comes, while it is
/// running. One found more than an hour late (Kaluta was not running) is
/// asked about instead.
extension AppModel {
    /// A scheduled send found later than this is asked about, not sent.
    static let scheduledGrace: Duration = .seconds(3600)

    /// What to do with a send found long after its time.
    enum OverdueChoice { case sendNow, keepDraft, askLater }

    /// A draft was scheduled in a composer: acknowledge it, with a way back.
    func sendScheduled(draftID: Int64, at date: Date, before: Date?, accountID: String) {
        guard let core else { return }
        undo.record(accountID: accountID, actionName: "Send Later",
                    noticeText: "Will send \(Self.reminderTime(at: date).phrase)",
                    undo: { [weak self] in
                        _ = try? await core.scheduleSend(draftID, at: before, in: accountID)
                        await self?.refreshScheduled()
                    },
                    redo: { [weak self] in
                        _ = try? await core.scheduleSend(draftID, at: date, in: accountID)
                        await self?.refreshScheduled()
                    })
        Task { await refreshScheduled() }
    }

    /// The scheduled drafts as stored, and the wait for the next of them.
    func refreshScheduled() async {
        guard let core else { return }
        scheduledSends = (try? await core.scheduledSends()) ?? []
        scheduledWait?.cancel()
        guard let next = scheduledSends.map(\.sendAt).min() else { return }
        let wait = max(0, CoreClient.date(next).timeIntervalSinceNow)
        scheduledWait = Task { [weak self] in
            guard (try? await Task.sleep(for: .seconds(wait))) != nil else { return }
            await self?.sendDueScheduled()
        }
    }

    /// Send what has come due (at its time, at launch, on waking); ask
    /// about anything found too late.
    func sendDueScheduled() async {
        guard let core, let due = try? await core.sendScheduledDue(grace: Self.scheduledGrace) else { return }
        for sent in due.sent {
            undo.show(Self.scheduledNotice(sent, went: true), accountID: sent.accountId, offersUndo: false)
        }
        for failed in due.failed {
            undo.show(Self.scheduledNotice(failed, went: false), accountID: failed.accountId, offersUndo: false)
        }
        for late in due.overdue {
            switch await overduePrompt(late) {
            case .sendNow:
                _ = try? await core.scheduleSend(late.draftId, at: nil, in: late.accountId)
                if await core.sendDraft(late.draftId, in: late.accountId) {
                    sendHeld(draftID: late.draftId, accountID: late.accountId)
                }
            case .keepDraft:
                _ = try? await core.scheduleSend(late.draftId, at: nil, in: late.accountId)
            case .askLater:
                break
            }
        }
        await refreshScheduled()
    }

    /// "Sent “Quarterly numbers” as scheduled", or why it is a draft again.
    static func scheduledNotice(_ send: ScheduledSendInfo, went: Bool) -> String {
        let name = send.subject.trimmingCharacters(in: .whitespaces).isEmpty ? "a message" : "“\(send.subject)”"
        return went ? "Sent \(name) as scheduled"
            : "Could not send \(name) as scheduled. It is in Drafts."
    }

    /// The Drafts strip's line for a scheduled draft: "“Subject” sends
    /// Thursday, November 19 at 8:00 AM".
    static func scheduledLine(_ send: ScheduledSendInfo, now: Date = .now) -> String {
        let name = send.subject.trimmingCharacters(in: .whitespaces).isEmpty ? "(no subject)" : send.subject
        return "“\(name)” sends \(reminderTime(at: CoreClient.date(send.sendAt), now: now).phrase)"
    }

    /// Asks in a dialog what to do with a send found long after its time.
    static func askAboutOverdue(_ send: ScheduledSendInfo) async -> OverdueChoice {
        let alert = NSAlert()
        let name = send.subject.trimmingCharacters(in: .whitespaces).isEmpty ? "A message" : "“\(send.subject)”"
        let when = CoreClient.date(send.sendAt).formatted(date: .abbreviated, time: .shortened)
        alert.messageText = "\(name) was not sent at its time"
        alert.informativeText = "It was scheduled for \(when), when Kaluta was not running. Send it now?"
        alert.addButton(withTitle: "Send Now")
        alert.addButton(withTitle: "Keep as a Draft")
        alert.addButton(withTitle: "Ask Me Later")
        switch alert.runModal() {
        case .alertFirstButtonReturn: return .sendNow
        case .alertSecondButtonReturn: return .keepDraft
        default: return .askLater
        }
    }
}

/// Over the Drafts list: the drafts that will be sent later, each opening
/// its draft.
struct ScheduledStrip: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let mine = model.scheduledSends.filter { $0.accountId == model.openAccountID }
        if model.selectedMailboxID == "DRAFT", !mine.isEmpty {
            VStack(alignment: .leading, spacing: Space.xs) {
                ForEach(mine, id: \.draftId) { send in
                    Button {
                        model.compose(.draft(id: send.draftId, task: nil))
                    } label: {
                        Label(AppModel.scheduledLine(send), systemImage: "clock")
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .hoverHelp("Open the draft to change it, its time, or send it now")
                }
            }
            .font(TypeRole.meta)
            .padding(.horizontal, Space.l)
            .padding(.bottom, Space.s)
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Scheduled to send")
        }
    }
}
