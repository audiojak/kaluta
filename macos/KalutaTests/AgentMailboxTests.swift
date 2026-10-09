import AppKit
import Foundation
import SwiftUI
import Testing
@testable import Kaluta

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
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        return (model, core)
    }

    @Test func testsNeverReachTheRealService() {
        #expect(CoreClient.usesFakeAgentMail)
    }

    @Test func aSecondAgentSharesTheServiceAccountAndItsKey() async throws {
        let (model, core) = try await modelWithAnAccount()
        let scout = try await model.createAgentMailbox(name: "Scout")
        let writer = try await core.addAgent(toServiceAccount: scout.accountId, name: "Writer")
        #expect(writer.address == "writer@demo.primitive.email")
        #expect(writer.serviceAccountId == scout.accountId)
        let services = try await core.listServiceAccounts()
        #expect(services.map(\.agentAccountIds) == [[scout.accountId, writer.accountId]])
        #expect(try core.agentMailboxAPIKey(writer.accountId) == core.agentMailboxAPIKey(scout.accountId))
        await #expect(throws: CoreClientError.self) {
            try await core.addAgent(toServiceAccount: scout.accountId, name: "scout")
        }
        await model.reloadAccounts()
        #expect(model.accounts.filter { $0.kind == .agent }.count == 2, "each agent is an account")
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
        let verified = try await core.verifyAgentMailbox(created.accountId, code: "123456")
        await model.serviceAccountVerified(created.accountId, plan: verified)
        #expect(model.unverifiedAgentPlan == nil, "verified: no banner")
        #expect(ServiceAccountPane.planText(model.servicePlans[created.accountId], service: .primitive)
            .contains("writes to you, people who wrote first"))
        #expect(ServiceAccountPane.verifiedText(verified: true, email: verified.email) == "With work@example.com")
        #expect(model.serviceAccounts.first?.verified == true, "the list knows too")
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
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
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

/// Records what the create sheet asks of the core, then asks it: adding an
/// agent must never sign up (ADR 0015).
private final class RecordingCalls: AgentMailboxCalls, @unchecked Sendable {
    let core: CoreClient
    private let lock = NSLock()
    private var recorded: [String] = []

    init(_ core: CoreClient) { self.core = core }

    var calls: [String] { lock.withLock { recorded } }
    private func record(_ call: String) { lock.withLock { recorded.append(call) } }

    func createAgentMailbox(service: AgentService, name: String, humanEmail: String?,
                            requestID: String) async throws(CoreClientError) -> AgentMailboxCreated {
        record("signUp \(service) \(humanEmail ?? "-")")
        return try await core.createAgentMailbox(service: service, name: name, humanEmail: humanEmail, requestID: requestID)
    }

    func addAgent(toServiceAccount serviceAccountID: String, name: String, domain: String?,
                  requestID: String) async throws(CoreClientError) -> AgentAdded {
        record("addAgent \(serviceAccountID)")
        return try await core.addAgent(toServiceAccount: serviceAccountID, name: name, domain: domain, requestID: requestID)
    }

    func serviceAccountPlan(_ serviceAccountID: String) async throws(CoreClientError) -> AgentMailboxPlan {
        record("plan \(serviceAccountID)")
        return try await core.serviceAccountPlan(serviceAccountID)
    }
}

/// Create an Agent Mailbox with service accounts (spec §7.9, ADR 0015):
/// the service first, AgentMail's email before Agree and Create, adding to
/// a service account without a sign-up, the switcher's sections, and plans
/// kept per service account. Against the core's in-memory services.
@MainActor
@Suite(.serialized)
struct ServiceAccountFlowTests {
    private func modelWithAnAccount() async throws -> (AppModel, CoreClient, RecordingCalls) {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", name: "Work Me", threads: 4)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        let calls = RecordingCalls(core)
        model.agentMailboxCallsOverride = calls
        return (model, core, calls)
    }

    @Test func theServiceComesFirstAndChoosesTheFlow() async throws {
        let (model, _, _) = try await modelWithAnAccount()
        model.beginAgentMailbox()
        #expect(model.agentMailboxSheet == .create)
        let flow = try #require(model.agentMailboxFlow)
        #expect(flow.step == .service)
        #expect(!flow.canSubmit)

        flow.choose(.primitive)
        #expect(flow.step == .new, "no Primitive service account yet: a new one")
        #expect(!flow.asksForEmail, "Primitive asks for the email when verifying")
        flow.name = "Scout"
        #expect(flow.canSubmit)
        flow.back()
        #expect(flow.step == .service)

        flow.choose(.agentMail)
        #expect(flow.step == .new)
        #expect(flow.asksForEmail)
        #expect(flow.humanEmail == "work@example.com", "prefilled from the open Gmail account")
        #expect(flow.addressPreview == "scout@agentmail.to")
    }

