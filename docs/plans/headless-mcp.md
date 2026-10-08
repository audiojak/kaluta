# Plan: a local MCP for outside agents, without the app open

Status: planning (2026-10-06). Nothing built. Split out of
[agent-mailboxes.md](agent-mailboxes.md); builds on its Primitive provider.

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

## Open questions

1. Is the helper the MCP binary itself (embedded and signed in the bundle)
   or a small XPC/launchd service the shim asks? The spike decides.
2. Can a stdio process an outside agent spawns be the embedded binary at
   all, given how Claude Code and Codex launch MCP servers (path in the
   app bundle, no `open`)?
3. How does a headless send pick up guide checks that today run in the
   core with the app's agents? Likely the same rules, checked
   deterministically where possible and flagged for the daily review
   otherwise.
4. Spec: §10 gains the mailbox mode; §12's "the MCP shim receives no
   secrets" is amended for the helper only.

## Steps (proposed)

1. Signing spike: the embedded helper reads a Keychain item the app wrote,
   with the app closed.
2. Outbox lock across processes; tests with two writers. *Done
   (oagc-uys.3), above.*
3. `--mailbox` mode: guide and facts tools, mail read tools, sending;
   through the core when the app runs, else headless.
4. Connect an Agent…; spec §10 and §12.
