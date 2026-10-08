import Foundation
import Observation
import os

/// One of Clean Up's views, the window's left column (spec §14.12).
enum CleanUpViewKind: String, CaseIterable, Identifiable {
    case sender, people, subject, mailingList, time, social, promotions, size

    /// The views the window lists, in order. Mailing Lists (C7), Social
    /// and Promotions (C6) join with a line each when their groups are
    /// ready.
    static let shown: [CleanUpViewKind] = [.sender, .people, .subject, .time, .size]

    var id: String { rawValue }

    var title: String {
        switch self {
        case .sender: "Sender"
        case .people: "People I've Emailed"
        case .subject: "Subject"
        case .mailingList: "Mailing Lists"
        case .time: "Time"
        case .social: "Social"
        case .promotions: "Promotions"
        case .size: "Size"
        }
    }

    var symbol: String {
        switch self {
        case .sender: "person.crop.circle"
        case .people: "person.2"
        case .subject: "text.bubble"
        case .mailingList: "list.bullet.rectangle"
        case .time: "calendar"
        case .social: "bubble.left.and.bubble.right"
        case .promotions: "tag"
        case .size: "externaldrive"
        }
    }

    /// The filter field's prompt; nil where the view has a fixed set of
    /// groups (Time, Size).
    var filterPrompt: String? {
        switch self {
        case .sender, .people: "Type a sender…"
        case .subject: "Type a subject…"
        case .mailingList: "Type a list…"
        case .social, .promotions: "Type a domain…"
        case .time, .size: nil
        }
    }

    /// Group rows have a second line (the address, other names; a size
    /// bucket's range).
    var hasDetailLine: Bool {
        switch self {
        case .sender, .people, .mailingList, .social, .promotions, .size: true
        case .subject, .time: false
        }
    }

    /// The messages column shows each message's size.
    var showsSize: Bool { self == .size }

    /// What the groups are called, for the empty state ("No Senders").
    var emptyTitle: String {
        switch self {
        case .sender: "No Senders"
        case .people: "No One You've Written To"
        case .subject: "No Subjects"
        case .mailingList: "No Mailing Lists Yet"
        case .time, .size: "No Mail"
        case .social: "No Social Mail"
        case .promotions: "No Promotions"
        }
    }

    var core: CleanupView {
        switch self {
        case .sender: .sender
        case .people: .people
        case .subject: .subject
        case .mailingList: .mailingList
        case .time: .time
        case .social: .social
        case .promotions: .promotions
        case .size: .size
        }
    }
}

/// The Clean Up window's state (spec §14.12): the groups of the chosen
/// view, the groups the user ticked, the messages in them, and the bulk
/// actions on them with one undo each.
///
/// Ticks, not the highlight, are what actions act on and what the
/// messages column shows. Highlighting a row (click, arrows, ⇧ and ⌘)
/// only moves the keyboard; a click on the checkbox or Space ticks.
/// Ticks stay while the filter changes, so groups found by several
/// filters can be acted on together; changing the view clears them, and
/// an action that succeeds clears them.
@MainActor
@Observable
final class CleanUpStore {
    /// Messages fetched per page as the messages column scrolls.
    static let pageSize = 200

    private(set) var accountID: String?
    var view: CleanUpViewKind = .sender {
        didSet {
            guard view != oldValue else { return }
            filter = ""
            clearTicks()
            Task { await reload() }
        }
    }
    var scope: CleanupScope = .inbox {
        didSet { if scope != oldValue { Task { await reload() } } }
    }
    var filter = "" {
        didSet { if filter != oldValue { Task { await loadGroups() } } }
    }

