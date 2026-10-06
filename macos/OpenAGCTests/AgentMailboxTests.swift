import Foundation
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
        #expect(AccountRow.planText(model.agentPlans[created.accountId]) == "Primitive · verified with work@example.com")
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