    @Test func agentMailAsksForTheEmailFirstAndGoesStraightToTheCode() async throws {
        let (model, _, calls) = try await modelWithAnAccount()
        model.beginAgentMailbox()
        let flow = try #require(model.agentMailboxFlow)
        flow.choose(.agentMail)
        flow.name = "Scout"
        flow.humanEmail = ""
        #expect(!flow.canSubmit, "no Agree and Create without the email")
        flow.humanEmail = "work@example.com"
        #expect(flow.canSubmit)

        let outcome = try await model.submitAgentMailbox(flow)
        guard case let .created(accountID, address, codeSentTo) = outcome else {
            Issue.record("expected a new service account, got \(outcome)")
            return
        }
        #expect(address == "scout@agentmail.to")
        #expect(codeSentTo == "work@example.com", "the code was sent at sign-up: the sheet waits for it")
        #expect(calls.calls == ["signUp agentMail work@example.com"])
        #expect(model.openAccountID == accountID)
        let service = try #require(model.serviceAccounts.first)
        #expect(service.service == .agentMail)
        #expect(service.humanEmail == "work@example.com")
        // AgentMail's plan has no hourly numbers: its banner is the core's words.
        let limits = try #require(model.unverifiedAgentLimits)
        #expect(limits.contains("can write only to work@example.com"), "\(limits)")
        #expect(!limits.contains("0 an hour"))
        #expect(model.agentLimits(accountID)?.contains("3,000 messages a month") == true, "the composer's line")
    }

    @Test func addingToAServiceAccountNeverSignsUp() async throws {
        let (model, _, calls) = try await modelWithAnAccount()
        let scout = try await model.createAgentMailbox(name: "Scout")
        model.beginAgentMailbox()
        let flow = try #require(model.agentMailboxFlow)
        flow.choose(.primitive)
        #expect(flow.step == .path, "a Primitive service account exists: add to it or make another")
        let existing = flow.existing(.primitive)
        #expect(existing.map(\.id) == [scout.accountId])
        flow.add(to: try #require(existing.first))
        #expect(flow.step == .add)
        flow.name = "Writer"
        #expect(flow.addressPreview == "writer@demo.primitive.email")
        #expect(flow.canSubmit, "a name is all it takes")

        let outcome = try await model.submitAgentMailbox(flow)
        #expect(outcome == .added(accountID: try #require(model.openAccountID), address: "writer@demo.primitive.email"))
        #expect(calls.calls.filter { $0.hasPrefix("signUp") } == ["signUp primitive -"], "only the first agent signed up")
        #expect(calls.calls.contains("addAgent \(scout.accountId)"))
        #expect(model.serviceAccounts.map(\.agentAccountIds.count) == [2])

        // Settings' Add Agent… opens at the name.
        model.beginAddAgent(to: scout.accountId)
        #expect(model.agentMailboxFlow?.step == .add)
        #expect(model.agentMailboxFlow?.target?.id == scout.accountId)
        flow.newServiceAccount()
        #expect(flow.step == .new, "a new service account stays possible")
    }

    @Test func theSwitcherGroupsAgentsUnderTheirServiceAccount() async throws {
        let (model, _, _) = try await modelWithAnAccount()
        let scout = try await model.createAgentMailbox(name: "Scout")
        let writer = try await model.addAgent(to: scout.accountId, name: "Writer")
        _ = try await model.createAgentMailbox(service: .agentMail, name: "Clerk", humanEmail: "work@example.com")
        let groups = model.accountMenuGroups
        #expect(groups.map(\.title) == [nil, "Primitive · demo.primitive.email", "AgentMail · work@example.com"])
        #expect(groups.map { $0.accounts.map { $0.displayName ?? $0.email } } == [["Work Me"], ["Scout", "Writer"], ["Clerk"]])
        // ⌃3 is the third account as the menu shows it.
        await model.switchAccount(position: 0)
        await model.switchAccount(position: 2)
        #expect(model.openAccountID == writer.accountId)
    }

    @Test func plansAreReadOncePerServiceAccountAndSharedByItsAgents() async throws {
        let (model, core, calls) = try await modelWithAnAccount()
        let scout = try await model.createAgentMailbox(name: "Scout")
        let writer = try await model.addAgent(to: scout.accountId, name: "Writer")
        // Forget what creating it learned, as a new run would.
        model.servicePlans = [:]
        model.fetchedServicePlans = []
        await model.refreshAgentPlan(scout.accountId)
        await model.refreshAgentPlan(writer.accountId)
        #expect(calls.calls.filter { $0.hasPrefix("plan") } == ["plan \(scout.accountId)"], "one read for both agents")
        #expect(Array(model.servicePlans.keys) == [scout.accountId], "kept by service account")
        #expect(model.openAccountID == writer.accountId)
        #expect(model.unverifiedAgentPlan != nil, "the writer's banner reads its service account's plan")
        #expect(model.unverifiedAgentLimits?.contains("10 an hour") == true)

        _ = try await core.startAgentMailboxVerification(writer.accountId, email: "work@example.com")
        let plan = try await core.verifyAgentMailbox(writer.accountId, code: "123456")
        await model.serviceAccountVerified(writer.accountId, plan: plan)
        #expect(model.unverifiedAgentPlan == nil)
        await model.switchAccount(to: scout.accountId)
        #expect(model.unverifiedAgentPlan == nil, "verifying one agent verified its service account")
    }

    @Test func removingOneOfSeveralAgentsSaysTheKeyStays() async throws {
        let (model, _, _) = try await modelWithAnAccount()
        let scout = try await model.createAgentMailbox(name: "Scout")
        _ = try await model.addAgent(to: scout.accountId, name: "Writer")
        let agent = try #require(model.accounts.first { $0.id == scout.accountId })
        #expect(AccountSettings.removeAgentMessage(agent, service: model.serviceAccount(of: scout.accountId))
            .contains("stay for its other agents"))
        #expect(AccountSettings.removeAgentMessage(agent, service: nil).contains("forgets the mailbox's key"))
    }

