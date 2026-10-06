import SwiftUI

/// What the app asks about the writing guide on its own (spec §14.9): to
/// learn from sent mail on an account that never has, and to review the
/// decisions when a run finishes.
enum GuidePrompt: Identifiable, Equatable {
    case firstRun
    case finished(decisions: Int)

    var id: String {
        switch self {
        case .firstRun: "first-run"
        case let .finished(n): "finished-\(n)"
        }
    }
}

extension AppModel {
    static func guideInviteKey(_ accountID: String) -> String { "guideInvite.\(accountID)" }

    /// The banner inviting a first run shows for the open account.
    var showsGuideBanner: Bool {
        guideBannerAccount != nil && guideBannerAccount == openAccountID && guideProgress?.run == nil
    }

    /// Invite a first learning run, once per account: when the account has
    /// sent mail, has never run one and was never asked. Asked and put
    /// off, a banner stays until a run starts or it is dismissed.
    func checkGuideInvite() async {
        guard !Snapshot.isRequested, let core, let account = openAccountID,
              !guideInviteChecked.contains(account), syncDisplay == .idle else { return }
        if (try? await core.guideProgress())?.run != nil {
            guideInviteChecked.insert(account)
            return
        }
        let none = GuideSampleFilter(excludePeople: [], excludeLabels: [])
        // No sent mail yet (still syncing): ask again when sync is idle.
        guard let info = try? await core.guideSampleInfo(count: 1, filter: none), info.sent > 0,
              account == openAccountID else { return }
        guideInviteChecked.insert(account)
        switch defaults.string(forKey: Self.guideInviteKey(account)) {
        case nil:
            defaults.set("offered", forKey: Self.guideInviteKey(account))
            if guidePrompt == nil, guideSheet == nil { guidePrompt = .firstRun }
        case "offered":
            guideBannerAccount = account
        default:
            break
        }
    }

    /// The invitation's answer: start opens the learn dialog in the section.
    func answerGuideInvite(start: Bool) {
        guidePrompt = nil
        if start {
            guideBannerAccount = nil
            selectedMailboxID = Self.guideMailboxID
            guideSheet = .learn
        } else {
            guideBannerAccount = openAccountID
        }
    }

    func dismissGuideBanner() {
        if let account = openAccountID { defaults.set("dismissed", forKey: Self.guideInviteKey(account)) }
        guideBannerAccount = nil
    }

    /// The learning run's decisions wait in Analysis (spec §14.10).
    func openGuideDecisionsNow() {
        openAnalysis(learning: true)
    }
}

/// The sheet for either prompt.
struct GuidePromptSheet: View {
    @Environment(AppModel.self) private var model
    let prompt: GuidePrompt

    var body: some View {
        switch prompt {
        case .firstRun:
            Dialog(title: "Learn How You Write?",
                   message: "OpenAGC can learn your writing style from the mail you have sent, so any AI that drafts "
                       + "for you writes the way you do. It reads your sent mail with your own agent, in the "
                       + "background, and asks you to confirm what it found when it is done.") {
                EmptyView()
            } buttons: {
                Button("Later") { model.answerGuideInvite(start: false) }
                    .keyboardShortcut(.cancelAction)
                    .hoverHelp("Not now; the Writing Guide keeps a reminder")
                Button("Choose What to Learn From…") { model.answerGuideInvite(start: true) }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Choose how many sent messages to analyse")
            }
        case let .finished(decisions):
            Dialog(title: "Your Writing Guide Is Ready to Review",
                   message: decisions == 1 ? "The analysis of your sent mail finished. 1 decision is waiting for you."
                       : "The analysis of your sent mail finished. \(decisions.formatted()) decisions are waiting for you.") {
                EmptyView()
            } buttons: {
                Button("Later") { model.guidePrompt = nil }
                    .keyboardShortcut(.cancelAction)
                    .hoverHelp("The decisions wait in the Writing Guide")
                Button("Review Now") { model.openGuideDecisionsNow() }
                    .keyboardShortcut(.defaultAction)
                    .hoverHelp("Go through the decisions: Return accepts, ⌫ rejects, e edits")
            }
        }
    }
}

/// Above the Inbox and in the Writing Guide while a first run was put off.
struct GuideInviteBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Banner("Learn how you write from your sent mail, so AI drafts sound like you.",
               systemImage: "text.badge.star", intent: .neutral) {
            Button("Learn…") { model.answerGuideInvite(start: true) }
                .hoverHelp("Choose how many sent messages to analyse")
            Button("Dismiss") { model.dismissGuideBanner() }
                .hoverHelp("Stop showing this; Learn from Sent Mail stays in the Writing Guide")
        }
    }
}
