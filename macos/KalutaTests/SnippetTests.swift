import AppKit
import Foundation
import Testing
@testable import Kaluta

/// Snippets (spec §14.5 amendment 2026-11-18; plan: inbox ideas, F5).
@MainActor
struct SnippetTests {
    @Test func thePlaceholderTakesTheFirstRecipientsFirstName() {
        let sam = AddressInfo(name: "Sam Rivera", email: "sam@example.com")
        #expect(SnippetText.firstName(of: sam) == "Sam")
        #expect(SnippetText.firstName(of: AddressInfo(name: nil, email: "alex@example.com")) == "alex")
        #expect(SnippetText.firstName(of: nil) == "")
        #expect(SnippetText.filled("<div>Hi {first name},</div>", recipient: sam) == "<div>Hi Sam,</div>")
        let odd = AddressInfo(name: "<b>Eve</b>", email: "e@example.com")
        #expect(SnippetText.filled("{first name}", recipient: odd) == "&lt;b&gt;Eve&lt;/b&gt;", "escaped")
    }

    @Test func savedTextIsNamedByItsFirstLineAndPlainTextKeepsItsLines() {
        #expect(SnippetText.name(for: "Thanks for reaching out!\nBest,") == "Thanks for reaching out!")
        #expect(SnippetText.name(for: "   \n") == "Snippet")
        #expect(SnippetText.name(for: String(repeating: "a", count: 60)).count == 40)
        #expect(SnippetText.html(fromPlain: "Hi\n\nBye") == "<div>Hi</div><div><br></div><div>Bye</div>")
        #expect(SnippetText.html(fromPlain: "a < b") == "<div>a &lt; b</div>")
    }

    @Test func whatIsTypedAfterTheSlashFindsSnippetsByName() {
        let s = { (id: Int64, name: String) in SnippetInfo(id: id, name: name, bodyHtml: "", updatedAt: 0) }
        let all = [s(1, "Address"), s(2, "Thanks"), s(3, "Meeting address")]
        #expect(SnippetText.matching(all, "").map(\.id) == [1, 2, 3])
        #expect(SnippetText.matching(all, "addr").map(\.id) == [1, 3], "starts with, then a word that does")
        #expect(SnippetText.matching(all, "zzz").isEmpty)
    }

    @Test func aSlashOffersSnippetsOnlyAtTheStartOfALine() {
        let text = "Hi\nthere" as NSString
        #expect(RichTextEditor.Coordinator.startsLine(text, at: 0))
        #expect(RichTextEditor.Coordinator.startsLine(text, at: 3))
        #expect(!RichTextEditor.Coordinator.startsLine(text, at: 2))
        #expect(!RichTextEditor.Coordinator.startsLine(text, at: 99))
    }

    @Test func snippetsAreSavedForEveryAccountAndInsertedInPlaceOfTheSlash() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let core = try #require(model.core)
        let id = try await core.saveSnippet(SnippetInfo(id: 0, name: "Thanks", bodyHtml: "<div>Thanks, {first name}!</div>",
                                                        updatedAt: 0))
        #expect(try await core.snippets().map(\.name) == ["Thanks"])
        await #expect(throws: CoreClientError.self) {
            try await core.saveSnippet(SnippetInfo(id: 0, name: " ", bodyHtml: "", updatedAt: 0))
        }

        // In a text view: the / just typed becomes the snippet, and undo puts it back.
        let textView = NSTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        textView.allowsUndo = true
        textView.string = "Hello\n/"
        let snippet = try #require(try await core.snippets().first)
        SnippetPopover.insert(snippet, in: textView, replacing: 6,
                              recipient: AddressInfo(name: "Sam Rivera", email: "sam@example.com"))
        #expect(textView.string.hasPrefix("Hello\nThanks, Sam!"))
        #expect(!textView.string.contains("/"))

        // Saved from a selection: undo deletes it again.
        await model.saveSnippet(from: NSAttributedString(string: "See you Friday\nBest"))
        #expect(try await core.snippets().map(\.name).contains("See you Friday"))
        #expect(model.snippetCommands(try await core.snippets()).contains { $0.title == "New Message with “Thanks”" })
        model.undo.undo(in: model.openAccountID)
        await model.undo.settled()
        #expect(try await core.snippets().map(\.name) == ["Thanks"])
        let gone = try await core.deleteSnippet(id)
        #expect(gone?.name == "Thanks")
        #expect(try await core.snippets().isEmpty)
    }
}
