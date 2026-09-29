import AppKit
import SwiftUI

/// New Message in the list column's toolbar, at its trailing edge where it
/// meets the reader, as in Mail.
struct ListToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model

    var body: some ToolbarContent {
        if model.isTaskList {
            // The selected task's own actions (spec §14.8).
            ToolbarItemGroup {
                Button(model.tasks.selected?.done == true ? "Mark as Not Done" : "Mark as Done",
                       systemImage: "checkmark.circle") { Task { await model.toggleSelectedTaskDone() } }
                    .help(ToolbarHelp.text(for: "Mark as Done", model: model) ?? "")
                    .disabled(model.tasks.selected == nil)
                Menu("Category", systemImage: "square.grid.2x2") {
                    ForEach(model.tasks.categories, id: \.self) { name in
                        Button(name) { Task { await model.setSelectedTaskCategory(name) } } // no-help: menu
                    }
                }
                .help(ToolbarHelp.text(for: "Category", model: model) ?? "")
                .disabled(model.tasks.selected == nil)
            }
        } else {
            // Filter and View Options beside the title, as in Mail.
            ToolbarItem { ListFilterMenu() }
            if model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil {
                ToolbarItem { ListViewOptionsMenu() }
            }
        }
        ToolbarSpacer(.fixed)
        ToolbarItem {
            Button("New Message", systemImage: "square.and.pencil") { model.compose(.new(to: nil)) }
                .help(ToolbarHelp.text(for: "New Message", model: model) ?? "")
                .disabled(model.isArchive)
        }
    }
}

/// The reader's toolbar, laid out like Mail's: groups of glass buttons (reply, reply all, forward | archive,
/// trash | labels | star) and the agent panel's toggle. Every button acts
/// on the same targets as the Message menu.
struct MessageToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model

    private var noTargets: Bool { !model.isMailOpen || model.actionTargets.isEmpty }
    private var noReplyTarget: Bool { !model.isMailOpen || model.replyTargetMessageID == nil || model.isArchive }

    var body: some ToolbarContent {
        ToolbarItemGroup {
            Button("Reply", systemImage: "arrowshape.turn.up.left") { model.reply(all: false) }
                .help(ToolbarHelp.text(for: "Reply", model: model) ?? "")
                .disabled(noReplyTarget)
            Button("Reply All", systemImage: "arrowshape.turn.up.left.2") { model.reply(all: true) }
                .help(ToolbarHelp.text(for: "Reply All", model: model) ?? "")
                .disabled(noReplyTarget)
            Button("Forward", systemImage: "arrowshape.turn.up.right") { model.forward() }
                .help(ToolbarHelp.text(for: "Forward", model: model) ?? "")
                .disabled(noReplyTarget)
        }
        ToolbarSpacer(.fixed)
        ToolbarItemGroup {
            Button("Archive", systemImage: "archivebox") { model.archiveSelection() }
                .help(ToolbarHelp.text(for: "Archive", model: model) ?? "")
                .disabled(noTargets)
            Button("Move to Trash", systemImage: "trash") { model.trashSelection() }
                .help(ToolbarHelp.text(for: "Move to Trash", model: model) ?? "")
                .disabled(noTargets)
            Button(model.isSpamMailbox ? "Not Junk" : "Mark as Junk",
                   systemImage: model.isSpamMailbox ? "tray.and.arrow.up" : "xmark.bin") {
                model.toggleJunkSelection()
            }
            .help(ToolbarHelp.text(for: model.isSpamMailbox ? "Not Junk" : "Mark as Junk", model: model) ?? "")
            .disabled(noTargets || !model.canJunk)
        }
        ToolbarSpacer(.fixed)
        ToolbarItem {
            LabelToolbarMenu()
                .disabled(noTargets)
        }
        ToolbarItem {
            Button(allStarred ? "Unstar" : "Star", systemImage: allStarred ? "star.fill" : "star") {
                model.toggleStarSelection()
            }
            .help(ToolbarHelp.text(for: allStarred ? "Unstar" : "Star", model: model) ?? "")
            .disabled(noTargets)
        }
        ToolbarSpacer(.fixed)
        ToolbarItem {
            Button(model.agent.isPresented ? "Hide Agent" : "Show Agent", systemImage: "sparkles") {
                model.agent.isPresented.toggle()
            }
            .help(ToolbarHelp.text(for: model.agent.isPresented ? "Hide Agent" : "Show Agent", model: model) ?? "")
        }
    }

    private var allStarred: Bool {
        let ids = Set(model.actionTargets)
        let rows = model.threads.rows.filter { ids.contains($0.id) }
        return !rows.isEmpty && rows.allSatisfy(\.isStarred)
    }
}

