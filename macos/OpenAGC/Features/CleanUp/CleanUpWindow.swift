import SwiftUI

/// The Clean Up window (spec §14.12): the open account's mail grouped by
/// the view chosen on the left, the groups in the middle, and the
/// messages of the ticked groups on the right. The toolbar's actions
/// apply to every message in the ticked groups, one undo each.
struct CleanUpWindow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let store = model.cleanUp
        NavigationSplitView {
            CleanUpSidebar()
                .navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 260)
        } content: {
            CleanUpGroupsColumn()
                .navigationSplitViewColumnWidth(min: 300, ideal: 380, max: 520)
        } detail: {
            CleanUpMessagesColumn()
        }
        .navigationTitle(model.cleanUpTitle)
        .navigationSubtitle(store.view.title)
        .toolbar { CleanUpToolbar() }
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .focusedSceneValue(\.isCleanUpWindow, true)
        .frame(minWidth: 900, minHeight: 520)
        .task(id: model.openAccountID) { await store.open(accountID: model.openAccountID) }
        .sheet(item: Binding(get: { store.loadQuestion }, set: { if $0 == nil { store.answerLoadQuestion(load: false) } })) {
            CleanUpLoadDialog(question: $0)
        }
        .onAppear {
            store.isShown = true
            ToolbarToolTips.install(model: model)
        }
        .onDisappear { store.isShown = false }
    }
}

extension FocusedValues {
    /// True in the Clean Up window's scene: ⌘Z undoes its actions.
    @Entry var isCleanUpWindow: Bool?
}

/// The views, as a source list, with the progress card under them.
private struct CleanUpSidebar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var store = model.cleanUp
        List(selection: Binding(get: { store.view }, set: { if let view = $0 { store.view = view } })) {
            Section("Group By") {
                ForEach(CleanUpViewKind.shown) { view in
                    Label(view.title, systemImage: view.symbol)
                        .tag(view)
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom, spacing: 0) { CleanUpSidebarFooter() }
    }
}

/// The left column's foot: the Inbox Zero card (spec §14.12), once its
/// numbers are in.
struct CleanUpSidebarFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let progress = model.cleanUp.progress {
            CleanUpProgressCard(progress: progress)
                .padding(.horizontal, Space.m)
                .padding(.bottom, Space.m)
        }
    }
}

/// The filter field and the groups.
private struct CleanUpGroupsColumn: View {
    @Environment(AppModel.self) private var model
    @State private var filter = ""

    var body: some View {
        let store = model.cleanUp
        CleanUpGroupList()
            .overlay { emptyState }
            .overlay(alignment: .bottom) { UndoNoticeView(origin: .cleanUp) }
            .columnHeader { header }
            .onChange(of: store.view) { filter = store.filter }
            // A short pause while typing: one query per word, not per key.
            .task(id: filter) {
                guard filter != store.filter else { return }
                try? await Task.sleep(for: .milliseconds(150))
                guard !Task.isCancelled else { return }
                store.filter = filter
            }
    }

    @ViewBuilder private var header: some View {
        let store = model.cleanUp
        VStack(alignment: .leading, spacing: 0) {
            if let prompt = store.view.filterPrompt {
                ListHeaderBar {
                    TextField(prompt, text: $filter)
                        .textFieldStyle(.roundedBorder)
                        .hoverHelp("Show only the groups whose name, other names or address contain this")
                }
            }
            if let load = store.headerLoad {
                CleanUpLoadBand(load: load)
            }
            if let error = store.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(TypeRole.meta)
                    .foregroundStyle(Tone.failure)
                    .padding(.horizontal, Space.l)
                    .padding(.vertical, Space.s)
            }
        }
    }

    @ViewBuilder private var emptyState: some View {
        let store = model.cleanUp
        if store.groupsLoaded, store.groups.isEmpty {
            if !store.filter.isEmpty {
                ContentUnavailableView.search(text: store.filter)
            } else {
                ContentUnavailableView(store.view.emptyTitle, systemImage: store.view.symbol,
                                       description: Text(Self.emptyText(store.view, scope: store.scope)))
            }
        }
    }

    static func emptyText(_ view: CleanUpViewKind, scope: CleanupScope) -> String {
        if view == .people { return "Senders you have written to are listed here." }
        return scope == .inbox ? "The Inbox is empty." : "There is no mail outside Spam and Trash."
    }
}

/// Every header loading (spec §14.12): how far it has got and, when Clean
/// Up widened the sync window, that it did and where to change it. An info
/// band over the groups, since the groups fill as the headers arrive.
struct CleanUpLoadBand: View {
    @Environment(AppModel.self) private var model
    let load: CleanUpHeaderLoad

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Space.m) {
            Image(systemName: load.done ? "checkmark.circle" : "arrow.down.circle")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: Space.hair) {
                Text(load.text)
                if load.widened {
                    Text(CleanUpHeaderLoad.note)
                        .font(TypeRole.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
            Spacer(minLength: 0)
            if load.listing {
                ProgressView().controlSize(.small)
            } else if !load.done {
                ProgressView(value: load.fraction)
                    .progressViewStyle(.linear)
                    .controlSize(.small)
                    .frame(width: 80)
                    .accessibilityLabel("Headers loaded")
            } else {
                Button("OK") { model.cleanUp.dismissHeaderLoad() }
                    .controlSize(.small)
                    .hoverHelp("Hide this note")
            }
        }
        .font(TypeRole.meta)
        .bandBackground(.info)
        .accessibilityElement(children: .contain)
    }
}

