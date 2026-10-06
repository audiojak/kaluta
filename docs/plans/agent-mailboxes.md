# Plan: agent mailboxes (Primitive first, AgentMail later)

Status: built overnight 2026-10-06 on branch `overnight-6` (oagc-uys.1,
.2, .4, .5, .8, .9); AgentMailbox (.6) left. See the end for what is left
to check by hand.

## Why

Agents need an address of their own: to sign up for services and to
correspond on the user's behalf as themselves. The user should be able to
read that mailbox, send as the agent, and give the agent a writing guide
and facts, exactly as they do for their own account. Agents that send
without this app (Claude Code, Codex, scripts) reach the guide and facts
through the local MCP ([headless-mcp.md](headless-mcp.md)). Later, a
rules server lets cloud agents do the same.

## Decisions (maintainer, 2026-10-06)

- **Purpose:** the mailbox is the agent's identity. The user can read it
  and send as the agent, for observability and flexibility, and to give it
  rules, guidelines and facts.
- **Setup stays in the app.** No website. The service's own sign-up API
  creates the account; the only thing from outside is a 6-digit code.
- **Verification:** the code is entered in the app. When the user's own
  account is open in OpenAGC, the app finds the verification message and
  offers to fill the code; otherwise the user types it.
- **No AI in setup.** It is a fixed sequence of API calls; an agent would
  make it slower, less predictable and untestable, and the app makes no
  model calls of its own (ADR 0011).
- **Primitive first**, then AgentMail, then others behind the same seam.
- **Outside agents:** both routes. The local MCP, which works without the
  app open, is its own plan: [headless-mcp.md](headless-mcp.md). Here, the
  mailbox's settings offer *Copy API Key* for agents that call the service
  themselves; the app sees their mail when it syncs.

### Resolved 2026-10-06

- **Autonomy:** a per-mailbox setting, defaulting to *send freely, flag
  breaches afterwards*. The user sees every send in the mailbox, and the
  daily review compares them with the guide. *Ask before each send* is the
  other choice. The spec's non-goal (§1.2) and approval rule (§10.2) are
  amended for agent mailboxes only.
- **Keychain helper and outbox lock:** moved with the headless MCP to
  [headless-mcp.md](headless-mcp.md).
- **Cloud:** not a cloud MCP bolted onto the app, but a separate **rules
  server** in this repository (guide, guidelines and facts), possibly
  speaking MCP. The user can deploy it themselves, or use one the project
  runs. The project may charge for running trusted infrastructure, never
  for features: the hosted and self-hosted servers are the same code. Own
  plan; amends §1.1.6.
- **Vendor terms:** not a concern. The app only makes it easier for users
  to create their own accounts for their agents.
- **Own domains:** users can put agent mailboxes on their own domain with
  Primitive (below).

## The services (researched 2026-10-06; check the OpenAPI specs before building)

| | Primitive | AgentMail |
|---|---|---|
| Sign up by API | `POST /v1/agent/accounts {terms_accepted, device_name}`, no auth, returns a `prim_` key and an address on `*.primitive.email` | `POST /v0/agent/sign-up {username, human_email}`, no auth, returns key and inbox |
| Before verifying | reply-only to known addresses, 10/h, 50/day | sends only to the human |
| Verify | `/v1/agent/claim/start` + `/claim/verify` (email code) | 6-digit code to `human_email` |
| Read | REST: `GET /emails`, `/emails/{id}/raw` (.eml), `/threads`, `/sent-emails` | REST, or IMAP (IDLE) with the key as password |
| Send | `POST /v1/send-mail`, `/emails/{id}/reply`, `Idempotency-Key` | REST, or SMTP 465/587 |
| Live updates without a public URL | long-poll `GET /changes` | WebSocket, IMAP IDLE |
| Drafts / labels | none found | yes |
| Addresses per account | one managed subdomain; more via custom domains | many inboxes per organisation |

Sources: primitive.dev `llms.txt`, `auth.md`, `openapi.json`, `pricing.md`;
docs.agentmail.to `agent-onboarding.md`, `imap-smtp.md`, `websockets.md`.

## Shape

