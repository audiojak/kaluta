# ADR 0016: A rules server serves agent mailboxes' guides and facts to cloud agents

- Status: Accepted
- Date: 2026-10-08
- Amends: spec §1.1.6 ("no project-operated backend of any kind") and
  §1.2 ("no cloud"); ADR 0001 register, a new entry beside **Agent↔mail**
  (a remote MCP server, separate from the app); spec §15.2 adversary 4
- Builds on: ADR 0011 (writing guide), ADR 0012 (global facts), ADR 0013
  (recording AI compositions), ADR 0014 (agent mailboxes), ADR 0015
  (service accounts)
- Spec: §10.6; plan `docs/plans/rules-server.md`

## Context

Agents on the Mac reach an agent mailbox's writing guide and facts
through the local MCP (§10.1, mailbox mode), with or without the app
open. Cloud agents cannot: a Claude cloud routine, a ChatGPT task or an
agent on another machine has no socket to the app and no process on the
Mac. They send as the mailbox with its service key (*Copy API Key*) and
write without its guide or facts. The app sees those sends only at sync,
and checks them only in the daily review.

They need the mailbox's guide and facts from somewhere they can reach,
and a place to check a draft and report a send. The spec said the project
runs no backend of any kind. On 2026-10-06 the maintainer chose a
separate rules server in this repository, which users may run themselves
or use one the project runs, charging for running trusted infrastructure
and never for features. On 2026-10-08 the maintainer took every
recommendation in the plan.

## Decision

- **A separate server, `openagc-rules`** (crate `rules-server`): one
  binary and one SQLite file. The app stays the source of truth; the
  server holds copies of what the user publishes.
- **Agent mailboxes only.** The user's own accounts are not served.
- **MCP for agents plus a small REST API,** one handler set. MCP over
  Streamable HTTP is the agents' surface; REST is for the app's
  publishing and for scripts.
- **The same tool names and answers as mailbox mode:** `guide_rules`
  (`to`, `message_type`), `facts_lookup` (`category`, `query`, only the
  facts shared), `check_draft` (`to`, `message_type`, `subject`,
  `body_markdown`, answered as `guide_check`, deterministic, no model
  calls) and `report_send` (what the agent sent). No mail tools.
- **A pure shared crate** holds the guide's deterministic check and the
  guide and facts renderers, extracted from `openagc-core`, so the
  server and the app answer alike. The server never depends on
  `openagc-core` (UniFFI; `cargo xtask check-deps`).
- **A read-only snapshot plus a report queue.** The app pushes a full
  snapshot on change, debounced, with a version that only goes up and
  `If-Match` on the previous one; the app is the only writer. Answers
  carry the version and when it was published. The server keeps the last
  few versions. Agents append reports; the app pulls them at sync.
  Proposals from cloud agents come later, through the same queue.
- **Reports are deleted once the app has pulled them,** and after 30 days
  regardless. The app matches them to sent mail by Message-ID and records
  them as AI compositions (ADR 0013, agent `cloud:<token name>`).
- **The server never sends and never holds a service key,** an OAuth
  token or mail. Agents check with `check_draft`, send through the
  service with their key, then `report_send`. The daily review stays the
  net: a send with no report is still reviewed.
- **Tokens.** The app publishes with a publisher token per mailbox per
  server, made at first publish and kept in the Keychain. Agent tokens
  are minted in the app (*Connect a Cloud Agent…*), named by the user,
  scoped to one mailbox, shown once and revocable; the server stores a
  hash, and the app only the id and name. Bearer tokens first, then
  OAuth with one-time connect codes from the app, the server being its
  own minimal authorization server with no user accounts.
- **What leaves the Mac:** accepted rules and guidelines with their
  scope and checks, never evidence quotes; audience-group addresses as
  salted hashes (the server hashes the `to` it is given); facts by a
  per-fact *Share with cloud agents* switch, on by default for the
  mailbox's own *Use freely* facts, off for *Ask before using* and
  global facts, never for *Never share*. Never mail. The publish sheet
  lists exactly what goes before the first push.
- **Encryption at rest, the key wrapped per agent token:** required on the
  project-hosted server, optional when self-hosted. It protects data at
  rest, not from a hostile operator, and the docs say so.
- **Hosting:** a static binary and a Docker image on GitHub's registry,
  built from this repository's releases; plain HTTP behind the user's TLS
  proxy. The project-hosted server is the same image and settings, comes
  only after self-hosting works, and is priced at cost. No feature and no
  build flag exists only there.

## Consequences

- The project may now run something, but nothing that sees mail: the
  §15 assets gain published guides and facts, and two adversaries are
  added (a leaked agent token, the server's operator).
- A cloud agent's view is only as fresh as the last push: with the app
  closed nothing changes, and answers say "as of" a time.
- The guide check moves into a pure crate before the server is built;
  mailbox mode and the in-app tools use it unchanged.
- **Auth order (checked 2026-10-08).** A claude.ai custom connector can
  send a static `Authorization` header only through *Request headers*,
  which the docs call a beta "available to a limited set of
  organizations"; otherwise it offers OAuth or no sign-in. Cloud routines
  use those connectors. So OAuth with connect codes comes before
  *Connect a Cloud Agent…* in the plan; bearer tokens remain for Claude
  Code, the Agent SDK, scripts and organisations with the beta.
- A leaked agent token reads one mailbox's published rules and shared
  facts and can file reports until revoked; it cannot read mail or send.

## Alternatives considered

- **A cloud MCP inside the app:** needs the Mac awake and reachable from
  the internet, and puts mail one step from the network.
- **Two-way sync, agents proposing rules:** deferred; it fits the report
  queue later.
- **End-to-end encryption:** the reader is a cloud model calling tools,
  and `check_draft` needs the plain rules.
- **The server sends:** it would hold the service key, a secret that acts.
- **A secret URL only:** simple for authless connectors, but URLs end up
  in logs; kept as a fallback.
