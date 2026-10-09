import AppKit
import Observation

/// The formatting bar's commands on the composer's text view (spec §14.5):
/// bold, italic, underline, strikethrough, bulleted and numbered lists,
/// quote, link and clear formatting — what `ComposerHTML` sends. Each is
/// one undoable change. `state` follows the selection so the bar shows
/// what is on.
@MainActor
@Observable
final class RichTextCommands {
    struct State: Equatable {
        var bold = false, italic = false, underline = false, strikethrough = false
        var bulleted = false, numbered = false, quoted = false
    }

    private(set) var state = State()
    @ObservationIgnored weak var textView: NSTextView?

    /// Indent of a quoted paragraph; `ComposerHTML` sends such paragraphs
    /// as a blockquote.
    static let quoteIndent: CGFloat = 24
    /// Indent of a list item's text, past its marker.
    static let listIndent: CGFloat = 36

    // MARK: Inline

    func toggleBold() { toggleTrait(.boldFontMask, on: !state.bold) }
    func toggleItalic() { toggleTrait(.italicFontMask, on: !state.italic) }

    func toggleUnderline() { toggleStyle(.underlineStyle, on: !state.underline) }
    func toggleStrikethrough() { toggleStyle(.strikethroughStyle, on: !state.strikethrough) }

    private func toggleTrait(_ trait: NSFontTraitMask, on: Bool) {
        editInline { storage, range in
            storage.enumerateAttribute(.font, in: range) { value, run, _ in
                let font = value as? NSFont ?? ComposerHTML.bodyFont
                let manager = NSFontManager.shared
                let changed = on ? manager.convert(font, toHaveTrait: trait) : manager.convert(font, toNotHaveTrait: trait)
                storage.addAttribute(.font, value: changed, range: run)
            }
        } typing: { attrs in
            let font = attrs[.font] as? NSFont ?? ComposerHTML.bodyFont
            let manager = NSFontManager.shared
            attrs[.font] = on ? manager.convert(font, toHaveTrait: trait) : manager.convert(font, toNotHaveTrait: trait)
        }
    }

    private func toggleStyle(_ key: NSAttributedString.Key, on: Bool) {
        editInline { storage, range in
            if on {
                storage.addAttribute(key, value: NSUnderlineStyle.single.rawValue, range: range)
            } else {
                storage.removeAttribute(key, range: range)
            }
        } typing: { attrs in
            attrs[key] = on ? NSUnderlineStyle.single.rawValue : nil
        }
    }

    /// Back to plain body text: no bold, italic, underline, strikethrough or
    /// link (lists and quotes stay; their buttons undo them).
    func clearFormatting() {
        editInline { storage, range in
            storage.addAttribute(.font, value: ComposerHTML.bodyFont, range: range)
            for key in [NSAttributedString.Key.underlineStyle, .strikethroughStyle, .link] {
                storage.removeAttribute(key, range: range)
            }
        } typing: { attrs in
            attrs[.font] = ComposerHTML.bodyFont
            attrs[.underlineStyle] = nil
            attrs[.strikethroughStyle] = nil
            attrs[.link] = nil
        }
    }

    /// Make the selection a link to `url` (or insert the address, with
    /// nothing selected). Only web and mail links are sent.
    func link(to url: URL) {
        guard let textView, let storage = textView.textStorage else { return }
        let range = textView.selectedRange()
        if range.length == 0 {
            var attrs = textView.typingAttributes
            attrs[.link] = url
            let text = NSAttributedString(string: url.absoluteString, attributes: attrs)
            guard textView.shouldChangeText(in: range, replacementString: text.string) else { return }
            storage.replaceCharacters(in: range, with: text)
            textView.didChangeText()
            textView.setSelectedRange(NSRange(location: range.location + text.length, length: 0))
        } else {
            guard textView.shouldChangeText(in: range, replacementString: nil) else { return }
            storage.addAttribute(.link, value: url, range: range)
            textView.didChangeText()
        }
        refresh()
    }

    /// The selected text, when it looks like an address to link to.
    var selectedURL: String? {
        guard let textView else { return nil }
        let text = (textView.string as NSString).substring(with: textView.selectedRange())
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return text.contains(".") && !text.contains(" ") ? text : nil
    }

    private func editInline(_ change: (NSTextStorage, NSRange) -> Void, typing: (inout [NSAttributedString.Key: Any]) -> Void) {
        guard let textView, let storage = textView.textStorage else { return }
        let ranges = textView.selectedRanges.map(\.rangeValue).filter { $0.length > 0 }
        if ranges.isEmpty {
            // Nothing selected: what is typed next.
            var attrs = textView.typingAttributes
            typing(&attrs)
            textView.typingAttributes = attrs
        } else {
            guard textView.shouldChangeText(inRanges: ranges.map { NSValue(range: $0) }, replacementStrings: nil)
            else { return }
            storage.beginEditing()
            for range in ranges { change(storage, range) }
            storage.endEditing()
            textView.didChangeText()
        }
        refresh()
    }

    // MARK: Paragraphs

    func toggleBulleted() { toggleList(ordered: false) }
    func toggleNumbered() { toggleList(ordered: true) }

