//! Folding the quoted history out of a message for the reader (spec §14.4):
//! in a thread every earlier message is shown already, so the copy a reply
//! carries below it ("On … wrote:" and the quote, or Outlook's "From: …
//! Sent: …" block) is collapsed behind a "•••" toggle, as in Gmail.
//!
//! Works on what the sanitizer stored, which has lost Gmail's and Outlook's
//! class names, so it goes by structure and wording. Sanitized HTML is
//! well-formed (ammonia serializes a parsed tree), so a small tokenizer is
//! enough. Mail with replies between quotes (answering inline) is left alone.

/// The wrapper the reader styles; a `<details>` so it works without script.
const OPEN: &str = "<details class=\"kaluta-quote\"><summary title=\"Show the quoted text\">•••</summary>";
const CLOSE: &str = "</details>";

/// Text of a reply between or after quotes beyond which the message is
/// taken to answer inline, and is not folded.
const INLINE_REPLY_CHARS: usize = 300;

const VOID: &[&str] =
    &["br", "hr", "img", "wbr", "col", "area", "base", "embed", "input", "link", "meta", "source", "track"];

#[derive(Debug)]
enum Node {
    Text(String),
    Element { name: String, open: String, children: Vec<Node>, closed: bool },
}

/// Sanitized HTML with its quoted history folded; unchanged when there is
/// none, or when folding would hide the whole message.
pub fn fold_quoted_html(html: &str) -> String {
    let mut nodes = parse(html);
    if fold(&mut nodes) {
        let mut out = String::with_capacity(html.len() + OPEN.len() + CLOSE.len());
        for n in &nodes {
            write(n, &mut out);
        }
        out
    } else {
        html.to_owned()
    }
}

/// Sanitized HTML without its quoted history, if it has one: the user's
/// own part of a reply, for reading what they wrote (spec §14.9, §14.10).
pub fn without_quoted_html(html: &str) -> Option<String> {
    let mut nodes = parse(html);
    if !fold(&mut nodes) {
        return None;
    }
    let mut out = String::with_capacity(html.len());
    for n in &nodes {
        write_unquoted(n, &mut out);
    }
    Some(out)
}

fn write_unquoted(node: &Node, out: &mut String) {
    match node {
        Node::Element { open, .. } if open == OPEN => {}
        Node::Element { name, open, children, closed } => {
            out.push_str(open);
            for c in children {
                write_unquoted(c, out);
            }
            if *closed && !VOID.contains(&name.as_str()) && !name.is_empty() {
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
        }
        Node::Text(t) => out.push_str(t),
    }
}

/// Plain text for HTML mail, with quotes marked as mail clients expect:
/// each line of a `<blockquote>` starts with `> ` (`> > ` when nested), so
/// a plain-text reader can tell the quoted history from the reply.
pub fn html_to_quoted_text(html: &str) -> String {
    let mut out = String::new();
    quoted_text(&parse(html), &mut out);
    // Lines end without trailing spaces; at most one blank line in a row.
    let mut tidy = String::with_capacity(out.len());
    let mut blank = 0;
    for line in out.lines() {
        let line = line.trim_end();
        blank = if line.is_empty() { blank + 1 } else { 0 };
        if blank > 1 {
            continue;
        }
        tidy.push_str(line);
        tidy.push('\n');
    }
    tidy.trim_matches('\n').to_owned()
}

fn quoted_text(nodes: &[Node], out: &mut String) {
    let mut plain = String::new();
    let flush = |plain: &mut String, out: &mut String| {
        if !plain.is_empty() {
            out.push_str(&crate::html_to_text(plain));
            plain.clear();
        }
    };
    for node in nodes {
        match node {
            Node::Element { name, children, .. } if name == "blockquote" => {
                flush(&mut plain, out);
                let mut inner = String::new();
                quoted_text(children, &mut inner);
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                for line in inner.trim_matches('\n').lines() {
                    let line = line.trim_end();
                    out.push_str(if line.is_empty() { ">" } else { "> " });
                    out.push_str(line);
                    out.push('\n');
                }
            }
            // A quote further in: this element's own text, then its parts.
            Node::Element { name, children, .. } if contains(node, "blockquote") => {
                flush(&mut plain, out);
                if !out.is_empty() && !out.ends_with('\n') && !VOID.contains(&name.as_str()) {
                    out.push('\n');
                }
                quoted_text(children, out);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => write(node, &mut plain),
        }
    }
    flush(&mut plain, out);
}

/// Plain text split at its quoted history: the reply, and the rest (from
/// the attribution or header line on), if any.
pub fn split_quoted_text(text: &str) -> (&str, Option<&str>) {
    let lines: Vec<(usize, &str)> = text
        .split_inclusive('\n')
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len();
            Some((start, line))
        })
        .collect();
    let quoted = |l: &str| l.trim_start().starts_with('>');
    for (i, (start, line)) in lines.iter().enumerate() {
        let t = line.trim();
        let header = t == "-----Original Message-----"
            || (t.starts_with("From:")
                && lines[i + 1..].iter().take(4).any(|(_, l)| l.trim_start().starts_with("Sent:")));
        let attribution = is_attribution(t)
            && lines[i + 1..].iter().find(|(_, l)| !l.trim().is_empty()).is_some_and(|(_, l)| quoted(l));
        let quote_block = quoted(line) && lines[i..].iter().all(|(_, l)| l.trim().is_empty() || quoted(l));
        if header || attribution || quote_block {
            let reply = &text[..*start];
            if reply.trim().is_empty() {
                return (text, None);
            }
            return (reply, Some(&text[*start..]));
        }
        // Answering inline: a quote with more reply after it.
        if quoted(line) {
            let after: usize = lines[i..].iter().filter(|(_, l)| !quoted(l)).map(|(_, l)| l.trim().len()).sum();
            if after > INLINE_REPLY_CHARS {
                return (text, None);
            }
        }
    }
    (text, None)
}

