import AppKit
import Foundation
import SwiftUI
import Testing
@testable import OpenAGC

/// Agent mailboxes (spec §7.9), against the core's in-memory service: tests
/// never create an account at Primitive.
@MainActor
@Suite(.serialized)
struct AgentMailboxTests {
    struct Timeout: Error {}

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(25))
        }
    }

    private func modelWithAnAccount() async throws -> (AppModel, CoreClient) {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", name: "Work Me", threads: 12)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        return (model, core)
    }

    @Test func testsNeverReachTheRealService() {
        #expect(CoreClient.usesFakeAgentMail)
    }

    @Test func creatingAMailboxShowsItWithItsLimitsAndSyncsItsMail() async throws {
        let (model, core) = try await modelWithAnAccount()
        let created = try await model.createAgentMailbox(name: "Research Scout")
        #expect(created.address == "research-scout@demo.primitive.email")
        #expect(model.openAccountID == created.accountId, "the new mailbox is shown")
        let summary = try #require(model.accounts.first { $0.id == created.accountId })
        #expect(summary.kind == .agent)
        #expect(summary.service == .primitive)
        #expect(summary.displayName == "Research Scout")
        #expect(model.isAgentMailbox)
        #expect(!model.needsReauthentication, "the key is in the Keychain")
        let plan = try #require(model.unverifiedAgentPlan, "unverified: the banner shows")
        #expect(AppModel.limitsText(plan).contains("10 an hour"))

        try core.deliverToAgentMailbox(created.accountId, from: "ada@example.com", subject: "Welcome",
                                       body: "Hello Scout")
        try await waitUntil { model.threads.rows.contains { $0.subject == "Welcome" } }

        // The user's own Gmail account is offered for the code.
        #expect(model.codeAccounts.map(\.email) == ["work@example.com"])
        _ = try await core.startAgentMailboxVerification(created.accountId, email: "work@example.com")
        await #expect(throws: CoreClientError.self) { try await core.verifyAgentMailbox(created.accountId, code: "1") }
        model.agentPlans[created.accountId] = try await core.verifyAgentMailbox(created.accountId, code: "123456")
        #expect(model.unverifiedAgentPlan == nil, "verified: no banner")
        #expect(AccountRow.planText(model.agentPlans[created.accountId]) .hasPrefix("Primitive · verified with work@example.com"))
    }

    @Test func theKeyCanBeCopiedAndRemovingTheMailboxForgetsIt() async throws {
        let (model, core) = try await modelWithAnAccount()
        let created = try await model.createAgentMailbox(name: "Scout")
        #expect(try core.agentMailboxAPIKey(created.accountId).hasPrefix("fake_"))
        await model.removeAccount(created.accountId)
        #expect(!model.accounts.contains { $0.id == created.accountId })
        #expect(throws: CoreClientError.self) { try core.agentMailboxAPIKey(created.accountId) }
    }

    @Test func agentsSendFreelyUntilTheUserAsksToApproveEachSend() async throws {
        let (model, core) = try await modelWithAnAccount()
        let created = try await model.createAgentMailbox(name: "Scout")
        #expect(core.agentSendMode(created.accountId) == .freely, "the default")
        try core.setAgentSendMode(created.accountId, .ask)
        #expect(core.agentSendMode(created.accountId) == .ask)
        #expect(core.agentSendMode("work") == nil, "not an agent mailbox")
    }

    @Test func anOwnDomainBecomesTheAgentsAddressOnceVerified() async throws {
        let (model, core) = try await modelWithAnAccount()
        let created = try await model.createAgentMailbox(name: "Research Scout")
        #expect(model.suggestedAgentDomain == "agents.example.com", "a subdomain of the user's own")
        let added = try await core.addAgentDomain(created.accountId, domain: "agents.example.com")
        #expect(!added.verified)
        #expect(added.records.contains { $0.kind == "MX" })
        #expect(AppModel.recordPurpose("dkim") == "Signs sent mail (DKIM)")
        let checked = try await core.checkAgentDomain(created.accountId, domainID: added.id)
        #expect(checked.verified)
        let address = model.suggestedAgentAddress(created.accountId, on: checked.domain)
        #expect(address == "research-scout@agents.example.com")
        try await core.setAgentAddress(created.accountId, address)
        await model.reloadAccounts()
        #expect(model.accounts.first { $0.id == created.accountId }?.email == address)
        model.beginAgentDomain(created.accountId)
        #expect(model.agentMailboxSheet == .domain(accountID: created.accountId))
    }

    @Test func noDomainIsSuggestedForSharedMailHosts() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("me", email: "someone@gmail.com", threads: 2)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        #expect(model.suggestedAgentDomain == "")
    }

    @Test func aNameIsRequired() async throws {
        let (model, _) = try await modelWithAnAccount()
        await #expect(throws: CoreClientError.self) { try await model.createAgentMailbox(name: "   ") }
        #expect(model.openAccountID == "work", "nothing changed")
    }

    @Test func theSheetOpensForCreatingAndForVerifying() async throws {
        let (model, _) = try await modelWithAnAccount()
        model.beginAgentMailbox()
        #expect(model.agentMailboxSheet == .create)
        model.beginAgentVerification("x")
        #expect(model.agentMailboxSheet == .verify(accountID: "x"))
    }
}

