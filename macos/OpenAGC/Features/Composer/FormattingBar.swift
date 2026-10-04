import AppKit
import SwiftUI

/// Under the message body, as in Gmail (spec §14.5): the formatting OpenAGC
/// sends, each button lit while it is on at the cursor. It also marks the
/// body as the place to type.
struct FormattingBar: View {
    let commands: RichTextCommands

    var body: some View {
        let state = commands.state
        HStack(spacing: Space.xs) {
            button("Bold", "bold", on: state.bold, help: "Bold (⌘B)", action: commands.toggleBold)
            button("Italic", "italic", on: state.italic, help: "Italic (⌘I)", action: commands.toggleItalic)
            button("Underline", "underline", on: state.underline, help: "Underline (⌘U)", action: commands.toggleUnderline)
            button("Strikethrough", "strikethrough", on: state.strikethrough, help: "Strikethrough",
                   action: commands.toggleStrikethrough)
            separator
            button("Bulleted List", "list.bullet", on: state.bulleted, help: "Bulleted list",
                   action: commands.toggleBulleted)
            button("Numbered List", "list.number", on: state.numbered, help: "Numbered list",
                   action: commands.toggleNumbered)
            button("Quote", "text.quote", on: state.quoted, help: "Quote: an indented block, sent as a quotation",
                   action: commands.toggleQuote)
            separator
            button("Link", "link", on: false, help: "Make the selection a link", action: askForLink)
            button("Clear Formatting", "eraser", on: false,
                   help: "Clear bold, italic, underline, strikethrough and links from the selection",
                   action: commands.clearFormatting)
        }
        .padding(.horizontal, Space.s)
        .padding(.vertical, Space.xs)
        .background(.quaternary.opacity(0.5), in: .capsule)
    }

    private func button(_ title: String, _ symbol: String, on: Bool, help: String,
                        action: @escaping () -> Void) -> some View {
        Button(title, systemImage: symbol, action: action)
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .frame(width: 26, height: 22)
            .background(on ? AnyShapeStyle(.tint.opacity(0.18)) : AnyShapeStyle(.clear), in: .rect(cornerRadius: Radius.control))
            .foregroundStyle(on ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
            .hoverHelp(help)
            .accessibilityAddTraits(on ? .isSelected : [])
    }

    private var separator: some View {
        Rectangle().fill(.separator).frame(width: 1, height: 14).padding(.horizontal, Space.xs)
    }

    /// Ask for the address in a small alert on the composer's window.
    private func askForLink() {
        guard let window = commands.textView?.window else { return }
        let alert = NSAlert()
        alert.messageText = "Link To"
        alert.informativeText = "A web address, or a mailto: address."
        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 300, height: 24))
        field.placeholderString = "https://"
        field.stringValue = commands.selectedURL ?? ""
        alert.accessoryView = field
        alert.addButton(withTitle: "Add Link")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn,
                  let url = Self.linkURL(field.stringValue) else { return }
            MainActor.assumeIsolated { commands.link(to: url) }
        }
    }

    /// What the user typed as a link: web or mail only, with a scheme added
    /// to a bare address. Pure.
    nonisolated static func linkURL(_ text: String) -> URL? {
        var text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !text.contains(" ") else { return nil }
        if !text.contains(":") { text = (text.contains("@") ? "mailto:" : "https://") + text }
        guard let url = URL(string: text), let scheme = url.scheme?.lowercased(),
              ["http", "https", "mailto"].contains(scheme) else { return nil }
        return url
    }
}
