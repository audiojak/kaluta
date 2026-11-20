import AppKit
import SwiftUI

/// The composer body: an `NSTextView` with rich text, lists and links, and
/// the standard Format menu (bold/italic/underline via ⌘B/⌘I/⌘U). The view
/// owns the text while editing; changes flow out through `text`.
struct RichTextEditor: NSViewRepresentable {
    @Binding var text: NSAttributedString
    var focusOnAppear = false
    /// Tab leaves the body for this, instead of typing a tab.
    var onTab: (() -> Void)?
    /// The formatting bar's commands act on this editor.
    var commands: RichTextCommands?
    /// A `/` was typed at the start of a line, at this location: offer
    /// the snippets.
    var onSlash: ((NSTextView, Int) -> Void)?
    /// The selected text, to save as a snippet (its context menu).
    var onSaveSnippet: ((NSAttributedString) -> Void)?

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        guard let textView = scroll.documentView as? NSTextView else { return scroll }
        textView.delegate = context.coordinator
        textView.isRichText = true
        textView.importsGraphics = false
        textView.allowsUndo = true
        textView.usesFindBar = true
        textView.usesFontPanel = false
        textView.isAutomaticLinkDetectionEnabled = true
        textView.isAutomaticQuoteSubstitutionEnabled = true
        textView.isAutomaticDashSubstitutionEnabled = true
        textView.isContinuousSpellCheckingEnabled = true
        textView.isGrammarCheckingEnabled = false
        textView.drawsBackground = false
        textView.textContainerInset = NSSize(width: 16, height: 12)
        textView.font = ComposerHTML.bodyFont
        textView.typingAttributes = [.font: ComposerHTML.bodyFont, .foregroundColor: NSColor.textColor]
        textView.setAccessibilityLabel("Message body")
        textView.textStorage?.setAttributedString(Self.display(text))
        context.coordinator.lastText = text
        context.coordinator.onTab = onTab
        context.coordinator.onSlash = onSlash
        context.coordinator.onSaveSnippet = onSaveSnippet
        context.coordinator.commands = commands
        commands?.textView = textView
        if focusOnAppear {
            DispatchQueue.main.async { textView.window?.makeFirstResponder(textView) }
        }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let textView = scroll.documentView as? NSTextView else { return }
        context.coordinator.onTab = onTab
        context.coordinator.onSlash = onSlash
        context.coordinator.onSaveSnippet = onSaveSnippet
        // Only replace the text when it changed from outside (a draft load).
        if text !== context.coordinator.lastText, !text.isEqual(to: textView.attributedString()) {
            textView.textStorage?.setAttributedString(Self.display(text))
            context.coordinator.lastText = text
        }
    }

    /// Loaded HTML has no color; show it in the current text color.
    private static func display(_ text: NSAttributedString) -> NSAttributedString {
        let copy = NSMutableAttributedString(attributedString: text)
        copy.addAttribute(.foregroundColor, value: NSColor.textColor, range: NSRange(location: 0, length: copy.length))
        return copy
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var text: Binding<NSAttributedString>
        var lastText: NSAttributedString?
        var onTab: (() -> Void)?
        var onSlash: ((NSTextView, Int) -> Void)?
        var onSaveSnippet: ((NSAttributedString) -> Void)?
        weak var commands: RichTextCommands?
        /// Where a `/` is being typed at the start of a line.
        private var slashAt: Int?

        func textViewDidChangeSelection(_ notification: Notification) {
            commands?.refresh()
        }

        init(text: Binding<NSAttributedString>) {
            self.text = text
        }

        func textView(_ textView: NSTextView, doCommandBy selector: Selector) -> Bool {
            guard selector == #selector(NSResponder.insertTab(_:)), let onTab else { return false }
            textView.window?.makeFirstResponder(nil)
            onTab()
            return true
        }

        func textView(_ textView: NSTextView, shouldChangeTextIn range: NSRange, replacementString: String?) -> Bool {
            slashAt = replacementString == "/" && range.length == 0
                && Self.startsLine(textView.string as NSString, at: range.location) ? range.location : nil
            return true
        }

        /// Whether `location` is at the start of a line.
        nonisolated static func startsLine(_ text: NSString, at location: Int) -> Bool {
            guard location > 0 else { return true }
            guard location <= text.length else { return false }
            let before = text.character(at: location - 1)
            return before == 0x0A || before == 0x0D || before == 0x2028 || before == 0x2029
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            let snapshot = NSAttributedString(attributedString: textView.attributedString())
            lastText = snapshot
            text.wrappedValue = snapshot
            if let at = slashAt, let onSlash {
                slashAt = nil
                DispatchQueue.main.async { onSlash(textView, at) }
            }
        }

        func textView(_ view: NSTextView, menu: NSMenu, for event: NSEvent, at charIndex: Int) -> NSMenu? {
            let range = view.selectedRange()
            guard range.length > 0, let onSaveSnippet else { return menu }
            let selection = view.attributedString().attributedSubstring(from: range)
            let item = ActionItem("Save as Snippet") { onSaveSnippet(selection) }
            menu.insertItem(item, at: 0)
            menu.insertItem(.separator(), at: 1)
            return menu
        }
    }
}