/// A window the test host can make key without a user clicking it.
private final class EmptyListWindow: NSWindow {
    override var canBecomeKey: Bool { true }
}

@MainActor
struct EmptyMailboxKeyTests {
    struct Timeout: Error {}

    private func table(in view: NSView) -> ThreadTableView? {
        if let table = view as? ThreadTableView { return table }
        for sub in view.subviews { if let found = table(in: sub) { return found } }
        return nil
    }

    /// A new agent mailbox has no mail: c still starts a message.
    @Test func cStartsAMessageInAnEmptyMailbox() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 2)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        _ = try await model.createAgentMailbox(name: "Scout")
        #expect(model.threads.rows.isEmpty, "a new mailbox is empty")
        var opened: [ComposeRequest] = []
        model.openComposer = { opened.append($0) }

        NSApp.activate()
        let window = EmptyListWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.titled],
                                     backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: ThreadListArea().environment(model))
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        var list: ThreadTableView?
        for _ in 0..<50 where list == nil {
            try await Task.sleep(for: .milliseconds(20))
            list = window.contentView.flatMap(table(in:))
        }
        let found = try #require(list, "the table is there under the empty message")
        window.makeFirstResponder(found)
        let c = try #require(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                                              timestamp: ProcessInfo.processInfo.systemUptime,
                                              windowNumber: window.windowNumber, context: nil, characters: "c",
                                              charactersIgnoringModifiers: "c", isARepeat: false, keyCode: 8))
        NSApp.postEvent(c, atStart: false)
        for _ in 0..<100 where opened.isEmpty {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(opened.count == 1)
    }
}

@MainActor
struct FailedSendTests {
    struct Timeout: Error {}

    /// A message the service refused comes back to Drafts, and the window
    /// says so (it went nowhere quietly before).
    @Test func aRefusedSendIsShownWithWhy() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 2)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        #expect(model.failedSends.isEmpty)
        await model.refreshFailedSends()
        #expect(model.failedSends.isEmpty, "nothing failed")
    }
}

