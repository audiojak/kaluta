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
        }
    }

    private var mailWindow: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } content: {
            content
                .navigationSplitViewColumnWidth(min: 300, ideal: 380, max: 560)
        } detail: {
            // The agent column sits beside the reader. (SwiftUI's
            // `.inspector` left its split item collapsed at zero width here.)
            HStack(spacing: 0) {
                detail
                    .frame(maxWidth: .infinity)
                    // The agent prompt floats over the reader as an inset
                    // glass capsule (macOS 26), not a bar pinned to a column.
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        AgentPromptBar()
                            .frame(maxWidth: 680)
                            .padding(.horizontal, Space.xl)
                            .padding(.bottom, Space.l)
                    }
                if model.agent.isPresented {
                    PaneDivider()
                    AgentInspector()
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
        return selectedMailbox.map { LabelTree.leafName($0.name) } ?? "OpenAGC"
    }

    /// Under the title: "12 unread · Important only"; sync status is in
    /// the sidebar's footer.
    private var listSubtitle: String {
        var parts: [String] = []
        if model.threads.searchQuery == nil, let mailbox = selectedMailbox, mailbox.unreadCount > 0 {
            parts.append("\(mailbox.unreadCount.formatted()) unread")
        }
        if model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil, model.inboxImportantOnly {
            parts.append("Important only")
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
                if model.threads.rows.isEmpty {
                    if model.threads.searchQuery != nil {
                        ContentUnavailableView.search(text: model.searchText)
                    } else {
                        ContentUnavailableView("No Conversations", systemImage: "tray")
                    }
                } else {
                    ThreadListView()
                }
            }
            .overlay(alignment: .bottom) { UndoNoticeView() }
            // No drawn rule under the header: the column runs beneath the
            // floating sidebar, and a full-width rule showed through its
            // glass (oagc-0cw). The bar sits in the column's safe area.
            .columnHeader { listHeader }
        }
    }

    /// The list column's header: in the Inbox, the category tabs and a
    /// View Options menu (Important Only, Show Categories) when the
    /// account uses categories, otherwise the Important-only switch; in
    /// every mailbox and search, the filter button.
    private var listHeader: some View {
        @Bindable var model = model
        let inbox = model.selectedMailboxID == "INBOX" && model.threads.searchQuery == nil
        let categories = inbox && InboxCategories.inUse(model.inboxCategoryCounts)
        return ListHeaderBar {
            if categories, model.showCategories {
                CapsuleTabs(tabs: model.inboxCategoryTabs.map {
                    CapsuleTabs.Tab(id: $0.id, title: InboxCategories.title($0.id),
                                    symbol: InboxCategories.symbol($0.id), count: Int($0.unreadCount))
                }, selection: Binding(get: { model.activeInboxCategory },
                                      set: { if let id = $0 { model.inboxCategory = id } }))
                .accessibilityLabel("Categories")
            }
            Spacer(minLength: 0)
            if inbox, !categories {
                Toggle("Important only", isOn: $model.inboxImportantOnly)
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                    .help("Show only the Inbox threads Gmail marked Important")
            }
            ListFilterMenu()
            if categories {
                Menu {
                    Toggle("Important Only", isOn: $model.inboxImportantOnly)
                    Toggle("Show Categories", isOn: $model.showCategories)
                } label: {
                    Label("View Options", systemImage: "ellipsis.circle")
                }
                .labelStyle(.iconOnly)
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .help("View Options")
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
        }
    }
}

private struct RunningImport: Identifiable {
    let id: String
}
