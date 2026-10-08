# ADR 0015: Agent mailboxes belong to service accounts

- Status: Accepted
- Date: 2026-10-08
- Amends: ADR 0014 (an API key per agent mailbox, and one account at the
  service per mailbox); spec §0 **Secrets** (one key per service account)
- Builds on: ADR 0004 (one store per account), ADR 0014 (agent mailboxes)
- Spec: §7.9, §12; plan `docs/plans/overnight-2026-10-08.md` (feature 1)

## Context

ADR 0014 made each agent mailbox its own account at the service, with its
own API key. Neither service allows that for more than one mailbox:

- **Primitive** refuses to verify a second account with an email that
  already verified one (`email_in_use`, found 2026-10-07). With one
  Primitive account per mailbox, only the first mailbox could ever be
  verified with the user's email. Its managed subdomain receives at any
  local part, and it sends from any verified domain on the account.
- **AgentMail** keeps one organisation per human email, and signing up
  again with that email *rotates* the organisation's key, which would
  break every mailbox already holding the old one. More mailboxes are
  more inboxes in the same organisation (`POST /v0/inboxes`), with the
  same key.

So the key, the verified email, the plan and the own domains belong to
something above the mailbox, which both services have and the app did
not.

## Decision

- **A service account** is what the service calls an organisation
  (AgentMail) or an account (Primitive). It holds the API key (one
  Keychain item, `mailbox.api_key.<service account>`), the verified human
  email, the plan and its limits, and the user's own domains. It is kept
  in the app's data directory (`services/<id>/service.json`) beside the
  accounts.
- **Each agent stays an account** as ADR 0014 and ADR 0004 have it: its
  own store, writing guide, facts, routines, undo and send setting.
  `agent.json` names its service account and, on AgentMail, its inbox.
- **Adding an agent to a service account** needs no sign-up, no terms and
  no code: the account already agreed and verified. On Primitive it makes
  no API call at all: the agent is a local part (`writer@…`) on the
  service account's managed subdomain or one of its verified domains,
  unique within the service account. On AgentMail it creates an inbox in
  the organisation.
- **Existing mailboxes become service accounts of one agent,** whose id
  is the agent's account id: the Keychain item keeps its name and nothing
  is re-keyed or signed up again.
- **Removing an agent** removes its account on the Mac; the service
  account and its key go with the last of its agents. Nothing is deleted
  at the service.
- **Display:** agents stay separate accounts, with no combined inbox. The
  account switcher groups them under their service account ("AgentMail ·
  you@example.com"). A service-account settings pane holds what is shared:
  verification, plan and limits, *Copy API Key*, *Rotate Key* and
  domains; an agent's settings keep its name, address and send setting.
  On AgentMail, once verified, *Copy API Key* offers a key scoped to the
  one inbox, and says the organisation's key reaches every agent in it.

## Consequences

- The FFI takes a service account id for the plan, verification, the key
  and domains; the per-agent calls resolve the agent's service account
  until the app no longer uses them.
- Several agents read one Primitive account's mail: each agent's provider
  keeps the mail addressed to it and the mail it sent, and mail to a
  local part no agent has goes to the service account's first agent.
- A verification, a key rotation or a new domain reaches every agent of
  the service account at once.
- AgentMail asks for the human email at sign-up, since without it the
  inbox only receives and a lost key cannot be recovered.
