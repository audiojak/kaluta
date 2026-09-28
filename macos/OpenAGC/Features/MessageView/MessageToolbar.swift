import SwiftUI

/// The reader's toolbar, laid out like Mail's: New Message at the leading
/// edge, then groups of glass buttons (reply, reply all, forward | archive,
/// trash | labels | star) and the agent panel's toggle. Every button acts
/// on the same targets as the Message menu.
struct MessageToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model

    private var noTargets: Bool { !model.isMailOpen || model.actionTargets.isEmpty }
    private var noReplyTarget: Bool { !model.isMailOpen || model.replyTargetMessageID == nil || model.isArchive }

    var body: some ToolbarContent {
        ToolbarItem {
            Button("New Message", systemImage: "square.and.pencil") { model.compose(.new(to: nil)) }
                .help(model.isArchive ? AppModel.cannotSendReason : "New Message (⌘N)")
                .disabled(model.isArchive)
        }
        ToolbarSpacer(.flexible)
        ToolbarItemGroup {
            Button("Reply", systemImage: "arrowshape.turn.up.left") { model.reply(all: false) }
                .help("Reply (⌘R)")
                .disabled(noReplyTarget)
            Button("Reply All", systemImage: "arrowshape.turn.up.left.2") { model.reply(all: true) }
                .help("Reply All (⇧⌘R)")
                .disabled(noReplyTarget)
            Button("Forward", systemImage: "arrowshape.turn.up.right") { model.forward() }
                .help("Forward (⇧⌘F)")
                .disabled(noReplyTarget)
        }
        ToolbarSpacer(.fixed)
        ToolbarItemGroup {
            Button("Archive", systemImage: "archivebox") { model.archiveSelection() }
                .help("Archive (E)")
                .disabled(noTargets)
            Button("Move to Trash", systemImage: "trash") { model.trashSelection() }
                .help("Move to Trash (⌘⌫)")
                .disabled(noTargets)
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
            .help(allStarred ? "Unstar (S)" : "Star (S)")
            .disabled(noTargets)
        }
        ToolbarSpacer(.fixed)
        ToolbarItem {
            Button(model.agent.isPresented ? "Hide Agent" : "Show Agent", systemImage: "sparkles") {
                model.agent.isPresented.toggle()
            }
            .help(model.agent.isPresented ? "Hide \(model.agent.providerName) (⌥⌘I)" : "Show \(model.agent.providerName) (⌥⌘I)")
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
        .help("Label (L)")
    }

    private func appliedToAll(_ labelID: String) -> Bool {
        let ids = Set(model.actionTargets)
        let rows = model.threads.rows.filter { ids.contains($0.id) }
        return !rows.isEmpty && rows.allSatisfy { $0.labelIds.contains(labelID) }
    }
}
