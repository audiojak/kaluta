# Plan: a local MCP for outside agents, without the app open

Status: built except the helper (2026-10-08, oagc-uys.10). With the app
closed a send is queued and goes out when OpenAGC next opens; sending at
once from the shim waits on the helper (oagc-uys.7). Split out of
[agent-mailboxes.md](agent-mailboxes.md); builds on its Primitive provider.
Spec §10.1 (*Mailbox mode*) and §12 hold the details.

## Why

Agents that run outside this app (Claude Code, Codex, scripts) should use
an agent mailbox's writing guide and facts, and send as the mailbox,
whether or not OpenAGC is open. Until this lands, such agents use *Copy
API Key* and call the service directly; the app sees their mail when it
syncs.

## Decisions (maintainer, 2026-10-06)

- **Outside agents use the local MCP by default.** The key stays in the
  Keychain, and every send is recorded and checked against the guide.
  *Copy API Key* (in agent-mailboxes) stays for agents that must call the
  service themselves.
- **The MCP works headless:** outside agents can read the guide and facts,
  and send, without the app open.
- **Keychain:** a helper embedded in the app bundle, signed with the same
  team and Keychain access group, reads the key.
- **Writers:** the outbox takes a cross-process lock, so the app and a
  headless send never send the same message twice.

## Shape

Today `openagc-mcp` is a stateless shim for sessions the app starts; it
talks to the running core over a socket and gets no secrets (§10.1, §12).
Outside agents need a second mode:

- `openagc-mcp --mailbox <address>`: stdio. Tools:
  - Read-only: the mailbox's guide (`guide_rules`), its facts
    (`facts_lookup`, as today), and its mail (`mail_search`,
    `mail_get_thread`).
  - `mail_send` / `mail_reply`: sends through the service, records the
    message as AI-written (ADR 0013), and checks it against the guide. The
    mailbox's autonomy setting (agent-mailboxes) applies.
- When the app is running, the shim goes through the core as now. When it
  isn't, it opens the account's stores read-only, and for a send it takes
  the outbox lock, queues, and sends through the helper.
- **Connect an Agent…** in the mailbox's settings writes the MCP entry into
  Claude Code's or Codex's config (or shows the line to paste), scoped to
  that mailbox.

## Before building: needs the maintainer

Debug builds sign with the Apple Development identity but have no
Keychain access-group entitlement and no provisioning profile, so secrets
fall back to the login keychain (`KeychainSecretStore.swift`). A separate
helper reading those items would raise an *Allow access?* prompt.
`keychain-access-groups` needs a provisioning profile on macOS: register
an App ID for team Y5W2BTVS33 and let Xcode fetch the profile. Only then
can the spike run, against a scratch Keychain service.

## The outbox lock (built, oagc-uys.3)

Step 2 is done; spec §7.4, *outbox claims*. There is no whole-outbox file
lock: each op is claimed in the store, in the `BEGIN IMMEDIATE`
transaction that picks it, only while no other op is in flight, with the
drainer's id and a 60 s lease it renews during the call. A drainer is
known to be alive by an `flock` it holds on `outbox-claims/<id>.lock`
beside the store; a dead one's ops come back at once, a hung one's when
its lease runs out. A send that comes back, or that was tried before, is
looked for at the provider (`MailProvider::already_sent`) before it is
sent again. What the `--mailbox` mode uses:

- **Queue a send** (app closed, the interim): open the account's store
  with `mail_store::Db::open` (it migrates and takes the write lock only
  per transaction), save the draft (`mail_store::drafts::save`) and call
  `mail_sync::send_draft(&db, draft_id, from, true, 0)`: one write
  transaction, safe beside a running app. No lock is taken to enqueue;
  SQLite serializes the writes.
- **Drain from this process** (once the helper can read the key): build
  a `mail_sync::SyncEngine::new(provider, db, observer)` for the account
  (it registers its claimant) and call `drain_outbox()` again whenever
  `next_outbox_retry()` says (within 2 s while the app has an op in
  flight; `DrainReport::busy`). Do not run sync, backfill or anything else
  that releases or rewrites outbox rows; never touch `in_flight` rows
  directly.