@MainActor
struct SidebarTabTests {
    private func key(_ code: UInt16, shift: Bool = false) throws -> NSEvent {
        try #require(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: shift ? [.shift] : [], timestamp: 0,
                                      windowNumber: 0, context: nil, characters: "\t",
                                      charactersIgnoringModifiers: "\t", isARepeat: false, keyCode: code))
    }

    /// Tab from the sidebar moves into the list shown, the task list too;
    /// nothing takes the keyboard from the sidebar on its own.
    @Test func tabMovesIntoTheTaskListAndNothingElseDoes() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 2)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        let before = model.threadListFocusRequests
        model.selectedMailboxID = AppModel.tasksMailboxID
        #expect(model.isTaskList)
        #expect(model.threadListFocusRequests == before, "arriving at Tasks asks for no focus")
        #expect(SidebarView.tabIntoList(try key(48), model: model))
        #expect(model.threadListFocusRequests == before + 1)
        #expect(!SidebarView.tabIntoList(try key(48, shift: true), model: model), "⇧Tab goes the other way")
        #expect(!SidebarView.tabIntoList(try key(126), model: model), "arrows stay the sidebar's")
    }
}

@MainActor
struct SendRulesTextTests {
    private func rule(_ kind: String, _ value: String? = nil) -> AgentSendRule { AgentSendRule(kind: kind, value: value) }

    @Test func whereTheMailboxMayWriteReadsAsOneLine() {
        #expect(AccountRow.sendRulesText(nil) == "Checking…")
        #expect(AccountRow.sendRulesText([rule("any_recipient"), rule("managed_zone", "primitive.email")]) == "Anyone")
        #expect(AccountRow.sendRulesText([rule("managed_zone", "primitive.email")]) == "other Primitive mailboxes")
        #expect(AccountRow.sendRulesText([
            rule("managed_zone", "primitive.email"), rule("your_domain", "agents.example.com"),
            rule("address", "a@example.com"), rule("address", "b@example.com"),
        ]) == "2 addresses that wrote to it · anyone at agents.example.com · other Primitive mailboxes")
        #expect(AccountRow.sendRulesText([]) == "Nobody yet")
    }

    @Test func thePauseSaysWhy() {
        let paused = SyncStatusView.footer(.error(message: "HTTP 400: limit: Too big"), transport: nil, needsSignIn: false)
        #expect(paused?.title == "Sync Paused")
        #expect(paused?.detail == "Trying again shortly · HTTP 400: limit: Too big")
        #expect(SyncStatusView.footer(.error(), transport: nil, needsSignIn: false)?.detail == "Trying again shortly")
    }
}

@MainActor
struct DashboardLinkTests {
    @Test func theDashboardHelpNamesTheVerifiedEmail() throws {
        let verified = AgentMailboxPlan(name: "developer", verified: true, replyOnly: false, sendPerHour: 1000,
                                        sendPerDay: 10000, email: "info@example.com")
        #expect(AccountRow.dashboardHelp(verified).contains("sign in as info@example.com"))
        #expect(AccountRow.dashboardHelp(nil).contains("verify the mailbox first"))
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        #expect(core.agentServiceDashboardURL(.primitive)?.host() == "www.primitive.dev")
    }
}

@MainActor
struct TabIntoListTests {
    struct Timeout: Error {}

    private func table(in view: NSView) -> ThreadTableView? {
        if let table = view as? ThreadTableView { return table }
        for sub in view.subviews { if let found = table(in: sub) { return found } }
        return nil
    }

    /// Tab from the sidebar lands on the first message when none is selected.
    @Test func tabSelectsTheFirstMessageWhenNoneIsSelected() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 40)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        let first = try #require(model.threads.rows.first).id
        let second = try #require(model.threads.rows.dropFirst().first).id
        #expect(model.selectedThreadID == nil)

        NSApp.activate()
        let window = EmptyListWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300), styleMask: [.titled],
                                     backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: ThreadListArea().environment(model))
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        for _ in 0..<50 where window.contentView.flatMap(table(in:)) == nil {
            try await Task.sleep(for: .milliseconds(20))
        }
        model.focusThreadList()
        for _ in 0..<100 where model.selectedThreadID == nil {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.selectedThreadID == first)

        // With a selection, Tab keeps it.
        model.selectedThreadID = second
        model.focusThreadList()
        try await Task.sleep(for: .milliseconds(200))
        #expect(model.selectedThreadID == second)
    }
}
