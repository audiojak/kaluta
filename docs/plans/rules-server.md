# Plan: a rules server for cloud agents

Status: draft (2026-10-08), for the maintainer's decisions. Nothing built.
Bead oagc-ikr. Follows [agent-mailboxes.md](agent-mailboxes.md) and
[headless-mcp.md](headless-mcp.md).

## Why

Agents on this Mac reach an agent mailbox's writing guide and facts
through the local MCP, with or without the app open. Cloud agents cannot:
a Claude cloud routine, a ChatGPT task or an agent on another machine has
no socket to the app and no stdio process on the Mac. They send as the
mailbox with its service key (*Copy API Key*), and write without its
guide, guidelines or facts. The app sees those sends only when it syncs,
and checks them only in the daily review.

They need two things from somewhere they can reach: the mailbox's guide
and facts, and a place to check a draft and report a send.

## Decisions (maintainer, 2026-10-06)

From [agent-mailboxes.md](agent-mailboxes.md), *Resolved 2026-10-06*:

> **Cloud:** not a cloud MCP bolted onto the app, but a separate **rules
> server** in this repository (guide, guidelines and facts), possibly
> speaking MCP. The user can deploy it themselves, or use one the project
> runs. The project may charge for running trusted infrastructure, never
> for features: the hosted and self-hosted servers are the same code. Own
> plan; amends §1.1.6.

What follows from it:

- The app stays the source of truth. The server holds copies.
- The server never holds mail, and never holds a service key or an OAuth
  token: it cannot read or send as anyone.
- No feature exists only on the hosted server. No build flags differ.

## Shape

One binary, `openagc-rules` (crate `rules-server`), with SQLite. It
depends on a new pure crate holding the guide's deterministic check and
the guide and facts renderers, extracted from `openagc-core`
(`guide_render.rs`), so the server and the app answer the same way.
It never depends on `openagc-core` (UniFFI; `cargo xtask check-deps`).

### Transport

- **A. Remote MCP over Streamable HTTP.** Claude's connectors, Claude
  Code (`claude mcp add --transport http`) and the Agent SDK take a URL.
  `rmcp`, already in the workspace, has a server transport for it (check
  the version). Tools keep the local names: an agent prompt works with
  either.
- **B. Plain REST, with a thin MCP adapter.** Easy to call from scripts
  and anything without MCP. Two surfaces to keep in step.
- **C. Both, one handler set.** MCP for agents, a small REST API for the
  app's publishing and for scripts.

Recommend **C**: MCP is the agents' surface; the app needs REST to
publish anyway, and read-only `GET`s for scripts cost little.

### What it holds

- **A. A read-only published snapshot.** The app pushes the guide and
  facts; agents read. Simple, and nothing on the server can change what
  the app shows.
- **B. Two-way.** Agents also propose rules or facts; the server queues
  them; the app pulls them into *Proposed* (§14.10, §14.11) for the user.

Recommend **A plus one append-only queue** of agent reports (sent
messages and checks, below), pulled and then deleted by the app.
Proposals from cloud agents wait for a later step; they fit the same
queue.

### Tools (agents)

- `guide_rules` (`to`, `message_type`): as in mailbox mode, from the
  snapshot, with its version and when it was published.
- `facts_lookup` (`category`, `query`): only the facts shared (below).
- `check_draft` (`to`, `message_type`, `subject`, `body_markdown`):
  the guide's checks (banned and required phrases, patterns, length),
  answered as `guide_check`. Deterministic, no model calls.
- `report_send` (`message_id` or `to`, `subject`, `sent_at`, body): queues
  a report for the app (below).

No mail tools. A cloud agent reads its mailbox through the service.

### Identity and auth

- **The app** publishes with a publisher token per mailbox per server,
  made at first publish and kept in the Keychain.
- **Agents** use tokens minted in the app (*Connect a Cloud Agent…*):
  named by the user ("Weekly outreach routine"), scoped to one mailbox,
  shown once, revocable. The server stores a hash. The report names the
  token, so the activity log says which agent sent.
- **OAuth** for clients that only take a URL. The server is its own
  minimal authorization server; its consent page asks for a one-time
  connect code shown in the app, never a password. No user accounts on
  the server.
- **A secret URL** (`/m/<token>/mcp`) is the fallback for authless
  connectors. Simple, but URLs end up in logs.

