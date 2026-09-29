# ADR 0007: Embedded AI runs in read-only agent sessions

- Status: Accepted
- Date: 2026-09-29
- Amends: ADR 0001 register, **Enforcement** ("permission engine inside
  the MCP tool call")
- Spec: §14.5 (composer), §14.8 (task suggestions)

## Context

Besides the agent column, the app now asks the agent for text inside other
features: the composer's writing help ("write a reply saying yes") and
task suggestions (`t`, `⇧T`). These need to read the thread, never change
mail, and they put other people's email into the prompt.

The first versions asked the agent, in the prompt, not to change anything,
and refused any *proposal* (an action waiting for approval). A review on
2026-09-29 found this was not enough: under the default policy, archive,
labels, read state and drafts run **without** a proposal. An email in a
`⇧T` batch saying "archive all of these" could have done it to up to 50
threads.

## Decision

Features that only need the agent's words start a **read-only session**
(`start_read_only_agent_session`): the core refuses every tool that is not
read-only, in the same place it enforces every other permission, whatever
the prompt says or the agent is told by an email. The session also sees
only the threads it was started with.

Prompts still say what not to do, and mail in a prompt is fenced (every
`<` becomes `‹` inside the email blocks, so no text can close or fake a
block), but neither is relied on for safety.

## Consequences

- Prompt injection through mail can at worst produce a bad suggestion,
  which the user sees and edits before anything is saved.
- Any new embedded use of the agent must choose read-only or go through
  the normal approval path; there is no third way.
- These sessions do not appear in the agent column and are closed after
  one turn.

## Alternatives considered

- **Refuse proposals only** (the first version): leaves tools that need no
  approval open.
- **A stricter default policy for all sessions**: would make the agent
  column ask about every archive; the column's behaviour is the user's
  choice in Settings › Permissions.
