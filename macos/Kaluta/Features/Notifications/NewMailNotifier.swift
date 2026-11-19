import AppKit
import Foundation
import os
import UserNotifications

/// New-mail notifications and the Dock badge (spec §14.7). Notifications
/// are only for mail that arrives while the app is not frontmost; clicking
/// one opens its thread. Both are on by default and can be turned off in
/// Settings.
@MainActor
final class NewMailNotifier: NSObject {
    static let notifyKey = "notifyNewMail"
    /// "N new proposals from your mail", once a day (spec §14.10); off unless
    /// the user turns it on.
    static let analysisKey = "notifyAnalysis"
    static let badgeKey = "showDockBadge"
    /// More new messages than this at once become one summary notification.
    static let summaryThreshold = 3

    /// Where notifications go; replaced in tests.
    var post: (UNNotificationRequest) -> Void
    /// Whether the app is frontmost; replaced in tests.
    var isAppActive: () -> Bool = { NSApp.isActive }
    /// Opens a thread (in an account, if the notification named one) when
    /// a notification is clicked.
    var openThread: ((String, String?) -> Void)?
    /// Opens the writing guide's decisions, in this account (or the open one).
    var openGuideDecisions: ((String?) -> Void)?
    /// Opens Analysis, in this account (or the open one).
    var openAnalysis: ((String?) -> Void)?
    /// A reply-landed alert's buttons: reply in the conversation, or be
    /// reminded of it tomorrow (thread, account).
    var replyInThread: ((String, String?) -> Void)?
    var remindTomorrow: ((String, String?) -> Void)?

    nonisolated static let replyCategory = "reply-landed"
    nonisolated static let replyAction = "reply"
    nonisolated static let remindAction = "remind-tomorrow"

    private let defaults: UserDefaults
    private let logger = Logger(subsystem: "org.kaluta.Kaluta", category: "notifications")
    private var askedForPermission = false

    init(defaults: UserDefaults = CoreClient.appDefaults(), post: ((UNNotificationRequest) -> Void)? = nil) {
        self.defaults = defaults
        defaults.register(defaults: [Self.notifyKey: true, Self.badgeKey: true, Self.analysisKey: false])
        self.post = post ?? { _ in }
        super.init()
        if post == nil {
            self.post = { [weak self] request in self?.deliver(request) }
        }
    }

    /// Call once at launch so clicks on earlier notifications route here.
    func install() {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories([Self.replyLandedCategory])
    }

    /// Meeting prep: a reminder with the brief, 15 minutes before the
    /// meeting. Scheduled again whenever the briefs load; the same id
    /// replaces it, so a moved meeting moves its reminder.
    func scheduleBrief(_ brief: MeetingBrief, accountID: String?, now: Date = .now) {
        guard let request = Self.briefRequest(brief, accountID: accountID, now: now) else { return }
        post(request)
    }

    static let briefLead: TimeInterval = 15 * 60

    static func briefRequest(_ brief: MeetingBrief, accountID: String?, now: Date = .now) -> UNNotificationRequest? {
        let wait = CoreClient.date(brief.meetingAt).addingTimeInterval(-briefLead).timeIntervalSince(now)
        guard wait > 1 else { return nil }
        let content = UNMutableNotificationContent()
        content.title = "\(brief.title) in 15 minutes"
        content.body = brief.brief
        content.threadIdentifier = "meeting-prep"
        content.userInfo = ["accountID": accountID ?? "", "brief": brief.meetingId]
        return UNNotificationRequest(identifier: "brief-\(brief.meetingId)", content: content,
                                     trigger: UNTimeIntervalNotificationTrigger(timeInterval: wait, repeats: false))
    }

    static var replyLandedCategory: UNNotificationCategory {
        UNNotificationCategory(identifier: replyCategory, actions: [
            UNNotificationAction(identifier: replyAction, title: "Reply", options: [.foreground]),
            UNNotificationAction(identifier: remindAction, title: "Remind Me Tomorrow", options: []),
        ], intentIdentifiers: [])
    }

    /// Someone answered a conversation the user watches: told even with
    /// Kaluta in front, and even with new-mail notifications off (the user
    /// asked for this one).
    func announceReply(_ mail: CoreClientEvent.NewMail, accountID: String?) {
        post(Self.replyRequest(mail, accountID: accountID))
    }

