import SwiftUI

/// Mail has stopped syncing because the sign-in was rejected (Google
/// revoked or expired it; for an agent mailbox, its service key is
/// missing or refused). Large, at the top of the mail window and of the
/// Dashboard, and there until it is fixed: a thin banner in the list was
/// easy to miss while no new mail arrived (oagc-8ssu).
struct SignInNotice: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let text = Self.text(email: model.accountEmail, agentMailbox: model.isAgentMailbox)
        HStack(alignment: .center, spacing: Space.xl) {
            Image(systemName: model.isAgentMailbox ? "key.slash" : "person.crop.circle.badge.exclamationmark")
                .font(TypeRole.display)
                .foregroundStyle(Tone.caution)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: Space.xs) {
                Text(text.title).font(TypeRole.title)
                Text(text.detail)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: Space.l)
            if !model.isAgentMailbox {
                Button("Sign In Again…") { Task { await model.signIn(with: .effective()) } }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .hoverHelp("Sign in to Google again in your browser; mail starts syncing as soon as you have")
            }
        }
        .padding(Space.xl)
        .background(RoundedRectangle(cornerRadius: Radius.panel).fill(Tone.Intent.attention.fill))
        .overlay(RoundedRectangle(cornerRadius: Radius.panel).strokeBorder(Tone.caution.opacity(Self.borderOpacity)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(text.title). \(text.detail)")
    }

    private static let borderOpacity = 0.45

    /// What stopped, for whom, and what it means until it is fixed.
    static func text(email: String?, agentMailbox: Bool) -> (title: String, detail: String) {
        if agentMailbox {
            return ("This agent mailbox has stopped syncing",
                    "Its key is missing or was refused. No new mail arrives until the key is back (Settings › Accounts).")
        }
        let whose = email.map { " for \($0)" } ?? ""
        return ("Mail has stopped syncing",
                "Gmail needs you to sign in again\(whose). Until you do, no new mail arrives and what you change waits to reach Gmail.")
    }
}
