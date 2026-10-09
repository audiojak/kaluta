import AppKit
import Foundation
import Testing
@testable import Kaluta

@MainActor
struct FormattingTests {
    private func editor(_ text: String) -> (NSTextView, RichTextCommands) {
        let view = NSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
        view.allowsUndo = true
        view.textStorage?.setAttributedString(NSAttributedString(string: text, attributes: [.font: ComposerHTML.bodyFont]))
        let commands = RichTextCommands()
        commands.textView = view
        return (view, commands)
    }

    @Test func boldAndUnderlineApplyToTheSelectionAndAreSent() {
        let (view, commands) = editor("Hello world")
        view.setSelectedRange(NSRange(location: 0, length: 5))
        commands.refresh()
        commands.toggleBold()
        commands.toggleUnderline()
        #expect(commands.state.bold && commands.state.underline)
        #expect(ComposerHTML.html(from: view.attributedString()) == "<p><b><u>Hello</u></b> world</p>")
        commands.toggleBold()
        #expect(!commands.state.bold)
    }

    @Test func listsAndQuotesAreToggledAndSent() {
        let (view, commands) = editor("One\nTwo\nAfter")
        view.setSelectedRange(NSRange(location: 0, length: 6))
        commands.refresh()
        commands.toggleBulleted()
        #expect(commands.state.bulleted)
        #expect(ComposerHTML.html(from: view.attributedString()) == "<ul><li>One</li><li>Two</li></ul><p>After</p>")
        view.setSelectedRange(NSRange(location: 1, length: 0))
        commands.refresh()
        commands.toggleBulleted()
        #expect(ComposerHTML.html(from: view.attributedString()).hasPrefix("<p>One</p>"), "back to a paragraph")

        let (quoted, q) = editor("Said before\nMy answer")
        quoted.setSelectedRange(NSRange(location: 0, length: 0))
        q.refresh()
        q.toggleQuote()
        #expect(q.state.quoted)
        #expect(ComposerHTML.html(from: quoted.attributedString()) == "<blockquote><p>Said before</p></blockquote><p>My answer</p>")
        let reloaded = ComposerHTML.attributedString(fromHTML: ComposerHTML.html(from: quoted.attributedString()))
        #expect(ComposerHTML.html(from: reloaded).hasPrefix("<blockquote>"), "a saved draft keeps its quote")
    }

    @Test func eachChangeIsOneUndo() {
        let (view, commands) = editor("Text")
        // The undo manager is the window's.
        let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: true)
        window.contentView = view
        view.setSelectedRange(NSRange(location: 0, length: 4))
        commands.toggleItalic()
        view.undoManager?.undo()
        #expect(ComposerHTML.html(from: view.attributedString()) == "<p>Text</p>")
    }

    @Test func linksAreWebOrMailOnly() {
        #expect(FormattingBar.linkURL("example.com")?.absoluteString == "https://example.com")
        #expect(FormattingBar.linkURL("ann@example.com")?.absoluteString == "mailto:ann@example.com")
        #expect(FormattingBar.linkURL("javascript:alert(1)") == nil)
        #expect(FormattingBar.linkURL("two words") == nil)
    }
}
