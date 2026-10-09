import AppKit
import os

/// Owns app-lifecycle concerns SwiftUI does not cover: the dock, Sparkle,
/// URL handling, and the self-snapshot used for headless UI checks.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // The test host starts with an empty Keychain service (a killed
        // run may have left items) and empties it again on quit.
        if CoreClient.isRunningTests { Self.emptyTestKeychain() }
        if CoreClient.isRunningTests || CoreClient.isScratchRun { Self.keepWindowStateOutOfThePreferences() }
        Snapshot.scheduleIfRequested(delegate: self)
    }

    /// Test hosts and snapshots share the app's bundle id, so AppKit would
    /// autosave their window and split-view frames into the user's real
    /// preferences (a snapshot's `-KalutaSnapshotWidth` once resized the
    /// user's saved window). Such runs forget every autosave name as
    /// windows appear and become key; Snapshot does it again before it
    /// resizes anything.
    private static func keepWindowStateOutOfThePreferences() {
        NSApp.windows.forEach(forgetWindowState)
        NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: nil,
                                               queue: .main) { note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated { forgetWindowState(window) }
        }
        // Cheap: only a window that has an autosave name gets the walk.
        NotificationCenter.default.addObserver(forName: NSWindow.didUpdateNotification, object: nil,
                                               queue: .main) { note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated {
                if !window.frameAutosaveName.isEmpty { forgetWindowState(window) }
            }
        }
    }

    /// Clear a window's frame autosave name and its split views'.
    static func forgetWindowState(_ window: NSWindow) {
        window.setFrameAutosaveName("")
        var views: [NSView] = window.contentView.map { [$0] } ?? []
        while let view = views.popLast() {
            (view as? NSSplitView)?.autosaveName = nil
            views.append(contentsOf: view.subviews)
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        if CoreClient.isRunningTests { Self.emptyTestKeychain() }
    }

    private static func emptyTestKeychain() {
        try? KeychainSecretStore(service: CoreClient.testSecretsService).deleteAll()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    /// Messages held for Undo Send go now rather than waiting (spec
    /// §14.6a): quitting is not a reason to lose or delay them. Nothing
    /// held (the usual case): quit at once. Otherwise the sends run off the
    /// main actor and the reply comes through the main run loop, not the
    /// main queue: `terminate` may itself be running on the main queue (a
    /// task called it), and AppKit's wait for the reply would deadlock a
    /// main-actor task.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let core = model?.core, core.unsentSendCountNow() > 0 else { return .terminateNow }
        Task.detached {
            let left = await core.sendHeldNow(timeout: .seconds(5))
            if left > 0 {
                Logger(subsystem: "org.kaluta.Kaluta", category: "app")
                    .info("\(left) held send(s) will go at next launch")
            }
            let main = CFRunLoopGetMain()
            CFRunLoopPerformBlock(main, CFRunLoopMode.commonModes.rawValue) {
                MainActor.assumeIsolated { NSApp.reply(toApplicationShouldTerminate: true) }
            }
            CFRunLoopWakeUp(main)
        }
        return .terminateLater
    }
}

extension NSWindow {
    /// Test hosts and scratch runs only: no window gets a frame autosave
    /// name and no frame is saved by name, so nothing reaches the user's
    /// preferences. On a fresh machine SwiftUI saves the main window's
    /// first frame while creating it, before any notification could clear
    /// the name. Installed once, as the app starts; cannot be undone.
    static func refuseFrameAutosave() {
        guard !frameAutosaveRefused else { return }
        frameAutosaveRefused = true
        swap(#selector(NSWindow.setFrameAutosaveName(_:)), #selector(NSWindow.kaluta_setFrameAutosaveName(_:)))
        swap(#selector(NSWindow.saveFrame(usingName:)), #selector(NSWindow.kaluta_saveFrame(usingName:)))
        // Split views save their column widths under their own names.
        swap(#selector(setter: NSSplitView.autosaveName), #selector(NSSplitView.kaluta_setAutosaveName(_:)),
             in: NSSplitView.self)
    }

    private static func swap(_ original: Selector, _ replacement: Selector, in cls: AnyClass = NSWindow.self) {
        guard let from = class_getInstanceMethod(cls, original),
              let to = class_getInstanceMethod(cls, replacement) else { return }
        method_exchangeImplementations(from, to)
    }

    @MainActor private(set) static var frameAutosaveRefused = false

    /// Swapped in for setFrameAutosaveName(_:): an empty name still clears
    /// (it reaches the original), anything else is refused.
    @objc private func kaluta_setFrameAutosaveName(_ name: NSWindow.FrameAutosaveName) -> Bool {
        if name.isEmpty { return kaluta_setFrameAutosaveName(name) }
        return false
    }

    /// Swapped in for saveFrame(usingName:): saves nothing.
    @objc private func kaluta_saveFrame(usingName name: NSWindow.FrameAutosaveName) {}
}

extension NSSplitView {
    /// Swapped in for the `autosaveName` setter under
    /// `NSWindow.refuseFrameAutosave()`: a name is never set.
    @objc fileprivate func kaluta_setAutosaveName(_ name: NSSplitView.AutosaveName?) {
        kaluta_setAutosaveName(nil)
    }
}
