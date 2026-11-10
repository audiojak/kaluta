import AppKit
import SwiftUI
import Testing
@testable import Kaluta

/// A window the test host can make key without a user clicking it.
private final class SayKeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}

/// ⌘Return on *What do you want to say?* generates the email (spec §14.13,
/// amended 2026-11-10): a real key press, from inside the field, on the
/// screen as the app draws it.
@MainActor
struct SayKeysTests {
    @Test func commandReturnInTheFieldGeneratesTheEmail() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        _ = try await core.seedWorkflowDemo(threads: 30)
        await model.beginWorkflow(.triage)
        let session = try #require(model.workflowSession)
        await model.performWorkflow(.replyNow)
        let writer = try #require(session.writer)
        if writer.step == .choose { await model.performWorkflow(.tellKaluta) }
        #expect(writer.step == .say)

        let window = SayKeyWindow(contentRect: NSRect(x: 0, y: 0, width: 1000, height: 800), styleMask: [.titled],
                                  backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: WorkflowHostView(session: session).environment(model))
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        for _ in 0..<100 where !(window.firstResponder is NSTextView) { try await Task.sleep(for: .milliseconds(20)) }
        #expect(window.firstResponder is NSTextView, "the field has the keyboard")
        writer.field = "Hey Jake!\nThat's great"
        try await Task.sleep(for: .milliseconds(100))

        let commandReturn = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: "\r", charactersIgnoringModifiers: "\r",
            isARepeat: false, keyCode: 36))
        #expect(window.performKeyEquivalent(with: commandReturn))
        for _ in 0..<100 where writer.step == .say { try await Task.sleep(for: .milliseconds(20)) }
        #expect(writer.step != .say, "⌘Return generated the email")
        #expect(writer.said == "Hey Jake!\nThat's great")
    }

    /// The screen's own ⌘Return (Send) on the words generates, too.
    @Test func sendOnTheWordsIsGenerate() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        let core = try #require(model.core)
        let row = try #require(try await core.threads(in: "INBOX", limit: 1).rows.first)
        let item = WorkflowItem(key: "thread:\(row.id)", threadId: row.id, taskId: nil, decision: nil)
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .replies, items: [item], startedAt: 0, resumed: false))
        let writer = WorkflowWriter(threadID: row.id, messageID: "m", replyAll: false, taskID: nil, email: "Can you come?")
        session.writer = writer
        model.workflowSession = session
        writer.field = "Yes, I'll be there."
        await model.performWorkflow(.send)
        #expect(writer.step != .say && writer.said == "Yes, I'll be there.")
    }
}
