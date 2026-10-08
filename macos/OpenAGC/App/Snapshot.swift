import AppKit
import SwiftUI
import os

/// Headless UI verification without Screen Recording permission: launched
/// with `-OpenAGCSnapshot /path.png`, the app renders its own main window
/// to a PNG and quits. Always pair it with `-OpenAGCDataDirectory <tmp>`
/// and `-OpenAGCDemo YES` so no real account is opened (see
/// scripts/snapshot.sh). Options (all user defaults, so also launch args):
///   -OpenAGCSnapshotDelay <seconds>     wait before capturing (default 3)
///   -OpenAGCSnapshotSelectFirst YES     select the first thread first
///   -OpenAGCSnapshotMailbox <id>        switch mailbox first
///   -OpenAGCSnapshotSelectIndex <n>     select row n instead
///   -OpenAGCSnapshotAppearance dark|light
///   -OpenAGCSnapshotSearch <query>      type a search first
///   -OpenAGCSnapshotCompose new|reply|forward   open a composer and
///                                       capture it instead
///   -OpenAGCSnapshotAgentPrompt <text>  ask the agent first (use with
///                                       -OpenAGCFakeAgents YES)
///   -OpenAGCSnapshotProposal <summary>  show a sample approval card
///   -OpenAGCSnapshotWidth <points>      resize the main window first
///   -OpenAGCSnapshotArchive YES         archive the selection first (shows
///                                       the undo notice)
///   -OpenAGCSnapshotAgentPanel YES      open the empty agent column and
///                                       focus the prompt (suggestions)
///   -OpenAGCSnapshotRoutine <runner>    open the Routines window (creating
///                                       a routine if there is none) and
///                                       capture it
///   -OpenAGCSnapshotSyncDebugger YES    open the Sync Debugger and capture it
///   -OpenAGCSnapshotCleanUp <view>      open Clean Up on a view (sender,
///                                       people, subject, mailingList, time,
///                                       social, promotions, size) with its
///                                       largest group ticked, and capture it
///   -OpenAGCSnapshotCleanUpScope all    …on All Mail instead of the Inbox
///   -OpenAGCSnapshotCleanUpProgress YES …with a month of sample history on
///                                       the progress card
///   -OpenAGCSnapshotCleanUpCard YES     …capturing the progress card alone (the
///                                       sidebar's glass hides it)
///   -OpenAGCSnapshotCleanUpUnsubscribe one|all  …and ask to unsubscribe from
///                                       the largest group or every group (the
///                                       confirmation; never confirmed)
///   -OpenAGCSnapshotCleanUpArchive YES  …and archive the ticked group (the
///                                       undo notice)
///   -OpenAGCSnapshotCleanUpLoad loading|ask  …showing every header loading
///                                       (the band) or the question asked
///                                       without IMAP (the sheet), sample
///                                       numbers: the demo has no sync window
///   -OpenAGCSnapshotGuide category|decisions  run a learning pass with the
///                                       fake agent on the demo mailbox, accept
///                                       some proposals, and show the Writing
///                                       Guide (a category, or the decisions)
///   -OpenAGCSnapshotGuidePrompt banner|invite|ready  the writing guide's
///                                       invitation banner, or a prompt sheet
///   -OpenAGCSnapshotAgentMailbox create|agentmail|path|add|switcher|pane|verify|banner|agentmail-banner|domain|domain-ready|two
///                                       the Create an Agent Mailbox sheet (its
///                                       service step; AgentMail's email step;
///                                       with a service account: add or new, and
///                                       the add step), the switcher's sections
///                                       (printed to stderr) and the service-account
///                                       settings (a window of their own), or a
///                                       new mailbox (fake service) with its
///                                       verify sheet, its limits banner (or an
///                                       AgentMail one's), its
///                                       own-domain sheet, or a second agent on
///                                       the same service account (spec §7.9)
///   -OpenAGCSnapshotTaskList YES        add demo tasks, show the task list
///                                       and select the first task
///   -OpenAGCSnapshotTask YES            open the task dialog on the selected
///                                       thread (use -OpenAGCFakeAgents YES)
///                                       and capture the sheet
///   -OpenAGCSnapshotMode pdf            draw through AppKit's PDF (print) path
///   -OpenAGCSnapshotMode layer          render the CALayer tree instead
///                                       (catches layer-only SwiftUI content)
@MainActor
enum Snapshot {
    private static let logger = Logger(subsystem: "ai.actual.openagc", category: "snapshot")

