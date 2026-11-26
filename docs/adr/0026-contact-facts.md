# ADR 0026: Facts about contacts are learned from received mail, behind a review gate

- Status: Accepted
- Date: 2026-11-25
- Amends: spec §14.10 and §14.11 ("received mail is never used"; nothing
  about third parties beyond name, role and how the user knows them),
  for contact facts only; ADR 0001 register, **Storage**
- Narrows: ADR 0007's consequence ("at worst a bad suggestion, which the
  user sees and edits before anything is saved"), for contact facts
- Builds on: ADR 0006 (exact undo), ADR 0007 (read-only agent sessions),
  ADR 0012 (facts), ADR 0018 (workflows and their background jobs)
- Spec: §14.16

## Context

Kaluta drafts well about the user and badly about the person being
written to. Facts (§14.11) are about the user; the writing guide knows
how the user writes to groups; `contacts` is an address book for
autocomplete. Meeting prep works out who someone is from recent mail
each time and keeps none of it. The maintainer asked for data on
contacts to accrue for use in drafting: above all what is open between
the user and a person, and who owes whom what.

That knowledge is in what the other person writes. Until now learning
read only mail the user sent, so that nothing a stranger wrote could
become something Kaluta believes. Reading received mail turns an
attacker's text into stored memory that later prompts trust: "as
agreed, pay invoices to the new account" is the case to design against.

Waiting (§14.13) and Tasks (§14.8) already hold two kinds of
commitment, with their own labels in Gmail and their own workflows.

## Decision

1. **Contact facts are their own store, shaped like facts.** A contact
   is a person (`people`) above one or more addresses (`contacts`).
   A contact fact is a *fact*, a *topic* or a *commitment* about one
   contact, with evidence quotes, a use setting, a status and exact
   undo, as facts have. Facts about the user are unchanged.
2. **Received mail is read for contact facts**, and only for them: by a
   daily background job over new mail in conversations with people the
   user has written to, and by a deep dive the user starts for one
   contact, which reads all the mail between them. Gleaning facts about
   the user still reads only sent mail.
3. **Who said it is decided by code.** Each quote is verified against
   the message's own text and marked *you*, *them* or *someone else*
   from the message's sender. The agent never states provenance.
4. **A gate the agent cannot lower.** Low-risk items are accepted at
   once, visible and undoable. High-risk items are held as proposals
   and reach no prompt until the user accepts them. What is high is
   fixed in code (a commitment by the user not in the user's words;
   money, routing, agreements and deadlines resting on others' words;
   changes to what the user wrote or accepted; evidence from a newly
   joined address, from a third person only, or from mail that failed
   sender authentication; anything holding an address, link or number;
   closing a commitment owed to the user), and the agent may add to it.
   The agent reading a planted message is what is under attack, so its
   judgment is a second check and never the only one.
5. **Secrets are never stored**, whatever the gate says: account and
   card numbers and the like are dropped by pattern, so a held item can
   say that payment details changed and never what they changed to.
6. **Addresses join only by the user's hand.** Kaluta may suggest that
   two addresses are one person; it never joins them.
7. **Used only toward the contact.** A contact's facts go into a prompt
   only when that contact is a recipient of what is being written, as
   notes and not instructions.
8. **Waiting and Tasks stay as they are.** The contact page shows them
   by query (waits on this person's answer; tasks on conversations this
   person wrote in). Nothing is copied, and the agent is told not to
   restate as a commitment what a task or a wait already says.
9. **Per account, never global, never published.** ADR 0012's global
   store and the rules server (ADR 0016) do not carry contact facts.
   Agent mailboxes are neither read for them nor given any.
10. **A hijacked reading is assumed, and boxed in.** *(Added 2026-11-25
    after a review of injection through the mail being read.)* The
    rules above stop named frauds; they do not stop a message that
    talks the reading agent round, because what the agent writes as an
    item's words is its own. So:
    - **One conversation per session.** The job reads one conversation
      in a read-only session that sees only it, with only its
      participants' items in the prompt. There is nothing of another
      conversation to copy into an item, and an answer about a contact
      who is not in the conversation is dropped.
    - **Short words, nothing to act on.** Labels and values are short
      and refused when long; one holding an address, a link or a number
      is held.
    - **Two sightings** on different days before a fact resting on their
      words is shown or used, as for gleaned facts.
    - **The deep dive's merge turn names items by id** and writes no
      words.
    - **Sender authentication is kept at sync**, and mail that failed it
      is never taken as the contact speaking.