- With the app running, the shim goes through the core as now and does
  neither.

## Built: mailbox mode (oagc-uys.10)

- `openagc-mcp --mailbox <address> [--data-dir <dir>]` serves
  `guide_rules`, `facts_lookup`, `mail_search`, `mail_get_thread`,
  `mail_send` and `mail_reply` (`docs/mcp.md`). Agent mailboxes only: the
  user's own accounts are refused at start and by the core.
- **App running:** the app binds its agent socket at launch and writes the
  path to `<data dir>/run/mcp-socket`; the shim's hello names the mailbox
  and the core opens an outside session (`outside-<client>-<id>`) through
  which every call runs the in-app tools: permission engine, guide check,
  ADR 0013 record, *When Agents Send* (approvals in the open window's
  agent panel, saying who asks), activity log.
- **App closed:** the shim runs `Core::headless` in its own process: no
  secrets, no sync, stores opened by `Db::open_existing` (no create, no
  migration, exact schema or a refusal in words; read-only connections
  until a send writes). The same code runs the tools; reads are not
  logged (read-only store), sends are.
- **Connect an Agent…** (an agent's row in Settings › Accounts): writes
  Claude Code's user-scope `mcpServers` entry in `~/.claude.json` or
  Codex's `[mcp_servers.<name>]` in `~/.codex/config.toml`, after showing
  the entry and the file and backing it up; or shows the command to paste.
  Code: `crates/openagc-core/src/outside.rs`, `agent_connect.rs`,
  `crates/openagc-mcp/src/main.rs`.

## The interim: queued sends

Until the helper exists, a send with the app closed is checked against
the guide, recorded, saved as a draft and queued with
`mail_sync::send_draft(&db, id, from, true, 0)` under the outbox claims;
the tool answers "Queued. It goes out when OpenAGC next opens." The shim
never reads the Keychain and never drains the outbox. A mailbox set to
*Ask before each send* refuses sends while the app is closed: approvals
are parked in the app's memory (§10.4) and cannot be stored for the app
to show later. Tests: `crates/openagc-core/src/outside/tests.rs`,
`crates/openagc-mcp/tests/mailbox.rs` (both modes, each tool, queued sends
sent exactly once by the "app"'s drain against fake agent mail).

## What the helper adds later

- The shim (or a helper it asks) reads the service account's key from the
  Keychain with the app closed, builds the agent's provider and drains
  what it queued: `SyncEngine::new(provider, db, observer)` and
  `drain_outbox()`, re-called when `next_outbox_retry()` says, as in *The
  outbox lock* above. The queued answer becomes "sent".
- §12's exception for the helper only; the rest of the shim stays
  secret-free.
- Possibly: approvals for *Ask before each send* stored in the account's
  store, so the app can show them when it opens, instead of refusing.

## Open questions

1. Is the helper the MCP binary itself (embedded and signed in the bundle)
   or a small XPC/launchd service the shim asks? The spike decides.
2. Can a stdio process an outside agent spawns be the embedded binary at
   all, given how Claude Code and Codex launch MCP servers (path in the
   app bundle, no `open`)?
3. *Answered (oagc-uys.10):* a headless send runs the core's own guide
   check (banned and required phrases, length: all deterministic) through
   the same tools, and is recorded for the daily review (ADR 0013).
4. *Answered:* §10.1 gains mailbox mode; §12 is amended: the shim holds no
   secrets, and only the future helper will.

## Steps (proposed)

1. Signing spike: the embedded helper reads a Keychain item the app wrote,
   with the app closed.
2. Outbox lock across processes; tests with two writers. *Done
   (oagc-uys.3), above.*
3. `--mailbox` mode: guide and facts tools, mail read tools, sending;
   through the core when the app runs, else headless. *Done (oagc-uys.10),
   sends queued until the helper.*
4. Connect an Agent…; spec §10 and §12. *Done (oagc-uys.10).*
