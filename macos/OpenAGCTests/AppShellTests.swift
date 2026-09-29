import AppKit
import Testing
@testable import OpenAGC

@MainActor
struct AppShellTests {
    @Test func appDelegateKeepsRunningWhenLastWindowCloses() {
        let delegate = AppDelegate()
        #expect(delegate.applicationShouldTerminateAfterLastWindowClosed(.shared) == false)
    }

    /// On a fresh machine a window saves its first frame into the real
    /// preferences as soon as it gets an autosave name (oagc-aoz).
    @Test func testHostWindowsGetNoFrameAutosaveName() {
        #expect(NSWindow.frameAutosaveRefused)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 200, height: 100),
                              styleMask: [.titled], backing: .buffered, defer: true)
        #expect(window.setFrameAutosaveName("openagc-test-window") == false)
        #expect(window.frameAutosaveName.isEmpty)
        window.saveFrame(usingName: "openagc-test-window")
        #expect(UserDefaults.standard.object(forKey: "NSWindow Frame openagc-test-window") == nil)
    }
}
