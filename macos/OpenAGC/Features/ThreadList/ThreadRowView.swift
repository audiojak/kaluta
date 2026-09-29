import AppKit

/// One thread-list row, calm as Mail's: who and when, the subject, two
/// lines of preview, and a hairline inset to the text between rows. Laid
/// out by hand (no Auto Layout) and reused by the table, so configuring a
/// row is a handful of property sets.
final class ThreadRowView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("ThreadRow")
    /// Four text lines (sender, subject, two of preview) and margins.
    static let height: CGFloat = 88

    private let unreadDot = NSView()
    private let repliedMark = NSImageView()
    private let senders = ThreadRowView.label(size: 13)
    private let count = ThreadRowView.label(size: 11)
    private let date = ThreadRowView.label(size: 11)
    private let subject = ThreadRowView.label(size: 12)
    private let snippet = ThreadRowView.label(size: 12, lines: 2)
    private let badges = NSImageView()
    private let separator = NSView()

    private static let padding: CGFloat = 12
    private static let dotSize: CGFloat = 8

    override init(frame: NSRect) {
        super.init(frame: frame)
        identifier = Self.identifier
        unreadDot.wantsLayer = true
        unreadDot.layer?.cornerRadius = Self.dotSize / 2
        unreadDot.layer?.backgroundColor = Tone.unreadNS.cgColor
        date.alignment = .right
        date.textColor = .secondaryLabelColor
        snippet.textColor = .secondaryLabelColor
        badges.imageScaling = .scaleProportionallyDown
        badges.contentTintColor = .secondaryLabelColor
        count.alignment = .right
        count.font = .systemFont(ofSize: 11, weight: .semibold)
        count.textColor = Tone.unreadNS
        repliedMark.image = NSImage(systemSymbolName: "arrowshape.turn.up.left.fill", accessibilityDescription: "Replied")?
            .withSymbolConfiguration(.init(pointSize: 9, weight: .medium))
        repliedMark.contentTintColor = .secondaryLabelColor
        repliedMark.imageScaling = .scaleProportionallyDown
        separator.wantsLayer = true
        for view in [unreadDot, repliedMark, senders, count, date, subject, snippet, badges, separator] {
            addSubview(view)
        }
    }

    /// The hairline between rows is hidden under the selection, where it
    /// would cut across the highlight.
    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { separator.isHidden = backgroundStyle == .emphasized }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        updateSeparatorColor()
    }

    private func updateSeparatorColor() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            separator.layer?.backgroundColor = NSColor.separatorColor.cgColor
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// A user label shown on a row: its full path and its color.
    struct Chip: Equatable {
        let path: String
        let color: String?
    }

    /// The labels to chip on a row: user labels other than the mailbox
    /// being shown, in path order, at most `limit`.
    static func chips(for row: ThreadRow, labels: [String: Chip], excluding mailboxID: String?, limit: Int = 3) -> [Chip] {
        row.labelIds
            .filter { $0 != mailboxID }
            .compactMap { labels[$0] }
            .sorted { $0.path.localizedStandardCompare($1.path) == .orderedAscending }
            .prefix(limit)
            .map { $0 }
    }

    /// Chips as text: each leaf name on a tint of its label color, then the
    /// snippet. Kept in the snippet line so rows keep their fixed height.
    static func snippetLine(_ snippet: String, chips: [Chip]) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let font = TypeRole.chip
        for chip in chips {
            out.append(NSAttributedString(string: "\u{2009}\(LabelTree.leafName(chip.path))\u{2009}", attributes: [
                .font: font,
                .foregroundColor: NSColor.labelColor,
                .backgroundColor: Tone.chipFill(hex: chip.color),
            ]))
            out.append(NSAttributedString(string: " ", attributes: [.font: font]))
        }
        out.append(NSAttributedString(string: snippet, attributes: [
            .font: TypeRole.rowSecondary,
            .foregroundColor: NSColor.secondaryLabelColor,
        ]))
        return out
    }

    /// `markImportant`: false where every row is Important anyway (the
    /// Important mailbox, the Inbox's Important-only view).
    func configure(with row: ThreadRow, chips: [Chip] = [], me: Set<String> = [], markImportant: Bool = true) {
        let unread = row.unreadCount > 0
        unreadDot.isHidden = !unread
        repliedMark.isHidden = !row.replied
        senders.stringValue = Self.senderLine(row, me: me)
        senders.font = TypeRole.rowSender(unread: unread)
        count.stringValue = Self.countText(row) ?? ""
        count.isHidden = count.stringValue.isEmpty
        updateSeparatorColor()
        date.stringValue = RowDateFormatter.string(forMillis: row.lastMessageAt)
        let subjectText = row.subject.isEmpty ? "(no subject)" : row.subject
        let subjectFont = TypeRole.rowSubject(unread: unread)
        subject.font = subjectFont
        if markImportant, Self.isImportant(row) {
            subject.attributedStringValue = Self.importantSubject(subjectText, font: subjectFont)
        } else {
            subject.stringValue = subjectText
        }
        if chips.isEmpty {
            snippet.stringValue = row.snippet
            toolTip = nil
        } else {
            snippet.attributedStringValue = Self.snippetLine(row.snippet, chips: chips)
            toolTip = chips.map(\.path).joined(separator: ", ")
        }
        badges.image = Self.badgeImage(row)
        badges.isHidden = badges.image == nil

        setAccessibilityLabel(
            [unread ? "Unread" : nil, Self.isImportant(row) ? "Important" : nil, row.replied ? "Replied" : nil,
             senders.stringValue, count.isHidden ? nil : "\(row.messageCount) messages", subjectText, date.stringValue,
             chips.isEmpty ? nil : "Labels: " + chips.map(\.path).joined(separator: ", "), row.snippet]
                .compactMap { $0 }.joined(separator: ", "))
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let p = Self.padding
        let w = bounds.width
        let textX = p + Self.dotSize + 6
        // As wide as the date's text, so the count sits right beside it.
        let dateWidth = min(90, Self.textWidth(date) + 6)
        let lineHeight: CGFloat = 17
        // Flipped-agnostic: compute from the top.
        let top = bounds.height - 10
        unreadDot.frame = NSRect(x: p, y: top - lineHeight + 5, width: Self.dotSize, height: Self.dotSize)
        repliedMark.frame = NSRect(x: p - 2, y: top - 2 * lineHeight + 2, width: Self.dotSize + 4, height: Self.dotSize + 4)
        date.frame = NSRect(x: w - p - dateWidth, y: top - lineHeight, width: dateWidth, height: lineHeight)
        let countWidth: CGFloat = count.isHidden ? 0 : Self.textWidth(count) + 8
        count.frame = NSRect(x: w - p - dateWidth - countWidth, y: top - lineHeight, width: countWidth, height: lineHeight)
        senders.frame = NSRect(x: textX, y: top - lineHeight,
                               width: max(0, w - textX - dateWidth - countWidth - p - 6), height: lineHeight)
        let badgeWidth: CGFloat = badges.isHidden ? 0 : 16
        badges.frame = NSRect(x: w - p - badgeWidth, y: top - 2 * lineHeight, width: badgeWidth, height: lineHeight)
        subject.frame = NSRect(x: textX, y: top - 2 * lineHeight, width: max(0, w - textX - p - badgeWidth - 4), height: lineHeight)
        snippet.frame = NSRect(x: textX, y: top - 4 * lineHeight, width: max(0, w - textX - p), height: 2 * lineHeight)
        let hairline = 1 / max(window?.backingScaleFactor ?? 2, 1)
        separator.frame = NSRect(x: textX, y: 0, width: max(0, w - textX), height: hairline)
    }

    // MARK: - Content

    /// Who the thread is with, as Mail puts it: the other people, not you
    /// (`me`: your addresses, lowercased); one by full name, several by
    /// first name ("Jeffrey & Andre", "Himanshi, Darshan, Austin …");
    /// "Me" when it is only you.
    static func senderLine(_ row: ThreadRow, me: Set<String> = []) -> String {
        let others = row.participants.filter { !me.contains($0.email.lowercased()) }
        var line: String
        switch others.count {
        case 0: line = row.participants.isEmpty ? "(unknown sender)" : "Me"
        case 1: line = fullName(others[0])
        case 2: line = "\(firstName(others[0])) & \(firstName(others[1]))"
        default:
            line = others.prefix(3).map(firstName).joined(separator: ", ")
            if others.count > 3 { line += " …" }
        }
        return line
    }

    /// The thread's message count, shown in the accent colour beside the
    /// date as Mail does; nothing for a single message.
    static func countText(_ row: ThreadRow) -> String? {
        row.messageCount > 1 ? "\(row.messageCount)" : nil
    }

    private static func fullName(_ a: AddressInfo) -> String {
        guard let name = a.name?.trimmingCharacters(in: .whitespaces), !name.isEmpty else { return a.email }
        return name
    }

    /// "Jeffrey Priebe" → "Jeffrey"; "Le, Minh" → "Minh"; no name → the
    /// address's local part.
    static func firstName(_ a: AddressInfo) -> String {
        guard let name = a.name?.trimmingCharacters(in: .whitespaces), !name.isEmpty else {
            return String(a.email.split(separator: "@").first ?? Substring(a.email))
        }
        if let comma = name.firstIndex(of: ",") {
            let given = name[name.index(after: comma)...].trimmingCharacters(in: .whitespaces)
            if let first = given.split(separator: " ").first { return String(first) }
        }
        return String(name.split(separator: " ").first ?? Substring(name))
    }

    /// Gmail's importance marker (the yellow chevron).
    static func isImportant(_ row: ThreadRow) -> Bool {
        row.labelIds.contains("IMPORTANT")
    }

    static func importantSubject(_ text: String, font: NSFont) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let config = NSImage.SymbolConfiguration(pointSize: 9, weight: .bold)
            .applying(.init(paletteColors: [Tone.importantNS]))
        if let marker = NSImage(systemSymbolName: "chevron.right.2", accessibilityDescription: "Important")?
            .withSymbolConfiguration(config) {
            let attachment = NSTextAttachment()
            attachment.image = marker
            attachment.bounds = NSRect(x: 0, y: -1, width: marker.size.width, height: marker.size.height)
            out.append(NSAttributedString(attachment: attachment))
            out.append(NSAttributedString(string: " "))
        }
        out.append(NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: NSColor.labelColor]))
        return out
    }

    private static let starImage = NSImage(systemSymbolName: "star.fill", accessibilityDescription: "Starred")
    private static let clipImage = NSImage(systemSymbolName: "paperclip", accessibilityDescription: "Has attachments")

    private static func badgeImage(_ row: ThreadRow) -> NSImage? {
        if row.isStarred { return starImage }
        if row.hasAttachments { return clipImage }
        return nil
    }

    /// A one-line label's text width in its font.
    private static func textWidth(_ field: NSTextField) -> CGFloat {
        ceil((field.stringValue as NSString).size(withAttributes: [.font: field.font ?? .systemFont(ofSize: 11)]).width)
    }

    private static func label(size: CGFloat, lines: Int = 1) -> NSTextField {
        let field = lines > 1 ? NSTextField(wrappingLabelWithString: "") : NSTextField(labelWithString: "")
        field.font = .systemFont(ofSize: size)
        field.lineBreakMode = lines > 1 ? .byWordWrapping : .byTruncatingTail
        field.maximumNumberOfLines = lines
        field.cell?.truncatesLastVisibleLine = true
        field.isSelectable = false
        return field
    }
}

