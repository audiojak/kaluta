# ADR 0011: A per-account writing guide, learned from sent mail through the user's own agent

- Status: Accepted
- Date: 2026-09-29
- Amends: ADR 0001 register, **Agents** (a new use of the user's agent:
  batch analysis) and **Storage** (migration `0010_writing_guide`)
- Builds on: ADR 0004 (per-account store), ADR 0006 (exact undo),
  ADR 0007 (read-only embedded sessions)
- Spec: §14.9; plan `docs/plans/writing-guide.md`

## Context

AI drafts in OpenAGC (writing help, agent column, routines) sounded like
an AI, not like the user. The maintainer wanted every AI-composed email to
follow a ruleset and style guide they set, built interactively from their
own past mail rather than written from scratch, and reviewable rule by
rule.

## Decision

- **Source: sent mail**, the user's own text only (quoted replies,
  forwarded originals and signatures stripped).
- **One guide per account**, in the account's store (ADR 0004), followed by
  every agent (Claude Code and Codex).
- **A fixed taxonomy of 57 categories** (A1–H4, spec §14.9), each marked
  learned or asked, checked on every batch so coverage does not depend on
  the agent. Entries are rules, guidelines or facts, with scope, evidence
  and an optional machine check. Precedence is fixed: rules over
  guidelines, narrow scope over wide, Settings › Permissions over the
  guide.
- **Processing only through the user's connected CLI agent**, in read-only
  sessions (ADR 0007). The app makes no model calls of its own and ships
  no API key; with no agent ready it says so and points to Settings ›
  Agents.
- **A background job, batches of 20, default 1,000 messages**, recorded
  batch by batch so it survives quitting; **no questions until every
  batch is processed**, so each proposal arrives once with all its
  evidence.
- **Quotes are verified**: a proposal's quotes must occur in the cited
  messages, or they are dropped; this is what makes the evidence
  trustworthy.
- **Checks only on AI drafts**, never on what the user types, run in the
  core.
- **Audience groups inferred and confirmed**, filled to five from obvious
  gaps.
- Every change is undoable (ADR 0006) and the guide keeps versions.

## Consequences

- A first run over 1,000 messages is 50 agent turns: minutes to tens of
  minutes on the user's CLI and its quota. Hence the background job,
  resumption and progress by batch.
- The user's sent mail is sent to their agent CLI in bulk. The dialog says
  so with the count before it starts; nothing leaves the Mac otherwise.
- The guide shapes prompts but cannot force an agent's prose; the core's
  checks catch what can be checked, and the rest is guidance.
- Merging between accounts is explicit and never carries evidence.

## Alternatives considered

- **A hand-written style guide only**: slow to write and misses habits
  the user does not notice in themselves.
- **One model call over all mail at once**: too large for a prompt, and
  one bad answer loses everything; batches are resumable and merge.
- **A bundled model or API key**: against the rule that all AI runs
  through the user's own agent, and a key to protect.
- **Questions after each batch**: asks about rules the next batch would
  change; rejected by the maintainer.