    private(set) var groups: [CleanupGroup] = []
    private(set) var groupsLoaded = false
    /// Ticked groups' keys, in the order they were ticked.
    private(set) var tickedKeys: [String] = []
    /// The ticked groups as last seen, for their titles when filtered away.
    private(set) var tickedGroups: [String: CleanupGroup] = [:]
    /// Messages in the ticked groups now.
    private(set) var messageCount = 0
    /// Bumped whenever the messages column must start over.
    private(set) var messageGeneration = 0
    /// What an action is doing, while it runs ("Archiving 813 messages…").
    private(set) var working: String?
    /// The last load or action that failed, in the core's words.
    var error: String?
    /// Changes still waiting to go to the provider (the outbox).
    private(set) var pending: UInt32 = 0
    /// Every header loading (spec §14.12), shown in a band over the groups.
    var headerLoad: CleanUpHeaderLoad?
    /// Asking before loading all mail without IMAP (a sheet).
    var loadQuestion: CleanUpLoadQuestion?
    /// Accounts whose user said Not Now this session.
    @ObservationIgnored var declinedLoads: Set<String> = []
    /// Accounts whose sync window Clean Up widened this session.
    @ObservationIgnored var widenedLoads: Set<String> = []

    @ObservationIgnored let core: CoreClient?
    @ObservationIgnored private let undo: MailUndo
    @ObservationIgnored private var pages: [Int: [CleanupMessage]] = [:]
    @ObservationIgnored private var loadingPages: Set<Int> = []
    @ObservationIgnored private let groupLoads = LatestLoad()
    @ObservationIgnored private let countLoads = LatestLoad()
    /// Bumped whenever the pages are dropped; a page that arrives after is ignored.
    @ObservationIgnored private var pageEpoch = 0
    @ObservationIgnored let logger = Logger(subsystem: "ai.actual.openagc", category: "cleanup")
    /// Bumped when a page arrives, so the table redraws its rows.
    private(set) var pageRevision = 0

    init(core: CoreClient?, undo: MailUndo) {
        self.core = core
        self.undo = undo
    }

    var ticked: Set<String> { Set(tickedKeys) }
    var hasTicks: Bool { !tickedKeys.isEmpty }
    var canAct: Bool { hasTicks && working == nil && accountID != nil && core != nil }

    /// "813 messages in 2 groups", or nil with nothing ticked.
    var summary: (messages: String, groups: String)? {
        guard hasTicks else { return nil }
        let messages = messageCount == 1 ? "1 message" : "\(messageCount.formatted()) messages"
        let groups = tickedKeys.count == 1 ? "1 group" : "\(tickedKeys.count.formatted()) groups"
        return (messages, groups)
    }

    // MARK: Loading

    /// Clean `accountID` (the main window's open account): a different
    /// account starts over.
    func open(accountID: String?) async {
        if accountID != self.accountID {
            self.accountID = accountID
            clearTicks()
            groups = []
            groupsLoaded = false
            error = nil
            pending = 0
            headerLoad = nil
            loadQuestion = nil
        }
        await reload()
        await prepareHeaderLoad()
    }

    /// The groups and the ticked groups' messages, as they are now.
    func reload() async {
        await loadGroups()
        await refreshMessages()
    }

    func loadGroups() async {
        guard let core, let accountID else { return }
        let (view, scope, filter) = (view, scope, filter)
        await groupLoads.run { [self] isCurrent in
            do {
                let loaded = try await core.cleanupGroups(accountID: accountID, view: view.core, scope: scope,
                                                          filter: filter)
                guard isCurrent(), accountID == self.accountID else { return }
                groups = loaded
                groupsLoaded = true
                for group in loaded where tickedGroups[group.key] != nil { tickedGroups[group.key] = group }
            } catch {
                guard isCurrent() else { return }
                self.error = Self.describe(error)
            }
        }
    }

    /// Count the ticked groups' messages again and drop the pages shown.
    /// `restart`: the messages column goes back to the top (new ticks);
    /// otherwise it keeps its place (mail changed elsewhere).
    func refreshMessages(restart: Bool = true) async {
        pages = [:]
        loadingPages = []
        pageEpoch += 1
        if restart { messageGeneration += 1 } else { pageRevision += 1 }
        let (accountID, view, scope, keys) = (accountID, view, scope, tickedKeys)
        await countLoads.run { [self] isCurrent in
            guard let core, let accountID, !keys.isEmpty else {
                messageCount = 0
                return
            }
            do {
                let count = try await core.cleanupCount(accountID: accountID, view: view.core, scope: scope, keys: keys)
                guard isCurrent() else { return }
                messageCount = Int(count)
            } catch {
                guard isCurrent() else { return }
                self.error = Self.describe(error)
            }
        }
    }