/// Mail-style dates: time today, "Mon" this week, "Sep 12" this year, else
/// a short date. Formatters are created once; making them per row stalls
/// scrolling.
enum RowDateFormatter {
    private static let time: DateFormatter = make("jmm")
    private static let weekday: DateFormatter = make("EEE")
    private static let monthDay: DateFormatter = make("MMMd")
    private static let full: DateFormatter = {
        let f = DateFormatter()
        f.dateStyle = .short
        f.timeStyle = .none
        return f
    }()

    static func string(forMillis millis: Int64, now: Date = .now, calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(millis) / 1000)
        if calendar.isDate(date, inSameDayAs: now) { return time.string(from: date) }
        if let days = calendar.dateComponents([.day], from: date, to: now).day, days < 7, date < now {
            return weekday.string(from: date)
        }
        if calendar.isDate(date, equalTo: now, toGranularity: .year) { return monthDay.string(from: date) }
        return full.string(from: date)
    }

    private static func make(_ template: String) -> DateFormatter {
        let f = DateFormatter()
        f.setLocalizedDateFormatFromTemplate(template)
        return f
    }
}

extension NSColor {
    /// `#rrggbb` → NSColor.
    convenience init?(hex: String) {
        let h = hex.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "#"))
        guard h.count == 6, let v = UInt32(h, radix: 16) else { return nil }
        self.init(srgbRed: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255,
                  blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }
}
