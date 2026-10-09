import AppKit

/// Converts between the composer's attributed text and the small HTML
/// subset we send (spec §14.5): paragraphs, line breaks, bold, italic,
/// underline, strikethrough, links, bulleted or numbered lists and quotes
/// (indented paragraphs, sent as a blockquote). Fonts,
/// colors and sizes are deliberately dropped so every message looks like
/// ordinary mail in the recipient's client. Pure, so it is unit-tested.
enum ComposerHTML {
    /// The editor's body font; loaded HTML is normalized to it.
    @MainActor static var bodyFont: NSFont { TypeRole.composerBody }

    // MARK: Attributed string → HTML

    static func html(from text: NSAttributedString) -> String {
        let string = text.string as NSString
        var out = ""
        var openList: (tag: String, list: NSTextList)?
        var inQuote = false
        var location = 0
        while location <= string.length {
            let range = string.paragraphRange(for: NSRange(location: location, length: 0))
            var content = range
            // Drop the paragraph terminator from the content.
            if content.length > 0, let last = string.substring(with: content).last, last.isNewline {
                content.length -= 1
            }
            let style = range.length > 0 && range.location < text.length
                ? text.attribute(.paragraphStyle, at: range.location, effectiveRange: nil) as? NSParagraphStyle
                : nil
            let list = style?.textLists.last
            let quoted = list == nil && isQuote(style)
            if let current = openList, list !== current.list {
                out += "</\(current.tag)>"
                openList = nil
            }
            if inQuote != quoted {
                out += quoted ? "<blockquote>" : "</blockquote>"
                inQuote = quoted
            }
            if let list, openList == nil {
                let tag = isOrdered(list) ? "ol" : "ul"
                out += "<\(tag)>"
                openList = (tag, list)
            }
            let inline = inlineHTML(text, range: content, stripListMarker: list != nil)
            if list != nil {
                out += "<li>\(inline.isEmpty ? "<br>" : inline)</li>"
            } else {
                out += "<p>\(inline.isEmpty ? "<br>" : inline)</p>"
            }
            if NSMaxRange(range) >= string.length {
                // The last paragraph; a trailing newline adds nothing.
                break
            }
            location = NSMaxRange(range)
        }
        if let current = openList { out += "</\(current.tag)>" }
        if inQuote { out += "</blockquote>" }
        // An empty document serializes to nothing, not an empty paragraph.
        return out == "<p><br></p>" ? "" : out
    }

    /// A quoted paragraph: indented, not a list item.
    static func isQuote(_ style: NSParagraphStyle?) -> Bool {
        (style?.headIndent ?? 0) >= 20 && (style?.firstLineHeadIndent ?? 0) >= 20
    }

    private static func isOrdered(_ list: NSTextList) -> Bool {
        switch list.markerFormat {
        case .decimal, .lowercaseAlpha, .uppercaseAlpha, .lowercaseRoman, .uppercaseRoman,
             .lowercaseLatin, .uppercaseLatin, .octal, .lowercaseHexadecimal, .uppercaseHexadecimal:
            true
        default:
            false
        }
    }

    private static func inlineHTML(_ text: NSAttributedString, range: NSRange, stripListMarker: Bool) -> String {
        var out = ""
        var skipTab = stripListMarker
        text.enumerateAttributes(in: range) { attrs, runRange, _ in
            var run = (text.string as NSString).substring(with: runRange)
            // NSTextView renders list markers as "\t•\t" prefixes in the text.
            if skipTab {
                if let marker = run.range(of: #"^\t[^\t]*\t"#, options: .regularExpression) {
                    run.removeSubrange(marker)
                }
                skipTab = false
            }
            guard !run.isEmpty else { return }
            var html = escape(run).replacingOccurrences(of: "\u{2028}", with: "<br>")
            let traits = (attrs[.font] as? NSFont)?.fontDescriptor.symbolicTraits ?? []
            if (attrs[.strikethroughStyle] as? Int ?? 0) != 0 { html = "<s>\(html)</s>" }
            if (attrs[.underlineStyle] as? Int ?? 0) != 0, attrs[.link] == nil { html = "<u>\(html)</u>" }
            if traits.contains(.italic) { html = "<i>\(html)</i>" }
            if traits.contains(.bold) { html = "<b>\(html)</b>" }
            if let href = linkTarget(attrs[.link]) {
                html = "<a href=\"\(escape(href))\">\(html)</a>"
            }
            out += html
        }
        return out
    }