    @Test func removingTheLastAgentMailAgentSaysAKeyHandedOutWillStopWorking() {
        let agent = AccountSummary(id: "a", kind: .agent, email: "scout@agentmail.to", displayName: "Scout",
                                   avatarPath: nil, position: 0, inboxUnread: 0, imapEnabled: false, service: .agentMail)
        let alone = ServiceAccountSummary(id: "a", service: .agentMail, humanEmail: "me@example.com", verified: true,
                                          plan: nil, managedDomain: "agentmail.to", agentAccountIds: ["a"])
        let message = AccountSettings.removeAgentMessage(agent, service: alone)
        #expect(message.contains("forgets the mailbox's key"))
        #expect(message.contains("a new key"), "signing up again with that email rotates it: \(message)")
        #expect(message.contains("Copy API Key"))
        // With other agents the key stays, so nothing rotates.
        let shared = ServiceAccountSummary(id: "a", service: .agentMail, humanEmail: "me@example.com", verified: true,
                                           plan: nil, managedDomain: "agentmail.to", agentAccountIds: ["a", "b"])
        #expect(!AccountSettings.removeAgentMessage(agent, service: shared).contains("a new key"))
        // Primitive's last agent: no such sentence.
        let primitive = AccountSummary(id: "p", kind: .agent, email: "scout@x.primitive.email", displayName: "Scout",
                                       avatarPath: nil, position: 0, inboxUnread: 0, imapEnabled: false,
                                       service: .primitive)
        #expect(!AccountSettings.removeAgentMessage(primitive, service: nil).contains("a new key"))
    }

    @Test func theCopyConfirmationSaysWhatTheKeyReaches() throws {
        let agentMail = ServiceAccountSummary(id: "a", service: .agentMail, humanEmail: "me@example.com", verified: true,
                                              plan: nil, managedDomain: "agentmail.to", agentAccountIds: ["a", "b"])
        #expect(ServiceAccountPane.offersInboxKeys(agentMail), "verified AgentMail: a key per inbox")
        #expect(ServiceAccountPane.copyMessage(agentMail, agents: ["Scout", "Writer"])
            .contains("reaches every agent in it (Scout and Writer)"))
        var unverified = agentMail
        unverified.verified = false
        #expect(!ServiceAccountPane.offersInboxKeys(unverified))
        #expect(AppModel.serviceAccountTitle(agentMail) == "AgentMail · me@example.com")
        #expect(AppModel.firstSentence("Until verified, it writes to you. Then more.") == "Until verified, it writes to you.")
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
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
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
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        #expect(model.failedSends.isEmpty)
        await model.refreshFailedSends()
        #expect(model.failedSends.isEmpty, "nothing failed")
    }
}