Recommend bearer tokens first, then OAuth with connect codes. To check
before building: whether a claude.ai custom connector (what a cloud
routine's `mcp_connections` names, §11.1) can carry a static header. If
not, OAuth comes first, since cloud routines are the main case.

### Privacy: what leaves the Mac

- **Guide entries:** accepted rules and guidelines, with scope and
  checks. Never evidence quotes, which come from sent mail.
- **Audience groups:** needed for scoped rules, but they list people's
  addresses. Published as salted hashes; the server hashes the `to` it is
  given.
- **Facts:** a *Share with cloud agents* switch on each fact. On by
  default for the mailbox's own *Use freely* facts. Off by default for
  *Ask before using* (an unattended agent cannot ask) and for global
  facts (ADR 0012: they are the user's own). Never for *Never share*.
- **Mail:** never. Reports carry what the agent wrote, not mail it read.

The publish sheet lists exactly what goes, before the first push.

### Encryption

- **A. Plain at rest.** Simplest; the operator can read what was
  published.
- **B. Encrypted at rest, key in the agent's token.** The snapshot is
  encrypted with a key wrapped for each token; the server decrypts in
  memory for a request and never stores the key. A leaked database or
  backup shows nothing. An operator who changes the code can still read
  during requests.
- **C. End to end.** The server returns ciphertext. Not possible: the
  reader is a cloud model calling tools, and `check_draft` needs the
  plain rules.

Recommend **B** on the project-hosted server, optional when
self-hosted. Say plainly that it protects data at rest, not from a
hostile operator.

### Send checking

- **A. Check before sending.** The agent calls `check_draft`, fixes what
  breaks, sends through the service with its key, then `report_send`.
- **B. Review afterwards, as today.** The app sees the send at sync and
  the daily review compares it with the guide.
- **C. The server sends.** It would need the service key. Rejected: the
  server holds no secrets that act.

Recommend **A, with B as the net**. The app pulls reports at sync,
matches them to sent mail by Message-ID, and records them as AI
compositions (ADR 0013, agent `cloud:<token name>`), so the daily review
sees them. A send with no report is still reviewed (B).

### Hosting

- One static binary and one SQLite file. A Docker image. Plain HTTP
  behind the user's TLS proxy; the docs show Caddy.
- Project-hosted: the same image, versions and settings. What is charged
  for is running it: the machine, the domain and TLS, backups, uptime,
  abuse handling. Self-hosting is documented as fully equal.

### Sync from the app

- The app pushes a full snapshot on change (debounced), with a version
  that only goes up, and `If-Match` on the previous one. The app is the
  only writer, so there are no conflicts; a refused push re-reads the
  server's version and pushes again.
- Answers carry the version and its age. With the app closed nothing
  changes; agents see "as of" a time.
- The server keeps the last few versions, so a report can name the
  version it was checked against.

### Relation to the headless MCP and the helper

- Same tool names and answers as mailbox mode (§10.1), so instructions
  carry over. Mailbox mode reads the store live and sends; the rules
  server serves a snapshot and does neither.
- The embedded helper (oagc-uys.7, oagc-zq3.1) lets the shim send with
  the Keychain key while the app is closed. The rules server needs no
  Keychain access beyond its publisher token, read by the app. Neither
  waits on the other.
- The guide check moves into the shared pure crate first; mailbox mode
  and the in-app tools use it unchanged.

## Spec impact

- **§1.1.6** "No project-operated backend of any kind" becomes: the
  project runs nothing that sees mail; it may run a rules server, the
  same code users can run, holding only what a user publishes to it.
  **§1.2** "no cloud" is amended to match.
- **New §10.6 Rules server:** topology, tools, tokens, the snapshot,
  reports and what is shared.
- **§12:** `rules.publish_token.<server>.<account>` and, with
  encryption, `rules.snapshot_key.<account>`. Agent tokens are shown
  once and kept by the app only as ids and names.
- **§15:** assets gain published guides and facts; adversary 4 (the
  project) still cannot see mail; new: a leaked agent token (reads one
  mailbox's rules and shared facts, can report, revocable) and the
  server's operator. Controls rows for each.
- **§0** register row, and ADR 0016.

## Open questions

1. Agent mailboxes only, or the user's own accounts too? (Recommend:
   agent mailboxes first.)
2. MCP plus a small REST API (C)?
3. Snapshot plus a report queue first, proposals later?
4. Fact defaults: the mailbox's *Use freely* facts shared, *Ask before
   using* and global facts not?
5. Audience-group addresses published as salted hashes?
6. Encryption at rest with the key in the token: required when
   project-hosted, optional when self-hosted?
7. Bearer tokens first, or OAuth first if claude.ai connectors cannot
   carry a header?
8. The server never sends and never holds a service key: agreed?
9. Who runs the project-hosted server, where, and at what price (at
   cost?), and only after self-hosting works?
10. Reports deleted once the app has pulled them, and after 30 days
    regardless?
11. Docker image published to GitHub's registry from this repository's
    releases?

## Steps (proposed)

1. ADR 0016 and the spec amendments above. No code.
2. Extract the guide check and the guide and facts renderers into a pure
   crate; the core uses it. No change in behaviour.
3. `rules-server`: SQLite, publish API with a publisher token, versioned
   snapshots, `guide_rules` and `facts_lookup` over MCP with bearer
   tokens. Tests with an in-process client. Dockerfile.
4. App: *Publish to a Rules Server…* in an agent mailbox's settings: the
   URL, the list of what is shared, push on change, a status line
   ("Version 12, published 3 minutes ago").
5. App: *Connect a Cloud Agent…*: name, mint, show once, revoke; the
   `claude mcp add --transport http …` line and routine instructions.
6. `check_draft` and `report_send`; the app pulls reports at sync and
   records them (ADR 0013).
7. OAuth with connect codes (before step 5 if claude.ai connectors
   cannot carry a header).
8. Encryption at rest.
9. The project-hosted instance: operations notes, backups, the price.
