# ADR 0021: Forwarding rules the user accepts may send without asking

- Status: Accepted
- Date: 2026-11-14
- Amends: spec §0 (register), §6.2 (store), §15.3 and §15.4 (controls)
- Spec: §14.15

## Context

The user forwards receipts to a bookkeeping address by hand, one at a
time, and asked for rules: when mail with a certain subject prefix is
always forwarded to the same address, Kaluta should propose a rule, and
an accepted rule should do the forwarding.

Until now nothing in Kaluta sent mail on a mail account without the user
looking at that message first. §15.3 says so for agents (`mail.send` and
`mail.forward` are always approval-gated; the one exception is an agent
mailbox the user set to *Send freely*, §7.9), and the workflows and
Follow-ups keep to it ("Kaluta never sends a follow-up itself"). A rule
that forwards on its own is the first exception, so it needs a decision.

Two things make it different from an agent sending:

- An agent decides what to send and to whom from text it read, some of
  it hostile. A rule decides nothing: sender, prefix and recipient were
  fixed by the user, and code compares headers.
- What leaves is a message the user was sent, unchanged, to an address
  the user chose. Nothing is written.

## Decision

- **Rules are proposed from what the user does, by code.** Kaluta counts
  the user's own forwards (same sender, same subject prefix, same single
  recipient); three or more with no matching mail left unforwarded is a
  proposal in Teach Kaluta, with the forwarded conversations as
  evidence. No agent takes part in watching, proposing or applying.
- **The mode is chosen per rule, at the moment of accepting.** Accepting
  asks *Forward automatically* or *Ask each time*, with neither chosen
  ahead; the choice can be changed later in Settings. *Ask each time*
  keeps the old guarantee entirely: nothing is sent until the user takes
  the suggestion.
- **A rule always names a sender**, and applies only to mail that
  arrived after it was accepted or last changed. The recipient is one
  address, fixed in the rule.
- **Rules are out of agents' reach.** No MCP tool reads, creates or
  changes them; they are not published to a rules server; §10.3's gating
  of agent sends is unchanged.
- **Every forward is accounted for**: once per message
  (`forward_rule_log`), capped per rule per day, in Sent, and marked in
  the conversation it came from.
- **Client-side.** The core applies rules after sync. They run only
  while Kaluta is open.

## Consequences

- "Kaluta never sends without the user approving that message" becomes
  "…unless the user made a forwarding rule automatic". §15.3 and §15.4
  say so.
- A forged `From` that matches an automatic rule gets its message
  forwarded to the rule's recipient. The damage is a junk message at an
  address the user chose (a receipts inbox); it cannot redirect mail
  elsewhere or leak other mail. Accepted; checking the sender's
  authentication results is left for later, since the store does not
  keep them today.
- Mail that arrives while Kaluta is closed is forwarded late, at the
  next sync. For receipts that is fine; the spec says so.
- The naming is crowded: the writing guide has *rules*, routines have
  leave-alone *rules*, and there is a *rules server*. These are always
  called *forwarding rules*, in the UI and in the code (`forward_rules`).

## Alternatives considered

- **Gmail's server-side filters and forwarding.** Would run with the app
  closed, but creating a forwarding address needs the restricted
  `gmail.settings.sharing` scope (Kaluta asks for `gmail.modify` only,
  §0) and a confirmation by the recipient, and does nothing for accounts
  that are not Gmail.
- **Ask each time only.** Keeps the security model untouched, but a
  confirmation for every receipt is the chore the user asked to be rid
  of. It stays as the other mode.
- **Automatic only.** No way to try a rule before trusting it, and no
  fit for patterns that hold most of the time.
- **Having the triage agent suggest forwards.** Lets text in a message
  argue for a forward and its recipient: the exfiltration path §15.3
  exists to close.
- **A subject prefix alone, as first asked.** Anyone can write a
  subject; with the sender in the rule, a stranger cannot trigger one
  without forging.
