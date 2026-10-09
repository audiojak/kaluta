import Foundation

/// Builds the single HTML document the reader shows for a thread: one
/// `<details>` block per message (collapsed without JavaScript), each body
/// a fragment sanitized in Rust (spec §14.4). Pure, so it is unit-tested.
enum EmailDocument {
    /// Everything but inline styles and our own image schemes is blocked.
    /// Remote images are still gated by the scheme handler.
    static let contentSecurityPolicy =
        "default-src 'none'; img-src kaluta-cid: kaluta-remote: data:; style-src 'unsafe-inline'"

    struct Message: Equatable {
        let id: String
        let fromName: String
        let fromEmail: String
        /// To and Cc, as they should read ("Me" for the user).
        let recipients: [String]
        let date: Date
        let snippet: String
        let isRead: Bool
        /// Sanitized fragment; `nil` until the body has been synced.
        let html: String?
        /// Not sent: drawn as a draft, not like mail that went.
        var isDraft = false
    }

    /// Each message is a card, as in Mail: an initials avatar, the sender,
    /// the date, a short To line (all of it on hover) and, once open, the
    /// body. Collapsed cards show one line of the message instead. A draft
    /// is an outlined, unfilled card marked "Draft", its time "Saved …".
    /// `onlyLatestOpen`: the composer's view of the thread being answered,
    /// where only the latest message starts open.
    static func thread(_ messages: [Message], isDark: Bool, onlyLatestOpen: Bool = false) -> String {
        var out = """
        <!doctype html><html><head><meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="\(contentSecurityPolicy)">
        <meta name="color-scheme" content="light dark">
        <style>\(stylesheet)</style></head><body>
        """
        for (index, message) in messages.enumerated() {
            let expanded = onlyLatestOpen ? index == messages.count - 1
                : isExpanded(message, index: index, count: messages.count)
            out += "<details class=\"msg\(message.isDraft ? " draft" : "")\"\(expanded ? " open" : "") id=\"m-\(escape(message.id))\"><summary>"
            out += "<div class=\"avatar\" style=\"background:\(avatarColor(message.fromEmail))\" aria-hidden=\"true\">"
            out += "\(escape(initials(name: message.fromName, email: message.fromEmail)))</div>"
            out += "<div class=\"meta\"><div class=\"hdr\">"
            if message.isDraft { out += "<span class=\"badge\">Draft</span>" }
            out += "<span class=\"from\">\(escape(message.fromName))</span>"
            if message.fromEmail != message.fromName {
                out += "<span class=\"addr\">\(escape(message.fromEmail))</span>"
            }
            let when = dateString(message.date)
            out += "<span class=\"date\">\(escape(message.isDraft ? "Saved \(when)" : when))</span></div>"
            if !message.recipients.isEmpty {
                let full = message.recipients.joined(separator: ", ")
                out += "<div class=\"to\" title=\"\(escape(full))\">To: \(escape(shortRecipients(message.recipients)))</div>"
            }
            out += "<div class=\"snippet\">\(escape(message.snippet))</div></div></summary>"
            if let html = message.html {
                let paper = isDark && looksStyled(html)
                out += "<div class=\"body\(paper ? " paper" : "")\">\(html)</div>"
            } else {
                out += "<div class=\"body pending\">Downloading message…</div>"
            }
            out += "</details>"
        }
        out += "</body></html>"
        return out
    }

    /// "Darshan Patel", "Me & Darshan Patel", "Darshan Patel, Austin Born + 10".
    static func shortRecipients(_ names: [String]) -> String {
        switch names.count {
        case 0: ""
        case 1: names[0]
        case 2: "\(names[0]) & \(names[1])"
        default: "\(names[0]), \(names[1]) + \(names.count - 2)"
        }
    }

    /// Initials and colour come from `Avatar`, as the account avatar's do.
    static func initials(name: String, email: String) -> String { Avatar.initials(name: name, email: email) }

    static func avatarColor(_ email: String) -> String { Avatar.hex(for: email) }

    /// The latest message and every unread one start open, like Mail.
    static func isExpanded(_ message: Message, index: Int, count: Int) -> Bool {
        index == count - 1 || !message.isRead
    }

