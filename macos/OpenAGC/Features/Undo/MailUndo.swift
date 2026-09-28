import AppKit
import Observation

/// What an undoable mail action did, in the words the notice and
/// Edit › Undo use (spec §14.6a).
struct UndoableAction: Equatable {
    enum Kind: Equatable {
        case archive, moveToInbox, trash, junk, notJunk, read, unread, star, unstar
        case label(String), unlabel(String)
        case send
    }

    let kind: Kind
    let count: Int

    private var conversations: String {
        count == 1 ? "1 conversation" : "\(count.formatted()) conversations"
    }

    /// The name after "Undo" in the Edit menu.
    var actionName: String {
        switch kind {
        case .archive: "Archive"
        case .moveToInbox: "Move to Inbox"
        case .trash: "Move to Trash"
        case .junk: "Mark as Junk"
        case .notJunk: "Not Junk"
        case .read: "Mark as Read"
        case .unread: "Mark as Unread"
        case .star: "Star"
        case .unstar: "Unstar"
        case .label: "Label"
        case .unlabel: "Remove Label"
        case .send: "Send"
        }
    }

    /// The notice's text: "Archived 3 conversations".
    var noticeText: String {
        switch kind {
        case .archive: "Archived \(conversations)"
        case .moveToInbox: "Moved \(conversations) to the Inbox"
        case .trash: "Moved \(conversations) to the Trash"
        case .junk: "Moved \(conversations) to Spam"
        case .notJunk: "Moved \(conversations) out of Spam to the Inbox"
        case .read: "Marked \(conversations) as read"
        case .unread: "Marked \(conversations) as unread"
        case .star: "Starred \(conversations)"
        case .unstar: "Unstarred \(conversations)"
        case let .label(name): "Labeled \(conversations) “\(name)”"
        case let .unlabel(name): "Removed “\(name)” from \(conversations)"
        case .send: "Sending…"
        }
    }
}

/// The acknowledgement shown after an action, with a way back.
struct UndoNotice: Identifiable, Equatable {
    let id = UUID()
    let text: String
    let accountID: String
}

/// Undo for the user's mail actions (spec §14.6a): one undo stack per
/// account, so ⌘Z after switching accounts undoes that account's actions,
/// and the notice that acknowledges each action.
@MainActor
@Observable
final class MailUndo {
    static let noticeDuration: Duration = .seconds(8)
    static let levels = 50

    /// Why the notice's countdown is paused.
    enum Pause: Hashable { case hover, focus, inactiveWindow }

    private(set) var notice: UndoNotice?
    /// Time left before the notice goes.
    private(set) var remaining: Duration = .zero
    private(set) var pauses: Set<Pause> = []
    /// Bumped whenever a stack changes, so menu titles re-evaluate.
    private(set) var revision = 0

    @ObservationIgnored private var managers: [String: UndoManager] = [:]
    @ObservationIgnored private let core: CoreClient?
    /// Told when an undo or redo could not be applied.
    @ObservationIgnored var onError: ((String) -> Void)?
    /// Drives the countdown; tests turn it off and call `advance(by:)`.
    @ObservationIgnored var runsClock = true
    @ObservationIgnored private var clock: Task<Void, Never>?
    /// False for the Undo Send notice, which follows the core's hold.
    @ObservationIgnored private var noticePausable = true
    /// The last undo or redo sent to the core; the next waits for it.
    @ObservationIgnored private var replaying: Task<Void, Never>?

    init(core: CoreClient?) {
        self.core = core
    }

    func manager(for accountID: String) -> UndoManager {
        if let existing = managers[accountID] { return existing }
        let manager = UndoManager()
        manager.levelsOfUndo = Self.levels
        manager.groupsByEvent = false
        managers[accountID] = manager
        return manager
    }

    /// An action the user just took: put it on its account's stack and
    /// acknowledge it.
    func record(_ token: UndoToken, _ action: UndoableAction) {
        let manager = manager(for: token.accountId)
        manager.beginUndoGrouping()
        register(token, action, undoing: true, on: manager)
        manager.endUndoGrouping()
        revision += 1
        show(action.noticeText, accountID: token.accountId)
    }

