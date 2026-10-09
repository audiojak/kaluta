import AppKit
import Foundation
import Testing
@testable import Kaluta

/// The task list's single keys (↩ r a f e c ⌫ j k) reach it through a
/// key monitor, before the list's own type-to-select can take the letters
/// (oagc-0pq.8). Real key events, posted to a window.
/// A window the test host can make key without a user clicking it.
private final class KeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}

@MainActor
struct TaskKeysTests {
    private func key(_ chars: String, code: UInt16, window: NSWindow) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                         windowNumber: window.windowNumber, context: nil, characters: chars,
                         charactersIgnoringModifiers: chars, isARepeat: false, keyCode: code)!
    }

    @Test func theListsKeysAreTakenBeforeTheTableSeesThem() async throws {
        NSApp.activate()
        let window = KeyWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.titled],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let table = NSTableView(frame: NSRect(x: 0, y: 0, width: 300, height: 200))
        table.addTableColumn(NSTableColumn(identifier: .init("tasks")))
        window.contentView = table
        window.makeKeyAndOrderFront(nil)
        let focused = window.makeFirstResponder(table)
        defer { window.orderOut(nil) }

        let keys = TaskListKeys()
        keys.table = table
        var taken: [String] = []
        keys.start { event in
            taken.append(event.keyCode == 36 ? "↩" : event.keyCode == 51 ? "⌫" : event.charactersIgnoringModifiers ?? "")
            return true
        }
        defer { keys.stop() }

        let presses: [(String, UInt16)] = [("\r", 36), ("r", 15), ("a", 0), ("f", 3), ("e", 14), ("c", 8),
                                           ("\u{7F}", 51), ("j", 38), ("k", 40)]
        for (chars, code) in presses {
            NSApp.postEvent(key(chars, code: code, window: window), atStart: false)
        }
        for _ in 0..<100 where taken.count < presses.count {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(taken == ["↩", "r", "a", "f", "e", "c", "⌫", "j", "k"],
                "taken \(taken); made first responder \(focused), accepts \(table.acceptsFirstResponder); first responder is table: \(window.firstResponder === table); key \(window.isKeyWindow)")

        // With the table not focused (typing in a field), keys are left alone.
        window.makeFirstResponder(nil)
        #expect(!keys.accepts(key("r", code: 15, window: window)))
    }
}
