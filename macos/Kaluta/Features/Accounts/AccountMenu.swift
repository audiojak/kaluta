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
            // "Accounts" names the toolbar item, so ToolbarToolTips finds it.
            Label {
                Text("Accounts")
            } icon: {
                // Toolbar menus draw their label as a template (one colour),
                // which turns a photo into a blank shape: a full-colour image.
                if let current {
                    Image(nsImage: AccountAvatar.toolbarImage(current))
                        .renderingMode(.original)
                } else {
                    Image(systemName: "person.crop.circle")
                }
            }
            .labelStyle(.iconOnly)
        }
        .menuIndicator(.hidden)
        .fixedSize()
        .help(Self.helpText(model))
        .accessibilityLabel("Account: \(current?.email ?? "none"). Switch account")
        .task { await model.reloadAccounts() }
    }

    private var current: AccountSummary? {
        model.accounts.first { $0.id == model.openAccountID }
    }

    static func helpText(_ model: AppModel) -> String {
        if let current = model.accounts.first(where: { $0.id == model.openAccountID }) {
            return "\(current.displayName ?? current.email) — \(current.email)\nSwitch account"
        }
        return model.openAccountID == AppModel.demoAccountID ? "Demo mailbox" : "Accounts"
    }
}

/// The account list, shared by the avatar menu and the app menu's
/// Accounts submenu: ⌃1–⌃9 switch by position.
struct AccountMenuItems: View {
    @Environment(AppModel.self) private var model
    let openSettings: () -> Void

    var body: some View {
        let groups = model.accountMenuGroups
        // ⌃1–⌃9 count down the menu as shown, across its sections.
        let positions = Dictionary(uniqueKeysWithValues: groups.flatMap(\.accounts).enumerated().map { ($1.id, $0) })
        ForEach(groups) { group in
            if let title = group.title {
                Section(title) {
                    items(group.accounts, positions: positions)
                }
            } else {
                items(group.accounts, positions: positions)
            }
        }
        if !model.accounts.isEmpty { Divider() } // menu
        Button("Add Account…") { Task { await model.addAccount() } } // no-help: menu item
            .disabled(!GoogleClientConfiguration.effective().isUsable)
        // An imported mailbox is an account of its own (spec §7.8); no
        // shortcut: ⌘⇧I is Load Remote Images.
        Button("Create an Agent Mailbox…") { model.beginAgentMailbox() } // no-help: menu item
        Button("Create an Account from an Archived Mailbox…") { Task { await model.beginImport() } } // no-help: menu item
            .disabled(model.runningImport != nil)
        Button("Accounts Settings…", action: openSettings) // no-help: menu item
    }

    private func items(_ accounts: [AccountSummary], positions: [String: Int]) -> some View {
        ForEach(accounts, id: \.id) { account in
            let index = positions[account.id] ?? 9
            Button { // no-help: menu item
                Task { await model.switchAccount(to: account.id) }
            } label: {
                let unseen = model.unseenAnalysisAccounts.contains(account.id)
                Image(nsImage: AccountAvatar.menuImage(account, current: account.id == model.openAccountID,
                                                       unseen: unseen))
                // The dot is in the picture; VoiceOver reads it from the title.
                Text(AccountMenuItems.title(account))
                    .accessibilityLabel(AccountMenuItems.title(account) + (unseen ? ", new proposals" : ""))
            }
            .keyboardShortcut(index < 9 ? KeyboardShortcut(KeyEquivalent(Character("\(index + 1)")), modifiers: .control) : nil)
        }
    }

    /// "Work Me — work@example.com (12)".
    static func title(_ account: AccountSummary) -> String {
        var title = account.displayName.map { "\($0) — \(account.email)" } ?? account.email
        if account.inboxUnread > 0 { title += " (\(account.inboxUnread))" }
        return title
    }
}

extension AccountAvatar {
    /// The avatar at toolbar size, as a non-template image.
    @MainActor
    static func toolbarImage(_ account: AccountSummary) -> NSImage {
        let renderer = ImageRenderer(content: AccountAvatar(account: account, size: 20))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        let image = renderer.nsImage ?? NSImage()
        image.isTemplate = false
        return image
    }

    /// The avatar as a menu image, with a check ring on the current account.
    @MainActor
    /// `unseen`: a dot for new proposals in its Analysis (spec §14.10).
    static func menuImage(_ account: AccountSummary, current: Bool, unseen: Bool = false) -> NSImage {
        let view = AccountAvatar(account: account, size: 18)
            .overlay(Circle().strokeBorder(Color.accentColor, lineWidth: current ? 2 : 0))
            .overlay(alignment: .topTrailing) { if unseen { NewDot() } }
            .padding(Space.hair)
        let renderer = ImageRenderer(content: view)
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        let image = renderer.nsImage ?? NSImage()
        image.isTemplate = false
        return image
    }
}

/// A section of the account switcher: the user's own accounts (no title),
/// then each service account's agents under its name (spec §7.9).
struct AccountMenuGroup: Identifiable, Equatable {
    let id: String
    let title: String?
    let accounts: [AccountSummary]
}

extension AppModel {
    /// The switcher's sections: the user's own accounts first, then agents
    /// grouped under their service account ("AgentMail · you@example.com").
    var accountMenuGroups: [AccountMenuGroup] {
        var groups: [AccountMenuGroup] = []
        let own = accounts.filter { $0.kind != .agent }
        if !own.isEmpty { groups.append(AccountMenuGroup(id: "own", title: nil, accounts: own)) }
        var placed = Set<String>()
        for service in serviceAccounts {
            let agents = service.agentAccountIds.compactMap { id in accounts.first { $0.id == id && $0.kind == .agent } }
            guard !agents.isEmpty else { continue }
            placed.formUnion(agents.map(\.id))
            groups.append(AccountMenuGroup(id: "service-\(service.id)", title: Self.serviceAccountTitle(service),
                                           accounts: agents))
        }
        // Agents whose service account is not listed yet.
        let others = accounts.filter { $0.kind == .agent && !placed.contains($0.id) }
        if !others.isEmpty { groups.append(AccountMenuGroup(id: "agents", title: "Agents", accounts: others)) }
        return groups
    }
}