    /// Register the step that reverses the last one: undo registers a
    /// redo and redo an undo, as UndoManager expects.
    private func register(_ token: UndoToken, _ action: UndoableAction, undoing: Bool, on manager: UndoManager) {
        manager.registerUndo(withTarget: self) { target in
            MainActor.assumeIsolated {
                target.replay(token, action, undoing: undoing, on: manager)
            }
        }
        manager.setActionName(action.actionName)
    }

    private func replay(_ token: UndoToken, _ action: UndoableAction, undoing: Bool, on manager: UndoManager) {
        register(token, action, undoing: !undoing, on: manager)
        revision += 1
        guard let core else { return }
        // In order: ⌘Z ⌘Z must reverse the last action, then the one before.
        let previous = replaying
        replaying = Task {
            await previous?.value
            do {
                if undoing { try await core.undo(token) } else { try await core.redo(token) }
            } catch let error as CoreClientError {
                onError?(error.message)
            } catch {
                onError?(String(describing: error))
            }
        }
    }

    func canUndo(in accountID: String?) -> Bool {
        _ = revision
        return accountID.map { manager(for: $0).canUndo } ?? false
    }

    func canRedo(in accountID: String?) -> Bool {
        _ = revision
        return accountID.map { manager(for: $0).canRedo } ?? false
    }

    /// "Undo Archive", or "Undo" when there is nothing to undo.
    func undoTitle(in accountID: String?) -> String {
        guard canUndo(in: accountID), let accountID else { return "Undo" }
        return manager(for: accountID).undoMenuItemTitle
    }

    func redoTitle(in accountID: String?) -> String {
        guard canRedo(in: accountID), let accountID else { return "Redo" }
        return manager(for: accountID).redoMenuItemTitle
    }

    func undo(in accountID: String?) {
        guard canUndo(in: accountID), let accountID else { return }
        manager(for: accountID).undo()
        dismissNotice()
    }

    func redo(in accountID: String?) {
        guard canRedo(in: accountID), let accountID else { return }
        manager(for: accountID).redo()
    }

    /// A send held for Undo Send: undo takes it back (if it has not gone
    /// yet) and reopens the draft; there is no redo, the user sends again.
    /// The notice lasts as long as the hold.
    func recordSend(accountID: String, holdFor hold: Duration, takeBack: @escaping @MainActor () async -> Void) {
        let manager = manager(for: accountID)
        manager.beginUndoGrouping()
        manager.registerUndo(withTarget: self) { target in
            MainActor.assumeIsolated {
                target.revision += 1
                Task { await takeBack() }
            }
        }
        manager.setActionName(UndoableAction(kind: .send, count: 1).actionName)
        manager.endUndoGrouping()
        revision += 1
        // The hold runs in the core whatever the pointer does, so this
        // notice never pauses: it goes when the message does.
        show(UndoableAction(kind: .send, count: 1).noticeText, accountID: accountID, for: hold, pausable: false)
    }

    // MARK: The notice

    /// Show a notice, replacing any other; VoiceOver hears it without
    /// focus moving (WCAG 4.1.3).
    func show(_ text: String, accountID: String, for duration: Duration = noticeDuration, pausable: Bool = true) {
        notice = UndoNotice(text: text, accountID: accountID)
        remaining = duration
        noticePausable = pausable
        pauses.remove(.hover)
        pauses.remove(.focus)
        announce(text)
        startClock()
    }

    func dismissNotice() {
        notice = nil
        clock?.cancel()
        clock = nil
    }

    func setPaused(_ reason: Pause, _ paused: Bool) {
        if paused { pauses.insert(reason) } else { pauses.remove(reason) }
    }

    /// Count the notice down; it goes when time runs out, unless paused.
    func advance(by elapsed: Duration) {
        guard notice != nil, pauses.isEmpty || !noticePausable else { return }
        remaining -= elapsed
        if remaining <= .zero { dismissNotice() }
    }

    private func startClock() {
        clock?.cancel()
        guard runsClock else { return }
        let step: Duration = .milliseconds(250)
        clock = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: step)
                guard !Task.isCancelled, let self else { return }
                self.advance(by: step)
                if self.notice == nil { return }
            }
        }
    }

    private func announce(_ text: String) {
        guard let window = NSApp?.mainWindow else { return }
        NSAccessibility.post(element: window, notification: .announcementRequested, userInfo: [
            .announcement: "\(text). Undo with Command Z.",
            .priority: NSAccessibilityPriorityLevel.high.rawValue,
        ])
    }
}
