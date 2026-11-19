import AppKit
import SwiftUI

/// The snippets to insert, by name: `/` at the start of a line in the
/// composer. ↑↓ select, Return inserts, Esc leaves the `/` as typed.
struct SnippetPickerView: View {
    let snippets: [SnippetInfo]
    let choose: (SnippetInfo) -> Void

    @State private var text = ""
    @State private var selected = 0
    @FocusState private var focused: Bool

    var body: some View {
        let rows = SnippetText.matching(snippets, text)
        VStack(alignment: .leading, spacing: Space.s) {
            TextField("Snippet name", text: $text)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit { if rows.indices.contains(selected) { choose(rows[selected]) } }
                .onKeyPress(.downArrow) { selected = min(selected + 1, max(rows.count - 1, 0)); return .handled }
                .onKeyPress(.upArrow) { selected = max(selected - 1, 0); return .handled }
                .onChange(of: text) { selected = 0 }
                .accessibilityLabel("Snippet")
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(rows.enumerated()), id: \.element.id) { index, snippet in
                    Button { choose(snippet) } label: {
                        Text(snippet.name)
                            .lineLimit(1)
                            .padding(.horizontal, Space.s)
                            .padding(.vertical, Space.xs)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(index == selected ? Tone.highlight : AnyShapeStyle(.clear),
                                        in: RoundedRectangle(cornerRadius: Radius.chip))
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .hoverHelp("Insert “\(snippet.name)”")
                }
                if rows.isEmpty {
                    Text(snippets.isEmpty ? "No snippets yet. Add them in Settings › Snippets, or select text and choose Save as Snippet."
                         : "No snippet by that name")
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.vertical, Space.xs)
                }
            }
            KeyHints(hints: [KeyHint(keys: "↑↓", does: "Select"), KeyHint(keys: "Return", does: "Insert"),
                             KeyHint(keys: "Esc", does: "Type /")])
        }
        .padding(Space.m)
        .frame(width: 300)
        .onAppear { focused = true }
    }
}

/// Shows the picker at a `/` just typed, and replaces it with the snippet.
@MainActor
enum SnippetPopover {
    static func show(in textView: NSTextView, slashAt location: Int, snippets: [SnippetInfo],
                     recipient: AddressInfo?) {
        guard let layout = textView.layoutManager, let container = textView.textContainer,
              location < (textView.string as NSString).length else { return }
        let glyphs = layout.glyphRange(forCharacterRange: NSRange(location: location, length: 1), actualCharacterRange: nil)
        var rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
        rect.origin.x += textView.textContainerOrigin.x
        rect.origin.y += textView.textContainerOrigin.y
        let popover = NSPopover()
        popover.behavior = .transient
        let picker = SnippetPickerView(snippets: snippets) { [weak textView] snippet in
            popover.performClose(nil)
            guard let textView else { return }
            insert(snippet, in: textView, replacing: location, recipient: recipient)
        }
        popover.contentViewController = NSHostingController(rootView: picker)
        popover.show(relativeTo: rect, of: textView, preferredEdge: .maxY)
    }

    /// The snippet in place of the `/` at `location`, as one undoable change.
    static func insert(_ snippet: SnippetInfo, in textView: NSTextView, replacing location: Int, recipient: AddressInfo?) {
        let text = NSMutableAttributedString(
            attributedString: ComposerHTML.attributedString(fromHTML: SnippetText.filled(snippet.bodyHtml, recipient: recipient)))
        text.addAttribute(.foregroundColor, value: NSColor.textColor, range: NSRange(location: 0, length: text.length))
        let storage = textView.string as NSString
        let range = location < storage.length && storage.character(at: location) == UInt16(UnicodeScalar("/").value)
            ? NSRange(location: location, length: 1) : NSRange(location: textView.selectedRange().location, length: 0)
        textView.window?.makeFirstResponder(textView)
        textView.insertText(text, replacementRange: range)
    }
}