    /// Only web and mail links survive; anything else becomes plain text.
    private static func linkTarget(_ value: Any?) -> String? {
        let url: URL? = switch value {
        case let url as URL: url
        case let string as String: URL(string: string)
        default: nil
        }
        guard let url, let scheme = url.scheme?.lowercased(), ["http", "https", "mailto"].contains(scheme) else {
            return nil
        }
        return url.absoluteString
    }

    static func escape(_ s: String) -> String {
        var out = ""
        out.reserveCapacity(s.count)
        for c in s {
            switch c {
            case "&": out += "&amp;"
            case "<": out += "&lt;"
            case ">": out += "&gt;"
            case "\"": out += "&quot;"
            default: out.append(c)
            }
        }
        return out
    }

    // MARK: HTML → attributed string

    /// Loads a saved draft's body into the editor with the editor's font,
    /// keeping bold and italic. The HTML came from `html(from:)` (or from
    /// the Rust sanitizer for quoted text), never from a remote page.
    @MainActor
    static func attributedString(fromHTML html: String) -> NSAttributedString {
        guard !html.isEmpty else { return NSAttributedString(string: "", attributes: [.font: bodyFont]) }
        // AppKit's HTML import drops blockquotes (and margins), so the
        // top-level quotes `html(from:)` writes are imported on their own
        // and indented as the editor quotes.
        let out = NSMutableAttributedString()
        for (part, quoted) in quoteParts(html) {
            guard let parsed = importHTML(part) else { continue }
            if quoted {
                let quote = NSMutableParagraphStyle()
                quote.headIndent = RichTextCommands.quoteIndent
                quote.firstLineHeadIndent = RichTextCommands.quoteIndent
                parsed.addAttribute(.paragraphStyle, value: quote, range: NSRange(location: 0, length: parsed.length))
            }
            out.append(parsed)
        }
        // HTML import ends with a newline for the last block; drop it.
        while out.string.hasSuffix("\n") {
            out.deleteCharacters(in: NSRange(location: out.length - 1, length: 1))
        }
        return out.length == 0 ? NSAttributedString(string: "", attributes: [.font: bodyFont]) : out
    }

    /// The HTML split at its top-level blockquotes: each part, and whether
    /// it was quoted. Pure.
    static func quoteParts(_ html: String) -> [(String, Bool)] {
        var parts: [(String, Bool)] = []
        var rest = Substring(html)
        while let open = rest.range(of: "<blockquote>"), let close = rest.range(of: "</blockquote>", range: open.upperBound..<rest.endIndex) {
            let before = rest[..<open.lowerBound]
            if !before.isEmpty { parts.append((String(before), false)) }
            parts.append((String(rest[open.upperBound..<close.lowerBound]), true))
            rest = rest[close.upperBound...]
        }
        if !rest.isEmpty { parts.append((String(rest), false)) }
        return parts
    }

    /// One piece of HTML in the editor's font, without colors.
    @MainActor
    private static func importHTML(_ html: String) -> NSMutableAttributedString? {
        guard let parsed = try? NSMutableAttributedString(
            data: Data(html.utf8),
            options: [.documentType: NSAttributedString.DocumentType.html,
                      .characterEncoding: String.Encoding.utf8.rawValue],
            documentAttributes: nil)
        else { return nil }
        let full = NSRange(location: 0, length: parsed.length)
        parsed.enumerateAttribute(.font, in: full) { value, range, _ in
            let traits = (value as? NSFont)?.fontDescriptor.symbolicTraits ?? []
            parsed.addAttribute(.font, value: font(bold: traits.contains(.bold), italic: traits.contains(.italic)),
                                range: range)
        }
        parsed.removeAttribute(.foregroundColor, range: full)
        parsed.removeAttribute(.backgroundColor, range: full)
        // Each part ends its last block with a newline; the next part
        // starts a new paragraph.
        if !parsed.string.hasSuffix("\n") {
            parsed.append(NSAttributedString(string: "\n", attributes: [.font: bodyFont]))
        }
        return parsed
    }

    @MainActor
    static func font(bold: Bool, italic: Bool) -> NSFont {
        var traits: NSFontDescriptor.SymbolicTraits = []
        if bold { traits.insert(.bold) }
        if italic { traits.insert(.italic) }
        let descriptor = bodyFont.fontDescriptor.withSymbolicTraits(traits)
        return NSFont(descriptor: descriptor, size: bodyFont.pointSize) ?? bodyFont
    }
}