/// Asked before loading all mail without IMAP (spec §14.12): how much
/// there is and how long it takes over the Gmail API.
struct CleanUpLoadDialog: View {
    @Environment(AppModel.self) private var model
    let question: CleanUpLoadQuestion

    var body: some View {
        Dialog(title: "Load All Mail", message: question.message) {
            Text(CleanUpLoadQuestion.detail)
                .font(TypeRole.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        } buttons: {
            CancelButton(title: "Not Now", help: "Clean up the mail already on this Mac (Esc)") {
                model.cleanUp.answerLoadQuestion(load: false)
            }
            Button("Load All Mail") { model.cleanUp.answerLoadQuestion(load: true) }
                .keyboardShortcut(.defaultAction)
                .hoverHelp("Download every message from Gmail; the groups fill as it arrives (Return)")
        }
    }
}

/// "813 messages in 2 groups", then the messages.
private struct CleanUpMessagesColumn: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let store = model.cleanUp
        CleanUpMessageList()
            .overlay {
                if !store.hasTicks {
                    ContentUnavailableView("No Groups Ticked", systemImage: "checklist",
                                           description: Text("Tick groups to see their messages and act on them all at once."))
                }
            }
            .columnHeader {
                if let summary = store.summary {
                    ListHeaderBar {
                        Text("\(Text(summary.messages).fontWeight(.semibold)) in \(Text(summary.groups).fontWeight(.semibold))")
                            .foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                        Button("Untick All") { store.clearTicks() }
                            .buttonStyle(.borderless)
                            .controlSize(.small)
                            .hoverHelp("Untick every group")
                    }
                }
            }
    }
}

/// Inbox or All Mail; the actions; progress while they run and while the
/// changes go to the provider. Like Mail's: archive, trash and spam in one
/// group, then Move.
private struct CleanUpToolbar: ToolbarContent {
    @Environment(AppModel.self) private var model

    var body: some ToolbarContent {
        let store = model.cleanUp
        ToolbarItem(placement: .navigation) {
            Picker("Scope", selection: Binding(get: { store.scope }, set: { store.scope = $0 })) {
                Text("Inbox").tag(CleanupScope.inbox)
                Text("All Mail").tag(CleanupScope.allMail)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .help(ToolbarHelp.text(for: "Scope", model: model) ?? "") // toolbar
        }
        if let status = Self.status(store) {
            ToolbarItem {
                HStack(spacing: Space.s) {
                    ProgressView().controlSize(.small)
                    Text(status).font(TypeRole.meta).foregroundStyle(.secondary)
                }
                .fixedSize()
            }
            .sharedBackgroundVisibility(.hidden)
        }
        ToolbarSpacer(.flexible)
        ToolbarItemGroup {
            Button("Archive", systemImage: "archivebox") { act(.archive) }
                .help(ToolbarHelp.text(for: "Archive", model: model) ?? "") // toolbar
                .disabled(!store.canAct)
            Button("Trash", systemImage: "trash") { act(.trash) }
                .help(ToolbarHelp.text(for: "Trash", model: model) ?? "") // toolbar
                .disabled(!store.canAct)
            Button("Spam", systemImage: "xmark.bin") { act(.spam) }
                .help(ToolbarHelp.text(for: "Spam", model: model) ?? "") // toolbar
                .disabled(!store.canAct)
        }
        ToolbarSpacer(.fixed)
        ToolbarItem {
            CleanUpMoveMenu()
                .disabled(!store.canAct)
        }
    }

    private func act(_ action: CleanupAction) {
        Task { await model.cleanUp.apply(action) }
    }

    /// What is under way: an action, or changes still going to the provider.
    static func status(_ store: CleanUpStore) -> String? {
        if let working = store.working { return working }
        if store.pending > 0 { return "Sending changes to Gmail… \(store.pending.formatted()) left" }
        return nil
    }
}

/// Move…: the Inbox and the user's labels, as in mail's label menu.
private struct CleanUpMoveMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Menu {
            Button("Inbox") { move(to: "INBOX") }
            let nodes = LabelTree.build(model.mailboxes.labels).flatMap(\.flattened).filter { !$0.isGroup }
            if !nodes.isEmpty {
                Divider() // menu
            }
            ForEach(nodes) { node in
                if let labelID = node.mailbox?.labelId {
                    Button(node.path) { move(to: labelID) }
                }
            }
        } label: {
            Label("Move", systemImage: "folder")
        }
        .menuIndicator(.visible)
        .help(ToolbarHelp.text(for: "Move", model: model) ?? "") // toolbar
    }

    private func move(to labelID: String) {
        Task { await model.cleanUp.apply(.move(labelId: labelID)) }
    }
}