### Core

- `AccountKind::AgentMailbox { service }` beside `Gmail` and `Archive`
  (`registry.rs:47`). `start_sync` and `start_all_sync` choose the
  provider by kind rather than always building Gmail's (`account.rs:446,
  633`).
- A `MailboxService` trait, one per service, separate from `MailProvider`:
  `sign_up`, `start_verify`, `verify`, `rotate_key`, `terms_url`. Swift
  drives it through UniFFI functions: `begin_agent_mailbox(service, name)`,
  `verify_agent_mailbox(id, code)`.
- A new crate `provider-primitive` (deps: `mail-domain`, `mail-mime`,
  `provider-api`; added to `xtask` check-deps):
  - `HttpClient` with `StaticToken`. The error classification is
    Google-shaped (`http.rs:177`) and needs a per-provider hook.
  - Messages come in as raw `.eml` and go through the existing MIME path.
  - No labels upstream. INBOX, Sent and Trash are synthesized; archive and
    user labels are local-only. Delete maps to `DELETE /emails/{id}` behind
    the usual approval.
  - `/changes` drives `changes_since`; the long-poll feeds the push loop
    (`BackfillSource::watch`).
  - No drafts upstream: drafts stay local until sent.
- Secrets: `mailbox.api_key.<account>` in the Keychain (a new key name,
  §12). `remove_account` deletes it.

### App

- Onboarding and *Add Account…* gain **Create an Agent Mailbox…**:
  1. Choose the service (Primitive only at first) and a name for the agent.
  2. The service's terms are shown with a link and an explicit *Agree and
     Create* (Primitive requires `terms_accepted`; we never accept for the
     user).
  3. The account is created and opens at once, with a banner showing the
     unverified limits and *Verify with your email*.
  4. Verify: the user's address is prefilled from their open account. When
     the code arrives there, *Fill Code from <address>*; else a code field.
- The mailbox is an ordinary account in the sidebar, with its own writing
  guide, facts, learning and undo, marked as an agent's.
- Account settings: service, address, verification state, *Copy API Key*
  (behind a confirmation that says what a holder can do), *Rotate Key*,
  and later *Connect an Agent…* (headless-mcp).

### Own domains (Primitive)

Researched 2026-10-06 (docs.primitive.dev `domains`, `endpoints`,
`sending`; `openapi.json`). There is no registrar or DNS API, so the user
adds DNS records at their own DNS host. Everything else stays in the app.

- **Add Domain…** in the mailbox's settings. It suggests a **subdomain**
  (`agents.example.com`). An MX record on the apex would take over the
  domain's existing mail; Primitive refuses that with `mx_conflict`, and
  the app then offers the subdomain.
- `POST /v1/domains` returns the records: MX, DKIM, SPF, DMARC, TLS-RPT,
  and the optional `_agents` opt-in. The app shows them in a table with
  *Copy* on each, and *Save Zone File* (`GET /domains/{id}/zone-file`) for
  hosts that import BIND files.
- Verification is a boolean from `POST /domains/{id}/verify`, with a
  result for each check. The app retries while the sheet is open and
  then in the background with backoff, and shows each record turning
  green. A notification says when the domain is ready.
- Once the domain is verified, the agent's address can be
  `name@agents.example.com`. Choosing it creates an exact recipient route,
  and mail sends from it, since sending works from any verified domain
  with an active DKIM key. *(Built differently: each agent mailbox is its
  own Primitive account, and Primitive lists everything sent to an
  account's domains as one inbox, so a domain belongs to one mailbox; no
  routes are created. Routes are for webhooks and functions.)*
- `conflict` (another organisation has claimed the domain) is shown as
  such, with nothing to retry.
- **Later:** Domain Connect or a Cloudflare token could write the records
  automatically, for hosts that support it. Checking in the background
  with a notification when the domain is ready (the sheet checks while it
  is open, and picks up where it was when reopened).

Unclear in the docs; check against a fake first, then with the maintainer's
own account by hand:
- Can the email-free agent plan add domains at all? If not, *Add Domain…*
  asks the user to verify first.
- Does mail reach `/emails` with only a route, or does a route need an
  endpoint or function?
- Is the "ownership TXT" a separate record?
- How many domains does each plan allow? `GET /v1/account` reports the
  limits.

## Open questions

1. **Spec scope.** §7.7 puts non-Gmail accounts out of scope. A new §7.9
   *Agent mailboxes* and an ADR (provider seam, secrets) come first.
2. **Tests.** Every test uses wiremock fakes of each service. Nothing in
   automation calls a real sign-up endpoint: each call creates a real
   account.

## Steps (proposed)

1. ADR and spec §7.9 (agent mailboxes, the autonomy setting, own domains);
   amend §1.2 and §10.2 for agent mailboxes.
2. Core: `AccountKind::AgentMailbox`, provider choice by kind, the
   `MailboxService` trait, the secret key name.
3. `provider-primitive`: sign up, verify, read, long-poll changes, send,
   reply. Wiremock fake and tests.
4. App: Create an Agent Mailbox (terms, create, verify, code auto-fill),
   the agent marker, account settings.
5. The autonomy setting (for agents the app starts; the headless MCP
   uses it too), and the daily review of every send from the mailbox,
   including those made with a copied key, against its guide.
6. Own domains with Primitive: Add Domain…, the records table, the
   verify loop, and addresses on the domain.
7. AgentMail: likely a generic IMAP/SMTP provider plus its sign-up, which
   also opens the door to Fastmail and generic IMAP.

Separate plans: [headless-mcp.md](headless-mcp.md); the rules server
(guide, guidelines and facts for cloud agents; self-hosted or
project-hosted).

## AgentMail: findings before building (2026-10-06, overnight)

Read from docs.agentmail.to (`openapi.json`, `agent-onboarding.md`,
`messages.md`, `labels.md`); not built, because two things need the
maintainer:

- **One organisation per human email, and signing up again rotates its
  key.** `POST /v0/agent/sign-up` with the same `human_email` returns the
  same organisation with a *new* API key, so the first mailbox's stored key
  would stop working. A second agent mailbox must be a new inbox in the
  same organisation (`POST /v0/inboxes`, free tier 3 inboxes), sharing one
  key. That changes the per-mailbox key model (§7.9): the key belongs to
  the organisation; each mailbox stores its `inbox_id`. Decision needed:
  share the key across agent mailboxes on AgentMail (one Keychain item per
  organisation), or one organisation per mailbox with a different email
  each (not practical).
- **The email comes first.** Without `human_email` the inbox is
  receive-only and a lost key cannot be recovered; with it, the code is
  sent at sign-up. So the create sheet asks for the user's email up front
  for AgentMail (and the service is chosen first), unlike Primitive.
- Unverified accounts may send only to the human (403
  `message_rejected`); verify is `POST /v0/agent/verify {otp_code}`; up to
  10 attempts per code, codes last 24 hours.

What maps well: a REST provider like Primitive's, but richer. Messages
have string labels (`unread` and the user's own; `received`/`sent`
system labels to confirm against a real account), `PATCH` adds and removes
labels, so archive, labels and read state can sync both ways; raw MIME is
`GET .../messages/{id}/raw`; sends take `to`, `cc` and `bcc` (no
one-recipient limit); drafts exist server-side. Changes: `GET
/v0/inboxes/{id}/events` lists `label.added`/`label.removed` events
(paged); whether arrival shows there or needs a listing by time is to be
checked. WebSockets push new mail without a public URL.

The plan's earlier idea of a generic IMAP/SMTP provider for AgentMail is
not needed: its REST API covers more (labels, drafts) than its IMAP does
(no drafts). A generic IMAP/SMTP provider stays worth building for
Fastmail and other IMAP accounts, as its own work.

## Left to check by hand (needs the maintainer's own Primitive account)

- Whether the email-free agent plan can add a domain at all.
- Whether `/emails/{id}/raw` answers with an API key (the docs say a
  "valid session"; the provider falls back to the record's text and
  HTML).
- The code email's sender domain (assumed `primitive.dev` or a subdomain).
- That `/send-mail` takes a display name in `from` (`"Scout" <addr>`).
