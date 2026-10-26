import AppKit
import Foundation
import Testing
@testable import Kaluta

@MainActor
struct ComposerAssistantTests {
    private func demo() async throws -> AppModel {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()))
        await model.start(openDemo: true)
        await model.agent.loadProviders()
        return model
    }

    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition() {
            guard ContinuousClock.now < deadline else { throw Timeout() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
    private struct Timeout: Error {}

    @Test func thePromptCarriesTheRequestTheDraftAndTheOriginal() {
        let prompt = ComposerAssistant.prompt(instruction: "Say yes", from: "me@x.com", to: ["ann@x.com"],
                                              subject: "Re: Lunch", draft: "  Hi Ann,\n", original: "Lunch Thursday?")
        #expect(prompt.contains("The user asks: Say yes"))
        #expect(prompt.contains("To: ann@x.com"))
        #expect(prompt.contains("<<<\nHi Ann,\n>>>"))
        #expect(prompt.contains("being answered or forwarded:\n<<<\nLunch Thursday?\n>>>"))
        #expect(prompt.contains("Do not create, change, send or delete any mail"))
        let fresh = ComposerAssistant.prompt(instruction: "Write it", from: "me@x.com", to: [], subject: "",
                                             draft: "", original: "")
        #expect(fresh.contains("(nobody yet)") && !fresh.contains("being answered"))
    }

    /// The agent's questions before it writes (spec §14.9): each with its
    /// fact label and category, and answers to pick from where the thread
    /// offers them; a yes-or-no question offers Yes and No by itself.
    @Test func questionsCarryTheirOptions() {
        let text = """
        {"questions": [
          {"question": "Which niches do you want?", "category": "work", "label": "Creator niches",
           "options": ["Tech", " Lifestyle ", "tech", "Food", "Wellness"]},
          {"question": "Will you attach the logo?", "category": "preferences"},
          {"question": "What is the budget?", "label": "Budget", "category": "Work", "options": [""]}
        ]}
        """
        let questions = try! #require(ComposerAssistant.questions(in: text))
        #expect(questions.map(\.choices) == [["Tech", "Lifestyle", "Food"], ["Yes", "No"], []],
                "three at most, each once; yes and no for a question that takes them")
        #expect(questions.map(\.fact) == ["Creator niches", "Will you attach the logo?", "Budget"])
        #expect(questions.map(\.category) == ["work", "preferences", "work"])
        #expect(ComposerAssistant.questions(in: "Happy to help.") == nil)
        #expect(ComposerAssistant.prompt(instruction: "x", from: "me", to: [], subject: "s", draft: "", original: "")
            .contains("\"options\": [\"Tech\", \"Lifestyle\"]"))
    }

    @Test func answersLoseTheirFenceAndSpace() {
        #expect(ComposerAssistant.cleaned("\n  Sounds good.\n") == "Sounds good.")
        #expect(ComposerAssistant.cleaned("```text\nSounds good.\n```") == "Sounds good.")
        #expect(ComposerAssistant.suggestions(replying: true).first == "Write a reply")
        #expect(ComposerAssistant.suggestions(replying: false).first == "Write this message")
    }

    @Test func theAgentWritesTheBodyAndUndoPutsItBack() async throws {
        let model = try await demo()
        let row = try #require(model.threads.rows.first)
        let detail = try #require(try await model.core!.thread(row.id))
        let message = try #require(detail.messages.last)
        let store = ComposerStore(core: model.core, attachmentsDirectory: CoreClient.testScratch())
        await store.load(.reply(messageID: message.id, all: false))
        store.body = NSAttributedString(string: "Draft so far", attributes: [.font: ComposerHTML.bodyFont])

        let assistant = ComposerAssistant()
        assistant.instruction = "Write a reply"
        await assistant.run(store: store, model: model,
                            original: ComposerAssistant.plainText(fromHTML: store.quotedHTML))
        try await waitUntil { assistant.state != .working }
        #expect(assistant.state == .done)
        // The fake agent echoes the prompt: the request, the draft and the original all reached it.
        #expect(store.body.string.hasPrefix("You said: You are helping write an email"))
        #expect(store.body.string.contains("The user asks: Write a reply"))
        #expect(store.body.string.contains("Draft so far"))
        #expect(assistant.instruction.isEmpty)
        #expect(model.agentSinks.isEmpty, "the session is closed and forgotten")
        #expect(model.agent.entries.isEmpty, "writing help stays out of the agent panel")

        assistant.undo()
        #expect(store.body.string == "Draft so far")
        #expect(assistant.state == .idle)
    }

    @Test func theAgentColumnsExamplesGoIntoThePrompt() async throws {
        let model = try await demo()
        let focus = model.agentFocusRequests
        model.fillPrompt(AgentSuggestion(text: "What needs a reply today?"))
        #expect(model.agentPromptDraft == "What needs a reply today?")
        #expect(model.agentFocusRequests == focus + 1)
        #expect(model.agent.entries.isEmpty, "nothing is sent until Return")
        model.fillPrompt(AgentSuggestion(text: "Draft a reply that …"))
        #expect(model.agentPromptDraft == "Draft a reply that ")
    }
}