    /// The message at `index` of the ticked groups' messages, newest first;
    /// nil while its page loads (the page is asked for).
    func message(at index: Int) -> CleanupMessage? {
        let page = index / Self.pageSize
        if let rows = pages[page] {
            let offset = index % Self.pageSize
            return rows.indices.contains(offset) ? rows[offset] : nil
        }
        loadPage(page)
        return nil
    }

    private func loadPage(_ page: Int) {
        guard let core, let accountID, !loadingPages.contains(page) else { return }
        loadingPages.insert(page)
        let epoch = pageEpoch
        let (view, scope, keys) = (view, scope, tickedKeys)
        Task {
            let rows = try? await core.cleanupMessages(accountID: accountID, view: view.core, scope: scope, keys: keys,
                                                       offset: UInt32(page * Self.pageSize), limit: UInt32(Self.pageSize))
            guard epoch == pageEpoch else { return }
            loadingPages.remove(page)
            pages[page] = rows ?? []
            pageRevision += 1
        }
    }

    // MARK: Ticks

    func isTicked(_ key: String) -> Bool { tickedGroups[key] != nil }

    /// Tick or untick one group (its checkbox).
    func toggle(_ key: String) {
        setTicked(!isTicked(key), keys: [key])
    }

    /// Space on the highlighted rows: tick them all, or untick them all
    /// when every one is ticked already.
    func toggleTicks(_ keys: [String]) {
        guard !keys.isEmpty else { return }
        setTicked(!keys.allSatisfy(isTicked), keys: keys)
    }

    func setTicked(_ on: Bool, keys: [String]) {
        let byKey = Dictionary(groups.map { ($0.key, $0) }, uniquingKeysWith: { first, _ in first })
        var changed = false
        for key in keys {
            if on, tickedGroups[key] == nil, let group = byKey[key] {
                tickedGroups[key] = group
                tickedKeys.append(key)
                changed = true
            } else if !on, tickedGroups.removeValue(forKey: key) != nil {
                tickedKeys.removeAll { $0 == key }
                changed = true
            }
        }
        if changed { Task { await refreshMessages() } }
    }

    func clearTicks() {
        guard hasTicks else { return }
        tickedKeys = []
        tickedGroups = [:]
        Task { await refreshMessages() }
    }

    // MARK: Actions

    /// Apply `action` to every message in the ticked groups: one undoable
    /// action, acknowledged in the window's undo notice.
    func apply(_ action: CleanupAction) async {
        guard let core, let accountID, canAct else { return }
        let keys = tickedKeys
        let (view, scope) = (view, scope)
        working = Self.workingText(action, count: messageCount)
        error = nil
        defer { working = nil }
        do {
            let result = try await core.cleanupApply(accountID: accountID, view: view.core, scope: scope, keys: keys,
                                                     action: action)
            tickedKeys = []
            tickedGroups = [:]
            await reload()
            if let token = result.undo {
                undo.record(accountID: accountID, actionName: result.actionName, noticeText: result.description,
                            origin: .cleanUp,
                            undo: { [weak self] in await self?.replay { try await core.undo(token) } },
                            redo: { [weak self] in await self?.replay { try await core.redo(token) } })
            } else {
                undo.show(result.description, accountID: accountID, offersUndo: false, origin: .cleanUp)
            }
        } catch {
            logger.error("clean up failed: \(error.message, privacy: .private)")
            self.error = error.message
            await reload()
        }
    }

    /// An undo or redo of an action taken here, then the groups again.
    private func replay(_ step: () async throws -> Void) async {
        do {
            try await step()
            error = nil
        } catch {
            self.error = Self.describe(error)
        }
        await reload()
    }

    /// An error in the core's words.
    static func describe(_ error: any Error) -> String {
        (error as? CoreClientError)?.message ?? String(describing: error)
    }

    /// What the toolbar says while an action runs.
    static func workingText(_ action: CleanupAction, count: Int) -> String {
        let messages = count == 1 ? "1 message" : "\(count.formatted()) messages"
        switch action {
        case .archive: return "Archiving \(messages)…"
        case .move: return "Moving \(messages)…"
        case .trash: return "Moving \(messages) to the Trash…"
        case .spam: return "Moving \(messages) to Spam…"
        }
    }

