import AppKit
import SwiftUI

// The design system's tokens (docs/design-system.md). Views outside
// `Design/` use these instead of literal numbers and colours;
// `scripts/design-lint.sh` checks it.

/// The spacing scale, for padding and stack spacing. Nothing in between:
/// when a layout seems to need 10, it gets 8 or 12.
enum Space {
    /// Inside chips and between a title and its subtitle.
    static let hair: CGFloat = 2
    /// Between an icon and its text; rows of a tight list.
    static let xs: CGFloat = 4
    /// Vertical padding of compact bars and banners.
    static let s: CGFloat = 6
    /// The default gap between controls; vertical padding of capsules.
    static let m: CGFloat = 8
    /// Horizontal padding of bars and banners in a column.
    static let l: CGFloat = 12
    /// Horizontal padding of glass capsules; padding of panels.
    static let xl: CGFloat = 16
    /// The reader's margins, as in Mail; gaps between sections.
    static let xxl: CGFloat = 20
    /// Padding of sheets.
    static let xxxl: CGFloat = 24
    /// Padding of full-window pages (onboarding).
    static let page: CGFloat = 32
}

/// Corner radii. Capsules use `.capsule`, not a radius.
enum Radius {
    /// Label chips in rows.
    static let chip: CGFloat = 4
    /// Attachment tiles, small filled controls.
    static let control: CGFloat = 6
    /// Cards: approval cards, tool calls, previews.
    static let card: CGFloat = 8
    /// Floating panels.
    static let panel: CGFloat = 12
}

/// Type roles. SwiftUI views use the `Font` values; the AppKit thread row
/// uses the `NSFont` ones so both columns read at the same sizes.
enum TypeRole {
    /// A column's or sheet's title.
    static let title = Font.title3.weight(.semibold)
    /// The one thing a page is about, read at arm's length: a decision's
    /// statement in Review mode, the count on its band.
    static let display = Font.title.weight(.semibold)
    /// Reading text beside `display`: quotes, explanations, scopes.
    static let reading = Font.title3
    /// Section headings in panels.
    static let heading = Font.headline
    /// Group labels inside a panel ("Find and summarise").
    static let groupLabel = Font.subheadline.weight(.semibold)
    /// Secondary text: bars, banners, notices, chips.
    static let meta = Font.callout
    /// Fine print under a control.
    static let caption = Font.caption

    /// Thread row: the senders line.
    static func rowSender(unread: Bool) -> NSFont { .systemFont(ofSize: 13, weight: unread ? .semibold : .regular) }
    /// Thread row: the subject line.
    static func rowSubject(unread: Bool) -> NSFont { .systemFont(ofSize: 12, weight: unread ? .medium : .regular) }
    /// Thread row: the snippet and the date.
    static var rowSecondary: NSFont { .systemFont(ofSize: 12) }
    /// A label chip's text, in rows and elsewhere.
    static var chip: NSFont { .systemFont(ofSize: 11, weight: .medium) }
    /// Plain text at body size (Settings rows, detail values).
    static let body = Font.body
    /// Code, ids and logs: body size, monospaced.
    static let code = Font.body.monospaced()
    /// Code at caption size (paths, ids in tables).
    static let codeCaption = Font.caption.monospaced()
    /// Numbers that change in place (Clean Up's progress card): `meta`
    /// with digits of one width, so columns of figures line up and a
    /// count does not jitter as it changes.
    static let numeric = Font.callout.monospacedDigit()
    /// A card's headline figure ("62%"): `title`, digits of one width.
    static let figure = Font.title3.weight(.semibold).monospacedDigit()
    /// The smallest status text (the sidebar's sync detail).
    static let fine = Font.caption2
    /// A first-run screen's heading (onboarding).
    static let welcome = Font.title.weight(.semibold)
    /// A thread row's unread count (AppKit).
    static var rowCount: NSFont { .systemFont(ofSize: 11, weight: .semibold) }
    /// A thread row's text at `size` (AppKit labels that set their own size).
    static func rowText(size: CGFloat) -> NSFont { .systemFont(ofSize: size) }
    /// The composer's address fields (AppKit).
    static var field: NSFont { .systemFont(ofSize: NSFont.systemFontSize) }
    /// The composer's body text: a point larger than the system's.
    @MainActor static var composerBody: NSFont { .systemFont(ofSize: NSFont.systemFontSize + 1) }
}

