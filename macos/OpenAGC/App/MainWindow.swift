import SwiftUI

/// The three-column main window: mailboxes, threads, message (spec §14.3).
struct MainWindow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openWindow) private var openWindow
    @State private var columnVisibility = NavigationSplitViewVisibility.all
    @FocusState private var searchFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            switch model.accountState {
            case .noAccount, .signingIn:
                OnboardingView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            default:
                mailWindow
            }
        }
        .focusedSceneValue(\.isMailWindow, true)
        .sheet(item: Binding(get: { model.taskDraft }, set: { if $0 == nil { model.closeTaskDialog() } })) { draft in
            TaskDialog(draft: draft)
        }
        .sheet(item: Binding(get: { model.taskDoneQuestion },
                             set: { if $0 == nil, model.taskDoneQuestion != nil { Task { await model.answerTaskDone(false) } } })) {
            TaskDoneDialog(question: $0)
        }
        .sheet(item: Binding(get: { model.bulkTasks }, set: { if $0 == nil { model.closeBulkTasks() } })) { draft in
            BulkTaskSheet(draft: draft)
        }
        .sheet(item: Binding(get: { model.importDraft }, set: { model.importDraft = $0 })) { draft in
            ImportMailboxSheet(draft: draft)
        }
        .sheet(item: Binding(get: { model.runningImport.map(RunningImport.init) }, set: { if $0 == nil { model.runningImport = nil } })) { running in
            ImportProgressSheet(accountID: running.id)
                .interactiveDismissDisabled()
        }
        .task { if model.accountState == .starting { await model.start() } }
        .onAppear {
            model.openComposer = { openWindow(id: "compose", value: $0) }
            model.openRoutines = { openWindow(id: "routines") }
            model.openSyncDebugger = { openWindow(id: "sync-debugger") }
            ToolbarToolTips.install(model: model)
        }
    }

    private var mailWindow: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } content: {
            content
                .navigationSplitViewColumnWidth(min: 300, ideal: 380, max: 560)
                .toolbar { ListToolbar() }
        } detail: {
            // The agent column sits beside the reader. (SwiftUI's
            // `.inspector` left its split item collapsed at zero width here.)
            HStack(spacing: 0) {
                // The agent prompt sits in a strip of its own under the
                // reader, so a thread ends above it instead of scrolling
                // beneath it (the reader's web view does not take a safe-area
                // inset, so a floating capsule covered the messages).
                VStack(spacing: 0) {
                    detail
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    if !model.agent.isPresented {
                        AgentPromptBar()
                            .frame(maxWidth: 680)
                            .padding(.horizontal, Space.xl)
                            .padding(.vertical, Space.l)
                    }
                }
                if model.agent.isPresented {
                    PaneDivider()
                    // While the conversation is open the prompt sits under
                    // it, like a chat, so a follow-up goes where the answer
                    // is; closed, it goes back under the reader.
                    VStack(spacing: 0) {
                        AgentInspector()
                            .frame(maxHeight: .infinity)
                        AgentPromptBar()
                            .padding(.horizontal, Space.l)
                            .padding(.vertical, Space.l)
                    }
                    .frame(width: 340)
                    .transition(.move(edge: .trailing))
                }
            }
            .animation(reduceMotion ? nil : .snappy(duration: 0.2), value: model.agent.isPresented)
            .toolbar { MessageToolbar() }
        }
        .navigationTitle(listTitle)
        .navigationSubtitle(listSubtitle)
        // macOS 26: no toolbar background or separator line; content runs
        // under the toolbar with the system's soft scroll-edge effect, so no
        // line stops short at the floating sidebar's edge.
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .searchable(text: Bindable(model).searchText, placement: .toolbar, prompt: "Search mail")
        .searchFocused($searchFocused)
        .onChange(of: model.searchFocusRequests) { searchFocused = true }
    }

    private var selectedMailbox: MailboxInfo? {
        model.mailboxes.mailboxes.first { $0.id == model.selectedMailboxID }
    }

    /// The list column's title, as Mail shows it: the mailbox's name.
    private var listTitle: String {
        if model.threads.searchQuery != nil { return "Search Results" }
        if model.isTaskList { return "Tasks" }
        return selectedMailbox.map { LabelTree.leafName($0.name) } ?? "OpenAGC"
    }

    /// Under the title: "12 unread · Important only"; sync status is in
    /// the sidebar's footer.
    private var listSubtitle: String {
        var parts: [String] = []
        if model.isTaskList {
            if model.tasks.showsDone { return "Done" }
            if model.tasks.openCount > 0 { parts.append("\(model.tasks.openCount.formatted()) open") }
            if model.tasks.dueCount > 0 { parts.append("\(model.tasks.dueCount.formatted()) due") }
            return parts.joined(separator: " · ")
        }
        if model.threads.searchQuery == nil, model.selectedMailboxID == "INBOX", let tab = model.activeInboxCategory,
           let counts = model.inboxCategoryTabs.first(where: { $0.id == tab }) {
            // As Mail puts it: "Primary · 667 unread".
            parts.append(InboxCategories.title(tab))
            if counts.unreadCount > 0 { parts.append("\(counts.unreadCount.formatted()) unread") }
        } else if model.threads.searchQuery == nil, let mailbox = selectedMailbox, mailbox.unreadCount > 0 {
            parts.append("\(mailbox.unreadCount.formatted()) unread")
        }
        if model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil, model.inboxImportantOnly {
            parts.append("Important only")
        }
        if model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil, model.hiddenTaskLabel != nil {
            parts.append("Tasks hidden")
        }
        if !model.listFilters.isEmpty {
            parts.append("Filtered: " + ListFilter.ordered(model.listFilters).map(\.title).joined(separator: ", "))
        }
        if model.isArchive { parts.append("Imported mailbox · cannot send") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder private var content: some View {
        switch model.accountState {
        case .starting:
            ProgressView().controlSize(.small)
        case .noAccount, .signingIn:
            EmptyView()
        case let .failed(message):
            ContentUnavailableView("Something Went Wrong", systemImage: "exclamationmark.triangle", description: Text(message))
        case .open:
            VStack(spacing: 0) {
                if model.needsReauthentication {
                    ReauthenticationBanner()
                }
                if let error = model.threads.searchError {
                    Label(error, systemImage: "exclamationmark.magnifyingglass")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(Space.m)
                }
                if model.threads.isSearchingServer {
                    HStack(spacing: Space.s) {
                        ProgressView().controlSize(.small)
                        Text("Also searching Gmail for older mail…")
                    }
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(Space.m)
                }
                if model.isTaskList {
                    TaskListView()
                } else if model.threads.rows.isEmpty {
                    if model.threads.searchQuery != nil {
                        ContentUnavailableView.search(text: model.searchText)
                    } else {
                        ContentUnavailableView("No Conversations", systemImage: "tray")
                    }
                } else {
                    ThreadListView()
                }
            }
            // Fill the column, so the header stays at the top when the list
            // is empty (it floated to the middle with a short VStack).
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .overlay(alignment: .bottom) { UndoNoticeView() }
            // No drawn rule under the header: the column runs beneath the
            // floating sidebar, and a full-width rule showed through its
            // glass (oagc-0cw). The bar sits in the column's safe area.
            .columnHeader { listHeader }
        }
    }

    /// The row under the title: the Inbox's category tabs, as Mail shows
    /// them, and a tip when there is one; nothing elsewhere. Filter and
    /// View Options are in the title bar (`ListToolbar`).
    @ViewBuilder private var listHeader: some View {
        VStack(spacing: 0) {
            categoryTabs
            taskTabs
            if let tip = model.currentTip {
                TipCard(systemImage: tip.systemImage, title: tip.title, text: tip.text, action: tip.action,
                        actionHelp: tip.actionHelp, dismiss: tip.dismiss,
                        onAction: { model.finishTip(tip, accept: true) },
                        onDismiss: { model.finishTip(tip, accept: false) })
                    .padding(.horizontal, Space.l)
                    .padding(.bottom, Space.m)
            }
        }
    }

    /// The task list's Open and Done, as the Inbox's category tabs.
    @ViewBuilder private var taskTabs: some View {
        if model.isTaskList {
            ListHeaderBar {
                CapsuleTabs(tabs: [
                    CapsuleTabs.Tab(id: "open", title: "Open", symbol: "circle", count: model.tasks.openCount),
                    CapsuleTabs.Tab(id: "done", title: "Done", symbol: "checkmark.circle"),
                ], selection: Binding(get: { model.tasks.showsDone ? "done" : "open" },
                                      set: { model.tasks.showsDone = $0 == "done" }), countNoun: "open")
                .accessibilityLabel("Show")
            }
        }
    }

    @ViewBuilder private var categoryTabs: some View {
        if !model.inboxCategoryTabs.isEmpty, model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil {
            ListHeaderBar {
                CapsuleTabs(tabs: model.inboxCategoryTabs.map {
                    CapsuleTabs.Tab(id: $0.id, title: InboxCategories.title($0.id),
                                    symbol: InboxCategories.symbol($0.id), count: Int($0.unreadCount))
                }, selection: Binding(get: { model.activeInboxCategory },
                                      set: { if let id = $0 { model.inboxCategory = id } }))
                .accessibilityLabel("Categories")
            }
        }
    }

    @ViewBuilder private var detail: some View {
        if model.selectedThreadID != nil {
            ThreadReaderView()
        } else {
            ContentUnavailableView("No Message Selected", systemImage: "envelope.open")
        }
    }
}

/// Google rejected the stored credentials (revoked or expired).
private struct ReauthenticationBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Banner("Gmail needs you to sign in again.", systemImage: "person.crop.circle.badge.exclamationmark",
               intent: .attention) {
            Button("Sign In") { Task { await model.signIn(with: .effective()) } }
                .hoverHelp("Sign in to Google again to keep syncing this account")
        }
    }
}

private struct RunningImport: Identifiable {
    let id: String
}