    /// A snapshot run: no prompts that would cover what is captured.
    static var isRequested: Bool { UserDefaults.standard.string(forKey: "OpenAGCSnapshot") != nil }

    static func scheduleIfRequested(delegate: AppDelegate) {
        let defaults = UserDefaults.standard
        guard let path = defaults.string(forKey: "OpenAGCSnapshot") else { return }
        let delay = defaults.object(forKey: "OpenAGCSnapshotDelay") as? Double
            ?? Double(defaults.string(forKey: "OpenAGCSnapshotDelay") ?? "") ?? 3
        switch defaults.string(forKey: "OpenAGCSnapshotAppearance") {
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        default: break
        }
        Task { @MainActor in
            // A slow start (a cold build, a busy machine): wait for the
            // app's model rather than capture a window without one.
            for _ in 0..<300 where !Self.isOpen(delegate.model?.accountState) {
                try? await Task.sleep(for: .milliseconds(100))
            }
            // Some AppKit animations (split view items) only run in the
            // active app.
            NSApp.activate()
            if let width = Double(defaults.string(forKey: "OpenAGCSnapshotWidth") ?? ""),
               let main = NSApp.windows.first(where: { $0.isVisible && !($0 is NSPanel) }) {
                var frame = main.frame
                frame.size.width = width
                // Never saved into the user's preferences.
                AppDelegate.forgetWindowState(main)
                main.setFrame(frame, display: true)
            }
            try? await Task.sleep(for: .seconds(delay / 2))
            if let mailbox = defaults.string(forKey: "OpenAGCSnapshotMailbox") {
                delegate.model?.selectedMailboxID = mailbox
                try? await Task.sleep(for: .milliseconds(500))
            }
            if let query = defaults.string(forKey: "OpenAGCSnapshotSearch") {
                delegate.model?.searchText = query
                try? await Task.sleep(for: .milliseconds(500))
            }
            if let rows = delegate.model?.threads.rows, !rows.isEmpty {
                if let index = Int(defaults.string(forKey: "OpenAGCSnapshotSelectIndex") ?? ""), rows.indices.contains(index) {
                    delegate.model?.selectedThreadID = rows[index].id
                } else if defaults.bool(forKey: "OpenAGCSnapshotSelectFirst") {
                    delegate.model?.selectedThreadID = rows[0].id
                }
            }
            if defaults.bool(forKey: "OpenAGCSnapshotAgentPanel"), let model = delegate.model {
                await model.agent.loadProviders()
                model.agent.isPresented = true
                try? await Task.sleep(for: .milliseconds(300))
                model.focusAgentPrompt()
                try? await Task.sleep(for: .milliseconds(500))
            }
            if defaults.bool(forKey: "OpenAGCSnapshotArchive"), let model = delegate.model {
                model.undo.runsClock = false
                model.archiveSelection()
                try? await Task.sleep(for: .milliseconds(500))
            }
            if let prompt = defaults.string(forKey: "OpenAGCSnapshotAgentPrompt"), let model = delegate.model {
                await model.agent.loadProviders()
                await model.askAgent(prompt)
                try? await Task.sleep(for: .milliseconds(800))
                // A sample approval card: the scripted agent cannot call tools.
                if let summary = defaults.string(forKey: "OpenAGCSnapshotProposal"), let session = model.agent.sessionID {
                    await model.agent.apply(sessionID: session, events: [
                        .actionProposed(actionId: 1, tool: "mail_send", summary: summary, draftId: 1),
                    ])
                }
            }
            var window: NSWindow?
            if let runner = defaults.string(forKey: "OpenAGCSnapshotRoutine"), let model = delegate.model {
                if model.routines.routines.isEmpty {
                    await model.routines.create(runner: RoutineRunner(rawValue: runner) ?? .claudeCloud)
                }
                model.openRoutines?()
                try? await Task.sleep(for: .milliseconds(800))
                window = NSApp.windows.last { $0.isVisible && ($0.identifier?.rawValue.hasPrefix("routines") ?? false) }
            }
            if defaults.bool(forKey: "OpenAGCSnapshotSyncDebugger"), let model = delegate.model {
                model.openSyncDebugger?()
                try? await Task.sleep(for: .milliseconds(1500))
                window = NSApp.windows.last { $0.isVisible && ($0.identifier?.rawValue.hasPrefix("sync-debugger") ?? false) }
            }
            if let view = defaults.string(forKey: "OpenAGCSnapshotCleanUp"), let model = delegate.model {
                model.openCleanUp?()
                try? await Task.sleep(for: .milliseconds(800))
                let store = model.cleanUp
                store.view = CleanUpViewKind(rawValue: view) ?? .sender
                if defaults.string(forKey: "OpenAGCSnapshotCleanUpScope") == "all" { store.scope = .allMail }
                await store.open(accountID: model.openAccountID)
                if let largest = store.groups.first {
                    store.setTicked(true, keys: [largest.key])
                    await store.refreshMessages()
                }
                // A month of history for the progress card: the demo has
                // none, so sample counts falling to today's Inbox.
                if defaults.bool(forKey: "OpenAGCSnapshotCleanUpProgress"), let core = model.core,
                   let account = model.openAccountID, let now = store.progress?.now {
                    let start = Double(now) * 2.6
                    let counts = (0..<30).map { day -> UInt64 in
                        let t = Double(day) / 29
                        let wobble = Double((day * 37) % 11) - 5
                        return UInt64(max(Double(now) + 37, start - (start - Double(now) - 37) * (1 - pow(1 - t, 2)) + wobble * 4))
                    }
                    try? await core.debugSeedInboxHistory(accountID: account, counts: counts, baseline: counts[0] + 140)
                    await store.loadProgress()
                }
                if defaults.bool(forKey: "OpenAGCSnapshotCleanUpArchive") {
                    model.undo.runsClock = false
                    await store.apply(.archive)
                }
                // The confirmation only: nothing is ever confirmed here (the
                // demo's addresses are not to be contacted).
                let unsubscribe = defaults.string(forKey: "OpenAGCSnapshotCleanUpUnsubscribe")
                if let unsubscribe {
                    if unsubscribe == "all" { store.setTicked(true, keys: store.groups.map(\.key)) }
                    await store.refreshMessages()
                    store.askUnsubscribe()
                }
                let load = defaults.string(forKey: "OpenAGCSnapshotCleanUpLoad")
                if load == "loading" {
                    store.headerLoad = CleanUpHeaderLoad(total: 43_118, remaining: 31_406, widened: true)
                } else if load == "ask", let account = model.openAccountID {
                    store.loadQuestion = CleanUpLoadQuestion(accountID: account, messages: 38_412, seconds: 9_219)
                }
                try? await Task.sleep(for: .milliseconds(1200))
                window = NSApp.windows.last { $0.isVisible && ($0.identifier?.rawValue.hasPrefix("cleanup") ?? false) }
                // The sidebar's glass hides the card from self-snapshots:
                // the left column's foot in a window of its own.
                if defaults.bool(forKey: "OpenAGCSnapshotCleanUpCard") {
                    window = cleanUpCardWindow(model)
                    try? await Task.sleep(for: .milliseconds(500))
                }
                if load == "ask" || unsubscribe != nil, let sheet = NSApp.windows.first(where: { $0.isSheet && $0.isVisible }) {
                    window = sheet
                }
            }
            if let guide = defaults.string(forKey: "OpenAGCSnapshotGuide"), let model = delegate.model, let core = model.core {
                await model.agent.loadProviders()
                _ = try? await core.startGuideRun(GuideRunRequest(kind: .latest, count: 40,
                                                                  filter: GuideSampleFilter(excludePeople: [], excludeLabels: []),
                                                                  focus: nil, agent: model.agent.providerID))
                for _ in 0..<100 where (try? await core.guideProgress())?.run?.status != .done {
                    try? await Task.sleep(for: .milliseconds(100))
                }
                let decisions = (try? await core.guideDecisions()) ?? []
                if guide != "decisions" {
                    for entry in decisions.prefix(2) {
                        _ = try? await core.applyGuideEdits([.decide(id: entry.id, status: .accepted)], reason: "snapshot")
                    }
                }
                model.selectedMailboxID = AppModel.guideMailboxID
                model.guideProgress = try? await core.guideProgress()
                await model.guide.load()
                if guide == "decisions" {
                    await model.analysis.load()
                    model.openProposedRules(learning: true)
                } else if guide == "analysis" {
                    // A day of reviews: proposals from edited AI drafts (spec §14.10).
                    try? await core.debugSeedAnalysis()
                    model.analysisProgress = try? await core.analysisProgress()
                    await model.analysis.load()
                    model.openProposedRules()
                    model.analysis.selection = model.analysis.proposals.first.map(AnalysisStore.tag)
                } else if guide == "facts" || guide == "facts-proposed" {
                    // Facts (spec §14.11): a few of each kind, one global.
                    func fact(_ c: String, _ l: String, _ v: String, _ u: FactUse = .free) -> FactEdit {
                        .add(fields: FactFields(category: c, label: l, value: v, use: u, asOf: nil), status: .accepted,
                             source: .you)
                    }
                    let made = try? await core.applyFactEdits([
                        fact("identity", "Preferred name", "Jo"), fact("identity", "Pronouns", "they/them"),
                        fact("availability", "Time zone", "Pacific"),
                        fact("availability", "Calendar link", "https://cal.example.com/jo"),
                        fact("people", "Sam Rivera", "My assistant", .ask), fact("work", "Occupation or role", "Founder"),
                        fact("contact", "Mailing address", "1 Main St", .never),
                    ], reason: "snapshot")
                    if let id = made?.facts.first(where: { $0.label == "Time zone" })?.id {
                        _ = try? await core.makeFactGlobal(id)
                    }
                    model.analysisProgress = try? await core.analysisProgress()
                    if guide == "facts-proposed" {
                        // Proposed facts, reviewed in the detail (spec §14.11).
                        try? await core.debugSeedFactProposals()
                        await model.analysis.load()
                        model.openFacts(proposed: true)
                        await model.facts.load()
                    } else {
                        model.openFacts()
                        await model.facts.load()
                        model.facts.selection = model.facts.facts.first { $0.label == "Calendar link" }.map(FactsStore.tag)
                    }
                } else {
                    model.showGuideCategory("A1")
                }
                try? await Task.sleep(for: .milliseconds(800))
            }
            // The writing guide's own prompts (spec §14.9): banner, invite, ready.
            if let prompt = defaults.string(forKey: "OpenAGCSnapshotGuidePrompt"), let model = delegate.model {
                switch prompt {
                case "banner": model.guideBannerAccount = model.openAccountID
                case "invite": model.guidePrompt = .firstRun
                default: model.guidePrompt = .finished(decisions: 12)
                }
                try? await Task.sleep(for: .milliseconds(800))
                if prompt != "banner", let sheet = NSApp.windows.first(where: { $0.isSheet && $0.isVisible }) {
                    window = sheet
                }
            }
            if let agent = defaults.string(forKey: "OpenAGCSnapshotAgentMailbox"), let model = delegate.model,
               let core = model.core {
                if agent == "create" {
                    model.beginAgentMailbox()
                } else if agent == "agentmail" {
                    // A new AgentMail service account: the email is asked first.
                    model.beginAgentMailbox()
                    try? await Task.sleep(for: .milliseconds(300))
                    model.agentMailboxFlow?.choose(.agentMail)
                    model.agentMailboxFlow?.name = "Research Scout"
                    if model.agentMailboxFlow?.humanEmail.isEmpty == true {
                        model.agentMailboxFlow?.humanEmail = "you@example.com"
                    }
                } else if agent == "agentmail-banner", CoreClient.usesFakeAgentMail,
                          let created = try? await model.createAgentMailbox(service: .agentMail, name: "Research Scout",
                                                                           humanEmail: "you@example.com") {
                    // AgentMail's banner: the core's words, not a plan's numbers.
                    try? core.deliverToAgentMailbox(created.accountId, from: "Ada Lovelace <ada@example.com>",
                                                    subject: "Your library card",
                                                    body: "Welcome! Your card number is on the attached sheet.")
                } else if ["path", "add", "switcher", "pane"].contains(agent), CoreClient.usesFakeAgentMail,
                          let scout = try? await model.createAgentMailbox(name: "Research Scout") {
                    // A service account to add to (ADR 0015).
                    if agent == "switcher" || agent == "pane" {
                        _ = try? await model.addAgent(to: scout.accountId, name: "Writer")
                        _ = try? await model.createAgentMailbox(service: .agentMail, name: "Clerk",
                                                                humanEmail: "you@example.com")
                        await model.switchAccount(to: scout.accountId)
                    }
                    if agent == "path" || agent == "add" {
                        model.beginAgentMailbox()
                        try? await Task.sleep(for: .milliseconds(300))
                        model.agentMailboxFlow?.choose(.primitive)
                        if agent == "add", let flow = model.agentMailboxFlow, let target = flow.existing(.primitive).first {
                            flow.add(to: target)
                            flow.name = "Writer"
                        }
                    }
                    if agent == "switcher" {
                        // AppKit menus do not snapshot: the sections as text.
                        for group in model.accountMenuGroups {
                            let names = group.accounts.map { $0.displayName ?? $0.email }.joined(separator: ", ")
                            FileHandle.standardError.write(Data("switcher section \(group.title ?? "(own)"): \(names)\n".utf8))
                        }
                    }
                    if agent == "pane" {
                        window = Self.serviceAccountWindow(model)
                    }
                } else if CoreClient.usesFakeAgentMail,
                          let created = try? await model.createAgentMailbox(name: "Research Scout") {
                    try? core.deliverToAgentMailbox(created.accountId, from: "Ada Lovelace <ada@example.com>",
                                                    subject: "Your library card",
                                                    body: "Welcome! Your card number is on the attached sheet.")
                    try? core.deliverToAgentMailbox(created.accountId, from: "Northwind Labs <hello@northwind.example>",
                                                    subject: "Confirm your sign-up",
                                                    body: "Click to confirm the account for research-scout.")
                    if agent == "verify" { model.beginAgentVerification(created.accountId) }
                    // Two agents on one service account (ADR 0015): the first's
                    // account id is the service account's.
                    if agent == "two",
                       let writer = try? await core.addAgent(toServiceAccount: created.accountId, name: "Writer") {
                        try? core.deliverToAgentMailbox(writer.accountId, from: "Grace Hopper <grace@example.com>",
                                                        subject: "Draft for review",
                                                        body: "Could you tighten the second paragraph?")
                        await model.reloadAccounts()
                    }
                    if agent == "domain" || agent == "domain-ready" {
                        let added = try? await core.addAgentDomain(created.accountId, domain: "agents.example.com")
                        if agent == "domain-ready", let added {
                            _ = try? await core.checkAgentDomain(created.accountId, domainID: added.id)
                        }
                        model.beginAgentDomain(created.accountId)
                    }
                }
                try? await Task.sleep(for: .milliseconds(1200))
                if !["banner", "agentmail-banner", "switcher", "pane"].contains(agent),
                   let sheet = NSApp.windows.first(where: { $0.isSheet && $0.isVisible }) {
                    window = sheet
                }
            }
            if defaults.bool(forKey: "OpenAGCSnapshotTaskList"), let model = delegate.model {
                await model.seedDemoTasks()
                model.selectedMailboxID = AppModel.tasksMailboxID
                try? await Task.sleep(for: .milliseconds(500))
                model.selectTask(model.tasks.sections().first?.tasks.first?.id)
                try? await Task.sleep(for: .milliseconds(800))
            }
            if defaults.bool(forKey: "OpenAGCSnapshotTask"), let model = delegate.model {
                await model.agent.loadProviders()
                try? await Task.sleep(for: .milliseconds(300))
                await model.openTaskDialog()
                try? await Task.sleep(for: .milliseconds(1200))
                window = NSApp.windows.first { $0.isVisible && $0.sheetParent != nil }
                FileHandle.standardError.write(Data("snapshot task sheet: \(window != nil)\n".utf8))
            }
            if let compose = defaults.string(forKey: "OpenAGCSnapshotCompose"), let model = delegate.model {
                try? await Task.sleep(for: .milliseconds(500))
                switch compose {
                case "reply": model.reply(all: true)
                case "forward": model.forward()
                default: model.compose(.new(to: nil))
                }
                try? await Task.sleep(for: .milliseconds(500))
                // The app may not be active when launched from a script, so
                // there is no key window; find the composer by its scene id.
                window = NSApp.windows.last { $0.isVisible && ($0.identifier?.rawValue.hasPrefix("compose") ?? false) }
                FileHandle.standardError.write(Data("snapshot composer window: \(window != nil)\n".utf8))
            }
            try? await Task.sleep(for: .seconds(delay / 2))
            if let model = delegate.model {
                FileHandle.standardError.write(Data("snapshot state: \(model.accountState) rows=\(model.threads.rows.count) agent=\(model.agent.isPresented)/\(model.agent.entries.count)/\(model.agent.providers.count)\n".utf8))
            } else {
                FileHandle.standardError.write(Data("snapshot state: no model\n".utf8))
            }
            if defaults.bool(forKey: "OpenAGCSnapshotDumpViews"), let root = (window ?? NSApp.windows.first)?.contentView?.superview {
                if let w = window ?? NSApp.windows.first {
                    let line = "window frame=\(w.frame.integral) contentMinSize=\(w.contentMinSize) contentMaxSize=\(w.contentMaxSize) screen=\(w.screen?.visibleFrame.integral ?? .zero)\n"
                    FileHandle.standardError.write(Data(line.utf8))
                }
                dump(root, depth: 0)
                for item in (window ?? NSApp.windows.first)?.toolbar?.items ?? [] {
                    let line = "toolbar item \(item.itemIdentifier.rawValue) label=\"\(item.label)\" toolTip=\(item.toolTip.map { "\"\($0)\"" } ?? "nil")\n"
                    FileHandle.standardError.write(Data(line.utf8))
                }
            }
            capture(window, to: URL(filePath: path))
            // Left open to look at (a scratch demo run only, like every snapshot).
            if defaults.bool(forKey: "OpenAGCSnapshotStay") { return }
            // A sheet left open keeps the app from quitting.
            delegate.model?.closeTaskDialog()
            delegate.model?.guidePrompt = nil
            delegate.model?.agentMailboxSheet = nil
            delegate.model?.cleanUp.loadQuestion = nil
            delegate.model?.cleanUp.unsubscribeQuestion = nil
            for sheet in NSApp.windows where sheet.sheetParent != nil { sheet.sheetParent?.endSheet(sheet) }
            try? await Task.sleep(for: .milliseconds(200))
            NSApp.terminate(nil)
        }
    }

    /// Settings › Accounts' service-account sections, in a window of their own
    /// (the Settings scene cannot be opened from here).
    private static func serviceAccountWindow(_ model: AppModel) -> NSWindow {
        let form = Form {
            ForEach(model.serviceAccounts, id: \.id) { service in
                Section {
                    ServiceAccountPane(service: service)
                    ForEach(service.agentAccountIds.compactMap { id in model.accounts.first { $0.id == id } }, id: \.id) {
                        AccountRow(account: $0, onRemove: {})
                    }
                } header: {
                    Text(AppModel.serviceAccountTitle(service))
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 640, height: 900)
        .environment(model)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 900), styleMask: [.titled],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: form)
        window.makeKeyAndOrderFront(nil)
        return window
    }

    /// Clean Up's left column foot (the progress card) at the column's
    /// width, in a window of its own.
    private static func cleanUpCardWindow(_ model: AppModel) -> NSWindow {
        let size = NSSize(width: 210, height: 300)
        let root = VStack(spacing: 0) {
            Spacer(minLength: 0)
            CleanUpSidebarFooter()
        }
        .frame(width: size.width, height: size.height)
        .background(Color(nsColor: .windowBackgroundColor))
        .environment(model)
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: root)
        window.makeKeyAndOrderFront(nil)
        return window
    }

    private static func isOpen(_ state: AppModel.AccountState?) -> Bool {
        if case .open = state { return true }
        return false
    }

    /// Debugging layouts: the view tree with frames, to stderr.
    private static func dump(_ view: NSView, depth: Int) {
        guard depth < 14 else { return }
        var detail = ""
        if let table = view as? NSTableView { detail = " rows=\(table.numberOfRows)" }
        if let outline = view as? NSOutlineView {
            let expanded = (0..<outline.numberOfRows).filter { outline.isItemExpanded(outline.item(atRow: $0)) }
            detail += " expandedRows=\(expanded)"
        }
        if let text = view as? NSTextField, !text.stringValue.isEmpty { detail = " \"\(text.stringValue)\"" }
        if let tip = view.toolTip { detail += " toolTip=\"\(tip)\"" }
        // The view's own minimum (Auto Layout fitting size): what can make
        // the window grow past the screen.
        let fit = view.fittingSize
        if fit.width > 0 || fit.height > 0 { detail += " fit=\(Int(fit.width))x\(Int(fit.height))" }
        let line = String(repeating: "  ", count: depth) + "\(type(of: view)) \(view.frame.integral) hidden=\(view.isHidden)\(detail)\n"
        FileHandle.standardError.write(Data(line.utf8))
        for sub in view.subviews { dump(sub, depth: depth + 1) }
    }

    private static func capture(_ preferred: NSWindow?, to url: URL) {
        guard let window = preferred ?? NSApp.windows.first(where: { $0.isVisible && !($0 is NSPanel) }),
              let view = window.contentView?.superview ?? window.contentView,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)
        else {
            logger.error("no window to snapshot")
            return
        }
        if UserDefaults.standard.string(forKey: "OpenAGCSnapshotMode") == "pdf" {
            // AppKit's print path draws some SwiftUI content that bitmap
            // caching misses on macOS 26.
            let pdf = view.dataWithPDF(inside: view.bounds)
            guard let image = NSImage(data: pdf),
                  let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
                  let png = bitmap.representation(using: .png, properties: [:]) else { return }
            try? png.write(to: url)
            return
        }
        if UserDefaults.standard.string(forKey: "OpenAGCSnapshotMode") == "layer",
           let layer = view.layer, let context = NSGraphicsContext(bitmapImageRep: rep) {
            context.cgContext.scaleBy(x: CGFloat(rep.pixelsWide) / view.bounds.width,
                                      y: CGFloat(rep.pixelsHigh) / view.bounds.height)
            layer.render(in: context.cgContext)
        } else {
            view.cacheDisplay(in: view.bounds, to: rep)
        }
        guard let png = rep.representation(using: .png, properties: [:]) else { return }
        do {
            try png.write(to: url)
            logger.info("snapshot written to \(url.path, privacy: .public)")
        } catch {
            logger.error("snapshot failed: \(error.localizedDescription, privacy: .public)")
        }
    }
}
