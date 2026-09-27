import AppKit
import os

/// Owns app-lifecycle concerns SwiftUI does not cover: the dock, Sparkle,
/// URL handling, and the self-snapshot used for headless UI checks.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: AppModel?

    func applicationDidFinishLaunching(_ notification: Notification) {
        Snapshot.scheduleIfRequested(delegate: self)
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
        guard let core = model?.core, core.heldSendCountNow() > 0 else { return .terminateNow }
        Task.detached {
            let left = await core.sendHeldNow(timeout: .seconds(5))
            if left > 0 {
                Logger(subsystem: "ai.actual.openagc", category: "app")
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
