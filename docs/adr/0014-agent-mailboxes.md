# ADR 0014: Agent mailboxes are accounts on an agent-mail service, created in the app

- Status: Accepted
- Date: 2026-10-06
- Amends: ADR 0001 register, **Gmail** (a second provider) and
  **Enforcement** (an agent mailbox may send without approval); spec §0
  **Secrets** (an API key per agent mailbox) and **Approvals**
- Builds on: ADR 0004 (one store per account), ADR 0005 (accounts without
  Gmail), ADR 0011 (writing guide), ADR 0013 (recording AI compositions)
- Spec: §7.9; plan `docs/plans/agent-mailboxes.md`

## Context

Agents need an address of their own, to sign up for services and to
correspond on the user's behalf as themselves. Services such as Primitive
and AgentMail host such mailboxes and let a program create one with a
single unauthenticated API call. The user should be able to read the
agent's mail, send as the agent, and give it a writing guide and facts,
without leaving the app.

Every account so far is the user's own Gmail, or an imported archive with
no provider. The sync engine, the provider trait and the permission engine
are all Gmail-shaped.

## Decision

- **A third account kind, `agent`,** with the service it lives on
  (`primitive` first). It is an account in every sense of §7.7: its own
  store, guide, facts, routines and undo.
- **Setup is a fixed sequence of API calls in the core,** never an agent:
  the service's sign-up returns an API key and an address; verification
  takes a code the service emails to the user. No website, and no model
  calls of the app's own (ADR 0011). The user accepts the service's terms
  in the app; the app never accepts them for the user.
- **A `MailboxService` seam beside `MailProvider`:** sign up, start and
  finish verification, read the plan and its limits. One crate per service
  (`provider-primitive`), depending only on `mail-domain`, `mail-mime` and
  `provider-api`, like `provider-gmail`.
- **The service's model is mapped onto the store's, not the other way
  round.** The store keeps Gmail's label model. A service without labels
  gets `INBOX`, `SENT` and `UNREAD` from the provider; archive, labels,
  stars, read state and trash are local only, as in an archive account.
  Nothing the user does deletes mail at the service.
- **The API key is a secret like a refresh token:** in the Keychain as
  `mailbox.api_key.<account>`, read in Swift and handed to the core through
  `SecretStore`. *Copy API Key* shows it to the user, behind a
  confirmation.
- **Sending without approval, per mailbox.** An agent mailbox has a send
  setting: *send freely, flag breaches afterwards* (default) or *ask
  before each send*. Freely, the External send tools of agents on that
  account run without approval; every send is recorded (ADR 0013) and the
  daily review compares it with the mailbox's guide. Deleting mail stays
  approval-gated, and the user's own accounts are unchanged.

## Consequences

- Sync and setup choose the provider by account kind instead of always
  building Gmail's.
- `HttpClient` reads a service's own error body as well as Google's.
- A service without labels loses local archive state if its change feed
  expires and the account resyncs (Primitive keeps changes 7 days and an
  idle cursor stays valid, so this needs a week of failed syncs).
- Service limits show through as they are: Primitive sends to one
  recipient per message and, until verified, only replies to addresses
  that wrote first. The composer and the agent tools say so rather than
  work around it.
- An agent outside the app sends with the copied key, or later through the
  headless local MCP (`docs/plans/headless-mcp.md`), which needs its own
  ADR for the Keychain helper and the outbox lock.
