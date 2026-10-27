//! Provider message → store message. This is where untrusted HTML is
//! sanitized, once, at sync time (spec §14.4).

use mail_domain::Body;
use mail_store::{IncomingAttachment, IncomingMessage};
use provider_api::FetchedMessage;

pub fn to_incoming(m: FetchedMessage) -> IncomingMessage {
    let (body, attachments, hidden) = match m.body {
        Some(b) => {
            let readable = mail_mime::readable(b.text, b.html.as_deref());
            let body = Body {
                text_plain: readable.text,
                has_remote_images: readable.html.as_ref().is_some_and(|s| s.has_remote_images),
                html_sanitized: readable.html.map(|s| s.html),
            };
            let attachments = b
                .attachments
                .into_iter()
                .map(|a| IncomingAttachment {
                    part_id: a.part_id,
                    provider_attachment_id: a.attachment_id,
                    filename: a.filename,
                    mime_type: a.mime_type,
                    size: a.size,
                    content_id: a.content_id,
                    is_inline: a.is_inline,
                    data: a.data,
                })
                .collect();
            (Some(body), attachments, readable.hidden)
        }
        None => (None, vec![], None),
    };
    IncomingMessage {
        id: m.id,
        thread_id: m.thread_id,
        rfc822_message_id: m.message_id_header,
        in_reply_to: m.in_reply_to,
        references: m.references,
        from: m.from,
        to: m.to,
        cc: m.cc,
        bcc: m.bcc,
        reply_to: m.reply_to,
        subject: m.subject,
        date: m.date.unwrap_or(m.internal_date),
        internal_date: m.internal_date,
        snippet: m.snippet,
        label_ids: m.label_ids,
        size_estimate: m.size_estimate,
        body,
        hidden,
        attachments,
        list: m.list,
        headers_json: None,
    }
}
