import Foundation

/// A person's or account's initials and colour (docs/design-system.md,
/// Components: Avatars). One function for the reader's sender circles and
/// the account avatar, so an address looks the same everywhere.
enum Avatar {
    /// Colours that carry white text in light and dark mode.
    static let palette = ["#5B8DEF", "#43A67F", "#D98E3C", "#B46BD6", "#D4626E",
                          "#3FA3B8", "#8C8F4A", "#7A7FD9", "#C2743F", "#4F9D5B"]

    /// "Darshan Patel" → "DP"; "Le, Minh" → "ML"; no usable name → the
    /// address's first letter; nothing at all → "?".
    nonisolated static func initials(name: String?, email: String) -> String {
        let name = (name ?? "").trimmingCharacters(in: .whitespaces)
        var words = name.split(whereSeparator: \.isWhitespace).map(String.init)
        if let comma = name.firstIndex(of: ",") {
            words = (name[name.index(after: comma)...] + " " + name[..<comma])
                .split(whereSeparator: \.isWhitespace).map(String.init)
        }
        let letters = words.filter { $0.first?.isLetter == true }
        guard name != email, let first = letters.first?.first else {
            return email.first.map { String($0).uppercased() } ?? "?"
        }
        let last = letters.count > 1 ? letters.last?.first.map(String.init) ?? "" : ""
        return (String(first) + last).uppercased()
    }

    /// The same palette entry for an address every time: FNV-1a with a
    /// finaliser over its lowercased bytes, so similar addresses spread.
    nonisolated static func paletteIndex(for email: String) -> Int {
        var x = email.lowercased().utf8.reduce(UInt32(2_166_136_261)) { ($0 ^ UInt32($1)) &* 16_777_619 }
        x ^= x >> 16
        x = x &* 0x7FEB_352D
        x ^= x >> 15
        x = x &* 0x846C_A68B
        x ^= x >> 16
        return Int(x % UInt32(palette.count))
    }

    /// `#rrggbb` for an address (the reader's HTML).
    nonisolated static func hex(for email: String) -> String { palette[paletteIndex(for: email)] }
}