/// Plain text for the reader: as `text_to_html`, with its quoted history
/// folded.
pub fn text_to_reader_html(text: &str) -> String {
    match split_quoted_text(text) {
        (reply, Some(quoted)) => {
            format!("{}{OPEN}{}{CLOSE}", crate::text_to_html(reply.trim_end()), crate::text_to_html(quoted))
        }
        (all, None) => crate::text_to_html(all),
    }
}

/// "On Tue, 1 Oct 2026 at 10:02, Ann <ann@x.com> wrote:" and its kin.
fn is_attribution(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.len() <= 400
        && ["wrote:", "schrieb:", "a écrit :", "a écrit:", "escribió:", "ha scritto:", "skrev:", "schreef:", "napisał:"]
            .iter()
            .any(|end| t.ends_with(end))
}

fn is_outlook_header(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("-----Original Message-----")
        || (t.len() <= 2000
            && (t.starts_with("From:") || t.starts_with("From :"))
            && (t.contains("Sent:") || t.contains("Date:"))
            && (t.contains("To:") || t.contains("Subject:")))
}

// MARK: Folding

/// Fold the first quote found, in place. Returns whether anything folded.
fn fold(nodes: &mut Vec<Node>) -> bool {
    let mut seen_text = false;
    fold_in(nodes, &mut seen_text)
}

fn fold_in(nodes: &mut Vec<Node>, seen_text: &mut bool) -> bool {
    for i in 0..nodes.len() {
        if let Some(from) = quote_start(nodes, i) {
            // Nothing before it: the whole message is the quote (a
            // forward, say); leave it as it is.
            if !*seen_text || inline_reply(&nodes[from..]) {
                return false;
            }
            let rest: Vec<Node> = nodes.drain(from..).collect();
            nodes.push(Node::Element { name: "details".into(), open: OPEN.into(), children: rest, closed: true });
            return true;
        }
        match &mut nodes[i] {
            Node::Text(t) => {
                if !decode(t).trim().is_empty() {
                    *seen_text = true;
                }
            }
            Node::Element { name, children, .. } => {
                if name == "img" {
                    *seen_text = true;
                }
                if fold_in(children, seen_text) {
                    return true;
                }
            }
        }
    }
    false
}

/// Where a quote starts at `nodes[i]`, if it does: a blockquote; an
/// attribution line before one; Outlook's header block (from the rule
/// above it, when there is one).
fn quote_start(nodes: &[Node], i: usize) -> Option<usize> {
    let Node::Element { name, .. } = &nodes[i] else {
        // A bare attribution text node right before a blockquote.
        let Node::Text(t) = &nodes[i] else { return None };
        return (is_attribution(&decode(t)) && next_is_quote(nodes, i)).then_some(i);
    };
    if name == "blockquote" {
        return Some(i);
    }
    let text = text_of(&nodes[i]);
    if is_attribution(&text) && next_is_quote(nodes, i) {
        return Some(i);
    }
    if is_outlook_header(&text) && !matches!(name.as_str(), "body" | "html") && !contains_block_children(&nodes[i]) {
        let rule = i.checked_sub(1).filter(|&p| matches!(&nodes[p], Node::Element { name, .. } if name == "hr"));
        return Some(rule.unwrap_or(i));
    }
    None
}

/// The next node with content is (or holds) a blockquote.
fn next_is_quote(nodes: &[Node], i: usize) -> bool {
    nodes[i + 1..]
        .iter()
        .find(|n| !matches!(n, Node::Text(t) if decode(t).trim().is_empty()) && !is_br(n))
        .is_some_and(|n| contains(n, "blockquote"))
}

