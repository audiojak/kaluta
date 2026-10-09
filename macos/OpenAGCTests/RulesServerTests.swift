import Foundation
import Testing
@testable import OpenAGC

/// Publishing an agent mailbox to a rules server (spec §10.6): the sheet's
/// list, the status line and the per-fact switch. No server is contacted
/// except a closed port on this Mac.
@MainActor
@Suite(.serialized)
struct RulesServerTests {
    struct Timeout: Error {}

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(10)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(25))
        }
    }

    private func agentMailbox() async throws -> (AppModel, CoreClient, String) {
        let core = try CoreClient(dataDirectory: CoreClient.testScratch())
        try await core.addDemoAccount("work", email: "work@example.com", name: "Work Me", threads: 4)
        let model = AppModel(core: core, defaults: UserDefaults(suiteName: "openagc-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: false)
        let created = try await model.createAgentMailbox(name: "Research Scout")
        func fact(_ c: String, _ l: String, _ v: String, _ u: FactUse) -> FactEdit {
            .add(fields: FactFields(category: c, label: l, value: v, use: u, asOf: nil), status: .accepted, source: .you)
        }
        _ = try await core.applyFactEdits([
            fact("work", "Occupation or role", "Research assistant", .free),
            fact("people", "Sam Rivera", "The user's assistant", .ask),
            fact("contact", "Mailing address", "1 Main St", .never),
        ], reason: "test")
        _ = try await core.applyGuideEdits([
            .add(fields: GuideEntryFields(category: "B6", kind: .rule, statement: "Call her Annie",
                                          scope: GuideScope(groups: [], people: ["ann@acme.example"], messageTypes: [],
                                                            languages: []), check: nil),
                 status: .accepted, source: .you, origin: nil),
        ], reason: "test")
        return (model, core, created.accountId)
    }

    @Test func theSheetListsWhatGoesAndSharingAFactAddsIt() async throws {
        let (model, core, id) = try await agentMailbox()
        let preview = try await core.rulesPreview(id)
        #expect(preview.entries.map(\.statement) == ["Call her Annie"])
        #expect(RulesPreviewList.detail(preview.entries[0]) == "Rule · to 1 person")
        #expect(preview.facts.map(\.label) == ["Occupation or role"], "Ask before using stays by default")
        #expect(preview.factsKept == 1)
        #expect(RulesPreviewList.footnote(preview).contains("1 fact stays on this Mac"))
        #expect(RulesPreviewList.footnote(preview).contains("Never mail, quotes from your mail, or keys"))

        await model.facts.load()
        let sam = try #require(model.facts.facts.first { $0.label == "Sam Rivera" })
        #expect(!sam.shareWithCloud)
        #expect(CloudShareToggle.caption(sam).contains("cannot ask you first"))
        let never = try #require(model.facts.facts.first { $0.label == "Mailing address" })
        #expect(CloudShareToggle.caption(never) == "Never share facts never go to a rules server.")
        let failure = await model.applyFactEdits([.share(id: sam.id, share: true)], actionName: "Share with Cloud Agents",
                                                 notice: "shared")
        #expect(failure == nil)
        await model.facts.load()
        #expect(model.facts.facts.first { $0.label == "Sam Rivera" }?.shareWithCloud == true)
        let after = try await core.rulesPreview(id)
        #expect(after.facts.map(\.label).sorted() == ["Occupation or role", "Sam Rivera"])
        #expect(after.facts.first { $0.label == "Sam Rivera" }?.askBeforeUsing == true)
    }

    @Test func aServerThatIsNotThereIsSaidAndNothingIsKept() async throws {
        let (_, core, id) = try await agentMailbox()
        await #expect(throws: CoreClientError.self) {
            _ = try await core.rulesPublishStart(id, serverURL: "http://rules.example.com", registrationToken: nil)
        }
        // A closed port on this Mac: registering fails, in words.
        do {
            _ = try await core.rulesPublishStart(id, serverURL: "http://127.0.0.1:9", registrationToken: nil)
            Issue.record("published to a closed port")
        } catch {
            #expect(error.message.contains("Could not reach 127.0.0.1:9"), "\(error.message)")
        }
        #expect(core.rulesPublishStatus(id) == nil)
    }

    @Test func theStatusLineSaysTheVersionAndWhen() async throws {
        let (model, core, id) = try await agentMailbox()
        let before = model.rulesRevision
        let threeMinutesAgo = Date().addingTimeInterval(-180)
        try core.debugSetRulesPublication(id, serverURL: "https://rules.example.com", version: 12,
                                          publishedAt: Int64(threeMinutesAgo.timeIntervalSince1970 * 1000), error: nil)
        try await waitUntil { model.rulesRevision > before }
        let status = try #require(core.rulesPublishStatus(id))
        #expect(RulesServerRow.statusLine(status, now: threeMinutesAgo.addingTimeInterval(180))
            == "https://rules.example.com · Version 12, published 3 minutes ago")
        var waiting = status
        waiting.pending = true
        #expect(RulesServerRow.statusLine(waiting).hasSuffix("· a change waits to go"))
        var first = status
        first.version = nil
        first.publishedAt = nil
        #expect(RulesServerRow.statusLine(first) == "https://rules.example.com · Publishing…")
        first.error = "Could not reach rules.example.com"
        #expect(RulesServerRow.statusLine(first) == "https://rules.example.com · Not published yet")

        // Stopping keeps the record (agents read the last version).
        try await core.rulesPublishStop(id, removeFromServer: false)
        let stopped = try #require(core.rulesPublishStatus(id))
        #expect(!stopped.enabled)
        #expect(RulesServerRow.statusLine(stopped).contains("Stopped; agents read the last version · Version 12"))
    }
}