    /// Mail changed (new mail, a sync, an action in the mail window): while
    /// the window is shown, its groups follow, at most every
    /// `refreshInterval`, and not while an action of its own runs.
    func mailChanged() {
        guard isShown, refreshScheduled == nil else { return }
        refreshScheduled = Task { [weak self] in
            try? await Task.sleep(for: Self.refreshInterval)
            guard let self, !Task.isCancelled else { return }
            self.refreshScheduled = nil
            if self.working == nil {
                await self.loadGroups()
                await self.refreshMessages(restart: false)
            }
        }
    }

    static let refreshInterval: Duration = .seconds(2)
    /// The window is open: changes elsewhere refresh it.
    @ObservationIgnored var isShown = false
    @ObservationIgnored private var refreshScheduled: Task<Void, Never>?

    /// The outbox's count for an account changed (a `.outboxStatus` event).
    func outboxChanged(pending: UInt32, accountID: String?) {
        guard accountID == nil || accountID == self.accountID else { return }
        self.pending = pending
    }
}

// MARK: Loading every header (spec §14.12)

/// What opening Clean Up does about mail outside the sync window.
enum CleanUpLoadPlan: Equatable {
    /// Nothing to load: every header is here or on its way, the account
    /// has no sync window (imported, agent, demo), or the user said Not
    /// Now this session.
    case nothing
    /// IMAP: headers are cheap, so the window becomes Everything at once.
    case widen
    /// The Gmail API: headers cost as much as whole messages, so ask first.
    case ask

    static func of(_ status: CleanupLoadStatus, declined: Bool) -> CleanUpLoadPlan {
        guard status.hasSyncWindow, status.window != .everything else { return .nothing }
        if status.cheapHeaders { return .widen }
        return declined ? .nothing : .ask
    }
}

/// The header load Clean Up started or found under way, for the band
/// over the groups.
struct CleanUpHeaderLoad: Equatable {
    /// The window's phases are being listed (Gmail is searched).
    var listing = false
    /// Messages waiting when the load began, or the most seen since.
    var total: UInt64 = 0
    var remaining: UInt64 = 0
    /// Without IMAP every message comes down whole: count bodies too.
    var whole = false
    /// Clean Up changed the sync window: the band says so.
    var widened = false

    var done: Bool { !listing && remaining == 0 }
    var fraction: Double { total == 0 ? 1 : Double(total - min(remaining, total)) / Double(total) }

    /// "Loading headers for all mail — 1,200 of 43,000".
    var text: String {
        if listing { return "Finding older mail in Gmail…" }
        if done { return whole ? "All mail is on this Mac" : "Headers for all mail are on this Mac" }
        let what = whole ? "Loading all mail" : "Loading headers for all mail"
        return "\(what) — \((total - min(remaining, total)).formatted()) of \(total.formatted())"
    }

    static let note = "This account's sync window is now Everything; change it in Settings › Accounts."
}

/// The question asked before loading all mail without IMAP.
struct CleanUpLoadQuestion: Identifiable, Equatable {
    var id: String { accountID }
    let accountID: String
    /// Messages in Gmail not on this Mac, if Gmail said.
    let messages: UInt64?
    let seconds: UInt64?

    var message: String {
        guard let messages, let seconds else {
            return "Clean Up groups the mail on this Mac. Downloading all older mail from Gmail over the Gmail API "
                + "can take hours for a large mailbox."
        }
        let count = messages == 1 ? "1 older message is" : "\(messages.formatted()) older messages are"
        return "Clean Up groups the mail on this Mac. \(count) still only in Gmail; over the Gmail API they take "
            + "about \(Self.duration(seconds)) to download."
    }

    static let detail = "The account's sync window becomes Everything (Settings › Accounts) and Clean Up fills as "
        + "the mail arrives. Signing in again for IMAP there makes downloads much faster."

    /// "2 hours, 52 minutes"; "a minute" for anything shorter.
    static func duration(_ seconds: UInt64) -> String {
        guard seconds >= 60 else { return "a minute" }
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.maximumUnitCount = 2
        formatter.allowedUnits = seconds >= 86_400 ? [.day, .hour] : [.hour, .minute]
        // Rounded to whole minutes, so the count is not falsely exact.
        let minutes = (seconds + 59) / 60
        return formatter.string(from: TimeInterval(minutes * 60)) ?? "\(minutes) minutes"
    }
}