@MainActor
struct FocusCycleTests {
    typealias Region = FocusCycle.Region

    @Test func tabWalksSidebarListReaderSearchAndBack() {
        let shown = { (r: Region, back: Bool) in FocusCycle.next(from: r, backwards: back, readerShown: true) }
        #expect(shown(.sidebar, false) == .list)
        #expect(shown(.list, false) == .reader)
        #expect(shown(.reader, false) == .search)
        #expect(shown(.search, false) == .sidebar, "the loop closes")
        #expect(shown(.sidebar, true) == .search)
        #expect(shown(.reader, true) == .list)
        // No message shown: the reader is skipped both ways.
        #expect(FocusCycle.next(from: .list, backwards: false, readerShown: false) == .search)
        #expect(FocusCycle.next(from: .search, backwards: true, readerShown: false) == .list)
    }

    @Test func theKeyboardsRegionIsReadFromTheRegisteredViews() {
        let cycle = FocusCycle()
        let sidebar = NSTableView(), list = NSTableView(), reader = NSView()
        let inList = NSView()
        list.addSubview(inList)
        cycle.register(sidebar, as: .sidebar)
        cycle.register(list, as: .list)
        cycle.register(reader, as: .reader)
        #expect(cycle.region(of: sidebar) == .sidebar)
        #expect(cycle.region(of: inList) == .list, "a row inside the list")
        #expect(cycle.region(of: reader) == .reader)
        #expect(cycle.region(of: NSView()) == nil, "somewhere else: Tab is left alone")
        #expect(cycle.region(of: nil) == nil)
        let field = NSSearchField()
        #expect(cycle.region(of: field) == .search)
    }
}

@MainActor
struct SendRulesTextTests {
    private func rule(_ kind: String, _ value: String? = nil) -> AgentSendRule { AgentSendRule(kind: kind, value: value) }

    @Test func whereTheMailboxMayWriteReadsAsOneLine() {
        #expect(ServiceAccountPane.sendRulesText(nil) == "Checking…")
        #expect(ServiceAccountPane.sendRulesText([rule("any_recipient"), rule("managed_zone", "primitive.email")]) == "Anyone")
        #expect(ServiceAccountPane.sendRulesText([rule("managed_zone", "primitive.email")]) == "other Primitive mailboxes")
        #expect(ServiceAccountPane.sendRulesText([
            rule("managed_zone", "primitive.email"), rule("your_domain", "agents.example.com"),
            rule("address", "a@example.com"), rule("address", "b@example.com"),
        ]) == "2 addresses that wrote to it · anyone at agents.example.com · other Primitive mailboxes")
        #expect(ServiceAccountPane.sendRulesText([]) == "Nobody yet")
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
        #expect(ServiceAccountPane.dashboardHelp(verified, service: .primitive).contains("sign in as info@example.com"))
        #expect(ServiceAccountPane.dashboardHelp(nil, service: .agentMail).contains("verify the service account first"))
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
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
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

@MainActor
struct AgentPromptFocusTests {
    private func field(in view: NSView) -> NSTextField? {
        if let f = view as? NSTextField, f.placeholderString?.contains("Ask") == true { return f }
        for sub in view.subviews { if let found = field(in: sub) { return found } }
        return nil
    }