/// An Outlook header is a short block of lines, not a wrapper around the
/// whole message: it holds no blockquote or table of its own.
fn contains_block_children(node: &Node) -> bool {
    match node {
        Node::Element { children, .. } => children.iter().any(|c| contains(c, "blockquote") || contains(c, "table")),
        Node::Text(_) => false,
    }
}

/// Reply text among the quotes (answering inline): too much to fold.
fn inline_reply(range: &[Node]) -> bool {
    let mut outside = 0;
    for n in range {
        outside += text_outside_quotes(n);
    }
    // The attribution or header itself is not reply text.
    let first = range.first().map(|n| if contains(n, "blockquote") { 0 } else { text_of(n).len() }).unwrap_or(0);
    outside.saturating_sub(first) > INLINE_REPLY_CHARS && !range.iter().any(|n| is_outlook_header(&text_of(n)))
}

fn text_outside_quotes(node: &Node) -> usize {
    match node {
        Node::Text(t) => decode(t).trim().len(),
        Node::Element { name, .. } if name == "blockquote" => 0,
        Node::Element { children, .. } => children.iter().map(text_outside_quotes).sum(),
    }
}

fn contains(node: &Node, tag: &str) -> bool {
    match node {
        Node::Text(_) => false,
        Node::Element { name, children, .. } => name == tag || children.iter().any(|c| contains(c, tag)),
    }
}

fn is_br(node: &Node) -> bool {
    matches!(node, Node::Element { name, .. } if name == "br")
}

fn text_of(node: &Node) -> String {
    let mut out = String::new();
    collect_text(node, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => out.push_str(&decode(t)),
        Node::Element { name, children, .. } => {
            if name == "br" {
                out.push('\n');
            }
            for c in children {
                collect_text(c, out);
            }
            if matches!(name.as_str(), "div" | "p") {
                out.push('\n');
            }
        }
    }
}

fn decode(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

// MARK: Parsing and writing

fn parse(html: &str) -> Vec<Node> {
    let mut stack: Vec<Node> =
        vec![Node::Element { name: String::new(), open: String::new(), children: vec![], closed: true }];
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = tag_end(html, i);
            let tag = &html[i..end];
            if let Some(name) = tag.strip_prefix("</") {
                let name = name.trim_end_matches('>').trim().to_ascii_lowercase();
                // Close up to the matching element (well-formed input closes
                // the top one).
                if let Some(depth) =
                    stack.iter().rposition(|n| matches!(n, Node::Element { name: n, .. } if *n == name))
                {
                    while stack.len() > depth.max(1) {
                        let done = stack.pop().unwrap_or(Node::Text(String::new()));
                        push_child(&mut stack, done);
                    }
                }
            } else {
                let name: String =
                    tag[1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
                let element =
                    Node::Element { name: name.clone(), open: tag.to_owned(), children: vec![], closed: false };
                if VOID.contains(&name.as_str()) || tag.ends_with("/>") || name.is_empty() {
                    push_child(&mut stack, element);
                } else {
                    stack.push(element);
                }
            }
            i = end;
        } else {
            let next = html[i..].find('<').map_or(html.len(), |n| i + n);
            push_child(&mut stack, Node::Text(html[i..next].to_owned()));
            i = next;
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().unwrap_or(Node::Text(String::new()));
        push_child(&mut stack, done);
    }
    match stack.pop() {
        Some(Node::Element { children, .. }) => children,
        _ => vec![],
    }
}

/// Mark a finished element closed and add it to its parent.
fn push_child(stack: &mut [Node], mut node: Node) {
    if let Node::Element { name, closed, .. } = &mut node
        && !VOID.contains(&name.as_str())
        && !name.is_empty()
    {
        *closed = true;
    }
    if let Some(Node::Element { children, .. }) = stack.last_mut() {
        children.push(node);
    }
}

/// The end of the tag starting at `start`, minding quoted attribute values.
fn tag_end(html: &str, start: usize) -> usize {
    let mut quote: Option<u8> = None;
    for (i, &b) in html.as_bytes()[start + 1..].iter().enumerate() {
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'>') => return start + 1 + i + 1,
            _ => {}
        }
    }
    html.len()
}