extension CleanUpStore {
    /// Opening Clean Up loads every header (spec §14.12): with IMAP the
    /// sync window becomes Everything at once; over the API the user is
    /// asked first, once a session; a load already under way is shown.
    func prepareHeaderLoad() async {
        guard let core, let accountID,
              let status = try? await core.cleanupLoadStatus(accountID: accountID),
              accountID == self.accountID else { return }
        switch CleanUpLoadPlan.of(status, declined: declinedLoads.contains(accountID)) {
        case .widen:
            await loadAllMail(whole: false)
        case .ask:
            let estimate = try? await core.cleanupLoadEstimate(accountID: accountID)
            guard accountID == self.accountID else { return }
            loadQuestion = CleanUpLoadQuestion(accountID: accountID, messages: estimate?.messages,
                                               seconds: estimate?.seconds)
        case .nothing:
            // Reopened while a load runs: show it again.
            let waiting = status.cheapHeaders ? status.headersWaiting : status.headersWaiting + status.bodiesWaiting
            if status.hasSyncWindow, status.window == .everything, waiting > 0 || widenedLoads.contains(accountID) {
                headerLoad = CleanUpHeaderLoad(total: waiting, remaining: waiting, whole: !status.cheapHeaders,
                                               widened: widenedLoads.contains(accountID))
            }
        }
    }

    /// The answer to the question: Load All Mail, or Not Now (remembered
    /// until the app quits).
    func answerLoadQuestion(load: Bool) {
        guard let question = loadQuestion else { return }
        loadQuestion = nil
        if load {
            Task { await loadAllMail(whole: true) }
        } else {
            declinedLoads.insert(question.accountID)
        }
    }

    /// Widen the sync window to Everything and follow the download.
    func loadAllMail(whole: Bool) async {
        guard let core, let accountID else { return }
        headerLoad = CleanUpHeaderLoad(listing: true, whole: whole, widened: true)
        do {
            _ = try await core.cleanupLoadEveryHeader(accountID: accountID)
            widenedLoads.insert(accountID)
            let status = try await core.cleanupLoadStatus(accountID: accountID)
            guard accountID == self.accountID else { return }
            let waiting = whole ? status.headersWaiting + status.bodiesWaiting : status.headersWaiting
            headerLoad = CleanUpHeaderLoad(total: waiting, remaining: waiting, whole: whole, widened: true)
        } catch {
            logger.error("loading every header failed: \(error.message, privacy: .private)")
            headerLoad = nil
            self.error = error.message
        }
    }

    /// The open account's sync progress (a `.syncStatus` event): the
    /// header load's count follows it.
    func syncChanged(pending: UInt32, headers: UInt32, accountID: String?) {
        guard accountID == nil || accountID == self.accountID, var load = headerLoad, !load.listing else { return }
        let remaining = UInt64(headers) + (load.whole ? UInt64(pending) : 0)
        load.remaining = remaining
        load.total = max(load.total, remaining)
        if load.done, !load.widened {
            headerLoad = nil
        } else {
            headerLoad = load
        }
    }

    /// Put the band away (its OK, once the load is done).
    func dismissHeaderLoad() {
        headerLoad = nil
    }
}

/// Loads where only the newest counts: awaiting `run` returns once the
/// newest load has finished, so a caller that awaits a reload sees its
/// result even when a newer reload (a setting changed meanwhile)
/// superseded its own.
@MainActor
final class LatestLoad {
    private var generation = 0
    private var latest: Task<Void, Never>?

    /// Run `body`, which applies its result only while `isCurrent()`.
    func run(_ body: @escaping @MainActor (_ isCurrent: @escaping @MainActor () -> Bool) async -> Void) async {
        generation += 1
        let mine = generation
        let task = Task { @MainActor in await body { self.generation == mine } }
        latest = task
        var awaited = task
        await task.value
        while let newer = latest, newer != awaited {
            awaited = newer
            await newer.value
        }
    }
}
