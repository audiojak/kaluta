import AppKit
import Foundation
import Testing
@testable import Kaluta

/// Rewrite in place (⌘J; spec §14.5 amendment 2026-11-18).
@MainActor
struct RewriteTests {
    @Test func withNothingSelectedItActsOnTheParagraph() {
        let text = "Hi Sam,\nThanks for the notes.\nBest" as NSString
        // A selection is used as it is.
        #expect(SelectionRewriter.target(in: text, selection: NSRange(location: 3, length: 3)) == NSRange(location: 3, length: 3))
        // The cursor in the second line: that line, without its line break.
        let line = SelectionRewriter.target(in: text, selection: NSRange(location: 12, length: 0))
        #expect(text.substring(with: line) == "Thanks for the notes.")
        // At the very end: the last line.
        #expect(text.substring(with: SelectionRewriter.target(in: text, selection: NSRange(location: text.length, length: 0)))
            == "Best")
    }

    @Test func theRequestSendsOnlyThePassageWithTheMessageAsContext() {
        let prompt = SelectionRewriter.prompt(request: "Make it shorter", selection: "We would be delighted to attend.",
                                              message: "Hi Sam,\nWe would be delighted to attend.\nBest", guide: "")
        #expect(prompt.contains("The user asks: Make it shorter"))
        #expect(prompt.contains("The passage to rewrite:\n<<<\nWe would be delighted to attend.\n>>>"))
        #expect(prompt.contains("for context only"))
        #expect(prompt.contains("Do not create, change, send or delete any mail"))
        #expect(SelectionRewriter.requests.count == 4)
    }

    /// The agent's answer replaces only the passage, and the text view's
    /// own undo takes it back; a passage changed meanwhile is left alone.
    @Test func theRewriteReplacesOnlyThePassageAndUndoes() async throws {
        let model = AppModel(core: try CoreClient(dataDirectory: CoreClient.testScratch()),
                             defaults: try #require(UserDefaults(suiteName: "kaluta-tests-\(UUID().uuidString)")))
        await model.start(openDemo: true)
        let store = ComposerStore(core: try #require(model.core), account: model.openAccountID)
        await store.load(.prefilled(to: ["sam@example.com"], cc: [], subject: "Notes", body: "Hi Sam,\nLong passage.\nBest"))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 300, height: 200), styleMask: [.titled],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let textView = NSTextView(frame: window.contentLayoutRect)
        textView.allowsUndo = true
        window.contentView = textView
        textView.string = "Hi Sam,\nLong passage.\nBest"
        let range = NSRange(location: 8, length: 13)
        let rewriter = SelectionRewriter()
        await rewriter.rewrite(range, in: textView, request: "Make it shorter", store: store, model: model)
        #expect(rewriter.state == .working)
        await rewriter.ingest([.textDelta(text: "Short."), .turnCompleted(inputTokens: nil, outputTokens: nil, costUsd: nil)])
        #expect(textView.string == "Hi Sam,\nShort.\nBest")
        #expect(rewriter.state == .idle)
        textView.undoManager?.undo()
        #expect(textView.string == "Hi Sam,\nLong passage.\nBest")

        // Typed over while Kaluta wrote: nothing is replaced.
        await rewriter.rewrite(range, in: textView, request: "Make it shorter", store: store, model: model)
        textView.string = "Hi Sam,\nSomething else.\nBest"
        await rewriter.ingest([.textDelta(text: "Short."), .turnCompleted(inputTokens: nil, outputTokens: nil, costUsd: nil)])
        #expect(textView.string == "Hi Sam,\nSomething else.\nBest")
        if case .failed = rewriter.state {} else { Issue.record("it says why it left the text") }
    }
}
