import Foundation

/// Reply-landed alerts (spec §14.7 amendment 2026-11-18): the user asks to
/// be told as soon as someone answers; the alert comes once, even with
/// Kaluta in front, with Reply and Remind Me Tomorrow.
extension AppModel {
    /// The selected conversations: watched, or (all watched already) no
    /// longer. ⌘Z takes it back.
    func toggleWatchSelection() async {
        let ids = actionTargets
        guard let core, let accountID = openAccountID, !ids.isEmpty else { return }
        let on = !ids.allSatisfy(watched.contains)
        var before: [String: Bool] = [:]
        for id in ids { before[id] = (try? await core.watchThread(id, on)) ?? false }
        await loadWatched()
        let count = ids.count == 1 ? "this conversation" : "\(ids.count) conversations"
        let text = on ? "Kaluta will tell you when someone answers \(count)" : "No longer watching \(count)"
        undo.record(accountID: accountID, actionName: on ? "Watch for Replies" : "Stop Watching", noticeText: text,
                    undo: { [weak self] in
                        for (id, was) in before { _ = try? await core.watchThread(id, was) }
                        await self?.loadWatched()
                    },
                    redo: { [weak self] in
                        for id in ids { _ = try? await core.watchThread(id, on) }
                        await self?.loadWatched()
                    })
    }

    func loadWatched() async {
        watched = (try? await core?.watchedThreads()) ?? []
    }

    /// Mail arrived in the open account: tell the user about answers to
    /// conversations they watch.
    func announceReplies(_ mail: [CoreClientEvent.NewMail], accountID: String?) async {
        guard let core, !mail.isEmpty,
              let landed = try? await core.repliesLanded(Array(Set(mail.map(\.threadID)))), !landed.isEmpty
        else { return }
        for thread in landed {
            guard let message = mail.last(where: { $0.threadID == thread }) else { continue }
            notifier.announceReply(message, accountID: accountID)
        }
        await loadWatched()
    }

    /// The notification's Remind Me Tomorrow: back in Triage tomorrow at 8.
    func remindTomorrow(threadID: String, accountID: String?) async {
        if let accountID, accountID != openAccountID, accounts.contains(where: { $0.id == accountID }) {
            await switchAccount(to: accountID)
        }
        _ = try? await core?.remindLater(threadID, at: Self.reminderTimes()[1].date)
    }

    /// The notification's Reply: the conversation, with a reply to its
    /// latest message open.
    func replyFromNotification(threadID: String, accountID: String?) async {
        await reveal(threadID: threadID, in: accountID)
        reply(all: false)
    }
}