fn write(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) => out.push_str(t),
        Node::Element { name, open, children, closed } => {
            out.push_str(open);
            for c in children {
                write(c, out);
            }
            if *closed && !VOID.contains(&name.as_str()) && !name.is_empty() {
                out.push_str(if open == OPEN { CLOSE } else { "" });
                if open != OPEN {
                    out.push_str("</");
                    out.push_str(name);
                    out.push('>');
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gmail_reply_folds_its_attribution_and_quote() {
        // As stored: the sanitizer dropped Gmail's class names.
        let html = r#"<div dir="ltr">Sounds good, see you then.</div><br><div><div dir="ltr">On Tue, Oct 1, 2026 at 10:02 AM Ann &lt;ann@x.com&gt; wrote:<br></div><blockquote style="margin:0px 0px 0px 0.8ex">Shall we meet Thursday?</blockquote></div>"#;
        let out = fold_quoted_html(html);
        assert!(
            out.starts_with(
                r#"<div dir="ltr">Sounds good, see you then.</div><br><div><details class="kaluta-quote">"#
            ),
            "{out}"
        );
        assert!(out.contains("wrote:<br></div><blockquote"), "{out}");
        assert!(out.ends_with("</blockquote></details></div>"), "{out}");
    }

    #[test]
    fn plain_text_marks_quotes() {
        let html = "<p>Thanks, Friday works.</p><p>On 2026-09-21, Ann wrote:</p>\
                    <blockquote><p>Lunch on Friday?</p><blockquote><p>Earlier note</p></blockquote></blockquote>";
        let text = html_to_quoted_text(html);
        assert_eq!(
            text, "Thanks, Friday works.\nOn 2026-09-21, Ann wrote:\n> Lunch on Friday?\n> > Earlier note",
            "{text:?}"
        );
        // Gmail's own structure: the quote inside a div.
        let gmail =
            r#"<div>Sounds good.</div><div><div>On Tue, Ann wrote:<br></div><blockquote>Thursday?</blockquote></div>"#;
        let text = html_to_quoted_text(gmail);
        assert!(text.starts_with("Sounds good."), "{text:?}");
        assert!(text.contains("wrote:\n> Thursday?"), "{text:?}");
        assert_eq!(html_to_quoted_text("<p>No quote</p>"), "No quote");
    }

    #[test]
    fn the_quoted_history_can_be_left_out() {
        let html =
            r#"<p>Hi Ann, Friday works.</p><p>On 2026-09-21, Ann wrote:</p><blockquote><p>Friday?</p></blockquote>"#;
        assert_eq!(without_quoted_html(html).as_deref(), Some("<p>Hi Ann, Friday works.</p>"));
        assert_eq!(without_quoted_html("<p>No quote</p>"), None);
    }

    #[test]
    fn an_outlook_reply_folds_from_the_rule() {
        let html = r#"<div>Thanks, done.</div><hr style="display:inline-block"><div><b>From:</b> Ann<br><b>Sent:</b> Tuesday<br><b>To:</b> Me<br><b>Subject:</b> Plan</div><div>Earlier text</div>"#;
        let out = fold_quoted_html(html);
        assert!(out.starts_with(r#"<div>Thanks, done.</div><details class="kaluta-quote">"#), "{out}");
        assert!(out.ends_with("<div>Earlier text</div></details>"), "{out}");
    }

    #[test]
    fn inline_answers_and_pure_quotes_are_left_alone() {
        let long = "x".repeat(400);
        let inline = format!("<div>Hi</div><blockquote>Q1</blockquote><div>{long}</div><blockquote>Q2</blockquote>");
        assert_eq!(fold_quoted_html(&inline), inline);
        let only = "<blockquote>Everything is quoted</blockquote>";
        assert_eq!(fold_quoted_html(only), only);
        let none = "<div>No quote <b>here</b><br>at all</div>";
        assert_eq!(fold_quoted_html(none), none);
    }

    #[test]
    fn attributes_with_angle_brackets_survive() {
        let html = r#"<div title="a > b">Reply</div><blockquote>old</blockquote>"#;
        let out = fold_quoted_html(html);
        assert!(out.starts_with(r#"<div title="a > b">Reply</div><details"#), "{out}");
    }

    #[test]
    fn plain_text_splits_at_the_attribution() {
        let text = "Yes, Thursday works.\n\nOn Tue, Ann wrote:\n> Shall we meet?\n> \n";
        let (reply, quoted) = split_quoted_text(text);
        assert_eq!(reply, "Yes, Thursday works.\n\n");
        assert_eq!(quoted, Some("On Tue, Ann wrote:\n> Shall we meet?\n> \n"));
        let outlook = "Done.\n-----Original Message-----\nFrom: Ann\nSent: Tue\n";
        assert_eq!(split_quoted_text(outlook).0, "Done.\n");
        let inline = format!("> q1\n{}\n> q2\n", "a".repeat(400));
        assert_eq!(split_quoted_text(&inline), (inline.as_str(), None));
        assert_eq!(split_quoted_text("> all quoted\n").1, None);
    }
}
