# ADR 0020: Bug reports and feature requests go by email, and are reviewed before they are public

- Status: Accepted
- Date: 2026-11-12
- Amends: spec §0 (Telemetry), §15 (no backend), §17 (diagnostics)
- Spec: §17.1; website `kaluta-website/src/lib/feedback`

## Context

People using Kaluta need a way to say "this is broken" or "I wish it did
this", with a picture, from inside the app. The project tracks work as
GitHub issues on `audiojak/kaluta`. Three things stood in the way:

- GitHub's API cannot attach a picture to an issue, so the picture has to
  be hosted somewhere and linked.
- Filing through GitHub from the app would need every user to have a
  GitHub account and to authorize an app, and a picture of a mail window
  would be public the moment it was filed.
- Kaluta has no backend and no telemetry, and says so. Anything the
  project starts to receive has to be something the user chose to send,
  and could read first.

Kaluta is a mail client: the one thing every user already has is an
account that can send mail.

## Decision

- **A report is an email the user sends.** *Help › Report a Bug…* and
  *Suggest a Feature…* write an ordinary message from the open account to
  `bug@support.kaluta.org` or `idea@support.kaluta.org`. It goes through
  the same outbox as any other mail, and it is in the user's Sent mail
  afterwards. The app contacts no Kaluta server itself.
- **The user sees everything first.** The sheet shows the title, the
  description, the details block, the log lines and the picture exactly
  as they will be sent. The picture and the log are each optional, and
  the log (warnings and errors only, scrubbed) is off until asked for.
- **The picture is of Kaluta's own window, drawn by Kaluta.** No Screen
  Recording permission. The user can cover parts of it; the covered
  pixels are replaced in the file that is sent, not hidden by a layer.
  A picture file dropped on the sheet can stand in for it. All text the
  Mac can read in the picture is painted over unless the user unticks
  that (amendment 2026-11-12).
- **The message is the interface** (§17.1): the kind is the recipient,
  the title is the Subject, and the body is the description followed by
  two delimited plain-text blocks. A report typed by hand in any mail
  program is equally valid.
- **Nothing is public until a maintainer approves it.** The addresses are
  hosted at an agent-mail service (Primitive). Its webhook calls a
  handler on kaluta.org, which emails a maintainer a preview and a signed
  review link. Approving files the issue through a GitHub App, without
  the sender's name or address, with the picture stored at a public
  address; rejecting publishes nothing.
- **No database and no second repository.** The mail service's stored
  copy of the mail is the private record. The handler keeps only a
  decision per report (and the published pictures) in blob storage.
- **Two replies, no more.** The sender is emailed the issue's link when
  it is published and once more when it is closed. The mail is deleted at
  the mail service when the issue is closed or the report is rejected.

## Consequences

- The project now receives personal data: the sender's name and address,
  what they wrote, and an unreviewed picture. The privacy page, the
  README and §15 say so; "no telemetry" still holds, because nothing is
  sent that the user did not write and send.
- A report cannot be sent from an account that cannot send (old mail,
  the demo account); the sheet says so and offers the address instead.
- The maintainer's review is the only thing between a careless picture
  and a public issue. The sheet warns, the redaction tool exists, and the
  review page shows the picture again, but the review is a person.
- The mail service and the site's host are processors of that data. The
  design does not depend on this particular mail service: any host that
  can call a webhook and return the stored message would do.
- Replies to the two notes arrive at the same addresses and are treated
  as new reports for a maintainer to read.

## Alternatives considered

- **GitHub sign-in in the app** (device flow, issues filed as the user).
  Needs a GitHub account, publishes at once, and still cannot attach the
  picture.
- **A private triage repository** holding reports until approved. A
  second place to keep personal data, for no gain over the mail already
  stored.
- **A cloud function of our own** receiving an upload from the app. A
  server the app talks to, an upload API to defend, and nothing in the
  user's Sent mail to show what left.