/// Semantic colours. Always system colours underneath, so light, dark,
/// increased contrast and the user's accent colour all follow.
enum Tone {
    /// The unread dot: the user's accent colour, as in Mail.
    static let unread = Color.accentColor
    static let unreadNS = NSColor.controlAccentColor
    /// Gmail's Important marker.
    static let important = Color.yellow
    static let importantNS = NSColor.systemYellow
    /// A label with no colour of its own.
    static let chipDefaultNS = NSColor.tertiaryLabelColor
    /// How strongly a label's colour fills its chip.
    static let chipFillOpacity: CGFloat = 0.28
    /// A chosen chip, where chips are picked (the task dialog).
    static let chipSelectedOpacity: CGFloat = 0.55

    /// A label chip's fill for a Gmail `#rrggbb` colour.
    static func chipFill(hex: String?) -> NSColor {
        (hex.flatMap(NSColor.init(hex:)) ?? chipDefaultNS).withAlphaComponent(chipFillOpacity)
    }

    /// Banner and card fills by intent.
    enum Intent {
        /// Needs the user (sign in again, approve a send).
        case attention
        /// Something to know (replying to, forwarding).
        case info
        /// A consequence to weigh (cannot send from here).
        case caution
        /// Nothing special: a resting card.
        case neutral

        var fill: AnyShapeStyle {
            switch self {
            case .attention: AnyShapeStyle(Color.yellow.opacity(0.14))
            case .info: AnyShapeStyle(.tint.opacity(0.10))
            case .caution: AnyShapeStyle(Color.orange.opacity(0.10))
            case .neutral: AnyShapeStyle(.quaternary.opacity(0.45))
            }
        }

        /// A card's outline; banners have none.
        var stroke: AnyShapeStyle {
            switch self {
            case .attention: AnyShapeStyle(Color.yellow.opacity(0.6))
            case .info, .caution, .neutral: AnyShapeStyle(Color.clear)
            }
        }
    }

    /// Status text. Red only for failure; orange for a consequence to
    /// weigh (a task that is overdue, a prompt missing its safety lines);
    /// green only for "approved".
    static let failure = Color.red
    static let failureNS = NSColor.systemRed
    static let caution = Color.orange
    static let cautionNS = NSColor.systemOrange
    static let approved = Color.green

    /// A task category's colour (spec §14.8): the starting set has one
    /// each, told apart at a glance; any other name takes one from the same
    /// palette by its name, so a category keeps its colour whatever its
    /// place in the list. Red, orange, yellow and pink are left out: they
    /// mean failure, caution and Important, or look like them.
    static func category(_ name: String) -> NSColor {
        let key = name.lowercased()
        if let fixed = startingCategories[key] { return fixed }
        let palette: [NSColor] = [.systemBlue, .systemPurple, .systemTeal, .systemGreen, .systemIndigo, .systemBrown,
                                  .systemCyan, .systemMint]
        // FNV-1a: stable across launches, unlike `hashValue`.
        var hash: UInt32 = 2_166_136_261
        for byte in key.utf8 { hash = (hash ^ UInt32(byte)) &* 16_777_619 }
        return palette[Int(hash % UInt32(palette.count))]
    }

    private static let startingCategories: [String: NSColor] = [
        "reply": .systemBlue, "decide": .systemPurple, "gather info": .systemTeal, "schedule": .systemGreen,
        "review": .systemIndigo, "admin": .systemBrown, "follow up": .systemCyan,
    ]

    /// The dot on a sidebar entry with something new since the user last
    /// looked (Analysis, spec §14.10): red, as app badges are, so it reads
    /// as "new", not as unread mail.
    static let newItems = Color.red
    /// Words the user added or kept where the AI wrote something else, in
    /// a side-by-side comparison; the AI's replaced words are struck
    /// through in the secondary colour instead.
    static let changedText = Color.accentColor.opacity(0.22)

    /// A highlighted (keyboard-selected) item inside glass.
    static let highlight = AnyShapeStyle(.tint.opacity(0.25))
    /// A small filled control resting on the background (attachments).
    static let controlFill = AnyShapeStyle(.quaternary.opacity(0.6))
}
