# Plan: a rules server for cloud agents

Status: decided (2026-10-08); steps 1 to 5 done, the server built, the
app publishing to it and OAuth with connect codes (2026-10-09). The maintainer took every
recommendation below (*Decisions, 2026-10-08*). Step 1 is done:
[ADR 0016](../adr/0016-rules-server.md) and spec §10.6 and its
amendments are written. OAuth now comes before *Connect a Cloud Agent…*
(*Checked 2026-10-08*, below).
Bead oagc-ikr (plan, closed); the build is epic oagc-gmn7, *Rules server for cloud
agents*. Follows [agent-mailboxes.md](agent-mailboxes.md) and
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

## Decisions (maintainer, 2026-10-08)

Every recommendation above, as answered:

1. **Agent mailboxes only** for now; the user's own accounts are not
   served.
2. **MCP for agents plus a small REST API** for the app's publishing and
   for scripts, one handler set (Transport C).
3. **A read-only snapshot plus a report queue** first; proposals from
   cloud agents later, through the same queue.
4. **Fact defaults:** the mailbox's *Use freely* facts are shared; *Ask
   before using* and global facts are not; *Never share* never is. Each
   fact has its own switch.
5. **Audience-group addresses are published as salted hashes.**
6. **Encryption at rest, the key wrapped per agent token:** required on
   the project-hosted server, optional when self-hosted. The docs say it
   protects data at rest, not from a hostile operator.
7. **Bearer tokens first, then OAuth with connect codes** — unless the
   check below finds claude.ai connectors cannot carry a header, in which
   case OAuth comes first.
8. **The server never sends and never holds a service key.** Agents
   check with `check_draft`, send through the service, then
   `report_send`; the daily review stays the net.
9. **The project-hosted server comes only after self-hosting works,**
   priced at cost; who runs it and where is decided then.
10. **Reports are deleted once the app has pulled them, and after 30 days
    regardless.**
11. **A Docker image on GitHub's registry**, built from this repository's
    releases, beside the static binary.

### Still to check before building

- Whether a claude.ai custom connector (what a cloud routine's
  `mcp_connections` names, §11.1) can carry a static `Authorization`
  header. It decides the order of steps 5 and 7.

**Checked 2026-10-08 (docs only; nothing signed in to).** Yes, but not
for everyone:

- A custom connector added by URL offers *Sign in now*, *Sign in when
  needed* (both OAuth) or *No sign-in*. With *No sign-in* the user can
  add up to four **Request headers**, `authorization` among the offered
  names, sent as entered on every request (`Bearer <token>`, scheme
  included), stored encrypted and not shown again. Headers cannot be
  changed afterwards: the connector is removed and added again.
- But: "Request header authentication is in beta and available to a
  limited set of organizations." Without it the dialog has no *Request
  headers* section.
- Routines use the claude.ai connectors on the account; a server added
  with `claude mcp add` does not reach them. A routine with exactly one
  repository may instead declare the server in a committed `.mcp.json`;
  OpenAGC's routines have no repository (§11.1), and a token must not
  be committed.
- Cloud sessions run no sign-in of their own: they use the authorization
  the user granted in claude.ai.

Sources:
[Add a connector that isn't in the directory](https://claude.com/docs/connectors/custom/add-unlisted#authenticate-with-request-headers),
[Get started with custom connectors using remote MCP](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp),
[Automate work with routines](https://code.claude.com/docs/en/routines#connectors).

So for most users a cloud routine, the main case, reaches the server
only through OAuth (or the secret URL). **OAuth moves before *Connect a
Cloud Agent…*** (steps below, renumbered). Bearer tokens stay the base:
Claude Code, the Agent SDK, scripts, and claude.ai organisations with
the headers beta use them.

## Steps (proposed; reordered 2026-10-08)

1. ADR 0016 and the spec amendments above. No code. *(Done 2026-10-08.)*
2. Extract the guide check and the guide and facts renderers into a pure
   crate; the core uses it. No change in behaviour. *(Done 2026-10-08:
   crate `writing-guide`, which also holds the snapshot's JSON format,
   `schema_version` 1, and the audience-address hash.)*
3. `rules-server`: SQLite, publish API with a publisher token, versioned
   snapshots, `guide_rules` and `facts_lookup` over MCP with bearer
   tokens. Tests with an in-process client. Dockerfile. *(Done
   2026-10-09, oagc-gmn7.2: `crates/rules-server`, operators' guide
   `docs/rules-server.md`. The people entries are scoped to are now
   hashed in the snapshot too. Registration: the first wins, an optional
   registration token closes it; the publisher's API also mints, lists and
   revokes agent tokens for the app's *Connect a Cloud Agent…*.)*
4. App: *Publish to a Rules Server…* in an agent mailbox's settings: the
   URL, the list of what is shared, push on change, a status line
   ("Version 12, published 3 minutes ago"). *(Done 2026-10-09,
   oagc-gmn7.3: `openagc-core` `rules_publish.rs`, the per-fact switch
   `facts.share_with_cloud` (migration 22), Settings' *Rules server* row
   and sheet; spec §10.6 and §12 say what was built.)*
5. OAuth with connect codes: the server as its own minimal authorization
   server, consent by a one-time code from the app. Moved up from 7:
   claude.ai connectors carry a header only in a limited beta. *(Done
   2026-10-09, oagc-gmn7.4: discovery, registration, the consent page,
   tokens with rotation, grants beside agent tokens, the core's
   `rules_connect_code_mint`, `rules_agents` and `rules_agent_revoke`;
   spec §10.6 *OAuth*. Not yet tried against claude.ai itself. Client ID
   Metadata Documents, claude.ai's recommended client identity, are not
   supported; claude.ai falls back to registration.)*
6. App: *Connect a Cloud Agent…*: name, mint, show once, revoke; a
   connect code for claude.ai connectors and routines, a token for the
   rest; the `claude mcp add --transport http …` line and routine
   instructions.
7. `check_draft` and `report_send`; the app pulls reports at sync and
   records them (ADR 0013).
8. Encryption at rest.
9. The project-hosted instance: operations notes, backups, the price.