    static func replyRequest(_ mail: CoreClientEvent.NewMail, accountID: String?) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = "\(mail.senderName) answered"
        content.subtitle = mail.subject.isEmpty ? "(no subject)" : mail.subject
        content.body = mail.snippet
        content.sound = .default
        content.categoryIdentifier = replyCategory
        content.threadIdentifier = mail.threadID
        content.userInfo = ["threadID": mail.threadID, "accountID": accountID ?? "", "replyLanded": true]
        return UNNotificationRequest(identifier: "reply-\(mail.messageID)", content: content, trigger: nil)
    }

    // MARK: Notifications

    /// Which account the mail is for: its id, and a label shown only when
    /// there is more than one account (spec §7.7).
    struct AccountTag: Equatable {
        let id: String
        let label: String?
    }

    func announce(_ mail: [CoreClientEvent.NewMail], account: AccountTag? = nil) {
        guard !mail.isEmpty, defaults.bool(forKey: Self.notifyKey), !isAppActive() else { return }
        for request in Self.requests(for: mail, account: account) {
            post(request)
        }
    }

    /// A learning run finished (spec §14.9): its decisions are waiting.
    /// Only when the app is not in front, like new mail.
    /// New proposals after a review: at most once a day per
    /// account, only when the app is not in front, only if turned on.
    /// `proposals` nil: some, not counted (another account).
    func announceAnalysis(proposals: Int?, accountID: String?, today: String) {
        let key = "analysisNotified.\(accountID ?? "")"
        guard proposals.map({ $0 > 0 }) ?? true, defaults.bool(forKey: Self.analysisKey), !isAppActive(),
              defaults.string(forKey: key) != today else { return }
        defaults.set(today, forKey: key)
        let content = UNMutableNotificationContent()
        content.title = "Writing Guide and Facts"
        content.body = switch proposals {
        case nil: "New proposals from your mail"
        case 1: "1 new proposal from your mail"
        case let n?: "\(n) new proposals from your mail"
        }
        content.threadIdentifier = "analysis"
        content.userInfo = ["analysis": true, "accountID": accountID ?? ""]
        post(UNNotificationRequest(identifier: "analysis-\(UUID().uuidString)", content: content, trigger: nil))
    }

    func announceGuide(decisions: Int, accountID: String? = nil) {
        guard decisions > 0, !isAppActive() else { return }
        post(Self.guideRequest(decisions: decisions, accountID: accountID))
    }

    static func guideRequest(decisions: Int, accountID: String? = nil) -> UNNotificationRequest {
        let content = UNMutableNotificationContent()
        content.title = "Writing guide analysis finished"
        content.body = decisions == 1 ? "1 decision is waiting for you." : "\(decisions) decisions are waiting for you."
        content.threadIdentifier = "writing-guide"
        // Clicking it opens the decisions (`openGuideDecisions`).
        content.userInfo = ["guide": true, "accountID": accountID ?? ""]
        return UNNotificationRequest(identifier: "writing-guide-\(UUID().uuidString)", content: content, trigger: nil)
    }

    /// One notification per message, or one summary for a burst. Pure.
    static func requests(for mail: [CoreClientEvent.NewMail], account: AccountTag? = nil) -> [UNNotificationRequest] {
        requests(untagged: mail).map { request in
            guard let account else { return request }
            let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
            var info = content.userInfo
            info["accountID"] = account.id
            content.userInfo = info
            if let label = account.label {
                content.subtitle = content.subtitle.isEmpty ? label : "\(label) · \(content.subtitle)"
                content.threadIdentifier = "\(account.id):\(content.threadIdentifier)"
            }
            return UNNotificationRequest(identifier: "\(account.id):\(request.identifier)", content: content, trigger: nil)
        }
    }

    private static func requests(untagged mail: [CoreClientEvent.NewMail]) -> [UNNotificationRequest] {
        if mail.count > summaryThreshold {
            let content = UNMutableNotificationContent()
            content.title = "\(mail.count) new messages"
            let senders = Array(NSOrderedSet(array: mail.map(\.senderName))) as? [String] ?? []
            content.body = ListFormatter.localizedString(byJoining: Array(senders.prefix(4)))
                + (senders.count > 4 ? " and others" : "")
            content.sound = .default
            content.threadIdentifier = "new-mail"
            content.userInfo = ["threadID": mail.last?.threadID ?? ""]
            return [UNNotificationRequest(identifier: "summary-\(mail.last?.messageID ?? "")", content: content, trigger: nil)]
        }
        return mail.map { m in
            let content = UNMutableNotificationContent()
            content.title = m.senderName
            content.subtitle = m.subject.isEmpty ? "(no subject)" : m.subject
            content.body = m.snippet
            content.sound = .default
            // Groups notifications of one conversation in Notification Center.
            content.threadIdentifier = m.threadID
            content.userInfo = ["threadID": m.threadID]
            return UNNotificationRequest(identifier: m.messageID, content: content, trigger: nil)
        }
    }

    private func deliver(_ request: UNNotificationRequest) {
        let center = UNUserNotificationCenter.current()
        Task {
            if !askedForPermission {
                askedForPermission = true
                // Asked the first time there is something to show, not at launch.
                _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
            }
            do {
                try await center.add(request)
            } catch {
                logger.error("notification failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    // MARK: Dock badge

    func updateBadge(inboxUnread: UInt32) {
        let show = defaults.bool(forKey: Self.badgeKey) && inboxUnread > 0
        let label = show ? (inboxUnread > 9_999 ? "9999+" : String(inboxUnread)) : nil
        if NSApp.dockTile.badgeLabel != label {
            NSApp.dockTile.badgeLabel = label
        }
    }
}

extension NewMailNotifier: UNUserNotificationCenterDelegate {
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        let threadID = info["threadID"] as? String
        let accountID = info["accountID"] as? String
        let guide = info["guide"] as? Bool ?? false
        let analysis = info["analysis"] as? Bool ?? false
        let action = response.actionIdentifier
        let account = accountID.flatMap { $0.isEmpty ? nil : $0 }
        if action == Self.remindAction, let threadID {
            await MainActor.run { remindTomorrow?(threadID, account) }
            return
        }
        if action == Self.replyAction, let threadID {
            await MainActor.run {
                NSApp.activate()
                replyInThread?(threadID, account)
            }
            return
        }
        await MainActor.run {
            NSApp.activate()
            if guide { openGuideDecisions?(accountID.flatMap { $0.isEmpty ? nil : $0 }) }
            if analysis { openAnalysis?(accountID.flatMap { $0.isEmpty ? nil : $0 }) }
            if let threadID, !threadID.isEmpty { openThread?(threadID, accountID) }
        }
    }

    /// Mail arriving while the app is frontmost is already visible; an
    /// answer the user asked to hear about is shown anyway.
    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter,
                                            willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        notification.request.content.categoryIdentifier == Self.replyCategory ? [.banner, .sound] : []
    }
}