    /// Turn the selected paragraphs into list items, or back into plain
    /// paragraphs when they already are that kind of list. Markers are
    /// typed into the text ("\t•\t"), as NSTextView continues lists.
    private func toggleList(ordered: Bool) {
        let removing = ordered ? state.numbered : state.bulleted
        let list = NSTextList(markerFormat: ordered ? .decimal : .disc, options: 0)
        rewriteParagraphs { index, text, style in
            let plain = Self.withoutMarker(text)
            if removing {
                style.textLists = []
                style.headIndent = 0
                style.firstLineHeadIndent = 0
                style.tabStops = NSParagraphStyle.default.tabStops
                return plain
            }
            style.textLists = [list]
            style.headIndent = Self.listIndent
            style.firstLineHeadIndent = 0
            style.tabStops = [NSTextTab(textAlignment: .natural, location: 11),
                              NSTextTab(textAlignment: .natural, location: Self.listIndent)]
            return "\t\(list.marker(forItemNumber: index + 1))\t" + plain
        }
    }

    /// Quote the selected paragraphs (an indented block, sent as a
    /// blockquote), or unquote them.
    func toggleQuote() {
        let removing = state.quoted
        rewriteParagraphs { _, text, style in
            let indent: CGFloat = removing ? 0 : Self.quoteIndent
            style.textLists = []
            style.headIndent = indent
            style.firstLineHeadIndent = indent
            return Self.withoutMarker(text)
        }
    }

    /// Rewrite each selected paragraph (its text without the terminator,
    /// and its paragraph style) as one undoable change.
    private func rewriteParagraphs(_ rewrite: (Int, String, NSMutableParagraphStyle) -> String) {
        guard let textView, let storage = textView.textStorage else { return }
        let whole = (storage.string as NSString).paragraphRange(for: textView.selectedRange())
        let result = NSMutableAttributedString()
        var index = 0
        var location = whole.location
        repeat {
            let range = (storage.string as NSString).paragraphRange(for: NSRange(location: location, length: 0))
            var text = (storage.string as NSString).substring(with: range)
            let terminator = text.last?.isNewline == true ? String(text.removeLast()) : ""
            let attrs = range.location < storage.length
                ? storage.attributes(at: range.location, effectiveRange: nil)
                : textView.typingAttributes
            let style = ((attrs[.paragraphStyle] as? NSParagraphStyle) ?? .default).mutableCopy() as? NSMutableParagraphStyle
                ?? NSMutableParagraphStyle()
            let newText = rewrite(index, text, style)
            // Keep the runs' own formatting where the text is unchanged.
            let old = storage.attributedSubstring(from: NSRange(location: range.location, length: (text as NSString).length))
            let paragraph = NSMutableAttributedString()
            if newText.hasSuffix(text) {
                let prefix = String(newText.dropLast(text.count))
                paragraph.append(NSAttributedString(string: prefix, attributes: attrs))
                paragraph.append(old)
            } else if text.hasSuffix(newText) {
                paragraph.append(old.attributedSubstring(from: NSRange(location: (text as NSString).length - (newText as NSString).length,
                                                                       length: (newText as NSString).length)))
            } else {
                paragraph.append(NSAttributedString(string: newText, attributes: attrs))
            }
            paragraph.append(NSAttributedString(string: terminator, attributes: attrs))
            paragraph.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: paragraph.length))
            result.append(paragraph)
            index += 1
            location = NSMaxRange(range)
        } while location < NSMaxRange(whole)
        guard textView.shouldChangeText(in: whole, replacementString: result.string) else { return }
        storage.replaceCharacters(in: whole, with: result)
        textView.didChangeText()
        // The cursor at the end of the changed paragraphs, before any newline.
        let end = whole.location + result.length - (result.string.hasSuffix("\n") ? 1 : 0)
        textView.setSelectedRange(NSRange(location: max(whole.location, end), length: 0))
        if let style = result.length > 0 ? result.attribute(.paragraphStyle, at: 0, effectiveRange: nil) : nil {
            textView.typingAttributes[.paragraphStyle] = style
        }
        refresh()
    }

    /// A list item's text without its "\t•\t" or "\t1.\t" marker.
    nonisolated static func withoutMarker(_ text: String) -> String {
        guard let marker = text.range(of: #"^\t[^\t]*\t"#, options: .regularExpression) else { return text }
        var copy = text
        copy.removeSubrange(marker)
        return copy
    }

    // MARK: State

    /// What is on at the selection (or for what is typed next).
    func refresh() {
        guard let textView, let storage = textView.textStorage else { state = State(); return }
        let range = textView.selectedRange()
        let attrs = range.length > 0 && range.location < storage.length
            ? storage.attributes(at: range.location, effectiveRange: nil)
            : textView.typingAttributes
        let traits = (attrs[.font] as? NSFont)?.fontDescriptor.symbolicTraits ?? []
        let style = attrs[.paragraphStyle] as? NSParagraphStyle
        let list = style?.textLists.last
        var next = State()
        next.bold = traits.contains(.bold)
        next.italic = traits.contains(.italic)
        next.underline = (attrs[.underlineStyle] as? Int ?? 0) != 0
        next.strikethrough = (attrs[.strikethroughStyle] as? Int ?? 0) != 0
        next.numbered = list.map { $0.markerFormat == .decimal } ?? false
        next.bulleted = list.map { $0.markerFormat != .decimal } ?? false
        next.quoted = list == nil && (style?.headIndent ?? 0) >= Self.quoteIndent - 1
        if next != state { state = next }
    }
}
