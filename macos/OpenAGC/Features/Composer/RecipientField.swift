import AppKit
import SwiftUI

/// A To/Cc/Bcc field: one token per address, completing from the contacts
/// the core has learned from mail (ranked by how often you write to them).
struct RecipientField: NSViewRepresentable {
    @Binding var addresses: [AddressInfo]
    let suggest: (String) -> [AddressInfo]
    var accessibilityLabel = "To"
    /// Take the cursor when the composer opens (a new message, a forward).
    var focusOnAppear = false

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSTokenField {
        let field = NSTokenField()
        field.delegate = context.coordinator
        field.tokenStyle = .rounded
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: NSFont.systemFontSize)
        field.completionDelay = 0
        field.tokenizingCharacterSet = CharacterSet(charactersIn: ",;")
        field.cell?.wraps = true
        field.cell?.isScrollable = false
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        field.setAccessibilityLabel(accessibilityLabel)
        field.objectValue = addresses.map(Token.init)
        if focusOnAppear {
            DispatchQueue.main.async { field.window?.makeFirstResponder(field) }
        }
        return field
    }

    /// As wide as offered and as tall as its tokens wrap to.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView field: NSTokenField, context: Context) -> CGSize? {
        let width = proposal.width ?? 240
        let height = field.cell?.cellSize(forBounds: NSRect(x: 0, y: 0, width: width, height: 10_000)).height ?? 22
        return CGSize(width: width, height: max(22, ceil(height)))
    }

    func updateNSView(_ field: NSTokenField, context: Context) {
        context.coordinator.parent = self
        // Compare the way `publish` reads the field, half-typed text
        // included. Counting tokens only made every keystroke look like a
        // change from outside: the field was reset mid-typing, which
        // published again, without end (the app hung and ran out of memory
        // forwarding a message, 2026-09-28).
        if Coordinator.addresses(in: field) != addresses {
            field.objectValue = addresses.map(Token.init)
        }
    }

    /// Wraps an address so the token field keeps it as one object.
    final class Token: NSObject {
        let address: AddressInfo
        init(_ address: AddressInfo) { self.address = address }
    }

    @MainActor
    final class Coordinator: NSObject, NSTokenFieldDelegate {
        var parent: RecipientField
        /// Completions from the last query, so a picked string maps back to
        /// the address with its display name.
        private var offered: [String: AddressInfo] = [:]

        init(_ parent: RecipientField) {
            self.parent = parent
        }

        func tokenField(_ tokenField: NSTokenField, completionsForSubstring substring: String,
                        indexOfToken tokenIndex: Int, indexOfSelectedItem selectedIndex: UnsafeMutablePointer<Int>?) -> [Any]? {
            offered = [:]
            let strings = Self.completions(for: substring, among: parent.suggest(substring))
            for (string, address) in strings { offered[string] = address }
            selectedIndex?.pointee = strings.isEmpty ? -1 : 0
            return strings.map(\.0)
        }

        /// What to offer for typed text. The token field completes inline
        /// with the first one, replacing what was typed, so each must start
        /// with the typed text: "Name <email>" when the name does, the bare
        /// address when the address does. Contacts that matched elsewhere
        /// (typing "dan" found "Jordan") are left out; before, "drew"
        /// became "Wrew".
        static func completions(for typed: String, among matches: [AddressInfo]) -> [(String, AddressInfo)] {
            let prefix = typed.trimmingCharacters(in: .whitespaces)
            guard !prefix.isEmpty else { return [] }
            var seen = Set<String>()
            return matches.compactMap { address in
                let full = editingString(address)
                let string = full.lowercased().hasPrefix(prefix.lowercased()) ? full
                    : address.email.lowercased().hasPrefix(prefix.lowercased()) ? address.email : nil
                guard let string, seen.insert(string).inserted else { return nil }
                return (string, address)
            }
        }

        func tokenField(_ tokenField: NSTokenField, representedObjectForEditing editingString: String) -> Any? {
            let trimmed = editingString.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return nil }
            return Token(offered[trimmed] ?? Self.parse(trimmed))
        }

        func tokenField(_ tokenField: NSTokenField, displayStringForRepresentedObject representedObject: Any) -> String? {
            guard let token = representedObject as? Token else { return representedObject as? String }
            return token.address.name.flatMap { $0.isEmpty ? nil : $0 } ?? token.address.email
        }

        func tokenField(_ tokenField: NSTokenField, editingStringForRepresentedObject representedObject: Any) -> String? {
            (representedObject as? Token).map { Self.editingString($0.address) }
        }

        func tokenField(_ tokenField: NSTokenField, hasMenuForRepresentedObject representedObject: Any) -> Bool { false }

        func controlTextDidChange(_ notification: Notification) { publish(notification) }
        func controlTextDidEndEditing(_ notification: Notification) { publish(notification) }

        private func publish(_ notification: Notification) {
            guard let field = notification.object as? NSTokenField else { return }
            let addresses = Self.addresses(in: field)
            if addresses != parent.addresses { parent.addresses = addresses }
        }

        /// The field's addresses: its tokens, and any text not yet made a
        /// token read as an address.
        static func addresses(in field: NSTokenField) -> [AddressInfo] {
            (field.objectValue as? [Any] ?? []).compactMap { item -> AddressInfo? in
                if let token = item as? Token { return token.address }
                if let string = item as? String, !string.trimmingCharacters(in: .whitespaces).isEmpty {
                    return parse(string)
                }
                return nil
            }
        }

        static func editingString(_ address: AddressInfo) -> String {
            guard let name = address.name, !name.isEmpty else { return address.email }
            return "\(name) <\(address.email)>"
        }

        /// "Name <email>", "<email>" or a bare address.
        static func parse(_ text: String) -> AddressInfo {
            let s = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if let open = s.lastIndex(of: "<"), let close = s.lastIndex(of: ">"), open < close {
                let email = String(s[s.index(after: open)..<close]).trimmingCharacters(in: .whitespaces)
                let name = s[..<open].trimmingCharacters(in: CharacterSet.whitespaces.union(CharacterSet(charactersIn: "\"")))
                return AddressInfo(name: name.isEmpty ? nil : name, email: email)
            }
            return AddressInfo(name: nil, email: s)
        }
    }
}
