import AppKit
import SwiftUI
import Testing
@testable import Kaluta

/// The invitation bar, as drawn (spec §14.9): a conversation with an
/// invitation shows Accept, Maybe and Decline without anything else asking
/// for it. Until 2026-11-10 the bar read the invitation only once it was
/// already showing, so it never showed.
@MainActor
struct InviteBarTests {
    private func host(_ view: some View) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1000, height: 800), styleMask: [.titled],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: view)
        window.orderFront(nil)
        return window
    }

    @Test func aWorkflowItemReadsItsInvitation() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        _ = try await core.seedWorkflowDemo(threads: 0)
        let detail = try #require(try await core.thread("demowf-meetup"))
        let item = WorkflowItem(key: "thread:demowf-meetup", threadId: "demowf-meetup", taskId: nil, decision: nil)
        let session = WorkflowSession(set: WorkflowSet(id: 1, kind: .triage, items: [item], startedAt: 0, resumed: false))
        session.threads["demowf-meetup"] = detail
        model.workflowSession = session
        #expect(model.invites["demowf-meetup"] == nil)

        let window = host(WorkflowItemView(session: session, item: item).environment(model))
        defer { window.orderOut(nil) }
        for _ in 0..<150 where model.invites["demowf-meetup"] == nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.invites["demowf-meetup"]?.title == "Founders Meetup on the 23rd", "the item asked for its invitation")
    }

    @Test func theReaderReadsItsInvitation() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        _ = try await core.seedWorkflowDemo(threads: 0)
        let store = ReaderStore(core: core)
        await store.show(threadID: "demowf-meetup")
        #expect(store.detail != nil)

        let window = host(ThreadReaderView(store: store, threadID: "demowf-meetup").environment(model))
        defer { window.orderOut(nil) }
        for _ in 0..<150 where model.invites["demowf-meetup"] == nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.invites["demowf-meetup"] != nil, "the reader asked for its invitation")
    }
}
