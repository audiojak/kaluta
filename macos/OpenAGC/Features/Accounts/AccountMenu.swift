import AppKit
import SwiftUI

/// The account switcher: a small avatar in the sidebar's title bar, with
/// the account list in its menu (spec §7.7). The address is in the tooltip,
/// not beside the picture.
struct AccountMenuButton: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        Menu {
            AccountMenuItems(openSettings: { openSettings() })
        } label: {
            if let current {
                AccountAvatar(account: current, size: 18)
            } else {
                Image(systemName: "person.crop.circle")
            }
        }
        .menuIndicator(.hidden)
        .fixedSize()
        .help(current.map { "\($0.displayName ?? $0.email) — \($0.email)\nSwitch account" }
            ?? (model.openAccountID == AppModel.demoAccountID ? "Demo mailbox" : "Accounts"))
        .accessibilityLabel("Account: \(current?.email ?? "none"). Switch account")
        .task { await model.reloadAccounts() }
    }

    private var current: AccountSummary? {
        model.accounts.first { $0.id == model.openAccountID }
    }
}

/// The account list, shared by the avatar menu and the app menu's
/// Accounts submenu: ⌃1–⌃9 switch by position.
struct AccountMenuItems: View {
    @Environment(AppModel.self) private var model
    let openSettings: () -> Void

    var body: some View {
        ForEach(Array(model.accounts.enumerated()), id: \.element.id) { index, account in
            Button {
                Task { await model.switchAccount(to: account.id) }
            } label: {
                Image(nsImage: AccountAvatar.menuImage(account, current: account.id == model.openAccountID))
                Text(AccountMenuItems.title(account))
            }
            .keyboardShortcut(index < 9 ? KeyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .control) : nil)
        }
        if !model.accounts.isEmpty { Divider() }
        Button("Add Account…") { Task { await model.addAccount() } }
            .disabled(!GoogleClientConfiguration.effective().isUsable)
        Button("Accounts Settings…", action: openSettings)
    }

    /// "Work Me — work@example.com (12)".
    static func title(_ account: AccountSummary) -> String {
        var title = account.displayName.map { "\($0) — \(account.email)" } ?? account.email
        if account.inboxUnread > 0 { title += " (\(account.inboxUnread))" }
        return title
    }
}

extension AccountAvatar {
    /// The avatar as a menu image, with a check ring on the current account.
    @MainActor
    static func menuImage(_ account: AccountSummary, current: Bool) -> NSImage {
        let view = AccountAvatar(account: account, size: 18)
            .overlay(Circle().strokeBorder(Color.accentColor, lineWidth: current ? 2 : 0))
            .padding(1)
        let renderer = ImageRenderer(content: view)
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.nsImage ?? NSImage()
    }
}
