import Foundation
import Testing
@testable import Kaluta

struct AgentSuggestionModelTests {
    private func texts(_ s: [AgentSuggestion]) -> [String] { s.map(\.text) }

    @Test func oneThreadSelected() {
        let chips = AgentSuggestions.chips(for: SuggestionContext(selectedCount: 1))
        #expect(texts(chips) == ["Summarise this thread", "What is being asked of me here?", "Draft a reply that …",
                                 "Add the label …"])
        #expect(chips[2].fillsOnly && chips[3].fillsOnly && !chips[0].fillsOnly)
        #expect(chips[2].fillText == "Draft a reply that ")
    }

    @Test func anAttachmentAddsItsQuestion() {
        let chips = AgentSuggestions.chips(for: SuggestionContext(selectedCount: 1, hasAttachment: true))
        #expect(texts(chips).prefix(2) == ["Summarise this thread", "What does the attachment say?"])
    }

    @Test func severalThreadsSelected() {
        let chips = AgentSuggestions.contextual(SuggestionContext(selectedCount: 3))
        #expect(texts(chips) == ["Which of these need a reply?", "Summarise these threads",
                                 "Archive these (reversible)", "Label these …"])
    }

    @Test func aSearchInProgress() {
        let chips = AgentSuggestions.contextual(SuggestionContext(searchQuery: "from:billing"))
        #expect(texts(chips).first == "Summarise these results")
        #expect(texts(chips).contains("Find the one that mentions …"))
    }

    @Test func aMailboxWithUnreadMail() {
        let chips = AgentSuggestions.contextual(SuggestionContext(mailboxID: "INBOX", unreadInMailbox: 4))
        #expect(texts(chips).prefix(2) == ["What's new since yesterday?", "Which unread messages need a reply?"])
    }

    @Test func anArchiveAccountIsNeverOfferedDrafting() {
        for context in [SuggestionContext(selectedCount: 1, canDraft: false), SuggestionContext(canDraft: false)] {
            #expect(!AgentSuggestions.chips(for: context).contains { $0.text.contains("Draft") })
        }
        #expect(AgentSuggestions.groups(canDraft: false).map(\.title) == ["Find and summarise", "Tidy up"])
        #expect(AgentSuggestions.groups(canDraft: true).map(\.title) == ["Find and summarise", "Draft for you", "Tidy up"])
        // Recent prompts are filtered the same way.
        let recent = ["Draft a reply saying yes", "Summarise this thread in French"]
        let chips = AgentSuggestions.chips(for: SuggestionContext(selectedCount: 1, canDraft: false), recent: recent)
        #expect(texts(chips).first == "Summarise this thread in French")
        #expect(!texts(chips).contains("Draft a reply saying yes"))
    }

    @Test func sendingIsNeverSuggested() {
        let every = AgentSuggestions.groups(canDraft: true).flatMap(\.examples)
            + [0, 1, 3].flatMap { AgentSuggestions.contextual(SuggestionContext(selectedCount: $0)) }
        #expect(!every.contains { $0.text.lowercased().hasPrefix("send") })
        let chips = AgentSuggestions.chips(for: SuggestionContext(), recent: ["Send it to Alex"])
        #expect(!texts(chips).contains("Send it to Alex"))
    }

    @Test func recentPromptsComeFirstWhenTheyFit() {
        let recent = ["Summarise this thread in two lines", "What did Sam ask for?", "Which of these are invoices?"]
        let one = AgentSuggestions.chips(for: SuggestionContext(selectedCount: 1), recent: recent)
        #expect(texts(one).first == "Summarise this thread in two lines")
        #expect(one.count == AgentSuggestions.chipLimit)
        let none = AgentSuggestions.chips(for: SuggestionContext(), recent: recent)
        #expect(texts(none).first == "What did Sam ask for?")
        let many = AgentSuggestions.chips(for: SuggestionContext(selectedCount: 2), recent: recent)
        #expect(texts(many).first == "Which of these are invoices?")
    }

    @Test func theRestRotateByDayButTheBestStaysFirst() {
        let context = SuggestionContext(selectedCount: 1)
        let monday = AgentSuggestions.chips(for: context, day: 0)
        let tuesday = AgentSuggestions.chips(for: context, day: 1)
        #expect(monday.first == tuesday.first)
        #expect(monday != tuesday)
        #expect(Set(monday) == Set(tuesday), "same examples, another order")
    }

    @Test func recentPromptsAreKeptPerAccountNewestFirstWithoutDuplicates() throws {
        let defaults = try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)"))
        let store = RecentPrompts(defaults: defaults)
        for i in 0..<25 { store.record("prompt \(i)", for: "a") }
        store.record("PROMPT 24", for: "a")
        store.record("other", for: "b")
        let a = store.prompts(for: "a")
        #expect(a.count == RecentPrompts.limit)
        #expect(a.first == "PROMPT 24" && !a.contains("prompt 24"))
        #expect(store.prompts(for: "b") == ["other"])
        store.clear(["a", "b"])
        #expect(store.prompts(for: "a").isEmpty && store.prompts(for: "b").isEmpty)
    }
}

@MainActor
struct AgentSuggestionPanelTests {
    private func demo() async throws -> AppModel {
        let dir = CoreClient.testScratch()
        let model = AppModel(core: try CoreClient(dataDirectory: dir),
                             defaults: UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")!)
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        return model
    }

    @Test func chipsFollowTheSelectionAndSearch() async throws {
        let model = try await demo()
        #expect(model.agentChips.first?.text == "What's new since yesterday?", "the demo Inbox has unread mail")
        model.selectedThreadID = model.threads.rows[0].id
        #expect(model.agentChips.first?.text == "Summarise this thread")
        model.selectedThreadIDs = Set(model.threads.rows.prefix(3).map(\.id))
        #expect(model.agentChips.first?.text == "Which of these need a reply?")
    }

    @Test func choosingFillsOrSendsAndIsRemembered() async throws {
        let model = try await demo()
        let focus = model.agentFocusRequests
        model.choose(AgentSuggestion(text: "Draft a reply that …"))
        #expect(model.agentPromptDraft == "Draft a reply that ")
        #expect(model.agentFocusRequests == focus + 1, "the field takes focus for the user's words")

        model.choose(AgentSuggestion(text: "What needs a reply today?"))
        #expect(model.agentPromptDraft.isEmpty)
        let deadline = ContinuousClock.now + .seconds(5)
        while model.agent.entries.isEmpty, ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.agent.entries.first?.kind == .prompt("What needs a reply today?"))
        #expect(model.agentChips.first?.text == "What needs a reply today?", "sent before: offered first")

        model.clearSuggestionHistory()
        #expect(model.agentChips.first?.text == "What's new since yesterday?")
    }
}