    /// In dark mode, mail that sets its own colors is shown on a light
    /// "paper" so dark text never lands on a dark background; unstyled mail
    /// adopts the system colors.
    static func looksStyled(_ html: String) -> Bool {
        let lower = html.lowercased()
        return ["bgcolor", "background", "color:", "color=", "<font"].contains { lower.contains($0) }
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
            case "'": out += "&#39;"
            default: out.append(c)
            }
        }
        return out
    }

    private static var dateFormatter: DateFormatter { DateStyle.readerHeader }

    private static func dateString(_ date: Date) -> String {
        dateFormatter.string(from: date)
    }

    /// A design token as CSS (docs/design-system.md): the reader keeps to
    /// the same spacing and radius scale as the app.
    private static func px(_ value: CGFloat) -> String { "\(Int(value))px" }

    /// Message cards: `Radius.card`, padded `Space.m`/`Space.l`; no outline
    /// (only attention cards have one). The body lines up past the avatar
    /// (32) and the gap (`Space.m`).
    private static let stylesheet = """
    :root { color-scheme: light dark; --card: color-mix(in srgb, CanvasText 4%, Canvas); }
    html, body { margin: 0; background: Canvas; color: CanvasText; overflow-x: hidden; }
    body { font: 14px/1.45 -apple-system, system-ui, sans-serif; padding: 8px 20px 24px; overflow-wrap: anywhere; }
    .msg { background: var(--card); border-radius: \(px(Radius.card)); padding: \(px(Space.m)) \(px(Space.l)); \
    margin: 0 0 \(px(Space.m)); }
    summary { list-style: none; cursor: default; display: flex; gap: \(px(Space.m)); align-items: flex-start; }
    summary::-webkit-details-marker { display: none; }
    .avatar { flex: none; width: 32px; height: 32px; border-radius: 50%; color: #fff; font: 600 12px/32px -apple-system, system-ui; \
    text-align: center; letter-spacing: 0.3px; }
    details:not([open]) .avatar { width: 26px; height: 26px; line-height: 26px; font-size: 11px; }
    .meta { flex: 1; min-width: 0; }
    .hdr { display: flex; gap: 8px; align-items: baseline; }
    .from { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .addr, .to, .date, .snippet { color: GrayText; }
    .addr, .to { font-size: 12px; }
    .addr { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    details:not([open]) .addr, details:not([open]) .to { display: none; }
    .date { margin-left: auto; font-size: 12px; white-space: nowrap; }
    .to { margin-top: \(px(Space.hair)); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .snippet { margin-top: \(px(Space.hair)); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    details[open] .snippet { display: none; }
    .body { margin: \(px(Space.l)) 0 \(px(Space.hair)) \(px(32 + Space.m)); overflow-x: auto; }
    .body img { max-width: 100%; height: auto; }
    .body table { max-width: 100%; }
    .body pre { white-space: pre-wrap; }
    .body.paper { background: #ffffff; color: #111111; color-scheme: light; border-radius: \(px(Radius.card)); \
    padding: \(px(Space.l)); }
    .body.pending { color: GrayText; font-style: italic; }
    .msg.draft { background: transparent; border: 1px dashed color-mix(in srgb, #FF9500 70%, transparent); }
    .badge { flex: none; font: 600 11px/16px -apple-system, system-ui; color: #C75C00; padding: 0 6px; border-radius: 4px; \
    background: color-mix(in srgb, #FF9500 18%, transparent); }
    @media (prefers-color-scheme: dark) { .badge { color: #FFB45C; } }
    blockquote { margin: \(px(Space.m)) 0; padding-left: \(px(Space.m)); border-left: 2px solid color-mix(in srgb, CanvasText 25%, transparent); color: GrayText; }
    a { color: LinkText; }
    .body details.kaluta-quote { margin: \(px(Space.l)) 0 0; }
    .body details.kaluta-quote > summary { display: inline-block; padding: 0 \(px(Space.m)); \
    border-radius: \(px(Radius.card)); cursor: pointer; \
    font: 700 12px/16px -apple-system, system-ui; letter-spacing: 1px; color: GrayText; \
    background: color-mix(in srgb, CanvasText 9%, transparent); }
    .body details.kaluta-quote > summary:hover { background: color-mix(in srgb, CanvasText 16%, transparent); }
    .body details.kaluta-quote[open] > summary { margin-bottom: \(px(Space.m)); }
    """
}