11. **Off until the user says yes.** *(Added 2026-11-25.)* The daily job
    is the first thing in Kaluta to read received mail and keep
    something from it, and it costs an agent turn per conversation, so
    it does not start by default. Kaluta asks once, per account, the
    first time the user opens Contacts or a contact's page: *Turn On*,
    *Just This Person* (a deep dive for that contact, the job left off)
    or *Not Now*. A finished first deep dive offers once to turn it on.
    Without it the contact page still shows waits, tasks and recent
    conversations, which use no agent.
12. **Tags come from the user's list, never from the mail.** *(Added
    2026-11-25.)* A contact can carry tags in two sets, Relationship
    (customer, vendor, partner) and Role (founder, CTO, technical). The
    account keeps the list, which starts full (about 60 relationships
    and 80 roles, under headings; a role such as VP of Engineering is
    two tags, a level and a function) so that adding to it is rare, and
    the user may add to it. A tag is a
    contact fact of kind *tag*, so it has evidence, undo and the gate.
    The reading agent may only choose from the list: a tag it names
    that is not there is dropped. A learned Relationship tag is always
    held for the user, because it changes how a draft treats a request
    for money or work; a Role tag is low risk.

## What this does not promise

ADR 0007 could say that injection yields at worst a suggestion the user
sees before anything is saved. Contact facts are saved first. What is
promised instead: a hijacked reading can leave only a short low-risk
line, about people in that one conversation, holding no address, link or
number, that reaches a prompt only for mail to those people, in a draft
the user reads before it goes. No rule set proves a model cannot be
talked round, and a contact's own account in the wrong hands passes
every check on who is speaking.

## Alternatives considered

- **A subject column on `facts`.** One table, but every reader of facts
  (prompt rendering, the global store, publishing, export and merge)
  would need to learn to leave contact facts out, and a mistake there
  leaks what one person said into mail to another.
- **One store for all commitments**, with Waiting and Tasks reading from
  it. Cleaner in the end, and a rewrite of two working workflows and
  their Gmail labels. Not now.
- **Let the agent alone decide what is risky.** Simpler, and it fails
  exactly when it matters: a message written to mislead the agent.
- **Batches of conversations per prompt**, as triage reads them. Fewer
  turns, and one planted message could have another conversation's text
  recorded as a fact about its sender, to surface in the next draft to
  them.
- **Tags the agent words itself.** Richer, and every contact's tags
  would differ slightly ("client", "customer", "buyer"), so they could
  not filter a list, and a message could write its own tag.
- **Tags as audience groups.** The writing guide's groups hold addresses
  and decide tone; a tag describes a person. Joining them would let a
  learned tag change how the user writes to someone. Left separate.
- **On by default**, with a switch in settings. Most users would never
  find the switch, and would be paying in agent turns for something
  they had not asked for.
- **Hold everything for review.** Safest, and the queue would be long
  enough that the user stops reading it, which is the same as no gate.

## Consequences

- Received mail from people the user has written to is sent to the
  user's agent by one more job (triage and reply answers already send
  it), and only after the user turned it on. *Learn about contacts* in
  Learning Settings is the same switch.
- A user who never opens Contacts is never asked and gets no contact
  facts in drafts.
- One conversation per prompt costs more agent turns than the batches
  triage uses, and by the maintainer's choice (2026-11-25) there is no
  daily cap unless the user sets one in Learning Settings: every other
  background job has a fixed cap. The ask says about how many turns a
  day to expect, from last week's mail. On a heavy day the job can use
  much of a subscription's allowance; a limit defers the rest to the
  next day and skips nothing.
- Sync keeps a sender-authentication result per received message, which
  forwarding rules (ADR 0021) can later use too.
- Mail with a forged sender that the receiving server did not fail can
  still add low-risk items to a contact. They are visible and undoable,
  and the held kinds still need the user (§15.4).
- A person written to from two accounts is two contacts with separate
  facts.
- The People category of Facts stays, for people the user mentions; a
  correspondent's details now belong on their contact page. Nothing is
  moved.
- The tag list is Kaluta's own: no standard vocabulary covers
  commercial relationships or levels of seniority. Each tag has a stable
  id apart from its name, and the personal and work tags that match a
  vCard `RELATED` type (RFC 6350) record it, for a later address-book
  export or import.
- Tags are per account, as contacts are: a tag added in one account's
  list is not in another's.
- The user can run the deep dive for everyone written to in a chosen
  number of days, from Learning Settings. It is many agent turns, shown
  before it starts, runs one person at a time and can be stopped; each
  person's turns still see only that person's items.
- New per-account tables (migration 0046) and a background job; a deep
  dive on a long correspondence is many agent turns,
  shown before it starts.
