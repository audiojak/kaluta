import AppKit
import SwiftUI
import Testing
@testable import Kaluta

/// A window the test host can make key without a user clicking it.
private final class SayKeyWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}

/// On *What do you want to say?*, ⌘Return sends the words and ⌘G generates
/// the email (spec §14.13, amended 2026-11-13): real key presses, from
/// inside the field, on the screen as the app draws it.
@MainActor
struct SayKeysTests {
    /// A key pressed in the field of *What do you want to say?*, as the
    /// app draws it; the writer it acted on.
    private func press(_ characters: String, keyCode: UInt16, typed: String,
                       until done: (WorkflowWriter, WorkflowSession) -> Bool) async throws -> (WorkflowWriter, AppModel) {
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
        let hosting = NSHostingView(rootView: WorkflowHostView(session: session).environment(model))
        // The window keeps its size, whatever the screen inside comes to
        // want (after a send it shows the next item).
        hosting.sizingOptions = []
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer {
            // Nothing goes on laying out in a window no test is looking at.
            window.contentView = nil
            window.orderOut(nil)
        }
        for _ in 0..<100 where !(window.firstResponder is NSTextView) { try await Task.sleep(for: .milliseconds(20)) }
        #expect(window.firstResponder is NSTextView, "the field has the keyboard")
        writer.field = typed
        try await Task.sleep(for: .milliseconds(100))

        let event = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber, context: nil, characters: characters,
            charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode))
        #expect(window.performKeyEquivalent(with: event))
        for _ in 0..<150 where !done(writer, session) { try await Task.sleep(for: .milliseconds(20)) }
        return (writer, model)
    }

    /// ⌘G in the field generates the email from what was typed, for the
    /// user to read: nothing is sent.
    @Test func commandGInTheFieldGeneratesTheEmail() async throws {
        let (writer, model) = try await press("g", keyCode: 5, typed: "Hey Jake!\nThat's great") { writer, _ in
            writer.step != .say
        }
        #expect(writer.step != .say && writer.step != .sending, "⌘G generated the email")
        #expect(writer.said == "Hey Jake!\nThat's great")
        #expect(model.workflowSession?.writer != nil, "still writing: not sent")
        let sent = try await model.core!.threads(in: "SENT", limit: 50).rows
        #expect(!sent.contains { $0.snippet.contains("Hey Jake!") })
    }

    /// ⌘Return in the field sends what was typed, as it is: the same key
    /// that sends everywhere else.
    @Test func commandReturnInTheFieldSendsTheWordsAsWritten() async throws {
        let (writer, model) = try await press("\r", keyCode: 36, typed: "Hey Jake!\nThat's great") { _, session in
            session.writer == nil
        }
        #expect(model.workflowSession?.writer == nil, "sent: back to the items")
        #expect(writer.aiText == nil, "Kaluta wrote nothing")
        let sent = try await model.core!.threads(in: "SENT", limit: 50).rows
        #expect(sent.contains { $0.snippet.contains("Hey Jake!") }, "\(sent.map(\.snippet).prefix(3))")
    }

    /// What the words offer: Send on ⌘Return, Generate on ⌘G, and no key
    /// that does both.
    @Test func theWordsOfferSendAndGenerateOnTheirOwnKeys() async throws {
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
        #expect(WorkflowCommand.send.keyLabel == "⌘Return" && WorkflowCommand.firstPass.keyLabel == "⌘G")
        #expect(WorkflowCommand.command(key: "g", characters: "g", modifiers: .command) == .firstPass)
        #expect(WorkflowCommand.command(key: .return, characters: "\r", modifiers: .command) == .send)

        // Nothing typed: Generate a reply leads; Send has nothing to send.
        var shown = model.workflowActions().filter(\.shown)
        #expect(shown.map(\.title) == ["Send", "Generate a reply", "Cancel"])
        #expect(shown[0].unavailable != nil && shown[1].style == .primary)
        await model.performWorkflow(.send)
        #expect(writer.step == .say, "⌘Return with nothing typed does nothing: it never generates")

        // Typed: Send leads, Generate is beside it.
        writer.field = "Yes, I'll be there."
        shown = model.workflowActions().filter(\.shown)
        #expect(shown.map(\.title) == ["Send", "Generate the email from this", "Cancel"])
        #expect(shown[0].unavailable == nil && shown[0].style == .primary && shown[1].style == .plain)
        await model.performWorkflow(.firstPass)
        #expect(writer.step != .say && writer.said == "Yes, I'll be there.")
    }
}