/// Labels for the targets, as a menu (the `l` popover's choices): a check
/// on labels every target has; choosing one adds or removes it.
private struct LabelToolbarMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Menu {
            let nodes = LabelTree.build(model.mailboxes.labels).flatMap(\.flattened).filter { !$0.isGroup }
            if nodes.isEmpty {
                Text("No labels yet")
            }
            ForEach(nodes) { node in
                if let labelID = node.mailbox?.labelId {
                    let applied = appliedToAll(labelID)
                    Button {
                        model.setLabel(labelID, applied: !applied)
                    } label: {
                        if applied {
                            Label(node.path, systemImage: "checkmark")
                        } else {
                            Text(node.path)
                        }
                    }
                }
            }
        } label: {
            Label("Label", systemImage: "tag")
        }
        .menuIndicator(.visible)
        .help(ToolbarHelp.text(for: "Label", model: model) ?? "")
    }

    private func appliedToAll(_ labelID: String) -> Bool {
        let ids = Set(model.actionTargets)
        let rows = model.threads.rows.filter { ids.contains($0.id) }
        return !rows.isEmpty && rows.allSatisfy { $0.labelIds.contains(labelID) }
    }
}

/// What each toolbar button says on hover, by its label. SwiftUI's
/// `.help` does not reach the window toolbar on macOS 26 (the
/// `NSToolbarItem`s keep a nil tool tip, so nothing shows), so
/// `ToolbarToolTips` also copies these onto the items; one source for both.
@MainActor
enum ToolbarHelp {
    /// The composer window's toolbar (no model needed).
    static func composer(_ label: String) -> String {
        switch label {
        case "Attach": "Attach files (⇧⌘A)"
        case "Discard": "Delete this draft"
        case "Send": "Send (⇧⌘D)"
        default: label
        }
    }

    static func text(for label: String, model: AppModel) -> String? {
        switch label {
        case "Attach", "Discard", "Send": composer(label)
        case "New Message": model.isArchive ? AppModel.cannotSendReason : "New Message (⌘N)"
        case "Reply": "Reply (⌘R)"
        case "Reply All": "Reply All (⇧⌘R)"
        case "Forward": "Forward (⇧⌘F)"
        case "Archive": "Archive (e)"
        case "Move to Trash": "Move to Trash (⌘⌫)"
        case "Mark as Junk": "Mark as Junk: move to Spam (⇧⌘J)"
        case "Not Junk": "Not Junk: move back to the Inbox (⇧⌘J)"
        case "Label": "Label (l)"
        case "Star": "Star (s)"
        case "Unstar": "Unstar (s)"
        case "Show Agent": "Show \(model.agent.providerName) (⌥⌘I)"
        case "Hide Agent": "Hide \(model.agent.providerName) (⌥⌘I)"
        case "Accounts": AccountMenuButton.helpText(model)
        case "Hide Sidebar": "Hide the sidebar"
        case "Show Sidebar": "Show the sidebar"
        case "Search": "Search mail (⌘F)"
        case "New Routine": "Create a routine that sorts important mail on a schedule"
        case "Mark as Done": model.tasks.selected?.done == true ? "Open the task again (e)" : "Mark the task done (e)"
        case "Mark as Not Done": "Open the task again (e)"
        case "Category": "Move the task to another category (c)"
        case "Filter":
            model.listFilters.isEmpty ? "Filter: show only unread, starred or with attachments"
                : "Filtered: " + ListFilter.ordered(model.listFilters).map(\.title).joined(separator: ", ")
        case "View Options":
            InboxCategories.inUse(model.inboxCategoryCounts) ? "View options: Important Only, Show Categories"
                : "View options: Important Only"
        default: nil
        }
    }
}

/// Keeps the window toolbar's tool tips in step with `ToolbarHelp`: the
/// labels change (Star/Unstar, Show/Hide Agent), so every window update
/// re-checks them; a few dictionary lookups.
@MainActor
enum ToolbarToolTips {
    private static var installed = false

    static func install(model: AppModel) {
        guard !installed else { return }
        installed = true
        NotificationCenter.default.addObserver(forName: NSWindow.didUpdateNotification, object: nil,
                                               queue: .main) { [weak model] _ in
            MainActor.assumeIsolated {
                guard let model else { return }
                for item in NSApp.windows.compactMap(\.toolbar).flatMap(\.items) {
                    let tip = ToolbarHelp.text(for: item.label, model: model)
                    if tip != nil, item.toolTip != tip { item.toolTip = tip }
                }
            }
        }
    }
}
