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
                .navigationSplitViewColumnWidth(min: 300, ideal: 380)
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
                            .padding(.horizontal, 16)
                            .padding(.bottom, 12)
                    }
                if model.agent.isPresented {
                    Divider()
                    AgentInspector()
                        .frame(width: 340)
                        .transition(.move(edge: .trailing))
                }
            }
            .animation(reduceMotion ? nil : .snappy(duration: 0.2), value: model.agent.isPresented)
        }
        .navigationSubtitle(windowSubtitle)
        // macOS 26: no toolbar background or separator line; content runs
        // under the toolbar with the system's soft scroll-edge effect, so no
        // line stops short at the floating sidebar's edge.
        .toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
        .searchable(text: Bindable(model).searchText, placement: .toolbar, prompt: "Search mail")
        .searchFocused($searchFocused)
        .onChange(of: model.searchFocusRequests) { searchFocused = true }
    }

    /// "Inbox · Important only · Syncing over IMAP — 6,406 left", as Mail
    /// puts mailbox status under the window title.
    private var windowSubtitle: String {
        var parts: [String] = []
        if let id = model.selectedMailboxID, let mailbox = model.mailboxes.mailboxes.first(where: { $0.id == id }) {
            parts.append(LabelTree.leafName(mailbox.name))
            if id == "INBOX", model.inboxImportantOnly { parts.append("Important only") }
        }
        if let status = SyncStatusView.subtitle(for: model) { parts.append(status) }
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
                listHeader
                if model.needsReauthentication {
                    ReauthenticationBanner()
                }
                if let error = model.threads.searchError {
                    Label(error, systemImage: "exclamationmark.magnifyingglass")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(8)
                }
                if model.threads.isSearchingServer {
                    HStack(spacing: 6) {
                        ProgressView().controlSize(.small)
                        Text("Also searching Gmail for older mail…")
                    }
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(8)
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
        }
    }

    /// The list column's header: the Inbox's Important-only switch, and a
    /// rule that separates the title area from the messages.
    @ViewBuilder private var listHeader: some View {
        if model.selectedMailboxID == "INBOX", model.threads.searchQuery == nil {
            @Bindable var model = model
            HStack {
                Spacer()
                Toggle("Important only", isOn: $model.inboxImportantOnly)
                    .toggleStyle(.switch)
                    .controlSize(.mini)
                    .font(.callout)
                    .help("Show only the Inbox threads Gmail marked Important")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 5)
        }
        Divider()
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
        HStack(spacing: 8) {
            Image(systemName: "person.crop.circle.badge.exclamationmark")
            Text("Gmail needs you to sign in again.").font(.callout)
            Spacer()
            Button("Sign In") { Task { await model.signIn(with: .effective()) } }
                .controlSize(.small)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.yellow.opacity(0.15))
    }
}

private struct RunningImport: Identifiable {
    let id: String
}
