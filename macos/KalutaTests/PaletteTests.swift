import Foundation
import Testing
@testable import Kaluta

/// The command palette (⌘K; plan: inbox ideas, F4): one list of commands
/// for the palette and the thread list's single keys.
@MainActor
struct PaletteTests {
    private func command(_ id: String, _ title: String, unavailable: String? = nil) -> MailCommand {
        MailCommand(id: id, title: title, unavailable: unavailable) {}
    }

    @Test func whatIsTypedRanksCommandsAndEndsWithAskingKaluta() {
        let commands = [command("inbox", "Move to Inbox"), command("archive", "Archive"),
                        command("go", "Go to Archive"), command("star", "Star or Unstar")]
        // Nothing typed: everything, in its order, and nothing to ask.
        #expect(PaletteModel(commands: commands, query: " ").rows.map(\.id) == ["inbox", "archive", "go", "star"])
        // A name that starts with it, then a word that does; asking is last.
        let arch = PaletteModel(commands: commands, query: "arch")
        #expect(arch.rows.map(\.id) == ["archive", "go", "ask"])
        #expect(arch.rows.last?.title == "Ask Kaluta: arch")
        // Inside a word counts, after the others.
        #expect(PaletteModel(commands: commands, query: "nsta").rows.map(\.id) == ["star", "ask"])
        #expect(PaletteModel.rank("Move to Inbox", "inb") == 1)
        #expect(PaletteModel.rank("Move to Inbox", "zzz") == nil)
        // No command: only the question.
        let question = PaletteModel(commands: commands, query: "when is the offsite")
        #expect(question.rows.map(\.id) == ["ask"])
        #expect(question.firstRunnable == 0)
    }

    @Test func aCommandThatCannotRunIsListedButNotTaken() {
        let commands = [command("reply", "Reply", unavailable: "Open a conversation first"), command("read", "Read later")]
        let model = PaletteModel(commands: commands, query: "re")
        #expect(model.rows.map(\.id) == ["reply", "read", "ask"])
        #expect(model.rows[0].unavailable == "Open a conversation first")
        #expect(model.firstRunnable == 1, "Return takes the first that can run")
        #expect(model.moved(from: 2, by: 1) == 2 && model.moved(from: 0, by: -1) == 0)
    }

    @Test func theListsSingleKeysAndThePaletteShareOneDefinition() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let commands = model.mailCommands
        #expect(Set(commands.map(\.id)).count == commands.count, "ids are unique")
        let keys = commands.compactMap(\.listKey)
        #expect(Set(keys).count == keys.count, "no two commands share a list key")
        for key in ["e", "u", "s", "#", "!", "r", "a", "f", "c", "t", "T", "/"] {
            #expect(model.listCommand(for: key) != nil, "\(key) runs a command")
        }
        #expect(model.listCommand(for: "l") == nil && model.listCommand(for: "b") == nil, "pickers stay with the list")
        #expect(commands.contains { $0.title == "Go to Inbox" })
        // Workflows are listed only while they are on, and starting one
        // from the mail view shows it (the window was left on mail).
        #expect(!commands.contains { $0.title == "Start Triage" })
        model.turnOnWorkflows()
        model.showMail()
        #expect(!model.showsWorkflows)
        let start = try #require(model.mailCommands.first { $0.title == "Start Triage" })
        start.run()
        #expect(model.showsWorkflows, "the Dashboard's side, where a workflow shows")
        for _ in 0..<200 where model.workflowSession == nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(model.workflowSession?.kind == .triage)
        model.leaveWorkflow()
        model.showMail()

        // With a conversation selected, E archives it, as the key always did.
        let thread = try #require(model.threads.rows.first?.id)
        model.selectedThreadID = thread
        #expect(model.listCommand(for: "e")?.unavailable == nil)
        model.listCommand(for: "e")?.run()
        #expect(!model.threads.rows.contains { $0.id == thread })
        // Nothing selected: listed, with the reason.
        model.selectedThreadID = nil
        model.selectedThreadIDs = []
        #expect(model.mailCommands.first { $0.id == "archive" }?.unavailable == "Select a conversation first")
    }
}