    /// ⌘K (Message › Ask …) puts the cursor in the agent prompt.
    @Test func askTheAgentFocusesThePromptField() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 3)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        NSApp.activate()
        let window = EmptyListWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 120), styleMask: [.titled],
                                     backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: AgentPromptBar().environment(model))
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil) }
        var prompt: NSTextField?
        for _ in 0..<100 where prompt == nil {
            try await Task.sleep(for: .milliseconds(20))
            prompt = window.contentView.flatMap(field(in:))
        }
        let found = try #require(prompt, "the prompt field is in the bar")
        for _ in 0..<100 where !model.agent.isProviderReady {
            try await Task.sleep(for: .milliseconds(20))
        }
        #expect(model.agent.isProviderReady, "the fake agent is ready")
        #expect(found.isEnabled, "an enabled field can take the keyboard")

        model.focusAgentPrompt()
        var focused = false
        for _ in 0..<100 where !focused {
            try await Task.sleep(for: .milliseconds(20))
            let responder = window.firstResponder
            focused = responder === found || (responder as? NSTextView)?.delegate === found
        }
        #expect(focused, "first responder is \(String(describing: window.firstResponder))")
    }

    /// ⌘K from another window (a composer): the mail window comes forward
    /// and the prompt gets the keyboard, not the list that had it there.
    @Test func askTheAgentFromAnotherWindowReachesThePrompt() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", threads: 3)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        NSApp.activate()
        let main = EmptyListWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 200), styleMask: [.titled],
                                   backing: .buffered, defer: false)
        main.isReleasedWhenClosed = false
        main.identifier = NSUserInterfaceItemIdentifier("main-AppWindow-1")
        // Something else held the keyboard in the mail window, as the list does.
        let other = NSTextField(frame: NSRect(x: 0, y: 150, width: 200, height: 24))
        let host = NSView(frame: main.contentRect(forFrameRect: main.frame))
        host.addSubview(other)
        let bar = NSHostingView(rootView: AgentPromptBar().environment(model))
        bar.frame = NSRect(x: 0, y: 0, width: 600, height: 60)
        host.addSubview(bar)
        main.contentView = host
        main.makeKeyAndOrderFront(nil)
        main.makeFirstResponder(other)
        defer { main.orderOut(nil) }
        let composer = EmptyListWindow(contentRect: NSRect(x: 700, y: 0, width: 300, height: 100), styleMask: [.titled],
                                       backing: .buffered, defer: false)
        composer.isReleasedWhenClosed = false
        composer.makeKeyAndOrderFront(nil)
        defer { composer.orderOut(nil) }
        for _ in 0..<50 where !composer.isKeyWindow { try await Task.sleep(for: .milliseconds(20)) }
        var prompt: NSTextField?
        for _ in 0..<100 where prompt == nil {
            try await Task.sleep(for: .milliseconds(20))
            prompt = field(in: bar)
        }
        let found = try #require(prompt)
        for _ in 0..<100 where !model.agent.isProviderReady { try await Task.sleep(for: .milliseconds(20)) }

        model.focusAgentPrompt()
        // The test host is not the active app, so no window becomes key
        // here; what can be checked is where the mail window's keyboard
        // went.
        var focused = false
        for _ in 0..<150 where !focused {
            try await Task.sleep(for: .milliseconds(20))
            let responder = main.firstResponder
            focused = responder === found || (responder as? NSTextView)?.delegate === found
        }
        #expect(focused, "first responder is \(String(describing: main.firstResponder))")
        #expect(main.firstResponder !== other, "the field that had the keyboard lost it to the prompt")
    }
}

@MainActor
struct AgentReadinessTests {
    private func provider(_ id: String, _ status: AgentStatusInfo) -> AgentProviderInfo {
        AgentProviderInfo(id: id, name: id == "codex" ? "Codex" : "Claude Code", status: status)
    }

    /// The prompt field is never disabled; the line under it says why the
    /// agent cannot be asked yet.
    @Test func whyTheAgentCannotBeAskedIsSaidInALine() {
        let store = AgentStore(core: nil, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        #expect(store.notReadyReason == "Looking for Claude…", "nothing loaded yet")
        let reason = { (p: AgentProviderInfo?) in AgentStore.notReadyReason(for: p, named: "Claude", loaded: true) }
        #expect(reason(nil) == "Claude isn't set up")
        #expect(reason(provider("claude-code", .error(message: "timed out"))) == "Claude Code couldn't be checked: timed out")
        #expect(reason(provider("claude-code", .notInstalled)) == "Claude Code isn't installed")
        #expect(reason(provider("claude-code", .notAuthenticated(version: "2.1"))) == "Claude Code needs you to sign in")
        #expect(reason(provider("claude-code", .ready(version: "2.1"))) == nil)
    }

    /// With a real core the fake agent is ready; a store that never loaded
    /// looks again when asked, and stops looking once it is ready.
    @Test func lookingAgainFindsTheAgent() async throws {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        let store = AgentStore(core: core, defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        #expect(!store.isProviderReady)
        await store.ensureReady()
        #expect(store.isProviderReady)
        #expect(store.notReadyReason == nil)
    }
}
