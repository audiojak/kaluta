# OpenAGC — Technical Specification

**OpenAGC** — Open Agent Gmail Client. An open-source, local-first, native macOS
email client built for personal AI agents.

| | |
|---|---|
| Status | Draft v1 — 2026-09-23 |
| Supersedes | *Open-Source Agentic Email Client — Product & Architecture Specification* (the original product spec, kept in the repo root for provenance) |
| Audience | Contributors and coding agents implementing the MVP |
| License | MIT |

This document turns the product spec into concrete technical decisions. Where
the product spec said "evaluate during planning", this document decides. Each
decision records *why*, so a future contributor can revisit it with the same
information. Decisions marked **Verified** were checked against primary sources
on 2026-09-23; sources are listed in Appendix A.

---

## 0. Decision Register

The short version. Everything below elaborates on these.

| Area | Decision |
|---|---|
| Platform | macOS 26 Tahoe or later, Apple Silicon only for MVP |
| UI | Swift 6.x, SwiftUI shell, AppKit where speed or fidelity demands it |
| Core | Rust (stable, edition 2024), one Cargo workspace |
| Swift↔Rust | UniFFI 0.32+, proc-macro mode, in-process static library built by an Xcode pre-build phase |
| Async runtime | tokio multi-thread runtime owned by the core, never UniFFI's ambient runtime |
| Storage | SQLite via `rusqlite` (bundled, FTS5), single-writer thread, WAL mode |
| Search | SQLite FTS5 external-content tables: `unicode61` for text, `trigram` for addresses |
| MIME | `mail-parser` (read), `mail-builder` (write) |
| HTML email | Sanitized in Rust with `ammonia` at sync time, cached; rendered in a locked-down `WKWebView` |
| Gmail | Hand-written `reqwest` client over the REST API; `history.list` polling; no push |
| Agent mailboxes | Accounts on an agent-mail service (Primitive first, AgentMail second), created in the app through its sign-up API; several agents share one service account and its key; REST provider (§7.9, ADR 0014, ADR 0015) |
| OAuth | Desktop-app flow with PKCE and loopback redirect; shipped client ID with bring-your-own override |
| Scope | `gmail.modify` only (plus `userinfo.email`) |
| Secrets | macOS Keychain, written and read from Swift; Rust receives tokens (and agent-mail service accounts' API keys, one per service account) through a foreign trait |
| Agents | Claude Code via `claude -p` stream-json subprocess; Codex via `codex app-server` JSON-RPC subprocess |
| Agent↔mail | OpenAGC's own MCP server (`rmcp`, stdio), spawned per agent session |
| Rules server | *(Amendment 2026-10-08, ADR 0016; decided, not built.)* Cloud agents read an agent mailbox's published guide and shared facts, check drafts and report sends through `openagc-rules`, a separate server in this repository (MCP over HTTP plus a small REST API, SQLite), self-hosted or run by the project; it never holds mail or a key that sends (§10.6) |
| Approvals | Enforced inside the Rust permission engine, inside the MCP tool call; agent-native permission systems are not relied on |
| Routines | Structured routine model → generated prompt; runs locally (OpenAGC agent stack) or as a Claude cloud routine created/updated/run through the user's own `claude` CLI (`RemoteTrigger`, verified), with paste hand-off as fallback; ChatGPT by hand-off only; OpenAGC never holds claude.ai/ChatGPT credentials |
| Composer | Rich text (`NSTextView`), sends `multipart/alternative` HTML + plain text |
| Distribution | Direct download, Developer ID, notarized, Sparkle 2 auto-update; **not** sandboxed, **not** App Store |
| Telemetry | None in MVP |
| Issue tracking | beads (`bd`) in the repo |

---

## 1. Goals and Non-Goals

### 1.1 Goals (MVP)

1. Connect one Gmail account with OAuth; keep the mailbox synced locally.
2. A native inbox, thread list, message viewer, composer, search, labels,
   archive, read/unread, attachments — fast enough that the network is never
   felt.
3. Detect installed Codex and Claude Code, let the user pick a default, send
   prompts, stream responses, cancel.
4. Expose the mailbox to agents through a minimal, capability-controlled MCP
   tool set, with sending and destructive actions gated on explicit approval.
5. Ship one editable **routine** — scheduled sorting of automated mail
   into cadence labels — runnable locally or handed off to the user's
   Claude/ChatGPT cloud (§11).
6. No project-operated backend of any kind.
   *(Amended 2026-10-08, ADR 0016: the project runs nothing that sees
   mail. It may run a rules server (§10.6), the same code users can run
   themselves, holding only what a user publishes to it.)*

### 1.2 Non-Goals (MVP)

Everything in the product spec's §27: no cloud, no mobile, no Windows/Linux,
no calendar, no autonomous background sending, no
embeddings, no shell access for agents, no App Store build.
*(Amended 2026-10-06: an agent mailbox (§7.9) may let its agents send
without approval; the user's own accounts never do.)*
*(Amended 2026-10-08, ADR 0016: "no cloud" means no mail in the cloud.
An agent mailbox's guide and chosen facts may be published to a rules
server for cloud agents (§10.6); mail never is.)*

### 1.3 The performance goal

"Lightning fast" is a requirement, not an aspiration. Concrete targets on an
M1 MacBook Air with a 100,000-message mailbox:

| Interaction | Target |
|---|---|
| Cold launch to interactive inbox | < 400 ms |
| Warm launch | < 150 ms |
| Scroll thread list | 120 fps on ProMotion, zero dropped frames at 60 fps |
| Select thread → body visible | < 50 ms (cached) |
| Keystroke → search results | < 30 ms |
| Archive / read / label | UI updates in < 16 ms (optimistic) |
| Compose window open | < 100 ms |

The architectural consequences of these numbers are in §13.

---

## 2. System Architecture

```text
┌───────────────────────────────────────────────────────────────────┐
│  OpenAGC.app (Swift 6, SwiftUI + AppKit)                          │
│                                                                   │
│  Views ─── Stores (@Observable) ─── CoreClient (Swift façade)     │
│                                          │                        │
│                                     UniFFI (static lib)           │
│                                          │                        │
│  ┌───────────────────────────────────────▼─────────────────────┐  │
│  │  openagc-core (Rust, in-process)                            │  │
│  │                                                             │  │
│  │  MailEngine   SyncEngine   Store(SQLite)   Search(FTS5)     │  │
│  │  GmailClient  Outbox       Sanitizer       MimeCodec        │  │
│  │  AgentManager PermissionEngine  EventBus   Settings         │  │
│  └───────┬──────────────────────┬──────────────────────────────┘  │
│          │ HTTPS                │ spawn + stdio                    │
└──────────┼──────────────────────┼─────────────────────────────────┘
           ▼                      ▼
        Gmail API      ┌──────────────────────┐
                       │ claude / codex CLI   │◄──stdio MCP──┐
                       └──────────────────────┘              │
                                            ┌────────────────┴──────┐
                                            │ openagc-mcp (Rust bin) │
                                            │ talks to core over a   │
                                            │ Unix socket            │
                                            └────────────────────────┘
```

Three processes are involved when an agent runs:

1. **OpenAGC.app** — UI plus the in-process Rust core. Owns the database, the
   Gmail session, the permission engine, and all state.
2. **The agent CLI** (`claude` or `codex`) — spawned by the core, owns its own
   authentication. OpenAGC never reads its credential files.
3. **`openagc-mcp`** — a small Rust binary bundled inside the app, spawned by
   the agent CLI as an MCP stdio server. It holds no state; every tool call
   is forwarded over a Unix domain socket to the core in the app process,
   where the permission engine decides and the store executes.

Why a separate MCP binary rather than having the CLI connect to the app
directly: both agent CLIs launch MCP servers as child processes over stdio.
A bundled shim is the only transport both support without configuration
files, and the socket hop keeps a single authoritative core. The same binary
later becomes the standalone MCP server the product spec envisions (§28),
running the core headless when the app is not open.

---

## 3. Repository Layout

```text
OpenAGC/
├── README.md
├── LICENSE                        MIT
├── CONTRIBUTING.md
├── docs/
│   ├── SPECIFICATION.md           this document
│   ├── architecture.md            narrative + diagrams, kept short
│   ├── security.md                threat model (§15) in contributor form
│   ├── mcp.md                     tool reference, generated from schemas
│   └── adr/                       Architecture Decision Records, NNNN-title.md
├── .beads/                        bd issue database
├── Cargo.toml                     workspace
├── rust-toolchain.toml            pinned stable
├── crates/
│   ├── openagc-core/              façade crate: UniFFI exports, runtime, event bus
│   ├── mail-domain/               plain types: Account, Thread, Message, Label, Draft…
│   ├── mail-store/                SQLite schema, migrations, queries, FTS
│   ├── mail-sync/                 SyncEngine, Outbox, backfill scheduler
│   ├── mail-mime/                 parse/build wrappers, sanitizer, text extraction
│   ├── provider-api/              `MailProvider` trait + shared HTTP/OAuth utilities
│   ├── provider-gmail/            Gmail REST client, OAuth desktop flow
│   ├── agent-api/                 `AgentProvider` trait, session + event types
│   ├── agent-claude/              Claude Code adapter
│   ├── agent-codex/               Codex app-server adapter
│   ├── agent-mcp/                 MCP tool definitions + handlers (rmcp)
│   ├── permissions/               capability model, policy, approval queue
│   ├── writing-guide/             guide check, guide/facts renderers, snapshot (pure)
│   └── openagc-mcp/               the stdio shim binary
├── macos/
│   ├── OpenAGC.xcodeproj          generated by XcodeGen from project.yml
│   ├── project.yml
│   ├── OpenAGC/
│   │   ├── App/                   @main, AppDelegate, menus, windows
│   │   ├── Core/                  CoreClient, event bridge, Keychain, OAuth browser
│   │   ├── Features/
│   │   │   ├── Sidebar/
│   │   │   ├── ThreadList/        AppKit-backed
│   │   │   ├── MessageView/       WKWebView host
│   │   │   ├── Composer/
│   │   │   ├── Search/
│   │   │   ├── Agent/             prompt bar, activity, approvals
│   │   │   └── Settings/
│   │   ├── Components/            shared SwiftUI views
│   │   └── Resources/
│   ├── OpenAGCTests/
│   ├── OpenAGCUITests/
│   └── (the OpenAGCCore target builds the Rust core and compiles its bindings; see §4.1)
├── scripts/
│   ├── build-core.sh              cargo build → uniffi-bindgen-swift → xcframework
│   ├── notarize.sh
│   └── make-appcast.sh
└── .github/workflows/
    ├── ci.yml                     cargo test, swift build, swift test
    └── release.yml                sign, notarize, appcast, GitHub release
```

Crate boundaries follow the dependency direction `domain ← store ← sync ←
core`; `provider-*` and `agent-*` depend only on their `*-api` crate and
`mail-domain` (providers may also use `mail-mime` to decode what they
fetch; it depends only on `mail-domain`). `cargo xtask check-deps`
enforces this. `openagc-core` is the only crate that knows about UniFFI.

---

## 4. Swift ↔ Rust Boundary

### 4.1 Mechanism — UniFFI **(Verified)**

UniFFI 0.32.x, proc-macro mode (`#[uniffi::export]`, `#[derive(uniffi::Record)]`
etc.), "library mode" binding generation so no UDL file is maintained.

Build pipeline (`scripts/build-core.sh`):

1. `cargo build -p openagc-core --target aarch64-apple-darwin` (release
   for Release builds) produces `libopenagc_core.a`.
2. `uniffi-bindgen-swift` generates `openagc_core.swift`, the C header and a
   plain `module openagc_coreFFI` modulemap (not `--xcframework`, which
   emits a `framework module`).
3. The script installs them under `build/core/{swift,include,lib}`,
   rewriting only files whose content changed so unchanged builds stay
   incremental.
4. In Xcode, the static `OpenAGCCore` framework target runs the script as
   an always-run pre-build phase with **declared output files**, compiles
   the generated Swift, and finds the C module through
   `SWIFT_INCLUDE_PATHS`; the app links `-lopenagc_core` from
   `LIBRARY_SEARCH_PATHS` and depends on `OpenAGCCore`.

*Amended during M0.* The original plan was a local SwiftPM package with an
XCFramework `binaryTarget`. Two Xcode behaviors ruled it out: SwiftPM
resolves binary targets before any build phase runs, and Xcode copies
XCFramework headers in a step planned before the Rust script runs, so a
regenerated header was silently stale until the following build. Reading
the artifacts from fixed paths, with the producing phase's outputs
declared, fixes both (verified: a new Rust export flows through a single
incremental `xcodebuild`, and a clean build succeeds). An XCFramework can
still be produced for distribution if the core is ever shipped separately.

Static linking is deliberate: it avoids `disable-library-validation` in the
hardened runtime and gives one Mach-O to sign.

Rejected alternatives: `swift-bridge` (0.1.x, single maintainer, no Swift→Rust
closures) and a hand-rolled C ABI (re-implements strings, errors, async and
callbacks for no benefit at this surface size).

### 4.2 Surface design

The boundary is coarse. Swift sees a handful of objects, records and enums,
not the crate graph.

```rust
#[derive(uniffi::Object)]
pub struct Core { /* runtime, store, engines */ }

#[uniffi::export]
impl Core {
    #[uniffi::constructor]
    pub fn new(config: CoreConfig, secrets: Arc<dyn SecretStore>,
               listener: Arc<dyn EventListener>) -> Result<Arc<Self>, CoreError>;

    // Mail (all read paths hit SQLite only)
    pub async fn list_mailboxes(&self) -> Result<Vec<Mailbox>, CoreError>;
    pub async fn list_threads(&self, q: ThreadQuery) -> Result<ThreadPage, CoreError>;
    pub async fn get_thread(&self, id: ThreadId) -> Result<ThreadDetail, CoreError>;
    pub async fn search(&self, q: SearchQuery) -> Result<SearchPage, CoreError>;
    pub async fn get_rendered_body(&self, id: MessageId) -> Result<RenderedBody, CoreError>;
    pub async fn get_attachment(&self, id: AttachmentId) -> Result<AttachmentFile, CoreError>;

    // Mutations (optimistic; enqueue to outbox, return immediately)
    pub fn archive(&self, ids: Vec<ThreadId>) -> Result<(), CoreError>;
    pub fn set_read(&self, ids: Vec<ThreadId>, read: bool) -> Result<(), CoreError>;
    pub fn modify_labels(&self, ids: Vec<ThreadId>, add: Vec<LabelId>, remove: Vec<LabelId>) -> Result<(), CoreError>;
    pub async fn save_draft(&self, draft: DraftInput) -> Result<DraftId, CoreError>;
    pub async fn send(&self, draft_id: DraftId) -> Result<(), CoreError>;

    // Account
    pub async fn begin_oauth(&self, client: OAuthClientConfig) -> Result<OAuthSession, CoreError>;
    pub async fn complete_oauth(&self, session: OAuthSession) -> Result<Account, CoreError>;
    pub fn sync_now(&self);

    // Agents
    pub async fn list_agent_providers(&self) -> Vec<AgentProviderStatus>;
    pub async fn start_agent_session(&self, cfg: SessionConfig) -> Result<SessionId, CoreError>;
    pub async fn send_agent_prompt(&self, s: SessionId, prompt: String, ctx: PromptContext) -> Result<(), CoreError>;
    pub fn cancel_agent_session(&self, s: SessionId);
    pub fn resolve_approval(&self, id: ApprovalId, decision: ApprovalDecision);
}
```

Rules:

- **Reads are `async`** and map to Swift `async throws`. They never block the
  main thread; UniFFI futures are driven by the Swift executor.
- **Cheap mutations are sync** and non-blocking: they write to the outbox
  table on the caller's thread (sub-millisecond) and return. Sync exports
  must never `block_on` (§4.4).
- **Pagination is keyset-based** (`after: (sort_key, thread_id)`), never
  offset-based, so scrolling a 100k-thread list stays O(page).
- **Inside Rust, all IDs are newtypes** (`ThreadId(String)`), never bare
  strings. At the FFI they cross as `String`: UniFFI custom newtypes
  become Swift typealiases, which add no type safety, so the Swift record
  field names (`threadId`, `labelIds`) carry the meaning instead. Plain
  data records are typealiased in `CoreClient.swift` for the app to use;
  calls into the core still go only through `CoreClient`.
- Every fallible call returns `CoreError`, a flat `uniffi::Error` enum with a
  `message: String` plus a machine-readable `kind`.

### 4.3 Events — Rust → Swift

One foreign trait, one enum:

```rust
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: CoreEvent);
}

#[derive(uniffi::Enum)]
pub enum CoreEvent {
    ThreadsChanged { mailbox: MailboxId, hint: ChangeHint },   // coalesced
    SyncStatus { state: SyncState, progress: Option<SyncProgress> },
    OutboxStatus { pending: u32, failed: u32 },
    AgentEvent { session: SessionId, event: AgentEvent },       // §9.5
    ApprovalRequested { request: ApprovalRequest },
    ApprovalResolved { id: ApprovalId },
    AccountChanged { account: Account },
    Error { kind: ErrorKind, message: String },
}
```

Swift wraps the listener in an `AsyncStream<CoreEvent>` delivered on the
main actor. Change events are **coalesced** in Rust (max one
`ThreadsChanged` per mailbox per 50 ms) so a sync of 500 messages produces a
handful of UI refreshes, not 500. Events carry a `ChangeHint { inserted,
updated, removed, invalidate }` so the list can patch rows in place. Hints
merge within a window (insert-then-remove cancels; update-after-insert stays
an insert) and degrade to `invalidate` above 200 ids. Warn/error `tracing`
records also arrive as `CoreEvent::Log` for Swift to log (§17).

### 4.4 Async runtime **(Verified gotcha)**

UniFFI's `async_runtime = "tokio"` attribute uses a process-wide
*current-thread* fallback runtime; `block_in_place` aborts on it and sync
exports calling `tokio::spawn` panic with "no reactor running". The core
therefore:

- builds its own `tokio::runtime::Builder::new_multi_thread()` (4 workers,
  named threads) in a `OnceLock` at `Core::new`;
- implements every exported `async fn` as
  `RUNTIME.spawn(async move { ... }).await`, so work always runs on our
  runtime regardless of which thread Swift called from;
- never uses `async_runtime` attributes;
- keeps exported sync fns free of `.await` and of runtime calls.

SQLite work does not run on tokio workers at all (§6.4).

---

## 5. Domain Model

`mail-domain` holds plain Rust types shared by every crate. Only the types
Swift needs are re-exported through UniFFI records.

```text
Account        id, email, provider, display_name, history_id, created_at
Mailbox        id, account_id, kind (Inbox|Starred|Sent|Drafts|Archive|Spam|Trash|Label), label_id?, name, unread_count, total_count
Label          id, account_id, gmail_id, name, kind (System|User), color?, visible
Thread         id, account_id, gmail_id, subject, snippet, last_message_at, first_message_at, message_count, unread_count, has_attachments, participants (denormalized), label_ids
Message        id, thread_id, gmail_id, rfc822_message_id, in_reply_to, references, from, to, cc, bcc, reply_to, subject, date, snippet, body_state (Metadata|Full), size_estimate, is_read, is_starred, is_draft, is_sent_by_me, raw_headers (json)
Body           message_id, text_plain?, html_sanitized?, html_original?, has_remote_images, has_blocked_content
Participant    message_id, role (From|To|Cc|Bcc|ReplyTo), name?, email
Attachment     id, message_id, gmail_attachment_id, filename, mime_type, size, content_id?, is_inline, local_path?
Draft          id, account_id, gmail_draft_id?, thread_id?, in_reply_to_message_id?, to, cc, bcc, subject, body_html, body_text, attachments, updated_at, dirty
OutboxOp       id, account_id, kind, payload (json), created_at, attempts, last_error?, state (Pending|InFlight|Failed), claimed_by?, lease_until? (§7.4 outbox claims)
AgentSession   id, provider, external_session_id?, started_at, ended_at?, state, prompt_count, cost_usd?
AgentAction    id, session_id, tool, args (json), risk (ReadOnly|Reversible|External), state (Executed|Pending|Approved|Rejected|Failed), result_summary?, created_at, resolved_at?
```

Threads are Gmail's threads: OpenAGC does not re-thread by `References`.
This keeps local state identical to what the user sees on the web and what
`threadId` means in the API.

---

## 6. Local Store

### 6.1 Engine — `rusqlite` with bundled SQLite **(Verified)**

`rusqlite` 0.40+ with `features = ["bundled", "functions"]`, which compiles
SQLite 3.53 with FTS5 and JSON1. `sqlx` was rejected: its SQLite driver
serializes onto a worker thread anyway, so async buys nothing for a local
file, and compile-time query checking fights dynamically built FTS queries.

Pragmas at open: `journal_mode=WAL`, `synchronous=NORMAL`,
`foreign_keys=ON`, `temp_store=MEMORY`, `mmap_size=256MB`,
`cache_size=-65536` (64 MB), `busy_timeout=5000`.

Location: `~/Library/Application Support/OpenAGC/<account-uuid>/mail.sqlite`.
One database per account so a future multi-account version is a loop, not a
migration.

### 6.2 Schema (v1)

Migrations are numbered SQL files embedded with `include_str!`, applied in
order, tracked by `PRAGMA user_version` (set in the same transaction as the
migration, so a crash cannot leave the two out of step; a database newer
than the build is refused). Every table has integer rowid primary keys;
Gmail IDs are unique-indexed text columns.

*The authoritative schema is `crates/mail-store/migrations/0001_initial.sql`.*
The sketch below was the plan; the implemented schema differs as noted
after it (single-row `account` table, `participants.position`, an
`attachments.part_id`, a `contacts` table, a virtual `@archive` label).

```sql
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY, uuid TEXT NOT NULL UNIQUE, email TEXT NOT NULL,
  display_name TEXT, history_id INTEGER, initial_sync_done INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL);

CREATE TABLE labels (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id),
  gmail_id TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
  color_bg TEXT, color_fg TEXT, list_visible INTEGER NOT NULL DEFAULT 1,
  UNIQUE(account_id, gmail_id));

CREATE TABLE threads (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id),
  gmail_id TEXT NOT NULL, subject TEXT, snippet TEXT,
  first_message_at INTEGER, last_message_at INTEGER NOT NULL,
  message_count INTEGER NOT NULL DEFAULT 0, unread_count INTEGER NOT NULL DEFAULT 0,
  has_attachments INTEGER NOT NULL DEFAULT 0,
  participants_json TEXT NOT NULL DEFAULT '[]',     -- [{name,email}] for the list row
  UNIQUE(account_id, gmail_id));
CREATE INDEX threads_by_last ON threads(account_id, last_message_at DESC, id DESC);

CREATE TABLE messages (
  id INTEGER PRIMARY KEY, thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  account_id INTEGER NOT NULL, gmail_id TEXT NOT NULL,
  rfc822_message_id TEXT, in_reply_to TEXT, references_json TEXT,
  from_name TEXT, from_email TEXT, subject TEXT, snippet TEXT,
  date INTEGER NOT NULL, internal_date INTEGER NOT NULL,
  size_estimate INTEGER, body_state TEXT NOT NULL DEFAULT 'metadata',  -- metadata|full
  is_read INTEGER NOT NULL DEFAULT 0, is_starred INTEGER NOT NULL DEFAULT 0,
  is_draft INTEGER NOT NULL DEFAULT 0, is_sent_by_me INTEGER NOT NULL DEFAULT 0,
  headers_json TEXT,
  UNIQUE(account_id, gmail_id));
CREATE INDEX messages_by_thread ON messages(thread_id, internal_date);
CREATE INDEX messages_by_rfc822 ON messages(rfc822_message_id);

CREATE TABLE message_labels (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  PRIMARY KEY(message_id, label_id));
CREATE INDEX message_labels_by_label ON message_labels(label_id, message_id);

-- Denormalized: which labels a thread carries (any message has it). Maintained by triggers.
CREATE TABLE thread_labels (
  thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
  label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
  last_message_at INTEGER NOT NULL,
  PRIMARY KEY(label_id, last_message_at DESC, thread_id));   -- covering index for the list

CREATE TABLE participants (
  message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  role TEXT NOT NULL, name TEXT, email TEXT NOT NULL);
CREATE INDEX participants_by_email ON participants(email);

CREATE TABLE bodies (
  message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
  text_plain TEXT, html_sanitized TEXT, html_original BLOB,   -- original zstd-compressed
  has_remote_images INTEGER NOT NULL DEFAULT 0, sanitizer_version INTEGER NOT NULL);

CREATE TABLE attachments (
  id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  gmail_attachment_id TEXT, filename TEXT, mime_type TEXT, size INTEGER,
  content_id TEXT, is_inline INTEGER NOT NULL DEFAULT 0, local_path TEXT);

CREATE TABLE drafts (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, gmail_draft_id TEXT,
  thread_id INTEGER, in_reply_to_message_id INTEGER,
  to_json TEXT, cc_json TEXT, bcc_json TEXT, subject TEXT,
  body_html TEXT, body_text TEXT, attachments_json TEXT,
  updated_at INTEGER NOT NULL, dirty INTEGER NOT NULL DEFAULT 1);

CREATE TABLE outbox (
  id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL, kind TEXT NOT NULL,
  payload_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, next_attempt_at INTEGER,
  state TEXT NOT NULL DEFAULT 'pending', last_error TEXT,
  claimed_by TEXT, lease_until INTEGER);  -- migration 0021, §7.4 outbox claims

CREATE TABLE sync_state (account_id INTEGER PRIMARY KEY, key TEXT, value TEXT);
CREATE TABLE backfill_queue (
  account_id INTEGER NOT NULL, message_id INTEGER NOT NULL,
  priority INTEGER NOT NULL, PRIMARY KEY(account_id, priority, message_id));

CREATE TABLE agent_sessions (
  id INTEGER PRIMARY KEY, uuid TEXT NOT NULL UNIQUE, provider TEXT NOT NULL,
  external_id TEXT, state TEXT NOT NULL, started_at INTEGER NOT NULL, ended_at INTEGER,
  prompt_count INTEGER NOT NULL DEFAULT 0, cost_usd REAL);
CREATE TABLE agent_actions (
  id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agent_sessions(id),
  tool TEXT NOT NULL, args_json TEXT NOT NULL, risk TEXT NOT NULL, state TEXT NOT NULL,
  result_summary TEXT, created_at INTEGER NOT NULL, resolved_at INTEGER);
CREATE TABLE agent_transcript (
  session_id INTEGER NOT NULL, seq INTEGER NOT NULL, role TEXT NOT NULL,
  content_json TEXT NOT NULL, PRIMARY KEY(session_id, seq));
```

`thread_labels` is the table the inbox actually reads: `WHERE label_id = ?
ORDER BY last_message_at DESC, thread_id DESC LIMIT 100` is an index-only
scan. The store's write API maintains it and the thread aggregates
(`unread_count`, `message_count`, participants, label ids) by recomputing
the affected threads in the same transaction as each change. *(Amended in
M1: the plan said SQL triggers; since the store is the only writer, doing
it in Rust gives the same guarantee and is far easier to test.)* Archive
is a virtual label row (`@archive`, kind `virtual`) whose `thread_labels`
entries mark threads with no INBOX label that are not wholly spam or trash,
so Archive lists use the same index as every other mailbox.

**Tasks (Amendment 2026-09-29, migration `0009_tasks`).** Each account's
store also holds its tasks (§14.8): `tasks` (thread and message by Gmail
id, not foreign key, so a task outlives its thread leaving the store;
title, notes, category, due day as `YYYY-MM-DD` or none, action `reply`,
`reply_all`, `forward` or `none`, status `open` or `done`, source `ai` or
`you`, Claude's one-line why, created and completed times),
`task_categories` (name, unique in any case, and position; seeded with the
starting set) and `task_meta` (the id of the account's `Task` label).

**Clean Up (Amendment 2026-10-08, migration `0018_cleanup`).** For the
Clean Up window (§14.12): `messages` gains `list_id` (the id inside
`List-Id`'s angle brackets, lower-cased, or the whole value without
them), `list_name` (its phrase, decoded), `list_unsubscribe` and
`list_unsubscribe_post` (as sent, unfolded), written by every fetch path
that sees the headers (IMAP header blocks and whole messages, the REST
API's `format=full`, mbox import) and kept when a later fetch lacks them.
`headers_json` stays unused. Covering indexes serve the grouping:
`(from_email COLLATE NOCASE, from_name, date)`, `(subject, date)`,
`(date)`, `(size_estimate, date)` and `(list_id, list_name, date)` where
`list_id` is set; Spam, Trash and drafts are left out through one small
set of message ids rather than per-row lookups. `inbox_history` (day as
`YYYY-MM-DD`, the Inbox's message count at its start) and `cleanup_meta`
(key/value: the progress baseline and when it was taken) back the
progress card.

### 6.3 Full-text search **(Verified)**

Two FTS5 tables *(amended in M1)*:

```sql
-- rowid = messages.id; contentless with contentless_delete (SQLite ≥ 3.43)
CREATE VIRTUAL TABLE messages_fts USING fts5(
  subject, from_text, to_text, body, attachment_names,
  content = '', contentless_delete = 1,
  tokenize = 'unicode61 remove_diacritics 2');

-- Everyone corresponded with, for autocomplete (frecency) and partial matches
CREATE TABLE contacts (id, email UNIQUE COLLATE NOCASE, name,
  sent_count, received_count, last_seen);
CREATE VIRTUAL TABLE contacts_fts USING fts5(
  name, email, content = 'contacts', content_rowid = 'id', tokenize = 'trigram');
```

The plan was external-content tables over views. External content requires
every delete to replay the *old* column values exactly, or the index is
silently corrupted; a contentless table with `contentless_delete=1` deletes
by rowid and stores no second copy either. The cost — no `snippet()` /
`highlight()` from the index — does not apply, since results are rendered
from the message tables. The contacts table replaces a trigram index over a
participants view: it is what composer autocomplete needs anyway (§14.5),
and its three triggers are trivial. The trigram table serves as-you-type
address matching (`"ohn"` matches `john@`), which `unicode61` prefix queries
cannot do inside an address.

Search query grammar (§8) compiles to `MATCH` plus structured `WHERE` clauses.
Ranking: `bm25(messages_fts, 10.0, 5.0, 5.0, 2.0, 1.0)` weighted toward
subject and sender, tie-broken by date.

### 6.4 Threading model — one writer, N readers

- **Writer**: one dedicated `std::thread` owning one connection. All
  mutations are messages on a bounded channel (`WriteOp` enum + oneshot
  reply). Batches are coalesced into single transactions (sync applies up to
  500 messages per transaction).
- **Readers**: a pool of 4 read-only connections (`SQLITE_OPEN_READONLY`),
  used from a small `rayon`-free blocking pool (`tokio::task::spawn_blocking`
  is fine here since readers never hold locks across awaits).
- WAL mode makes readers never block on the writer.
- Prepared statements are cached per connection (`prepare_cached`).
- Write transactions begin `IMMEDIATE`, so a writer in another process on
  the same store (the headless MCP) is waited for, up to a 30 s busy
  timeout, rather than failing a transaction that read before it wrote
  (§7.4 outbox claims).

---

## 7. Gmail Integration

### 7.1 Client — hand-written over REST **(Verified)**

`google-gmail1` (google-apis-rs) is in maintenance mode and drags in a
hyper/yup-oauth2 stack. OpenAGC uses ~12 endpoints; a thin `reqwest` client
with `serde` types is smaller and fully under our control.

Endpoints used: `users.getProfile`, `labels.list`, `messages.list`,
`messages.get`, `messages.modify`, `messages.batchModify`, `messages.send`,
`messages.trash`, `history.list`, `drafts.create/update/delete/send`,
`attachments.get`. Batching uses the multipart batch endpoint (≤50 calls per
batch, Google's recommendation).

HTTP: `reqwest` with `rustls`, HTTP/2, gzip, one shared client. Retries:
exponential backoff with jitter on 429/5xx, honoring `Retry-After`; 401 →
one token refresh then fail; 403 `rateLimitExceeded` → back off 60 s.

### 7.2 Quota **(Verified — changed 2026-05-01)**

Gmail quotas are now **per minute**: 6,000 quota units per user per project
per minute. Costs: `messages.get` **20**, `threads.get` 40, `messages.list`
5, `history.list` 2, `messages.modify` 5, `batchModify` 50, `messages.send`
100, `attachments.get` 20, `drafts.create` 10. Batch requests do not reduce
unit cost.

Consequence: at most ~300 `messages.get` per minute per user, ~5/s. A
100,000-message mailbox cannot be bulk-fetched; it takes ~5.5 hours at full
rate. Sync must therefore be prioritized (§7.4) and the app must be fully
usable during backfill. A token-bucket rate limiter in `provider-gmail`
enforces 5,500 units/min (leaving headroom for user-initiated calls, which
take priority over backfill).

**Amendment (2026-09-25).** The first real sync still drew ~10 rate-limit
responses a minute at 5,500 units/min with 8 fetches in flight. The limiter
now starts at 5,000 units/min and adapts: a rate-limit response drains it,
pauses every caller for the `Retry-After` period, and cuts the refill rate
by 30 % (floor 20 % of nominal); each clean minute raises it 10 % back
towards nominal. Retries no longer sleep independently, so one 429 no longer
turns into eight 60-second stalls.

### 7.3 OAuth **(Verified)**

- Flow: OAuth 2.0 for installed apps — "Desktop app" client type, PKCE
  (S256), loopback redirect `http://127.0.0.1:<ephemeral-port>/callback`.
  Custom URI schemes are deprecated by Google; not used.
- Library: `oauth2` crate 5.x for the protocol; the loopback listener is a
  tiny `tokio` HTTP server that accepts exactly one request and closes.
- The browser is opened by Swift (`NSWorkspace.open`) so the user's default
  browser and existing Google session are used; no embedded web view for
  login (Google blocks it).
- Scopes: `https://www.googleapis.com/auth/gmail.modify` and
  `https://www.googleapis.com/auth/userinfo.email` (plus `openid` and
  `userinfo.profile` for the avatar since the §7.7 amendment). Every useful
  combination (`readonly`+`compose`+`send`) is equally *restricted* in
  Google's classification, so splitting scopes buys nothing and complicates
  consent. `mail.google.com` (full access) is never requested.
  *(Amended 2026-09-28, IMAP-first sync, §7.4: sign-in asks for
  `https://mail.google.com/` plus `gmail.modify`, `openid` and `profile`.
  `gmail.modify` is already restricted, so verification and CASA are
  unchanged; the consent wording is broader. An account signed in without
  the full scope keeps working over the API until its next sign-in.)*
- **Client ID policy (decided):** OpenAGC ships a project OAuth client ID
  and, because installed apps cannot keep secrets (Google's own statement),
  the client secret is in the repo and treated as public. Settings ›
  Accounts › Advanced lets the user substitute their own client ID/secret
  ("Bring your own client"). Until the shipped client passes Google's
  restricted-scope verification and CASA assessment, it is in *testing*
  mode: capped at 100 test users, showing the unverified-app warning. The
  onboarding screen explains this and offers the BYO path as the
  no-warning alternative. Verification is tracked as a project task, not a
  code task.
- Tokens: refresh token and current access token are stored by Swift in the
  Keychain (§12). Rust obtains them through the `SecretStore` foreign trait
  and caches the access token in memory until expiry − 60 s.

### 7.4 Synchronization

**Bootstrap (first run)**

1. `labels.list` → labels table.
2. `getProfile` → record `historyId` **before** listing, so nothing that
   arrives during bootstrap is lost.
3. `messages.list` with `labelIds=INBOX`, then `q=newer_than:30d`, then
   everything, paging `maxResults=500`. Each page inserts placeholder
   message rows (`body_state='metadata'`, date unknown) and pushes IDs into
   `backfill_queue` with priority: Inbox unread (0) → Inbox (1) → last 30
   days (2) → last year (3) → rest (4).
4. The backfill worker drains the queue in priority order calling
   `messages.get?format=full` in batches of 50, respecting the rate limiter.
   Full format includes headers, parts and bodies, but **not** attachment
   bytes (those are fetched on demand).
5. The inbox is interactive as soon as priority 0–1 has drained — typically
   under a minute for a few hundred inbox messages.

A first-run screen shows "Syncing your inbox… older mail keeps loading in the
background" with a progress figure from the queue depth.

**Amendment (2026-09-25, first real mailbox).** Two changes after syncing a
43,000-message account:

- *Sync window.* Downloading everything is not the default: at 300
  `messages.get` per minute that mailbox needs 2.4 hours, and neither
  `format=metadata` nor `threads.get` is cheaper per message. The inbox and
  the last 30 days always come down; beyond that a per-account window
  (Settings › Accounts › *Download mail from*: last month, **6 months**
  (default), last year, everything) bounds phases 3–4. Widening re-lists and
  queues the extra mail; narrowing drops queued fetches beyond the window and
  keeps what is stored. Mail outside the window stays on the server and is
  not searchable locally (server-side search is a follow-up).
  *(Amended 2026-10-08: opening Clean Up, §14.12, sets the window to
  everything, headers only, and says so; without IMAP it asks first.
  Implemented: `SyncEngine::load_every_header` stores *Everything* and a
  body window that keeps bodies where they were (*the whole window*
  becomes the old window's span; Settings gains *Full messages for: Last
  year*), then re-lists the window's phases, so with IMAP the older mail
  lands in the headers-only tier and the body backfill never fetches it.
  An account not syncing stores both and marks its queue's tiering stale,
  so the next start lists the wider window. Amended 2026-10-08: the body
  window changes only when headers are cheap (over the API it is moot and
  stays as the user set it), and a widening decided on cheap headers
  changes nothing if IMAP was refused since, so the window asks first.)*
- *Order.* The queue drains in listing order within a priority, i.e. newest
  first (`backfill_queue.seq`); it used to order by Gmail id, which is oldest
  first. Fetches triggered by history (mail the user touched elsewhere) go to
  the front.

**Incremental (steady state)**

- Poll `history.list?startHistoryId=<last>` every 30 s while the app is
  frontmost, every 5 min in the background, and immediately on
  foreground/wake/network-regain. `history.list` costs 2 units, so polling
  is cheap.
- History records map to store ops: `messagesAdded` → fetch (priority 0),
  `messagesDeleted` → delete, `labelsAdded/Removed` → update labels and
  derived counters.
- On HTTP 404 (history expired, ~1 week) → re-bootstrap but keep local
  bodies; only metadata and labels are re-listed.
- No `users.watch`/push: it requires a Cloud Pub/Sub topic, i.e. a server.

**Outbox (local → Gmail)**

Every mutation is written to `outbox` and applied to local tables in the
same transaction (optimistic). A worker drains the outbox FIFO per account:
`archive` → `batchModify removeLabelIds=[INBOX]`, `set_read` →
`batchModify`, `send` → `messages.send`, drafts → `drafts.create/update`.
Failures retry with backoff (max 5); permanent failures flip the local
change back, emit `Error`, and show a non-modal banner. The sync poll after
a successful outbox op will see our own change in history and no-op.

Conflict rule: server wins for labels/read state on the next history sync;
the outbox is drained *before* history is applied so local intent is not
overwritten while in flight.

**Amendment (2026-10-08): outbox claims, several drainers.** *Implemented
(oagc-uys.3).* The app and a second process (the headless MCP,
[headless-mcp.md](plans/headless-mcp.md)) may both drain one account's
outbox. No op is sent twice, none out of order, and neither process waits
forever on the other.
- *Claims in the store, not a file lock on the whole outbox.* A drainer
  takes an op in one `BEGIN IMMEDIATE` transaction that picks the oldest
  ready op (as before: one waiting on a retry holds back the rest; held
  sends step aside) and marks it `in_flight` with `claimed_by` (the
  drainer's id) and `lease_until` (now + 60 s), and only when **no** op
  is in flight. So across processes, too, one op is in flight at a time
  and ops go strictly in order (an unarchive never passes its archive). A
  drainer that finds another's op in flight stops (`DrainReport.busy`)
  and looks again within 2 s (`next_outbox_retry`). A coarse `flock` on
  the whole drain was not needed: single flight in the claim gives the
  same ordering, and a held per-op claim says exactly which op a dead
  process left.
- *Who is alive.* Each drainer (one per sync engine) holds an exclusive
  `flock` on `outbox-claims/<id>.lock` beside the store for its life; the
  kernel drops it when the process dies, however it dies, and a reused pid
  cannot fool it. A drainer renews its lease every 15 s while the
  provider call runs, and records the call's outcome only if it still
  holds the claim. Lock files of dead drainers are cleared at
  registration, but only by whoever holds the file locked itself and only
  while the name still refers to that file (same inode); a new drainer,
  after locking its file, checks the name still refers to it and starts
  again with a fresh file if a sweep took it in the instant between
  creating and locking it *(amendment 2026-10-08, oagc-cp3.5)*.
- *Recovery,* at the start of each drain: an op in flight goes back to
  pending if it is the drainer's own (an interrupted drain), unnamed (left
  by a build from before claims: the old single-process recovery at
  launch), or its claimant is gone (lock file free) or its lease ran out
  (a hung process). A crash is recovered at once at the next drain, by the
  app at its next launch or by the other process within 2 s.
- *Sends are never blindly retried.* A send returned to pending counts an
  attempt, and any send tried before (`attempts > 0`: left in flight, or
  an error that may have come after the provider took it) is first looked
  for: `MailProvider::already_sent`. Gmail searches `rfc822msgid:` (its
  `messages.send` has no idempotency key; it keeps the composer's
  Message-ID, §7.5); AgentMail looks for its `X-OpenAGC-Outbox-Id` header
  (and its `Idempotency-Key` holds 24 h); Primitive cannot look and relies
  on its Message-ID `Idempotency-Key`. Found, the send is taken as sent
  (the draft is done with, the optimistic copy adopted) and not sent again.
  Label changes are idempotent and go again as they are.
- *Busy store.* Every store write takes the write lock at `BEGIN`
  (`IMMEDIATE`), so a second process's write is waited for (busy timeout
  30 s on the writer) instead of failing a deferred transaction at its
  first write; no busy error reaches the user.
- Tests: two engines on one store file, and a second OS process racing
  this one over 1,000 queued changes against a counting fake: each op
  reaches the provider exactly once and in queue order; a process killed
  inside a send is recovered at once, and the send is found rather than
  sent again (or sent once, if it never reached the provider).

**Amendment (2026-09-26): bulk backfill over IMAP.** *Superseded by the
2026-09-28 amendment, IMAP-first sync, at the end of this section; kept for
the history of the decision.* Planned; the REST
backfill stays as the fallback and the only path for incremental sync.

*Why.* The REST API charges 20 units per `messages.get` whatever the format,
and the first real mailbox showed the per-user quota below the documented
6,000 units/min. Even the six-month window is hours of backfill; the
"everything" setting will always feel broken over REST. Gmail's IMAP
endpoint has no unit quota, only bandwidth (~2,500 MB per user per day) and
15 concurrent connections, and an `ENVELOPE`/`BODYSTRUCTURE` fetch is
nearly free, which also makes a headers-first sync possible.

*Design: hybrid.* IMAP is used for bulk download only. History
(`history.list`), all writes and the outbox stay on REST, because Gmail IMAP
has no change log (no `CONDSTORE`/`QRESYNC`) and the API's ids are the
source of truth. The two join on Gmail's IMAP extensions: `X-GM-MSGID` and
`X-GM-THRID` are the API's message and thread ids (decimal over IMAP, hex in
the API), `X-GM-LABELS` carries the labels, and `[Gmail]/All Mail` is one
UID-ordered stream, newest last. Concretely:

1. `provider-gmail` gains an `ImapBackfill` (async IMAP over TLS, XOAUTH2)
   behind a `BackfillSource` trait; `MailProvider::fetch_messages` remains
   the REST implementation. The engine asks the backfill source for the
   queued ids; the IMAP source resolves them with `UID SEARCH X-GM-MSGID`
   in batches and fetches `BODY.PEEK[]` for up to 200 messages per command
   on up to 4 connections, honouring the 15-connection cap with headroom.
   Attachment parts larger than 1 MB are skipped via `BODYSTRUCTURE` and
   fetched on demand over REST as today.
2. The raw RFC 822 bytes go through `mail_mime::parse` into the same
   `IncomingMessage` the REST path produces, so the store, sanitizer and
   search see no difference. `X-GM-LABELS` plus `\Seen`/`\Flagged` map to
   label ids (`UNREAD`, `STARRED`); system labels use the API names.
3. Listing stays on REST (`messages.list` is 5 units per 500 ids), so the
   window and priority phases are unchanged. A headers-first pass fills
   `messages` with `body_state='metadata'` rows (1,000 per command) so the
   list is browsable minutes in; bodies follow, and opening a header-only
   message moves it to the front of the queue. *(Implemented with
   `BODY.PEEK[HEADER]` parsed by `mail-mime`, not `ENVELOPE`, so headers
   decode exactly like full messages. REST sources skip the pass: a
   header fetch costs the same 20 units as a whole message.)*
4. Scope: IMAP needs `https://mail.google.com/`, a superset of
   `gmail.modify`. Both are restricted scopes, so verification (§7.3) is
   unchanged, but the consent screen then asks to "read, compose, send and
   permanently delete all your email". *(Implementation decision,
   2026-09-26: opt-in per account, Settings › Accounts › Download faster
   over IMAP, which signs in again with `mail.google.com` instead of
   `gmail.modify`; the default stays least-privilege, and a Cloud project
   must list the scope before it can be granted.)* The granted scopes are
   recorded per account. If IMAP `AUTHENTICATE` fails (a Workspace admin
   can disable IMAP), the engine logs it once and stays on REST.
5. Budget: the source tracks bytes per day and yields to REST at 2,000 MB.
   Incremental fetches (new mail from history) stay on REST: they are few
   and latency matters more than units there.

*Testing.* Fakes only: a fake IMAP server in-process (a small
`tokio`-based responder that speaks the subset used: `CAPABILITY`,
`AUTHENTICATE XOAUTH2`, `SELECT`, `UID SEARCH`, `UID FETCH`, `LOGOUT`),
plus the existing `FakeProvider` for the REST half. No test ever connects
to `imap.gmail.com`.

*Not in scope.* IMAP as the sole provider (non-Gmail accounts), IDLE push,
and label writes over IMAP.

**Amendment (2026-09-27): tiered download.** Implemented. With IMAP, headers
are cheap and bodies are not; most old mail is never opened. So an account
using IMAP downloads in two tiers:

- *Headers* for the whole sync window (six months by default, or
  everything): subject, sender, recipients, date, labels, flags. The list,
  labels, sorting and routines' first pass work from these.
- *Bodies* only where they matter: the Inbox and a *body window* (default
  the last 30 days; Settings › Accounts › *Full messages for*: last 30
  days / last 6 months / the whole window), plus on demand: a message the
  user opens (already prioritized), a header-only message an agent or
  routine reads (fetched at interactive priority before the tool answers),
  and search matches (server search fetches them).

Details:
1. Queue: ids outside the body window are listed with a *headers-only*
   priority that the body backfill never drains; the headers pass covers
   them. Widening the body window moves them to body priorities.
   *(Implemented: each age tier has its own priority, 3 = six months,
   4 = a year, 5 = older, and headers-only is that priority + 10. The
   headers pass stores their headers and drops them from the queue. The
   tiering a queue was listed under is recorded in `sync_state`; when sync
   starts with another one (IMAP turned on or off, another body window) the
   window's phases are re-listed. If the source stops offering cheap
   headers mid-run (IMAP refused), the headers-only tier is promoted to
   body fetches rather than left unlisted. A header-only refresh keeps an
   existing snippet.)* *(Amended 2026-10-08, oagc-merk.8: except the tier
   Clean Up created by widening the window with IMAP (§14.12; recorded in
   `sync_state` as `cleanup_headers_only`): it waits for IMAP instead,
   keeps its tiering across restarts, and resumes headers only when IMAP
   comes back; the Clean Up window asks *Load All Mail* / *Not Now* first,
   and only *Load All Mail* promotes it. A window the user sets in
   Settings, or an *Everything* window from before Clean Up, is promoted
   as above.)*
2. `ensure_bodies(ids)`: fetch header-only messages now (IMAP when
   available, else REST) and store them; agent tools (`mail_get_thread`,
   `mail_get_message`, `mail_get_attachment_text`) and the reader call it.
3. Search: free-text queries also run server search when the account has
   header-only mail in the searched mailbox, not only when local results
   are few; header-only rows match on headers locally in the meantime.
   *(Implemented per account rather than per mailbox: any header-only
   message, found through a partial index, makes a query with a word or
   phrase outside an operator ask Gmail after the usual pause. Matches that
   are stored with headers only download their bodies like an opened
   message.)*
4. Snippets: a partial fetch of the first bytes of the text part
   (`BODY.PEEK[1]<0.2048>`, decoded best effort) gives list snippets
   without whole bodies. *(Implemented as `BODY.PEEK[TEXT]<0.2048>` in the
   same command as the headers, for every message: the header block plus
   those bytes, cut back to the last whole line, go through `mail-mime` like
   a full message, which handles multipart, quoted-printable, base64,
   charsets and HTML-only mail alike. `RFC822.SIZE` gives the true size;
   header bytes count towards the daily IMAP budget.)*
5. Accounts on the REST API keep today's behaviour (bodies for the whole
   window): headers cost the same quota there. Settings suggests IMAP for
   large mailboxes and hides the body-window choice without it.
   *(Implemented: Settings › Accounts shows "Full messages for: Last 30
   days / Last 6 months / Everything downloaded" under the IMAP toggle when
   it is on, and otherwise, above 20,000 stored messages, a one-line nudge
   towards IMAP. `SyncStatus` carries `pending_headers`; the sync line reads
   "Syncing over IMAP — headers 6,406 left" while headers-only mail is
   queued, then the body count.)*

*Status (2026-09-27): implemented (oagc-m90), tested against the fakes
only.*

Trade-off, accepted: text search over old mail waits on Gmail's server
search, and an agent reading old mail pauses while it downloads.

*Measured (2026-09-28, oagc-vaq):* `crates/mail-sync/tests/scale.rs`,
release build, Apple M4 Pro. 100,000 messages in 33,334 threads over two
years (1,539 threads in the Inbox), listed through the fake provider with
a cheap-headers source standing in for IMAP, so the times are the engine's
and the store's, not the network's:

| Step | Time |
| --- | --- |
| List every id and queue it by tier | 1.75 s |
| Inbox browsable (the headers pass does the Inbox first) | 1.80 s |
| Every message listed (headers pass done) | 7.4 s |
| Bodies for the Inbox and the last 30 days (6,668) | 0.5 s |

Queue right after listing: 770 unread Inbox, 2,307 other Inbox and 3,591
from the last 30 days for bodies; 20,000 (to six months), 24,667 (to a
year) and 48,665 (older) for headers only. Peak memory rose about 100 MB
over the fake mailbox's own 280 MB; the store took 98 MB. No body outside
the window was fetched. A 5,000-message version of the same run is in the
gate and checks the tiers, that only in-window bodies are fetched, and a
generous time bound. Rerun:
`cargo test --release -p mail-sync --test scale -- --ignored --nocapture`.

**Amendment (2026-09-28): IMAP-first sync.** Implemented
(docs/plans/imap-first-sync.md; supersedes the 2026-09-26 "hybrid"
design). IMAP is the default transport for Gmail accounts, not an option.
The API serves a job only when it is faster, when there is no other way to
get the data, or when IMAP fails.

| Job | Transport |
| --- | --- |
| Listing the window's phases | IMAP `UID SEARCH X-GM-RAW "…"` (the phase as a Gmail search) |
| Headers and bodies | IMAP; messages over 2 MB through the API |
| Spam, Trash, Drafts | IMAP folders found by `LIST` special-use (`\Junk`, `\Trash`, `\Drafts`, `\All`; English names as fallback), labelled `SPAM`/`TRASH`/`DRAFT` from the folder. Spam and Trash are listed whole with the last month (Gmail empties them after 30 days) |
| New mail | IMAP `IDLE` on All Mail, its own connection, re-issued every 25 min; any change runs an incremental round at once. The 30 s / 5 min poll stays as the backstop |
| Changes made elsewhere | API `history.list`: Gmail IMAP keeps no change log |
| Inbox categories | IMAP `X-GM-RAW "in:inbox category:…"` per category, API as fallback: IMAP leaves categories out of a message's labels, so they are applied to stored Inbox mail at start, when the download queue empties, and every 10 minutes (only added; moves to Primary arrive through history). With categories in use, the Inbox's unread count is Primary's, as in Gmail |
| Draft ids, label colours | API: no other way |
| Writes (outbox), send, drafts | API: one call per change with exact undo (§14.6a), threading, draft ids |
| Server search | IMAP `X-GM-RAW`, the API as fallback |

*Fallback.* Each operation falls back to the API on its own. Three IMAP
failures in a row open a breaker for 15 minutes (then one try); a refresh
closes it. A refused login or the day's bandwidth budget (2,000 MB) makes
IMAP unavailable without counting as failures. A refused login is first
tried again once with a fresh token (an access token can expire while the
Mac sleeps; token expiry is kept by the wall clock), and a refusal lasts
an hour before IMAP is tried again (amended 2026-10-02). IDLE failures back off and
never open the breaker. When the API is serving for one of these reasons,
the sidebar's sync footer says so in a quiet note ("Using the Gmail API ·
IMAP was refused for this account").

*Record.* Every operation is recorded (job, transport, why the API, time,
items, success; the last 200 per account) and shown in the Sync Debugger
(§14.7a).

*Testing.* Fakes only: the in-process IMAP server speaks `LIST` with
special-use attributes, per-folder `SELECT`/`EXAMINE`, `UID SEARCH` with
`X-GM-RAW`, `IDLE`, and refused logins; a failing source exercises the
fallback and the breaker. Nothing connects to Gmail.

### 7.5 Sending and threading **(Verified)**

Outgoing mail is built with `mail-builder`: `multipart/alternative` with
`text/plain` (generated from the HTML) and `text/html`, plus
`multipart/mixed` for attachments. Replies set `In-Reply-To` and
`References` from the parent, keep the normalized `Subject` (`Re:`), and
pass `threadId` in the `messages.send` body — Gmail requires all three to
thread. The raw RFC 5322 bytes are base64url-encoded into `raw`. Sent
messages appear in the local store via the next history sync; the outbox
inserts an optimistic sent copy that is reconciled by `rfc822_message_id`.

### 7.7 Multiple accounts **(Amendment 2026-09-26)**

Multi-account was an MVP non-goal (§1.2); it is now in scope, in the form
the maintainer asked for: several Gmail accounts, **one visible at a time**,
switched from an avatar button. Nothing is ever merged across accounts: no
unified inbox, no cross-account search, no cross-account agent tools.

**Model.** An account is what §6.1 already makes it: one directory under
`accounts/<id>/` with its own `mail.sqlite`, attachments cache, routines,
agent sessions and Keychain items. New pieces:

- `accounts/index.json`: `[{id, email, display_name?, avatar_file?,
  added_at, position}]`. Built on first launch after the upgrade by
  scanning the directories (each store records its `account_email`), then
  kept in step by sign-in and sign-out. The current account id lives in
  `UserDefaults` on the Swift side, not in the index.
- Identity: sign-in requests `openid` and `profile` alongside the
  existing scopes (both non-sensitive; no verification change). The token
  response's ID token carries `name` and `picture`, read without a
  further call *(implementation note: the ID token comes straight from
  Google's token endpoint over TLS and is used only for display, so it is
  not signature-checked)*; `https://openidconnect.googleapis.com/v1/userinfo`
  refreshes them weekly. Only `https` pictures on `*.googleusercontent.com`
  are fetched, at most 1 MB. The picture is downloaded to `accounts/<id>/avatar.jpg`
  (refreshed weekly) and shown at 24 pt; without one, an initials disc
  coloured deterministically from the address. Adding an account uses the
  same sign-in flow with `prompt=select_account`, so Google shows the
  chooser instead of silently reusing the browser's current session.
- Core: `Core` holds every opened account (`HashMap<id, Account>`) and
  their `SyncService`s, plus a *current* id. Sync, notifications and local
  routines run for **all** accounts in the background; only the UI is
  scoped. Every `CoreEvent` carries `account_id`; Swift drops events that
  are not for the current account except `NewMail` (notification) and
  `SyncStatus`/unread counts (menu badges). API: `list_accounts()`,
  `set_current_account(id)`, `remove_account(id)`; `open_account` becomes
  internal. Agent sessions and tools stay bound to the account they were
  started on; an agent never sees another account's mail.
- Switching: the window swaps its model (`AppModel` per account, kept warm
  once opened) so the sidebar, list, thread and composer all re-bind;
  unsent composer drafts belong to their account and survive the switch.

**UI (§14.3 addendum).** An avatar button in the sidebar header, left of
the search field's row. Clicking it opens a menu: one row per account
(avatar, display name, address, unread count), a check mark on the current
one, then *Add Account…*, *Create an Account from an Archived Mailbox…*
(§7.8) and *Accounts Settings…*. Keyboard: `⌃1`–`⌃9`
switch by position; `⌃⌥A` opens the menu. The Dock badge sums unread
across accounts; the app menu's *Accounts* submenu mirrors the button.
Notifications name the account when more than one exists and switch to it
when clicked. Settings › Accounts lists every account with its own sign-in
state, sync window, name and *Remove…*; removing the current account
switches to the next. A Gmail account's name shows beside its address and,
once the user sets it, stays when the Google profile changes (empty goes
back to the profile's); an imported mailbox's name is the name it is
listed by (amended 2026-10-02).

**Not in scope.** A unified inbox, moving mail between accounts, per-account
signatures beyond what the composer already does, and non-Gmail accounts
(the IMAP work in §7.4 is backfill only). *(Amended 2026-10-06: agent
mailboxes on an agent-mail service, §7.9, are the exception.)*

### 7.8 Archive accounts — mbox import **(Amendment 2026-09-26)**

An *archive account* is a mailbox imported from an mbox file (Google
Takeout produces one per Gmail account, as do most clients). It is an
account in every sense of §7.7 — its own directory, store, routines and
agent sessions, its own entry behind the avatar button — except that it is
**not connected to anything**: it cannot send, reply, forward, sync or
draft. It exists to be read, searched, sorted and reasoned over.

**Model.**
- `accounts/<id>/account.json`: `{kind: "archive", name, source_path,
  imported_at, messages, threads, bytes}`. The index (§7.7) gains a `kind`
  (`gmail` | `archive`). No Keychain items, no `SyncService`, no outbox,
  no provider; `MailProvider` is not implemented — the archive is written
  once by the importer through `MailWriter`, the same path sync uses.
- Ids: the message id is the SHA-256 of the raw message (first 16 bytes,
  hex), so re-importing the same file is idempotent; `Message-ID` is also
  recorded and used to skip duplicates across files. Thread ids come from
  `X-GM-THRID` when present (Takeout), otherwise from `References` /
  `In-Reply-To` with a normalized-subject fallback, the way JWZ threading
  does it.
- Labels: Takeout's `X-Gmail-Labels` header (comma-separated; `Inbox`,
  `Sent`, `Starred`, `Unread`, `Important`, user labels with `/` nesting)
  maps onto the label table; `Unread`/`Starred` become the read/star
  flags. Files without the header get `INBOX` for received mail and
  `SENT` for mail from any address the user names at import time
  (pre-filled from the most frequent `From`). Label changes in an archive
  are **local only** and allowed: sorting an archive is a main use case.
- Attachments are extracted at import into the attachments table (data
  inline as today) so Quick Look, drag-out and the agent's attachment text
  tools work; nothing is fetched on demand because there is nowhere to
  fetch from.

**Import.** *Accounts › Create an Account from an Archived Mailbox…*, also
in Settings › Accounts (moved from *File › Import Mailbox…* on 2026-10-02,
since it adds an account; no shortcut: ⌘⇧I is already *Load Remote Images*) accepts an `.mbox` file or a folder
of them (Takeout splits large exports). A sheet asks for the account name,
the user's addresses (for `SENT` and reply detection) and shows size and an
estimate. The import runs on the core runtime: a streaming mbox reader
(`From ` separator lines, `>From ` unescaping, CRLF and LF), `mail_mime::
parse` per message, batches of 200 per store transaction, progress events
(`ImportProgress {account_id, done, total_bytes, bytes}`), cancellable
(what is written stays; re-running resumes by idempotent ids). Rough cost:
a 5 GB Takeout is ~5 GB on disk plus FTS, and tens of minutes.

**What works and what does not.**
- Works: sidebar, list, thread view, search (§8, local FTS is the whole
  point), labels (local), starring, read state, attachments, the agent
  panel with every read tool and the local label tools, local routines
  (they are agent sessions; the *Sort important mail* template runs
  locally against the archive), activity log and approvals.
- Disabled, with a one-line reason in place of the control: compose,
  reply, reply-all, forward, drafts, archive-to-server, trash to the
  server (trash is local: a `TRASH` label), *Publish to Claude* (no
  connector could reach the archive), sync status. `Sign In Again` and
  the sync window do not appear in Settings for an archive; *Re-import…*
  and *Remove…* do.
- Agent tools: `mail_send`, `mail_draft_*`, `mail_reply` return a
  structured "this account cannot send" error so an agent stops rather
  than retries. The system prompt says the account is an archive.

**Testing.** Synthetic mbox fixtures only (generated by a test helper from
`FetchedMessage`-style data, with and without `X-Gmail-Labels`, with
`>From ` escapes, a 20 MB attachment, and a corrupt message in the
middle). No real mailbox ever enters the repository or a test.

### 7.6 Provider abstraction

```rust
#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn profile(&self) -> Result<Profile>;
    async fn list_labels(&self) -> Result<Vec<RemoteLabel>>;
    async fn list_message_ids(&self, filter: ListFilter, page: Option<PageToken>) -> Result<IdPage>;
    async fn fetch_messages(&self, ids: &[RemoteId]) -> Result<Vec<RawMessage>>;
    async fn changes_since(&self, cursor: &SyncCursor) -> Result<ChangeSet>;
    async fn modify(&self, ops: &[LabelOp]) -> Result<()>;
    async fn send(&self, raw: &[u8], thread: Option<RemoteId>) -> Result<RemoteId>;
    async fn drafts(&self) -> &dyn DraftProvider;
    async fn fetch_attachment(&self, msg: RemoteId, att: RemoteId) -> Result<Bytes>;
}
```

`mail-sync` is written against this trait; `provider-gmail` is the only
implementation in MVP. `SyncCursor` and `RemoteId` are opaque so an IMAP
provider (UIDVALIDITY/MODSEQ) fits later.

### 7.9 Agent mailboxes **(Amendment 2026-10-06)**

An *agent mailbox* is an address that belongs to one of the user's agents,
hosted by an agent-mail service (Primitive first; AgentMail second). Agents
use it to sign up for services and to correspond on the user's behalf as
themselves. It is an account in every sense of §7.7 (its own directory,
store, writing guide, facts, routines, undo and entry behind the avatar
button), shown with an agent marker. The user reads it and can send as the
agent. ADR 0014, ADR 0015; plans `docs/plans/agent-mailboxes.md`,
`docs/plans/overnight-2026-10-08.md`.

**Service accounts (Amendment 2026-10-08, ADR 0015).** Agent mailboxes
belong to a *service account*: what the service calls an organisation
(AgentMail) or an account (Primitive). One service account holds the API
key, the human email it was verified with, the plan and its limits, and
the user's own domains; it has one agent or several, each an account of
its own. Neither service allows one account per mailbox: Primitive
refuses to verify a second account with an email that verified one
(`email_in_use`), and AgentMail keeps one organisation per human email
and rotates its key when the sign-up is repeated.

**Model.**
- The index (§7.7) gains the kind `agent` with `service` (`primitive`,
  later `agentmail`). `display_name` is the agent's name; it is the From
  display name.
- A service account is `services/<id>/service.json` in the data directory
  (service, human email, verified, the plan as last read, the managed
  domain agents take addresses on, own domains as last read). Its id is
  the account id of the agent it was created with. The plan, limits and
  verification state are read again from the service (`GET /account`)
  when an agent opens and after verifying; the record keeps the last
  answer.
- An agent's `agent.json` names its `service_account` and, on AgentMail,
  its `inbox_id`.
- The service account's API key lives in the Keychain as
  `mailbox.api_key.<service account id>` (§12). Removing an agent removes
  its account on the Mac; removing the last agent of a service account
  also deletes the record and the key.
- Mailboxes created before 2026-10-08 have no `service_account`: each is
  read as a service account of one agent whose id is the agent's account
  id, so the Keychain item keeps its name and nothing is re-keyed.
- The core holds a `MailboxService` per service (sign up, start and finish
  verification, plan and limits, add a mailbox) beside each agent's
  `MailProvider`, and chooses the provider by kind.

*(Implemented 2026-10-08, core: `agent_mailbox/service_account.rs`. The
migration happens on read and writes nothing by itself: `agent.json`
without `service_account` is read with the agent's own account id, and a
missing `service.json` is read from that agent's `agent.json` (service,
created, the managed domain from its address). The record is written the
first time it changes (a plan read, a verification, a domain, an agent
added, or the first agent removed while others remain), and `agent.json`
gains the field when next written; reading again gives the same answer.
FFI: `list_service_accounts` (id, service, human email, verified, the
plan as last read, the managed domain, agents' account ids in the
accounts' order), `agent_service_account`, `add_agent(service account,
name, domain, request id)` (the request id becomes the account id, so a
retry returns the same agent; `domain` is a verified own domain, else the
managed one), and `service_account_plan`,
`start_service_account_verification`, `verify_service_account`,
`find_service_account_code`, `service_account_api_key`,
`rotate_service_account_key`, `service_account_send_rules`,
`service_account_domains`, `add_service_account_domain`,
`check_service_account_domain`, `service_account_domain_zone_file`. The
per-agent calls (`agent_mailbox_plan`, `verify_agent_mailbox`,
`agent_domains`, …) remain as wrappers that resolve the agent's service
account. `MailboxService` gains `add_mailbox` (AgentMail's inbox; the
default says the service adds none) and `rotate_key` (default
unavailable: no Primitive endpoint is wired, so *Rotate Key* works only
where a service implements it). Removing an agent, or an orphaned store,
deletes the key only when no other agent on disk names its service
account; it waits for an agent being added, which checks the key is
still there once it may proceed, and a removal that fails part way
leaves its agent counted (amended 2026-10-08 after review, oagc-uys.23).
A retry of `add_agent` whose agent is on disk finishes its store and
index entry (oagc-uys.21). A new service account takes the id of its
first agent, as a migrated one does.)*

**Creating one.** *Accounts › Create an Agent Mailbox…*, also on the
welcome screen and in Settings › Accounts. A sheet:
1. Asks for the agent's name, and shows the service (only Primitive at
   first) with one line on what it is and its free tier.
2. Shows the service's terms as a link with *Agree and Create*. Primitive
   requires `terms_accepted: true`; the app sends it only from that button.
3. Calls the service's sign-up (`POST /v1/agent/accounts`, no
   authentication, an `Idempotency-Key` so a retry cannot make two
   accounts). The answer carries the API key and the address
   (`<name>@<sub>.primitive.email`). The account is registered and opened
   at once, and syncs.
4. Offers verification in the same sheet (*Verify with Your Email*), or
   later from a banner in the mailbox and from its settings.

No agent is involved: setup is a fixed sequence of calls in the core.
Signing up makes a new service account with its first agent.

**Adding an agent to a service account.** When a service account for
the chosen service exists, the sheet offers *Add to <service account>*:
the agent's name only, with no terms, sign-up or code, since the service
account already agreed and verified. A new service account stays
possible (Primitive refuses to verify it with an email already used).
- Primitive: no API call. The agent's address is its name as a local
  part on the service account's managed subdomain
  (`writer@jade-emu.primitive.email`), or on one of its verified own
  domains; the subdomain receives at any local part, and the account
  sends from any of its verified domains. An address another agent of the
  service account has is refused.
- AgentMail: a new inbox in the organisation (`POST /v0/inboxes`), whose
  `inbox_id` the agent keeps.

*(Implemented 2026-10-08, oagc-uys.14, app: `Features/Accounts/AgentMailbox.swift`
(`AgentMailboxFlow`). The sheet's first step is the service (Primitive,
AgentMail, a line each on what it is and its free tier); with a service
account for it, *Add to <service account>* (one per service account) or
*New Service Account…*; then the name. Adding shows the address it will
have (`name@<managed subdomain>`, or on Primitive a picker of the service
account's verified own domains; `name@agentmail.to`) and calls only
`add_agent`. A new AgentMail account asks for *Your email* (prefilled from
the open Gmail account, else the first; a menu picks another of the
user's accounts) above *Agree and Create*, and after the sign-up the sheet
is at the code step, since AgentMail emailed it: the core now notes the
sign-up as the moment a code was asked for, so *Fill Code from <address>*
finds it (from `agentmail.to`) as after *Send Code*. Settings' *Add
Agent…* opens the sheet at the name on that service account.)*

**AgentMail.** The second service (`https://api.agentmail.to`). Its
sign-up takes the agent's name and the user's email together, so the
sheet asks for the email before *Agree and Create* (prefilled from the
open account): without it the inbox only receives, and a lost key cannot
be recovered. The code is emailed at sign-up and *Fill Code from
<address>* works as for Primitive. Signing up again with the same email
would rotate the organisation's key, so the app never does it for a
service account it has. Labels sync both ways (read state, archive and
user labels, through the outbox); deleting stays local. Its sync, sending
and limits are specified with its provider.

*(Implemented 2026-10-08, oagc-uys.6: `crates/provider-agentmail`; core
`agent_mailbox.rs`, `service_account.rs`.)*
- **Sign-up.** `POST /v0/agent/sign-up {username, human_email, source}`:
  the username is the agent's name as a local part; the answer's `inbox_id`
  is the address (else `<username>@agentmail.to`) and is kept in
  `agent.json`. `create_agent_mailbox(service, name, human_email,
  request_id)` refuses AgentMail without an email, an email that already
  has an AgentMail service account on this Mac, and a request id that
  already holds a key without the sign-up's answer. The answer (address,
  inbox, plan) is kept in `services/<id>/sign-up.json` before the key is
  stored, so a retry after a failure part way finishes with that key, and
  a retry of a creation that got as far as its agent finishes its store
  and index entry (the file goes once registered; oagc-uys.21, .22). So
  the core never signs up twice for one organisation.
- **Verification.** The code is sent at sign-up. *Resend* is
  `POST /v0/agent/human` with the same email only (another email would
  replace the human, which AgentMail allows twice per organisation; the
  core refuses it). `POST /v0/agent/verify {otp_code}`; codes last 24
  hours. AgentMail's organisation (`GET /v0/organizations`) gives limits
  but not whether it is verified: the plan's name comes from its inbox
  limit (3 free, 10 developer, 150 startup) and the core keeps *verified*
  once a verification succeeded. Its per-hour and per-day fields are 0:
  AgentMail's limits are per month and per new recipient, worded by
  `agent_service_limits` / `service_account_limits` (the agent's prompt
  says the same). Until verified, the composer refuses recipients other
  than the human email.
- **Adding an agent** is `POST /v0/inboxes {username, display_name,
  client_id}` with the sheet's request id as `client_id`, so a retry
  returns the same inbox. A taken name and a full plan are said in words.
- **Inbox keys.** `agent_inbox_api_key(account)` makes a key for that
  agent's inbox only (`POST /v0/inboxes/{id}/api-keys`), once verified; a
  new key each call, not stored here.
- **Sync.** Message and thread ids are AgentMail's. Listing is per inbox,
  paged (`limit`, `page_token`); the Inbox and Sent are kept client-side
  by label, unread, starred and user labels go as `labels`, `newer_than`
  as `after`, Trash lists nothing. A message is fetched as its record
  (labels, time) and its raw MIME: `…/raw` answers with a signed
  `download_url`, downloaded without the key and parsed by `mail-mime`;
  without one, the record's own fields, attachments fetched on demand.
- **Labels.** `unread` ↔ `UNREAD` (marking read adds AgentMail's
  conventional `read`); `sent` ↔ `SENT`; a message without `sent` is
  received and in `INBOX` unless it carries the app's `archived` label
  (archive adds it, *Move to Inbox* removes it); `starred` ↔ `STARRED`;
  `spam` → `SPAM`; any other label is a user label whose id is its name.
  Changes go out through the outbox as `PATCH …/messages/{id}
  {add_labels, remove_labels}`, or `…/messages/batch-update` for up to 50
  messages. Trash, spam and delete stay on this Mac, including the Inbox
  removal that goes with them (no `archived` label for mail moved to
  Trash or Spam; amended 2026-10-08). Labels sync both ways
  (`LabelSync::Both`, amended 2026-10-08 after review, oagc-uys.17): a
  stored message fetched again (a resync, a refetch) takes AgentMail's
  read state, Inbox, Sent, stars and user labels, so changes the event
  list missed are repaired, and keeps what is only this Mac's: Trash and
  Spam (mail in either stays out of the Inbox) and store labels AgentMail
  cannot carry (`DRAFT`, `IMPORTANT`, categories). User labels survive the
  label refresh (there is no label listing at AgentMail). Gmail's labels
  stay the provider's (`LabelSync::Provider`) and Primitive's this Mac's
  (`LabelSync::Local`).
- **Changes.** The sync cursor holds the newest message time seen, the
  messages seen within an hour of it, and the newest label event seen. A
  poll (every 30 s while active, and at once on a push, below) lists
  `messages?after=<newest − 1 h>` and reports what it had not seen, then
  reads `…/events` newest first, page by page, down to the last event
  seen, and applies `label.added`/`label.removed` oldest first; other
  events in the list (`message.received`, …, without a top-level message
  id or label) are read past. `spam` or `trash` added also takes the
  message out of the Inbox, as the labels read when it is fetched; taken
  off, the Inbox comes back if AgentMail has the message there (one read
  of it, since archived or sent mail stays out). Not reaching the last
  event seen within 20 pages is an expired cursor (a full resync), and so
  is more than 20 pages of new mail since the last poll (the listing is
  newest first: stopping there would lose the older ones; amended
  2026-10-08 after review, oagc-uys.16, .18, .25).
- **Push** *(implemented 2026-10-08, oagc-uys.15; docs.agentmail.to
  websockets and its AsyncAPI, read that day)*. One WebSocket per
  organisation, `wss://ws.agentmail.to/v0?api_key=<key>` (the key also as
  `Authorization: Bearer`; AgentMail's AsyncAPI documents only the query,
  re-read 2026-10-08, so it stays), never logged: errors are redacted and
  `tungstenite`/`tokio_tungstenite` records are off in the log filter
  whatever `OPENAGC_LOG` asks, since tungstenite traces the handshake
  request (oagc-uys.20). It is shared by every agent of it
  that syncs: it sends `{"type":"subscribe","inbox_ids":[…],
  "event_types":["message.received","message.sent"]}` for every agent's
  inbox, ten to a message, and one more for an agent that starts later;
  the server answers `subscribed`. An `event` (`message`, `send`, … with
  an `inbox_id`) only wakes that agent's sync, which polls as above: the
  payload is not stored, and label changes still come from polling the
  event list, which the socket does not carry. The socket is the agent's
  push source (`AgentMailPush`, a `BackfillSource` whose `watch` waits
  for its inbox, as Primitive's long-poll does); the 30 s / 5 min poll
  runs alongside. On connecting or reconnecting every agent polls once
  (mail may have arrived meanwhile). It pings every minute and
  reconnects after 150 s without a frame; failures back off from 1 s to
  a minute, and five in a row (a refused key, an `error` answer, no
  network, or a connection dropped within 30 s of subscribing) rest it
  for 15 minutes, during which `watch` fails and the agents poll only. A
  connection dropped after that reconnects after the first backoff, not
  at once (every reconnection wakes every agent; oagc-uys.19). Inboxes
  added while it connects are subscribed: the queue of added inboxes is
  emptied before the list is read (oagc-uys.24). TLS is `tokio-rustls` with the webpki roots, as for
  IMAP (`tokio-tungstenite` without TLS features); plain `ws://` only to
  the loopback address. Spam, blocked and unauthenticated mail events
  need permissions a key may lack, which would fail the subscription, so
  they are not asked for. Tests use a local fake (`ws_fake`); nothing
  connects to AgentMail. Fake agent mailboxes, and tests with another
  API base and no fake socket, poll only.
- **Sending.** The composer's MIME becomes AgentMail's JSON (`to`, `cc`,
  `bcc`, `reply_to`, `subject`, `text`, `html`, base64 `attachments`,
  `headers`). A reply goes to `…/messages/<In-Reply-To>/reply` (explicit
  recipients, `reply_all` false) so AgentMail threads it; if AgentMail
  does not know that id, `…/messages/send` with `In-Reply-To` and
  `References` headers. Over 6 MB is refused before sending.
- **Never twice.** Each send carries `X-OpenAGC-Outbox-Id` (the
  composer's Message-ID, the same on every retry of that outbox entry) and
  an `Idempotency-Key` derived from it (AgentMail now offers one, kept 24
  hours; the plan expected none). The client never repeats a send itself.
  After an answer that leaves it unknown whether the mail went (a timeout,
  a 5xx), or when the message was queued more than ten minutes ago (an
  earlier run may have tried it), the next attempt first lists the
  inbox's mail since it was queued, looks for that header (fetching the
  messages with the same subject whose row lacks headers), and takes a
  match as the send.
- **Errors** are AgentMail's `{name, code, message, fix}`, read through
  `HttpClient`'s error hook: `message_rejected` before verification is
  said as "can write only to the email it was created with";
  `missing_permission` asks to verify first; `resource_taken`,
  `inbox_paused` and a full plan in words; `conflict` (a send with that
  key still running) and 429 are retried later.

**Verification.** Verification belongs to the service account: once
verified, every agent in it is. Until verified, a Primitive account is on
its `agent` plan: it can only reply to addresses that have already sent it
authenticated mail, at most 10 sends an hour and 50 a day. Verifying moves
it to the free `developer` plan, which still sends only to: people who
wrote to it first, the email it was verified with, its own verified
domains, Primitive addresses, and domains that opt in to agent mail (an
`_agents` DNS record). Other recipients are refused (found 2026-10-06
with the maintainer's account); the refusal is said in those words, and
the mailbox's settings say whom it writes to.
- The email field is prefilled with the current Gmail account's address
  (any of the user's accounts can be chosen). *Send Code* calls
  `POST /v1/agent/claim/start {email}`; the sheet then waits for a code,
  with *Resend* after the service's `resend_after_seconds`.
- **Fill from mail.** When that address is one of the user's accounts in
  OpenAGC, the app looks at that account's newest mail, received after
  *Send Code*, from the service's domain (`primitive.dev`), for a six-digit
  code, and offers *Fill Code from <address>*. Nothing is read beyond
  those messages and nothing is filled without the click. This is the one
  place the app reads one account's mail for another (§7.7), and only for
  the user.
- `POST /v1/agent/claim/verify {verification_code}`; a wrong or expired
  code is said in the sheet.

**Sync (Primitive).** Over its REST API (`https://api.primitive.dev/v1`,
the key as a Bearer token).
- Message ids are `in:<uuid>` for received mail (`/emails`) and
  `out:<uuid>` for sent mail (`/sent-emails`); thread ids are the service's
  `thread_id`, else the message's own id.
- Received mail is fetched as raw RFC 822 (`/emails/{id}/raw`) and parsed
  like any other. Sent mail has no raw form; it is rebuilt from the sent
  record (`/sent-emails/{id}`: headers, text and HTML bodies). Attachments
  of sent mail are listed but not downloaded.
- Labels: received mail gets `INBOX` and, when new, `UNREAD`; sent mail
  gets `SENT`. Archive, labels, stars, read state and trash are **local
  only**, as in an archive account (§7.8); nothing the user does deletes
  mail at the service. The labels list is the system labels.
- Changes: `GET /changes` (the cursor taken before the first listing, as
  §7.4 asks). `email.visible` and `sent_email.created` add a message,
  `email.deleted` and `sent_email.deleted` remove one, other kinds are
  ignored. A `410 cursor_expired` resyncs (changes are kept 7 days; an
  idle cursor stays valid). Mail already stored keeps its labels, read
  state and stars through a resync or a refetch: the provider says its
  labels are local (`label_sync` is `LabelSync::Local`), and its labels
  apply only to mail new to the store.
- A sent message takes the id `/send-mail` returned at once
  (`adopts_sent_copies`), since Primitive may give it a Message-ID of its
  own: the optimistic copy is never left beside the real one.
- Push: the same feed long-polls (`wait=20`) in place of IMAP IDLE, so new
  mail shows within a second or two while the app is open. Pages are at
  most 100 (Primitive answers 400 above that).
- A paused sync says why: the `SyncStatus` event carries the provider's
  message, and the sidebar footer shows it after "Trying again shortly".
- No drafts at the service: drafts stay on the Mac until sent. No server
  search: search is the local index (§8), which holds the whole mailbox.
- **Several agents on one account** (ADR 0015). Primitive lists all of an
  account's mail as one inbox, so each agent's provider keeps its share:
  received mail whose recipient is one of the agent's addresses (its
  address, and its managed one after *Use This Address*), sent mail whose
  From is one of them. Addresses compare without case and without a
  `+tag`. Mail to or from an address no agent has goes to the service
  account's first agent (the oldest); while the account has several
  agents it is labelled *To Other Addresses* (a local user label, listed
  in that agent's sidebar), and the To line says which address. Each
  agent long-polls the change feed with its own cursor.

*(Implemented 2026-10-08, oagc-uys.13: `provider-primitive/src/routing.rs`;
the core gives each provider a routing read from the agents on disk as it
syncs (`primitive_routing`), so an agent added or removed is seen without
restarting the others; when the first agent is removed, the next oldest
takes unclaimed mail. The recipient is the record's `to_email` (taken as
the envelope recipient), else the raw message's `Delivered-To`,
`X-Original-To`, `To` and `Cc`; a message with neither is the first
agent's. Listing rows that carry `to_email` or `from_header` are filtered
before fetching; the rest are decided when fetched, and another agent's
message comes back as not found, which the sync engine drops. With one
agent nothing is marked. `GET /changes` takes the client's cursor
(`since=`) and reading does not consume it, so the agents' long-polls do
not take changes from each other; every agent wakes on every change and
fetches the new records to decide (N agents read each new message's
record N times: accepted for now). The agents of a service account share
one rate limiter, as they share the key's request limit. Sends go from
the agent's own address whatever the composer's From says. The agent's
system prompt names the other agents whose sends count against the same
limits.)*

**Sending (Primitive).** `POST /v1/send-mail` with the From address, one
recipient, subject, text and HTML bodies, `in_reply_to` and `references`
for replies, attachments inline (base64, at most 30 MiB), and an
`Idempotency-Key` derived from the Message-ID so a retried send is sent
once. Limits shown as they are:
- **One recipient per message.** The service takes a single `to` and no
  Cc or Bcc. The composer says so when an agent mailbox has more than one
  recipient and will not send; agent tools get a structured
  `one_recipient_only` error.
- Bodies at most 256 KB together. An empty subject goes as
  "(no subject)": Primitive refuses an empty one, Gmail does not.
- Refusals from the service's gates (the agent plan's reply-only rule, its
  hourly and daily caps) come back as a failed send with the service's
  message, and the banner offers verification.

**Sending as the agent without approval.** An agent mailbox has a setting
in its account settings, *When Agents Send*:
- *Send freely; flag what breaks the guide* (default). On this account,
  `mail.send` and `mail.forward` run without approval (§10.3 amended).
  Each send is checked against the mailbox's writing guide when it goes
  (the guide's checks: banned and required phrases, length): what it
  breaks is told to the agent in the tool's result
  (`writing_guide_breaches`) and kept in the activity log with the send.
  Every send is recorded as an AI composition (ADR 0013).
- *Ask before each send*: the §10.4 approval flow, as on the user's own
  accounts.
`mail.delete` stays approval-gated either way. The user's own accounts
are unchanged. *(Amended 2026-10-08, oagc-uys.10:* a send from an agent
mailbox always goes through its outbox, even when its sync is not running,
so it reaches the service when sync next runs instead of being kept on
this Mac only. Agents outside the app get the same setting, §10.1.) The setting lives in the mailbox's `agent.json`
(`send_mode`). An agent working in the mailbox is told in its system
prompt whose mailbox it is, the name it sends as, and the service's
limits (one recipient per message).

**Account settings.** *(Amended 2026-10-08, ADR 0015: what is shared moves
to a service-account settings pane, reached from each agent's settings;
an agent's own settings keep its name, address, *When Agents Send* and
*Remove…*. The account switcher groups agents under their service
account, "Primitive · you@example.com". On AgentMail, once verified,
*Copy API Key* offers a key scoped to the agent's inbox and says the
organisation's key reaches every agent in it. *Rotate Key* replaces the
service account's key for all its agents at once.)*
*(Implemented 2026-10-08, oagc-uys.14: the switcher's sections are the
user's own accounts, then one per service account, titled "AgentMail ·
<the user's email>" or "Primitive · <its subdomain>" (an unverified
Primitive account has no email, and its subdomain names it); ⌃1–⌃9 follow
the menu's order. The service-account pane is a section of Settings ›
Accounts per service account, under the same title, holding what is
shared (service and plan, verified email or *Verify…*, the limits in the
core's words (`service_account_limits`), on Primitive *Can write to* and
*Domains* with *Add Domain…*, *Add Agent…*, *Copy API Key…*, *Open
<service>…*), followed by its agents' rows (name, address with *Use Your
Own Domain…* on Primitive, *When Agents Send*, *Remove…*). *Copy API
Key…* names the agents the key reaches; on verified AgentMail it offers
*Copy Key for <agent> Only* (`agent_inbox_api_key`) first. *Rotate Key*
is not shown: neither service implements it (Primitive has no endpoint
wired; AgentMail rotates only on a repeated sign-up, which the core never
makes), and it returns when one does. The app keeps plans by service
account (`servicePlans`), read once per service account per run; the
unverified banner reads its agent's service account's plan: Primitive's
line is from the plan's hourly and daily numbers, AgentMail's is the
first sentence of the core's limits (its plan reports 0 an hour and 0 a
day), and the composer of an agent mailbox shows the limits in full.)* Service, address,
plan and verification state with *Verify…*, *Can write to* (the service's send rules, `GET
/send-permissions`: anyone, addresses that wrote first, the user's own
domains, other Primitive mailboxes; sending to anyone is an entitlement
Primitive grants on request), *Open at primitive.dev…* (the dashboard's
sign-in; the help names the verified email to sign in as), *When Agents
Send*, *Copy API Key* (a confirmation says that
whoever holds the key can read and send the mailbox's mail), *Remove…*.
Removing deletes the account on the Mac, and the key with the service
account's last agent; the service account stays at the service (the
sheet says so). For the last agent of an AgentMail service account the
sheet also says that creating it again with the same email gives the
organisation a new key, so a key shared with *Copy API Key* or used on
another Mac stops working (amended 2026-10-08 after review, oagc-uys.26).

**Own domains.** *Use Your Own Domain…* in the mailbox's settings puts
the agent on a domain the user owns:
- The domain field suggests a subdomain of the user's own address
  (`agents.example.com`; nothing for shared hosts such as gmail.com). The
  agent receives all mail sent to the domain, so a subdomain leaves the
  user's own mail alone. Primitive refuses a domain whose mail goes
  elsewhere (`mx_conflict`) and one another account has claimed
  (`conflict`); both are said in plain words.
- `POST /v1/domains` answers with the records to create (MX, SPF, DKIM,
  DMARC, TLS-RPT, ownership). The sheet lists them (type, name, value, what
  each is for, found or not) with *Copy* on each and *Save Zone File…*
  (`GET /domains/{id}/zone-file`).
- *Check Now* (`POST /domains/{id}/verify`), and every 20 seconds while the
  sheet is open. Reopening the sheet picks up the domain where it was.
- Verified: the sheet offers the agent's address on it (its name, made an
  address) and *Use This Address*. The mailbox then sends and receives as
  that address (`agent.json` keeps the service's own address too, and the
  agent can go back to it); sync restarts with it.
- A domain belongs to the service account *(amended 2026-10-08, ADR
  0015)*: every agent in it may take an address on it, and Primitive
  lists everything sent to an account's domains as one inbox, which each
  agent's provider splits by recipient.

**Testing.** Every test runs against a wiremock fake of the service's API.
Nothing in automation calls a real service: each sign-up creates a real
account.

**Not in scope.** Agents outside the app sending at once while the app is
closed (they queue until it opens, §10.1; the helper in
`docs/plans/headless-mcp.md` would send), sending to several
recipients by splitting a message, deleting mail at the service, a
combined inbox of all agents, and services other than Primitive and
AgentMail.

---

## 8. Search

Search is local-only in MVP. The query language is a Gmail-compatible
subset so users and agents can reuse what they know:

```text
from:alice@example.com   to:me   cc:   subject:"q3 report"   has:attachment
label:Receipts   in:inbox|archive|sent|drafts|trash   is:unread|read|starred
after:2026-01-01   before:2026-02-01   newer_than:7d   older_than:1y
filename:pdf   larger:1M   smaller:100K   -term   "exact phrase"   OR
```

Parsing is a small hand-written recursive-descent parser in `mail-store`
producing a `SearchQuery` AST; the same AST is what `mail.search` (MCP)
accepts as structured JSON, so the agent can either pass a Gmail-style
string or fields. Free text goes to `messages_fts MATCH`; `from:`/`to:`
prefixes go to `addresses_fts` when they look partial, or to an exact
`participants.email` lookup when they contain `@` and no wildcard.

As-you-type search debounces 40 ms and cancels superseded queries (each
query carries a generation number; results for stale generations are
dropped in Rust before crossing the FFI).

---

## 9. Agent Architecture

### 9.1 Provider abstraction

```rust
#[async_trait]
pub trait AgentProvider: Send + Sync {
    fn id(&self) -> ProviderId;                        // "claude-code" | "codex"
    async fn detect(&self) -> AgentStatus;             // Installed{version}, NotInstalled, NotAuthenticated, Error
    async fn start_session(&self, cfg: SessionConfig, sink: EventSink) -> Result<Box<dyn AgentSession>>;
}

#[async_trait]
pub trait AgentSession: Send {
    async fn send(&mut self, turn: TurnInput) -> Result<()>;
    async fn cancel(&mut self) -> Result<()>;
    fn external_id(&self) -> Option<String>;           // for resume
}
```

`SessionConfig` carries: the MCP shim path and socket, the system-prompt
addendum, the allowed tool list, the model override, and limits
(`max_turns`, `max_budget_usd` where the provider supports it).

Nothing outside `agent-claude` / `agent-codex` knows a CLI flag.

### 9.2 Detection **(Verified)**

Detection runs at launch and when Settings › Agents opens; results are
cached for the session and re-checked on demand.

| | Claude Code | Codex |
|---|---|---|
| Locate binary | `PATH` lookup (login shell `$PATH` resolved once via `/bin/zsh -lc 'echo $PATH'`), then `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`; user-overridable path in Settings | same |
| Version | `claude --version` | `codex --version` |
| Auth | No status command exists. Probe: `claude -p "ping" --output-format json --max-turns 1 --tools "" --strict-mcp-config --mcp-config '{"mcpServers":{}}'`; `result.subtype == "error"` with `not_logged_in` → NotAuthenticated. Probe is run once per launch, off the main thread, with a 15 s timeout. | `codex login status` → exit 0 = authenticated (stdout says ChatGPT vs API key); exit 1 = not |
| Never | read `~/.claude/.credentials.json` or Keychain items belonging to Claude | read `~/.codex/auth.json` |

Minimum supported versions (the ones verified for this spec): Claude Code
2.1.x, Codex CLI 0.145+. Older versions show "Update required" in Settings.

### 9.3 Claude Code adapter **(Verified)**

Transport: one `claude` subprocess per **turn**, resumed by session ID.

```text
claude -p <prompt>
  --output-format stream-json --verbose --include-partial-messages
  --strict-mcp-config
  --mcp-config '{"mcpServers":{"openagc":{"type":"stdio","command":"<app>/Contents/MacOS/openagc-mcp","args":["--socket","<path>","--session","<id>"]}}}'
  --tools ""                      # no built-in Bash/Read/Write/Edit/Web tools
  --allowedTools "mcp__openagc__*"
  --permission-mode dontAsk       # anything not pre-allowed is denied, never prompted
  --append-system-prompt-file <bundle>/agent-system-prompt.md
  --max-turns 40
  [--model <user choice>] [--resume <session_id>]
```

- Events parsed from stdout NDJSON: `system/init` (capture `session_id`,
  verify `openagc` appears in `mcp_servers` with no error), `stream_event`
  (text deltas → `AgentEvent::TextDelta`; `tool_use` blocks →
  `ToolCallStarted`), `assistant`, `tool_result`, `result` (→ `TurnCompleted
  {cost_usd, usage}` or `TurnFailed`).
- Cancellation: send `SIGINT`, wait ≤ 3 s for `result`, then `SIGKILL`.
  SIGINT records the session so the next turn can `--resume`.
- `--permission-prompt-tool` is deliberately **not** used: its contract is
  not publicly documented, and OpenAGC's approvals happen inside the tool
  call anyway (§10). With `--tools ""` and `dontAsk`, the only thing Claude
  can do is call our MCP tools.
- Subscription note: as of September 2026 Anthropic permits Pro/Max
  subscriptions to be used through the CLI and Agent SDK, including from
  third-party hosts, but the policy has changed twice in 2026. Settings ›
  Agents › Claude offers an optional `ANTHROPIC_API_KEY` (stored in
  Keychain, injected into the subprocess environment) as the fallback. The
  README states this plainly.

### 9.4 Codex adapter **(Verified)**

Transport: one long-lived `codex app-server` subprocess per app launch
(started lazily), JSON-RPC 2.0 over stdio, newline-delimited, `"jsonrpc"`
field omitted on the wire as the protocol specifies.

*(Amended in M3, verified against codex-cli 0.145: one app-server per
OpenAGC **session**, since the MCP server's `--session` binding is
process-level configuration. `--ignore-user-config` does not exist; the
adapter replaces the whole `mcp_servers` table with `-c` and turns off the
shell, exec, browser, apps, plugins, hooks and other features with
`--disable`. `tools.web_search`/`tools.view_image` are not valid keys;
`web_search="disabled"` is. The flag set was checked with
`--strict-config`; see `crates/agent-codex/schema/README.md`.)* This is the protocol
the VS Code extension uses. It is labelled experimental by OpenAI but is
the only path that gives host-mediated approvals and interruption;
`codex exec` has neither and `codex mcp-server` was removed in 0.154.

Configuration is passed as `-c` overrides plus `--ignore-user-config` so the
user's own MCP servers and tools are not exposed to the mail agent:

```text
codex app-server --listen stdio:// --ignore-user-config
  -c 'mcp_servers.openagc.command="<app>/Contents/MacOS/openagc-mcp"'
  -c 'mcp_servers.openagc.args=["--socket","<path>","--session","<id>"]'
  -c 'mcp_servers.openagc.default_tools_approval_mode="auto"'
  -c 'features.shell_tool=false' -c 'features.unified_exec=false'
  -c 'tools.web_search=false' -c 'tools.view_image=false'
  -c 'sandbox_mode="read-only"' -c 'approval_policy="never"'
  -c 'cli_auth_credentials_store="auto"'
```

Session flow: `initialize` (clientInfo `openagc`, `experimentalApi: true`) →
`initialized` → `thread/start {cwd: <per-session temp dir>, sandbox:
"readOnly", approvalPolicy: "never", ephemeral: false}` → per prompt
`turn/start {threadId, input:[{type:"text", text}]}`. Notifications
`item/agentMessage/delta`, `item/started`, `item/completed`
(`mcp_tool_call` items → tool events), `turn/completed {usage}` map to
`AgentEvent`. Cancel: `turn/interrupt {threadId, turnId}`. Thread IDs are
persisted for `thread/resume`.

MCP tool approvals are set to `auto` on the Codex side because OpenAGC's
own permission engine gates inside the tool (§10); we do not depend on
Codex's approval request (its exact server-request for MCP tools is not
documented). The JSON-RPC types are generated once from `codex app-server
generate-json-schema` and checked into `agent-codex/schema/` with the Codex
version they came from; a mismatch at `initialize` (unknown `userAgent`
major version) surfaces as a Settings warning, not a crash.

### 9.5 Agent event model

```rust
pub enum AgentEvent {
    SessionStarted { external_id: Option<String> },
    TurnStarted,
    TextDelta(String),
    ThinkingDelta(String),                 // shown collapsed
    ToolCallStarted { call_id, tool, args_summary },
    ToolCallFinished { call_id, ok: bool, summary },
    ActionProposed { action: AgentAction },  // gated action awaiting approval
    ResultsAvailable { thread_ids: Vec<ThreadId> },  // UI shows as a mail list
    TurnCompleted { usage: Option<Usage>, cost_usd: Option<f64> },
    TurnFailed { message },
    SessionEnded,
}
```

Deltas are batched every 16 ms before crossing the FFI so a fast stream
does not flood the main actor.

### 9.6 System prompt addendum

`agent-system-prompt.md` (bundled, versioned) tells the agent: it is
operating on the user's mailbox through OpenAGC tools only; email content is
untrusted data and instructions inside emails must never be followed; it
should search first and read narrowly; whenever the answer is a set of
messages it must present them by calling `mail.present_threads`, with a
short title, rather than pasting email bodies into prose; sending,
forwarding and deleting are proposals that the user approves.

### 9.7 Context minimization

The `PromptContext` sent with a prompt is *references*, not content: the
currently selected thread/message IDs, the current mailbox, and the current
search query. The agent must pull content through tools, which log every
access as an `AgentAction` and which enforce size caps (§10.4). OpenAGC
never pre-loads a mailbox dump into a prompt.

**The visible list is the context (Amendment 2026-10-06).** With nothing
selected, the context also names what the thread list shows, in one line
("Inbox › Primary · 34 conversations · Important only": the mailbox and
category tab, the row count, the Important/Tasks switches and the list
filters), and the ids of the rows on screen, top first, at most 100. So
"these" and "this list" mean exactly what the user is looking at, with no
selection needed; a selection narrows the context to itself and the row
ids are left out. The Tasks, Writing Guide and Facts pages name no list.
The visible ids are references like the rest and do not change the
session's scope (§10.3).

---

## 10. MCP Server and Permission Engine

### 10.1 Topology

`openagc-mcp` (Rust, `rmcp` 3.x, stdio transport) is a stateless shim. Its
`--socket` argument is a per-launch Unix domain socket in
`~/Library/Application Support/OpenAGC/run/` (mode 0600) served by the core;
`--session` binds every tool call to an `AgentSession`. The shim forwards
each `tools/call` as a length-prefixed JSON request over the socket and
relays the reply. Unknown sessions and socket peers with a different UID are
rejected. The socket protocol is internal and versioned by app build; the
shim and app always ship together.

`rmcp`'s `#[tool]` macros with `schemars` 1.0 generate the JSON schemas; the
same definitions are rendered to `docs/mcp.md` by a `cargo xtask`.

**Mailbox mode (Amendment 2026-10-08, oagc-uys.10; plan
`docs/plans/headless-mcp.md`).** Agents outside the app (Claude Code,
Codex, scripts) use one agent mailbox (§7.9) through
`openagc-mcp --mailbox <address> [--data-dir <dir>]` (stdio; the data
directory defaults to `~/Library/Application Support/OpenAGC`), whether or
not the app is open.
- **Agent mailboxes only.** The address is looked up in the agents'
  `agent.json` files (address or managed address, any case) and must be
  listed in `accounts/index.json`. The user's own accounts, imported
  mailboxes and unknown addresses are refused before anything is served
  (stderr, exit status 2), and again by the core.
- **Tools** (`docs/mcp.md`, *Mailbox mode*): `guide_rules` (the writing
  guide for the given recipients and type, whose mailbox it is, the name
  it sends as, the service's limits and its send mode), `facts_lookup`,
  `mail_search` and `mail_get_thread` (the in-app tools), and `mail_send`
  / `mail_reply`, which write and send in one call: the core's
  `mail_create_draft` then `mail_send`, so the permission engine, the
  session's rate limit, the draft-ownership rule, the guide check
  (`guide_check` in the answer, and `writing_guide_breaches` when sent
  freely) and the ADR 0013 record (source agent, agent
  `outside:<client>`) are the in-app ones. A send that is declined, times
  out or fails leaves no draft behind.
- **The app running.** At launch the app binds the agent socket
  (`serve_outside_agents`) and writes its path to `<data dir>/run/mcp-socket`.
  The shim connects with a hello that names the mailbox and its MCP
  client's name (from `initialize`); the core opens an *outside session*,
  `outside-<client>-<random>`, bound to the mailbox's account, and runs
  each call through it. *When Agents Send* applies: sent freely, or the
  §10.4 approval, shown in the open window's agent panel (whichever
  account it shows) with who asks and from which mailbox ("Claude Code
  outside OpenAGC, as scout@…: Send …"). The activity log (§10.5) records
  every call under the outside session; its *Agent* column names it.
- **The app closed.** The shim runs the core headless in its own process
  (`Core::headless`): no secrets (its secret store refuses every read; the
  Keychain is the app's, §12), no events, no log file, no sync and no
  outbox drain. Stores are opened with `Db::open_existing`: never created
  or migrated, refused in words unless their schema is exactly this
  build's ("from an older OpenAGC … open OpenAGC once to update it", or a
  newer one, "update OpenAGC"); reads use read-only connections and only
  a send opens the writer. Reads are not written to the activity log (the
  store stays read-only); sends are, under the outside session. A send is
  checked and recorded as above, then queued in the outbox
  (`mail_sync::send_draft`, one write transaction beside a running app,
  §7.4 outbox claims; no Undo Send hold) and answered
  `{"queued": true, "message": "Queued. It goes out when OpenAGC next
  opens."}`. The app's sync sends it, once, when it next runs. *Ask before
  each send* is refused (`needs_openagc`): approvals are parked in the
  app's memory and cannot outlive or cross processes. The user's *Ask
  Before* choices for reversible tools live in the app's preferences and
  do not apply here; the defaults do (a draft is allowed).
- **Switching.** Each call goes to the app when its socket answers, else
  to the headless core: an app opened later is used from the next call.
  If the app quits during a call, a read is answered headless; a send is
  not tried again (it may have been queued) and answers `app_unavailable`.
- **Connect an Agent…** in an agent's row of Settings › Accounts writes the
  MCP entry for Claude Code (the user-scope entry `claude mcp add --scope
  user` makes: `mcpServers.openagc-<local part>` in `~/.claude.json`,
  `{"type": "stdio", "command": <app>/Contents/MacOS/openagc-mcp, "args":
  ["--mailbox", <address>]}`) or Codex (`[mcp_servers.openagc-<local
  part>]` in `~/.codex/config.toml`, or `$CODEX_HOME/config.toml`, with
  `command`, `args` and `tool_timeout_sec = 900`). The sheet shows the
  exact entry and file first; writing copies the file to
  `<file>.openagc-backup-<time>`, replaces an entry of the same name
  (never a second one), keeps the rest of the file and its permissions,
  and refuses a file it cannot edit safely (not JSON, or TOML that would
  not read back with the entry). The command to paste instead (`claude mcp
  add …` / `codex mcp add …`) is always shown.

### 10.2 Tool set (MVP)

*(Amended in M3: tool names use underscores, `mail_search` rather than
`mail.search`, because the Anthropic and OpenAI APIs only accept
`[a-zA-Z0-9_-]` in tool names. Dotted names below map one-to-one.)*

| Tool | Risk | Description |
|---|---|---|
| `mail.search` | ReadOnly | Query string or structured `SearchQuery`; returns thread summaries (id, subject, participants, date, snippet, labels, unread). Max 50 per call, cursor for more. |
| `mail.get_thread` | ReadOnly | Messages in a thread with `text_plain` bodies (HTML converted), truncated per message at 20 KB with a `truncated` flag; attachments listed as metadata. |
| `mail.get_message` | ReadOnly | One message, same shape; `include_quoted: bool` (default false strips quoted replies). |
| `mail.list_labels` | ReadOnly | Labels with counts. |
| `mail.get_attachment_text` | ReadOnly | Extracted text for `text/*`, PDF (via PDFKit in the app, through a foreign trait; *amended in M3: `pdf-extract` depends on the unmaintained `ttf-parser` and would parse untrusted PDFs in-process*), and `.docx`; cap 100 KB. No binary bytes are ever returned. |
| `mail.present_threads` | ReadOnly | Instructs the UI to show a result set; returns nothing. |
| `mail.create_draft` | Reversible | Reply or new; body as Markdown, converted to HTML+text by the core. Returns draft id. |
| `mail.update_draft` | Reversible | |
| `mail.archive` | Reversible | Thread ids, max 200 per call. |
| `mail.mark_read` / `mail.mark_unread` | Reversible | |
| `mail.add_label` / `mail.remove_label` | Reversible | User labels only; `SPAM`/`TRASH` are refused here. |
| `mail.create_label` | Reversible | Name (nested with `/`), optional color; idempotent — returns the existing label if present. Needed by routines (§11). |
| `mail.send` | External | Sends an existing draft id. **Always** approval-gated, except on an agent mailbox set to send freely (§7.9). |
| `mail.forward` | External | Creates a forward draft and requests send approval in one step. |
| `mail.delete` | External | Moves to Trash (never permanent). Approval-gated. |

Not exposed in MVP: raw HTML, attachment bytes, account settings, anything
that reaches the filesystem or the network.

### 10.3 Permission engine

`permissions` is a pure crate: `fn decide(policy: &Policy, action: &ProposedAction) -> Decision`
where `Decision ∈ {Allow, RequireApproval, Deny}`.

Default policy:

| Risk | Default | Configurable to |
|---|---|---|
| ReadOnly | Allow | — |
| Reversible | Allow | RequireApproval (per tool) |
| External | RequireApproval | — (cannot be set to Allow in MVP; *amended 2026-10-06:* `send` and `forward` are Allow on an agent mailbox set to send freely, §7.9) |

Every tool call passes through `decide` **inside the core, before the store
is touched**. This is the only enforcement point; nothing in the agent CLIs
is trusted to enforce anything.

Additional hard limits, independent of policy:

- Bulk caps: a single Reversible call may touch ≤ 200 threads; a session may
  touch ≤ 2,000 without a fresh user prompt.
- Rate: ≤ 60 tool calls per minute per session.
- `mail.send` requires the draft to have been created in the same session
  (or an explicitly user-attached draft), and the recipients must match
  what the user sees in the approval sheet at approval time (the draft is
  frozen while pending).
- Sessions run with `SessionConfig.scope`: `Mailbox` (default, all mail) or
  `Selection` (only the thread IDs passed in `PromptContext`); read tools
  outside the scope return an empty result.

### 10.4 Approval flow

1. Tool call arrives with `RequireApproval` → core inserts an `agent_actions`
   row in state `Pending`, emits `ApprovalRequested`, and **parks the tool
   call** (the MCP request stays open).
2. The UI shows the approval inline in the agent panel (§14.6). For sends,
   the sheet shows the full rendered draft with recipients; the user may
   edit, which updates the draft.
3. The user approves/rejects → `resolve_approval` → the parked call resumes
   and executes, or returns an MCP error `{code: "rejected_by_user"}` that
   the agent sees as a normal tool error.
4. Timeout: 10 minutes pending → auto-reject with `{code: "approval_timeout"}`.
   Cancelling the session rejects all pending actions.
5. Batch: proposals arriving within 2 s of each other are grouped in one
   sheet with per-item checkboxes; "Approve all" still records one
   `AgentAction` per item.

Both agent CLIs tolerate long-running tool calls (their MCP tool timeouts
are configured to 15 minutes for the `openagc` server).

### 10.5 Audit

Every tool call, its decision, and a one-line result summary is an
`agent_actions` row. Settings › Agents › Activity lists them and can export
JSONL. Read tools record which thread/message IDs were returned, so "what
did the agent see?" is always answerable. *(Amended 2026-10-08:* reads by
an agent outside the app while the app is closed are not recorded, since
that process keeps the store read-only; its sends are, §10.1.)

### 10.6 Rules server **(Amendment 2026-10-08, ADR 0016)**

*(Decided 2026-10-08; not built.)* Plan `docs/plans/rules-server.md`.

Cloud agents (a Claude cloud routine, a ChatGPT task, an agent on another
machine) cannot reach the app or run `openagc-mcp` on the Mac. A rules
server gives them an agent mailbox's guide and facts, checks their
drafts and takes their reports. It serves agent mailboxes only (§7.9);
the user's own accounts are never published.

**Topology.** `openagc-rules` (crate `rules-server`) is one binary with
one SQLite file, separate from the app. It speaks MCP over Streamable
HTTP to agents and a small REST API to the app and scripts, from one
handler set. It runs plain HTTP behind the user's TLS proxy (the docs
show Caddy). It depends on a pure crate, `writing-guide`, holding the
guide's deterministic check, the guide and facts renderers and the
snapshot format, which `openagc-core` uses too, so both answer alike. It never depends on
`openagc-core`. The app is the source of truth; the server holds copies.
One server may hold several mailboxes, each apart from the others.

**Tools.** The names and answers are mailbox mode's (§10.1), so an
agent's instructions work with either. There are no mail tools: a cloud
agent reads and sends through the service with its key.

| Tool | Input | Answer |
|---|---|---|
| `guide_rules` | `to`, `message_type` | The guide for those recipients and that type, whose mailbox it is, the name it sends as and the service's limits, from the snapshot, with its version and when it was published |
| `facts_lookup` | `category`, `query` | The matching facts shared with cloud agents, with the version |
| `check_draft` | `to`, `message_type`, `subject`, `body_markdown` | `guide_check`: what the draft breaks (banned and required phrases, patterns, length). Deterministic; no model calls |
| `report_send` | `message_id` or `to`, `subject`, `sent_at`, body, and the snapshot version it was checked against | `{"queued": true}`; the report waits for the app |

REST: `PUT` a snapshot (publisher token), `GET` and `DELETE` reports
(publisher token), and read-only `GET`s of the guide and facts (agent
token).

**Tokens.** Every request carries a token as `Authorization: Bearer`.
- *Publisher token:* one per mailbox per server, made when the app
  first publishes and kept in the Keychain (§12). Only it can push a
  snapshot or pull reports.
- *Agent tokens:* minted in the app (*Connect a Cloud Agent…*), named by
  the user ("Weekly outreach routine"), scoped to one mailbox, shown
  once, revocable. The server stores only a hash; the app keeps the id
  and name. A report names its token, so the activity log says which
  agent sent.
- *OAuth* for clients that take only a URL: the server is its own
  minimal authorization server; its consent page asks for a one-time
  connect code shown in the app, never a password. There are no user
  accounts on the server. A claude.ai custom connector, which a cloud
  routine uses, can send a fixed `Authorization` header only where the
  *Request headers* beta is offered (checked 2026-10-08), so OAuth comes
  before *Connect a Cloud Agent…* in the build.
- A secret URL (`/m/<token>/mcp`) is the fallback for connectors with no
  sign-in; URLs end up in logs, and the sheet says so.

**Snapshot and versions.** The app pushes a full snapshot on change,
debounced, with a version that only goes up and `If-Match` on the
previous one. The app is the only writer: a refused push re-reads the
server's version and pushes again. Every answer carries the version and
its age; with the app closed nothing changes, and agents see the guide
"as of" a time. The server keeps the last few versions, so a report can
name the one it was checked against. Settings shows "Version 12,
published 3 minutes ago".

**Reports and retention.** Reports carry what the agent wrote, never
mail it read. The app pulls them at sync, matches them to sent mail by
Message-ID and records them as AI compositions (§14.10, agent
`cloud:<token name>`), so the daily review sees them. A send with no
report is still reviewed. The server deletes a report once the app has
pulled it, and every report after 30 days regardless. Proposals from
cloud agents will use the same queue later.

**What is shared.**
- Accepted rules and guidelines, with their scope and checks; never
  evidence quotes, which come from sent mail.
- Audience groups, with each address as a salted hash: the salt goes
  with the snapshot, and the server hashes the `to` it is given before
  matching. The hash is lower-case hex SHA-256 of the salt, a zero byte
  and the trimmed, lower-cased member (an address, or `@domain` for a
  domain); a recipient matches by its address or its `@domain`.
- Facts, by a *Share with cloud agents* switch on each fact (§14.11). On
  by default for the mailbox's own *Use freely* facts; off by default
  for *Ask before using* (an unattended agent cannot ask) and for global
  facts (ADR 0012); never for *Never share*.
- Never mail, keys or tokens of any service.
The publish sheet lists exactly what goes before the first push.

**Encryption at rest.** The snapshot is encrypted with a key
(`rules.snapshot_key.<account>`, §12) wrapped for each agent token; the
server unwraps it in memory for a request and never stores it. A leaked
database or backup shows nothing. Required on the project-hosted server,
optional when self-hosted. It does not protect against an operator who
changes the code, and the docs say so.

**Sending.** The server never sends and never holds a service key, an
OAuth token or mail. An agent calls `check_draft`, fixes what breaks,
sends through the service with its key, then calls `report_send`.

**Hosting.** A static binary, and a Docker image on GitHub's registry
built from this repository's releases. The project-hosted server is the
same image, versions and settings; it comes only after self-hosting
works, is priced at cost, and charges for running it (machine, domain
and TLS, backups, uptime, abuse handling), never for features. No
feature and no build flag exists only there.

---

## 11. Routines — Scheduled Mail Sorting

A **routine** is a recurring agent task that files automated mail into
cadence-based labels so the inbox holds only mail that needs a person.
OpenAGC ships one template, lets the user reshape it with a structured
editor, and runs it either on the AI vendor's cloud (the default the
maintainer asked for) or locally through OpenAGC's own agent stack.

### 11.1 What the platforms actually allow **(Verified)**

There is no *public* API for creating routines on either vendor, and
Anthropic's policy prohibits third-party apps from holding claude.ai
credentials. But the user's own Claude Code CLI can manage routines, and
OpenAGC already drives that CLI for everything else. That is the path.

| Runner | Runs where | Gmail access | Can OpenAGC create/edit it? | Trigger it? | Read run logs? |
|---|---|---|---|---|---|
| **Claude cloud routine** (claude.ai/code/routines) | Anthropic cloud, hourly minimum, fully autonomous; no repository required | claude.ai Gmail connector (`gmail.modify`; write tools incl. `label_thread`, `unlabel_thread`, `create_label`) | **Yes, through the user's `claude` CLI.** Headless `claude -p` exposes a built-in `RemoteTrigger` tool (`list`/`get`/`create`/`update`/`run`/`list_runs`/`get_run_log`) that calls `/v1/code/triggers` with the CLI's own login. Verified 2026-09-23: `claude -p … --allowedTools RemoteTrigger` returned the account's routines with HTTP 200. Requires a claude.ai subscription login in the CLI (not an API key). The API is internal and undocumented, so a paste hand-off remains the fallback | Yes — `RemoteTrigger run`, or the documented per-routine fire endpoint | Yes — `list_runs` + `get_run_log` via the same tool |
| **Claude Desktop scheduled task** | On the Mac inside Claude Desktop (a local `SKILL.md`; the maintainer has one, currently disabled in favor of the cloud routine) | Same connector | No supported API; not targeted | No | No |
| **ChatGPT scheduled task** | OpenAI cloud (web/mobile tasks); Codex desktop "automations" are local-only and Codex Cloud cannot schedule | OpenAI Gmail app (`gmail.modify`; approval semantics for unattended writes not documented) | **No** — hand-off only | No | No |
| **OpenAGC local runner** | On the Mac, in OpenAGC, using the user's `claude`/`codex` CLI and OpenAGC's MCP tools | OpenAGC's own store + outbox | Yes — it is ours | Yes | Yes — full transcript and per-thread audit |

Consequences:

- **Claude cloud is first-class.** OpenAGC creates, updates, enables,
  runs and inspects the routine by spawning the user's `claude` binary
  with a `RemoteTrigger` instruction (§11.5). OpenAGC never sees a
  claude.ai credential; the CLI does the call, exactly as it does for
  agent sessions. Because the endpoint is internal, the adapter is
  isolated in `agent-claude::routines`, feature-flagged, and degrades to
  the paste hand-off if the tool disappears or returns an error.
- The routine JSON OpenAGC produces is the shape the CLI already uses
  (verified from the maintainer's live routine): `name`,
  `cron_expression`, `enabled`, `job_config.ccr.{environment_id, events[],
  session_context.{model, allowed_tools}}`, `mcp_connections[]` naming the
  Gmail connector. `session_context.allowed_tools` is set to `[]` plus
  nothing — the routine needs only the connector; the CLI's defaults add
  `Bash/Read/Write/…` which the routine does not need and should not have.
- **ChatGPT** is integrated by hand-off: OpenAGC puts the prompt on the
  clipboard and opens the creation surface.
- OpenAGC learns what a cloud run did two ways: the run log through
  `get_run_log` (Claude only) and, for every runner, **from Gmail itself**:
  the next history sync sees labels applied and `INBOX` removed by an actor
  other than OpenAGC's outbox (§11.6).
- The **local runner** is the only path where OpenAGC's permission engine
  applies in full, and it is the "preview classification" engine for
  editing a routine before publishing it.

### 11.2 Routine model

A routine is data, not a prompt. The prompt is generated.

```text
Routine
  id, name, enabled, template_id ("sort-important" | custom)
  runner            ClaudeCloud | ClaudeDesktop | ChatGPTCloud | Local
  schedule          RRULE (RFC 5545) — rendered to cron / natural language per runner
  agent             Claude | Codex   (local runner only; cloud implies the vendor)
  scope             SearchQuery      default: is:important newer_than:1d -in:sent -in:draft
  parent_label      "Marked Important"
  leave_alone       LeaveAloneRules  { human_threads: true, replied_by_me: true, starred: true, spam_trash: true, custom: [text] }
  buckets           [Bucket]         ordered
  unmatched         LeaveAndReport | ApplyLabel(label)
  report            ReportSpec       { counts: true, list_individually: [bucket ids], max_lines: 15 }
  identity          { primary_email, aliases: [email], frequently_cc: [email] }   filled from the account; editable
  limits            { max_threads_per_run: 200, get_thread_only_when_needed: true }
  advanced_prompt   Option<String>   set only when the user edits the generated prompt by hand; freezes generation
  cloud             { trigger_id?, routine_url?, environment_id?, published_fingerprint?, published_at? }

Bucket
  id, order, label_name ("1-Daily"), color (LabelColor), cadence (Daily | Weekly | Monthly)
  title, description        one paragraph: what belongs here
  positive_examples [text]  sender domains / subject shapes
  negative_examples [text]  "not this — see bucket X" cross-references
  priority_when_ambiguous   e.g. "alert AND receipt → Daily"
  list_individually_in_report bool
```

Stored in `routines` (JSON payload column) and `routine_runs` in the
account database (§6.2 additions below). `routines.sync_fingerprint` hashes
the generated prompt so the UI can show "changed since last published to
cloud".

### 11.3 The shipped template — *Sort important mail*

Derived from the maintainer's production *cloud* routine (the hourly one,
which is the refined successor of the local Desktop task), generalized:
identity is filled from the connected account, company-specific examples
become generic ones, everything else (six buckets, leave-alone rules,
report shape) is kept because it is the tested behavior. Refinements the
cloud version added and the template keeps: an unattended-run preamble
("execute autonomously, only take the write actions this task asks for,
never send/reply/trash/spam, when in doubt report"); already-sorted
threads that resurface because a new message arrived get the *same* label
re-applied and are re-archived, never re-classified or counted as new;
transient label errors are retried once; an empty run says so in one
line.

| # | Label | Color | Cadence | Contents |
|---|---|---|---|---|
| 1 | `1-Daily` | red | daily | automated mail needing human action soon: service/infra alerts, failed or declined payments, expiring cards, suspension warnings, deadlines within a week, legal/compliance notices, genuinely actionable "Action Required". Alert *and* receipt → here |
| 2 | `2-Weekly-Newsletters` | blue | weekly | newsletters, digests, mailing lists, industry roundups, vendor marketing the user did not ask about |
| 3 | `3-Weekly-Events` | green | weekly | platform event invitations and logistics: meetups, conferences, webinars, ticketing invites, RSVP notifications, early-bird deadlines. Event mail from a named collaborator is human correspondence → leave alone |
| 4 | `4-Weekly-Finance` | orange | weekly | receipts, invoices, statements, payment confirmations, renewals, expense tools. Successful and routine only; failed/overdue → 1-Daily |
| 5 | `5-Monthly-Pitches` | gray | monthly | cold outbound, sales follow-ups, recruiting-service and sourcing blasts, sponsorship solicitations, unsolicited demo requests |
| 6 | `6-Weekly-Hiring` | purple | weekly | inbound applicants for the user's own roles: new-application notifications, ATS/job-board candidate notices, interview-scheduling bots. Direction matters: candidates *in* → here; recruiting *vendors* → 5-Monthly-Pitches; a named candidate writing directly → leave alone |

Leave-alone rules (default on): genuine person-to-person threads ("would a
person notice and care that no one replied?"), anything the user has
replied to from any of their addresses, starred threads, spam/trash. The
tie-break sentence — *wrongly deferring a real conversation is much worse
than leaving one extra message in the inbox* — is part of the template
and shown in the editor as the principle behind the rules.

Report: counts per bucket plus untouched count; `1-Daily` and
`6-Weekly-Hiring` listed individually; unmatched automated mail listed so
buckets can be tuned; errors and the 200-thread cap reported.

Template versions are bundled as `routines/sort-important.v1.json`; a
routine remembers which template version it started from so OpenAGC can
offer "template updated — review changes" later without overwriting edits.

### 11.4 Prompt generation

`routine-prompt` (a module in `agent-api`) renders `Routine` to Markdown
in this fixed order, which mirrors the proven structure of the source
routine: unattended-run preamble → purpose → identity → tool map → Step 1
ensure labels → Step 2 find candidates (including the already-sorted
re-apply rule) → Step 3 leave alone → Step 4 sort (one paragraph per
bucket, in order, with cross-references; one retry on transient errors)
→ Step 5 report → hard rules ("never trash, delete, or mark as spam";
"stop on permission errors and say so"; "stop at N threads and say so").

The prompt is tool-agnostic except for a short **tool map** preamble
generated per runner, because each surface names Gmail operations
differently:

| Operation | Claude connector | ChatGPT Gmail app | OpenAGC MCP (local) |
|---|---|---|---|
| list labels | `list_labels` | (search/read tools; label names) | `mail.list_labels` |
| create label | `create_label` (colorPreset) | not available → prompt says "if a label is missing, report and stop" | `mail.create_label` |
| search | `search_threads` (pageSize, THREAD_VIEW_MINIMAL) | `search_emails` / `search_email_ids` | `mail.search` |
| read | `get_thread` | `read_email_thread` | `mail.get_thread` |
| apply label | `label_thread` (IDs) | `apply_labels_to_emails` | `mail.add_label` |
| archive | `unlabel_thread` with `INBOX` | `apply_labels_to_emails` removing INBOX where supported, else report | `mail.archive` |

The ChatGPT column is the least verified (OpenAI publishes no tool
reference); its map is marked *best-effort* in the UI and the prompt tells
the agent to describe its available tools in the report if any named tool
is missing.

Generated prompts are deterministic for a given `Routine` + runner +
template version, and are snapshot-tested (§18).

### 11.5 Client UX

**Settings › Routines** (also reachable from the sidebar as a "Routines"
section listing each routine with its last-known activity).

- **List**: name, runner badge (☁︎ Claude / ☁︎ ChatGPT / ⌘ Local / Desktop),
  schedule in words, last activity ("Sorted 84 threads · 2 h ago" from
  §11.6 attribution), enabled toggle.
- **Editor** (structured):
  - *Schedule*: presets (hourly, every morning at…, weekdays, weekly) plus
    custom RRULE; a runner-specific note ("Claude cloud: hourly minimum,
    UTC, may drift a few minutes").
  - *Scope*: search query with a live "matches N threads right now" count
    from the local store.
  - *Leave alone*: toggles plus free-text extra rules.
  - *Buckets*: reorderable cards, each with label name, color swatch,
    cadence, description, examples, "not this" cross-references, and a
    "list individually in report" toggle. A *Preview classification*
    button runs the local runner in dry-run mode over the current scope
    and shows a table of thread → proposed bucket without applying
    anything.
  - *Report* options.
  - *Advanced › Edit prompt*: shows the generated Markdown; editing it
    sets `advanced_prompt` and disables the structured controls with a
    "Reset to generated" path back.
- **Runner** picker with an honest explanation per option:
  - *Claude cloud*: "Runs on Anthropic's cloud on your Claude plan, even
    when this Mac is off. Requires Gmail connected at claude.ai and one
    GitHub repository on the routine (Anthropic requires one; any repo
    works). OpenAGC can't create it for you — you'll paste it once."
  - *Claude Desktop*: same wording, local, uses the Desktop app's
    scheduled tasks.
  - *ChatGPT*: "Runs on OpenAI's cloud on your ChatGPT plan. Requires the
    Gmail app connected in ChatGPT. Unattended label changes may require
    approval in ChatGPT."
  - *Local (OpenAGC)*: "Runs here with your installed Claude Code or
    Codex, through OpenAGC's tools and approval rules. Needs this Mac awake
    at the scheduled time."
- **Publish to Claude cloud** (primary path): OpenAGC builds the routine
  JSON and spawns the user's CLI:

  ```text
  claude -p "<instruction>" --output-format json --max-turns 4
    --allowedTools RemoteTrigger --tools ""    # RemoteTrigger is built in; no other tools
    --strict-mcp-config --mcp-config '{"mcpServers":{}}'
    --permission-mode dontAsk
  ```

  where `<instruction>` is: "Call RemoteTrigger with action `create` and
  exactly this body, then reply with only the raw JSON result: `{…}`".
  The environment is scrubbed of any `CLAUDECODE*`/`CLAUDE_CODE_*` nested
  session variables (their presence makes the CLI hang waiting on a
  parent session — observed during verification). The `result` JSON is
  parsed for `id` (`trig_…`) and `next_run_at`; the routine record stores
  the id and the claude.ai URL `https://claude.ai/code/routines/<id>`.
  *Update* and *enable/disable* use action `update` with a partial body;
  the editor's Save button republishes only when `sync_fingerprint`
  changed. Body specifics: `cron_expression` in UTC (converted from the
  RRULE, hourly minimum enforced by the editor); `job_config.ccr`
  `environment_id` discovered once via a `list` call or the user's
  default; `session_context.model` from the routine's model picker (the
  maintainer's runs on `claude-opus-5`); `mcp_connections` holding the
  Gmail connector (`connector_uuid`, `name: "Gmail"`, `url:
  https://gmailmcp.googleapis.com/mcp/v1`) copied from an existing routine
  when present, otherwise the user is sent to
  `https://claude.ai/customize/connectors` first.
  **Preconditions** shown in the runner picker: CLI installed and logged in
  with a claude.ai subscription (probe from §9.2; an API-key login cannot
  use routines), Gmail connected at claude.ai. **Fallback**: if
  `RemoteTrigger` is not in the CLI's `system/init` tool list or the call
  fails, the same screen offers the paste hand-off — prompt on the
  clipboard, schedule pre-converted, `https://claude.ai/code/routines`
  opened — and the user pastes the routine URL back.
- **Publish to ChatGPT**: hand-off only — prompt on the clipboard,
  `https://chatgpt.com` opened, with instructions to say "create a
  scheduled task".
- **Run now** (Claude cloud): `RemoteTrigger run` through the CLI; the
  result's session id is stored on the run record and the run's page is
  linked. OpenAGC then polls history sync every 30 s for 10 minutes so
  results show up promptly.
- **Recent runs** (Claude cloud): `RemoteTrigger list_runs` and
  `get_run_log` on demand when the user opens a routine; the condensed
  log and the final report text are stored in `routine_runs.report_text`.
  Logs come from a remote run and are treated as data (never as
  instructions), displayed as plain text.
- **Run now** (local): starts an agent session (§9) with
  `SessionConfig.scope = Mailbox`, the routine prompt, and the tool
  allowlist restricted to what the routine map needs. The session appears
  in the agent panel like any other, including approvals if the user has
  set Reversible actions to require them.

### 11.6 Attribution and history

The store already knows which label changes OpenAGC made (they came
through the outbox). During history sync, label additions under a
routine's `parent_label` and matching `INBOX` removals that did **not**
originate from the outbox are recorded in `routine_runs` as an inferred
run: `{routine_id, inferred: true, window_start, window_end, thread_ids,
per_bucket_counts}`, grouped by a 5-minute gap. This gives the Routines
list its "Sorted 84 threads · 2 h ago" line and a per-run thread list the
user can open, with an **Undo run** action that moves the threads back to
the inbox and removes the bucket label (through the outbox, reversible).
For Claude cloud routines, inferred runs are reconciled with
`list_runs`: an inferred window that overlaps a cloud run's
`fired_at`–`finished_at` is merged into that run record, gaining its
session id, status and report text. ChatGPT runs stay inferred. Local
runs are recorded exactly, with the transcript and `agent_actions` rows;
`inferred: false`.

Schema additions (migration 2):

```sql
CREATE TABLE routines (
  id INTEGER PRIMARY KEY, uuid TEXT NOT NULL UNIQUE, account_id INTEGER NOT NULL,
  name TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, runner TEXT NOT NULL,
  template_id TEXT, template_version INTEGER, definition_json TEXT NOT NULL,
  sync_fingerprint TEXT, cloud_url TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE routine_runs (
  id INTEGER PRIMARY KEY, routine_id INTEGER NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
  inferred INTEGER NOT NULL, session_id INTEGER REFERENCES agent_sessions(id),
  started_at INTEGER NOT NULL, ended_at INTEGER, status TEXT NOT NULL,
  counts_json TEXT NOT NULL, report_text TEXT, undone_at INTEGER);
CREATE TABLE routine_run_threads (
  run_id INTEGER NOT NULL REFERENCES routine_runs(id) ON DELETE CASCADE,
  thread_id INTEGER NOT NULL, bucket_id TEXT, PRIMARY KEY(run_id, thread_id));
```

### 11.7 Local scheduler

The local runner uses an in-app scheduler, not launchd: OpenAGC is a
long-running app and the routine needs the core's store and agent stack.
`RoutineScheduler` (Rust, tokio timer) evaluates RRULEs (`rrule` crate)
against local time, fires when the app is running and the account is
synced within the last 10 minutes, skips and records "missed — app not
running" otherwise, and never overlaps runs of the same routine. Wake from
sleep runs any routine whose scheduled time passed during sleep, once. A
"Launch OpenAGC at login" toggle (`SMAppService`) is offered when the
user picks the local runner.

### 11.8 Security notes specific to routines

- Cloud runners operate under the vendor's connector permissions, outside
  OpenAGC's permission engine. The runner picker says so.
- The generated prompt hard-codes the non-negotiables (never trash, never
  spam, never send) regardless of bucket edits; the advanced editor shows a
  warning if those lines are removed.
- The `claude` subprocess used for publishing runs with `--tools ""`,
  `dontAsk`, no MCP servers, and a routine body OpenAGC constructed; the
  only thing it can do is call `RemoteTrigger`. Its JSON result is parsed
  strictly (id, URL, next run) and never rendered as instructions.
- Run logs fetched from the cloud can quote email content the run read;
  they are displayed as plain text in a read-only view and never fed
  into a local agent prompt.
- Inferred attribution is a heuristic. The Undo action re-checks the
  thread's current labels before acting so it never undoes something the
  user changed since.

### 11.9 Scope for the MVP

Included: the model, template, generator, structured editor, local runner
with dry-run preview, Claude cloud publish/update/run/run-history through
the user's CLI with paste hand-off as fallback, inferred attribution and
Undo. Deferred: ChatGPT hand-off polish beyond clipboard + instructions
(until OpenAI documents its tool set), importing an existing cloud
routine into the structured editor (parsing a hand-written prompt back
into buckets), multiple templates, event-triggered runs, sharing routines
between users.

---

## 12. Secrets and Keychain **(Verified)**

Keychain access is done **in Swift**, using `SecItem*` with
`kSecUseDataProtectionKeychain`, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`,
service `ai.actual.openagc.oauth` and account `<account-uuid>`. The Rust
`security-framework` crate does not expose the data-protection keychain or
access groups cleanly, and the entitlements live on the Swift side anyway.

Rust receives secrets through a foreign trait:

```rust
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn get(&self, key: String) -> Result<Option<String>, CoreError>;
    fn set(&self, key: String, value: String) -> Result<(), CoreError>;
    fn delete(&self, key: String) -> Result<(), CoreError>;
}
```

Keys: `oauth.refresh_token.<account>`, `oauth.access_token.<account>`,
`oauth.client_secret.custom` (BYO only), `anthropic.api_key` (optional),
`mailbox.api_key.<service account>` (an agent-mail service account's key,
shared by its agents, §7.9; for mailboxes created before 2026-10-08 the
service account's id is the agent's account id, so the name is
unchanged).
*(Amended 2026-10-08, ADR 0016; not built.)* `rules.publish_token.<server>.<account>`
(the publisher token for one agent mailbox on one rules server, §10.6) and,
with encryption at rest, `rules.snapshot_key.<account>`. Agent tokens for
the rules server are shown once and never stored by the app, which keeps
only their ids and names.
Routines need no secret of their own: the CLI holds the claude.ai login.
The shipped OAuth client ID/secret is compiled in. Secrets are never written
to logs, the database, or crash reports; `tracing` fields carrying tokens
are wrapped in a `Redacted` newtype whose `Debug` prints `***`.

The MCP shim and the agent subprocesses receive **no** secrets in their
environment; the agent CLIs manage their own credentials.

*(Amendment 2026-10-08, oagc-uys.10.)* In mailbox mode (§10.1) with the
app closed, `openagc-mcp` runs the core headless with a secret store that
refuses every read, write and delete: it holds no secrets and never
reaches the Keychain, which only the app's Swift side can read. It
therefore cannot send; it queues, and the app sends when it next opens.
Sending from the shim with the app closed needs the service account's key:
the plan is a helper embedded in the app bundle, signed with the app's
team and Keychain access group, which will be the one exception to this
rule (`docs/plans/headless-mcp.md`, oagc-uys.7).

---

## 13. Performance Architecture

Every target in §1.3 traces to one of these rules.

1. **The UI reads only SQLite.** No view ever awaits the network. Sync and
   outbox workers write to the database and emit coalesced events; views
   re-query.
2. **Thread list is AppKit.** `ThreadListView` is an `NSViewRepresentable`
   hosting an `NSTableView` with `usesAutomaticRowHeights = false`, fixed
   row height, view reuse, and a Swift `ThreadRowModel` array as data
   source. SwiftUI `List` on macOS re-diffs and re-layouts on every state
   change and cannot hold 120 fps with 100k rows; `NSTableView` can. Rows
   are plain `NSView`s with `CATextLayer`s, not SwiftUI cells.
3. **Paged, keyset-cursor loading.** The list holds a window of ~300 rows
   and fetches the next page when scrolling within 100 rows of the edge.
   Total counts come from `threads.unread_count` aggregates, not
   `COUNT(*)`.
4. **Denormalize for the list.** `threads.participants_json`, `snippet`,
   counts and `thread_labels` exist so a row renders from one table row
   with no joins.
5. **Pre-render bodies at sync.** Sanitized HTML and extracted plain text
   are computed in the backfill worker and stored. Opening a message is a
   primary-key read plus `loadHTMLString`.
6. **Warm `WKWebView` pool.** Two pre-created web views with the base
   stylesheet already loaded; selecting a thread swaps content in the
   idle one, then swaps views. First paint is well under 50 ms.
7. **Optimistic mutations.** Archive/read/label update local rows
   synchronously (the outbox write is < 1 ms) and the row animates out
   before Gmail hears about it.
8. **Prefetch on selection intent.** Hovering or arrow-keying to a row
   prefetches its rendered body into an LRU (50 entries) in Swift.
9. **Startup order.** `Core::new` opens the database and returns before
   sync starts; the first `list_threads` for Inbox runs before the runtime
   spawns any network work. Window restoration is deferred until the first
   page is on screen.
10. **Measured, not assumed.** `signpost`s around FFI calls, list reload,
    and web view swaps; a `perf` XCTest suite asserts p95 for the §1.3
    table against a generated 100k-message fixture database, run in CI on
    a self-hosted Apple Silicon runner.

---

## 14. macOS Application

### 14.1 Requirements

- macOS 26.0+, Apple Silicon. Intel is not built for MVP (a `x86_64` slice
  can be added later; nothing precludes it).
- Xcode 27 (macOS 27 SDK), Swift 6.4 in Swift 6 language mode with complete
  strict concurrency; deployment target macOS 26.0.
- Project generated by **XcodeGen** from `macos/project.yml` so the
  `.xcodeproj` is not hand-merged; it is committed for convenience.

### 14.2 Structure

- `@main struct OpenAGCApp: App` with an `NSApplicationDelegateAdaptor` for
  menu, dock, Sparkle and URL handling.
- **Stores** are `@Observable @MainActor` classes: `AccountStore`,
  `MailboxStore`, `ThreadListStore`, `ThreadDetailStore`, `SearchStore`,
  `ComposerStore`, `AgentStore`, `SettingsStore`. Each subscribes to the
  `CoreEvent` stream and re-queries only what its hint says changed.
- **CoreClient** is a `Sendable` wrapper around the UniFFI `Core` object
  and is the *only* file that imports `OpenAGCCore`.
- Windows: main window (`NavigationSplitView` three-column), composer
  windows (`WindowGroup` keyed by draft id), Settings (`Settings` scene).

### 14.3 Main window

- Sidebar: account avatar button (§7.7), mailboxes and labels, unread
  badges, drag-to-label target. *(Amended 2026-09-26: labels are a
  tree.)* Gmail nests labels by `/` in the name (`Marked Important/1-Daily`).
  The sidebar shows them as an outline: a parent row with a disclosure
  triangle, children indented, sorted by name at each level. A parent that
  is itself a label is selectable and shows its own unread count; when
  collapsed it shows own + descendants. A path segment that exists only
  as a prefix (Gmail allows `a/b` without `a`) becomes a plain group row,
  not selectable. Expansion state is remembered per account. `←`/`→`
  collapse and expand, drag-to-label works on every label row, and a
  label's colour applies to its own row only. Label chips in the list
  show the leaf name with the full path as tooltip; the `l` popover and
  the label menus present the same tree with a filter field, and
  *Create label…* accepts a `/` path (parents are created as needed, as
  the routine template already requires).
- Thread list (AppKit): sender, subject, snippet, date, unread dot,
  attachment icon, label chips; multi-select; swipe actions (archive,
  read); context menu; keyboard: `↑↓`/`j k` move, `e` archive, `u`
  toggle read, `s` star, `l` label popover, `#`/`⌫` trash, `r` reply,
  `a` reply-all, `f` forward, `c` compose, `/` search; menu equivalents
  `⌃⌘A` archive, `⌘⌫` trash, `⌘⇧U` read/unread, `⌘⇧L` star, `⌘R`
  reply, `⌘⇧R` reply-all, `⌘⇧F` forward, `⌘N` new, `⌘F` search,
  `⌘1`–`⌘6` mailboxes, `⌘⇧N` check for new mail, `⌘K` agent prompt.
  *(Amended in M2: `r` is reply, Gmail-style; `u` toggles read either
  way.)*
- Thread view: one locked-down `WKWebView` renders the whole thread as a
  single document, one `<details>` block per message (the latest and any
  unread open, the rest collapsed to a snippet, no JavaScript needed), with
  a SwiftUI header (subject, message count, remote-images banner) above it.
  *(Amended in M1: the plan was a SwiftUI header plus a web view per
  message; one document avoids measuring each web view's height and costs
  one load per selection.)* Attachments strip with Quick Look
  (`QLPreviewPanel`) and drag-out.
- Agent prompt: "Ask Claude…"/"Ask Codex…" with the provider switcher. The
  field is never disabled, so ⌘K always lands in it and words can be typed
  ahead; *Send* (and Return) wait for a ready agent, and a line under the
  field says why it is not ("Claude Code isn't installed", "needs you to
  sign in", "couldn't be checked: …") with *Agent Settings…*. Focusing the
  field, or trying to send, looks for the agent again; the core does not
  cache a probe that failed, only its settled answers *(amended
  2026-10-06: a probe that failed at launch left the bar disabled for the
  whole run while the composer, which checks nothing, kept working)*.
  *(Amended 2026-09-27: a glass capsule floating over the bottom of the
  reader column, inset like the macOS 26 sidebar, rather than a bar pinned
  under the thread list. The list column has a header: the Inbox's
  Important-only switch, then a rule separating the title area from the
  messages.)* *(Amended 2026-09-28: the capsule sits in a strip of its own
  under the reader, with a margin above it, so a thread ends above it
  rather than scrolling beneath it.)*

**Amendment (2026-09-28): Mail-like layout.** Implemented 2026-09-28. The
sidebar reads Favorites (Inbox, Starred, Sent), then the account's other
mailboxes with its labels under the account's name, then Routines; sync
status is its footer (a thin progress bar, "Downloading Messages", what is
left). The list column is titled with the mailbox and its unread count.
New Message sits at the list column's trailing edge, where it meets the
reader, as in Mail (implemented 2026-09-28: the list column's own
toolbar; SwiftUI right-aligns a detail column's items, and `.navigation`
put it beside the title). The reader's toolbar starts at its leading edge
with Reply / Reply All / Forward, Archive / Trash / Mark as Junk, a Label
menu, Star and the agent toggle, with search at the trailing edge.

**Amendment (2026-09-28): Gmail categories.** Implemented 2026-09-28.
When the account uses Gmail's categories, the Inbox shows Mail-style tabs
above the list: Primary, Promotions, Social, Updates, Forums (Primary
always, the others only with mail; no tabs when only Primary has mail),
each a narrowed Inbox listing like Important-only (`INBOX+CATEGORY_…`;
as a narrowing, Primary means "in no other category", so Inbox mail Gmail
never categorised is Primary). Tabs are capsules with the category's
symbol and unread count, the chosen one also its name. The chosen tab is
remembered per account and falls back to Primary while it has no mail;
tabs combine with Important-only (`INBOX+IMPORTANT+CATEGORY_SOCIAL`) and
search ignores them. With categories the Important-only switch moves into
a View Options menu in the list header beside "Show Categories", which
turns the tabs off (per account). Revealing a thread from a notification
opens its tab. A thread in two categories lists in both but counts in the
first. Categories come from the Gmail API; IMAP-only accounts have none.
Moving a thread to another category is not in scope (Gmail's filters
decide).

**Amendment (2026-09-28): list filters.** Implemented 2026-09-28. A
filter button in the list column's header (every mailbox and search), as
in Mail, narrows the current mailbox or search: Unread, Starred, With
Attachments (combinable, with Clear Filters). An active filter fills the
button and shows in the subtitle ("Filtered: Unread, Starred"). Filters
apply locally: a listing gets them as narrowings on the thread's own
columns (`INBOX+@unread+@attachments`, combining with Important-only and
category tabs), a search as operators (`is:unread`, `is:starred`,
`has:attachment`) after the typed query, which is grouped in
parentheses so an `OR` is filtered as a whole. Clearing the search shows
the listing as it is then. They are per window, kept when
changing mailboxes, cleared when a notification reveals a thread, and not
remembered between launches.

**Amendment (2026-09-28): junk.** Implemented 2026-09-28. *Mark as Junk*
(toolbar, Message menu ⇧⌘J, context menu, `!` in the thread list as in
Gmail, VoiceOver's Actions) moves threads to Spam (adds `SPAM`, removes
`INBOX`); in Spam the same command reads *Not Junk* and moves them to the
Inbox; it is not offered in Sent or Drafts. Both are undoable (§14.6a:
"Moved 2 conversations to Spam") and go
through the outbox as label changes. Agents and `modify_labels` still may
not set `SPAM`: only these two user actions (`mark_junk`, `not_junk`) do.

### 14.4 Message rendering **(Verified)**

Sanitization happens in Rust (`ammonia` 4.x) at sync time with a strict
policy: allowlisted tags (no `script`, `iframe`, `object`, `form`, `input`,
`meta`, `link`, `base`), `style` attribute allowed but filtered to a
property allowlist (`color`, `background-color`, `font-*`, `text-*`,
`margin*`, `padding*`, `border*`, `width`, `height`, `display`, `float`,
`vertical-align`), `<style>` blocks dropped in MVP, all URLs rewritten:
`http(s)` image `src` → `openagc-blocked://` placeholder unless remote
images are allowed for that sender, `cid:` → `openagc-cid://<attachment>`,
links keep their `href` but get `target="_blank" rel="noopener"`. A
`sanitizer_version` column lets a policy change re-sanitize lazily.

`WKWebView` configuration: JavaScript disabled
(`defaultWebpagePreferences.allowsContentJavaScript = false`), a custom
`WKURLSchemeHandler` for `openagc-cid://` serving inline attachments from
disk, a `WKNavigationDelegate` that cancels every navigation and hands links
to `NSWorkspace` after a phishing check (visible text host ≠ href host →
confirmation sheet), a `<meta http-equiv="Content-Security-Policy"
content="default-src 'none'; img-src openagc-cid: data:; style-src 'unsafe-inline'">`
injected into every document, `isInspectable = false` in release. Remote
images: blocked by default, "Load images" per message, "Always for this
sender" stored in settings; loading them re-renders with `img-src https:`.

Read on view: a thread with unread mail is marked read (a normal change,
so Gmail sees it too) once it has been shown for 0.8 s and is still the
selection, so moving through the list with j/k or the arrows does not
mark every thread passed over; a thread opened in its own window is read
as it opens. There is no undo notice; *u* marks it unread again
(amended 2026-10-04).

Dark mode: a base stylesheet sets `color-scheme: light dark` and inverts
only when the email declares no background color.

Quoted history: a thread already shows every earlier message, so the copy
a reply carries below it ("On … wrote:" and the quote, or Outlook's
"From: … Sent: …" block, in HTML or plain text) is folded behind a "•••"
toggle (a `<details>`, so it works without script). It is found at display
time, by structure and wording, since the stored HTML has lost the
senders' class names. A message that answers between quotes, or is
nothing but a quote (a forward), is shown whole (amended 2026-10-02).

### 14.5 Composer

Rich text via `NSTextView` in an `NSViewRepresentable`, `NSAttributedString`
model, toolbar for bold/italic/underline/lists/link/quote; attachments by
drag-drop or picker; recipients field with a token view fed by
`addresses_fts` (frecency-ranked). Serialization: attributed string → HTML
by a small Swift serializer emitting a constrained tag set (`p, br, b, i, u,
a, ul, ol, li, blockquote`) so the HTML is predictable; plain-text part
generated in Rust from the HTML. Reply quoting inserts the sanitized parent
HTML inside `<blockquote>` with a "On <date>, <name> wrote:" line.
Autosave to `drafts` every 2 s of idleness; Gmail draft sync through the
outbox every 30 s or on close.

**Amendment (2026-10-02): answering with the conversation in view.** A
reply's window shows the conversation above the draft, in a pane that can
be resized or hidden: the latest message open, earlier ones as rows that
open when clicked, as in the reader (drafts left out). A forward keeps the
original under the draft. Writing help's field is as large as the main
window's prompt and grows with what is typed (⌥Return for a new line).
Tab in the body goes to writing help. The body sits in a frame of its own
with a formatting bar under it, as in Gmail, in every composer: bold,
italic, underline, strikethrough, bulleted and numbered lists, quote (sent
as a `blockquote`), link and clear formatting, each lit while it is on at
the cursor and each one undoable. The usual shortcuts work: ⌘B, ⌘I, ⌘U,
⇧⌘X strikethrough, ⇧⌘8 bulleted, ⇧⌘7 numbered, ⇧⌘9 quote, ⇧⌘K link
(⌘K asks the agent from every window, the mail window coming forward;
amended 2026-10-06, it was ⌘K in the composer) and ⌘\ clear. Fonts, sizes and colors stay out, so
mail looks ordinary in the recipient's client. In the main window, Tab walks
the columns: sidebar → list → message (when one is shown) → search → sidebar,
and ⌥Tab or ⇧Tab the other way. Landing on the list selects its first row
when none is selected (shown in the reader, as a click would) and keeps the
selection otherwise; the Writing Guide, Facts and Tasks lists take their
turn the same way *(amended 2026-10-06; before, Tab went only from the
sidebar to the list and selected nothing)*.

**Amendment (2026-09-28): drafts from the Drafts mailbox.** Drafts sync
through Gmail's drafts list (`drafts.list`) on every incremental round,
since Gmail's change history leaves drafts out and the sync window
(one month by default) would miss older ones: every draft's message is
stored in full whatever the window, draft messages whose draft is gone
(sent or discarded elsewhere) are removed, and `server_drafts` records
which Gmail draft holds which message. Any other conversation opens in a
window of its own from a double-click or Return in the list, as in Apple
Mail (up to ten selected at once; amended 2026-10-02). A draft opens
in a composer from *Edit Draft* in the reader, a double-click or Return:
the local draft already mirroring it is reused; a draft written elsewhere
becomes a local draft on first open (recipients, subject, body, and its
attachments fetched and copied beside the other draft attachments),
keeping the Gmail draft id so saving replaces that draft rather than
adding a second one. Drafts show their paperclip in the list like any
thread.

*(Amended 2026-09-28.)* A reply or forward shows the message being
answered under the editor by default, in a pane whose divider can be
dragged, with Hide Original / Show Original; the message is downloaded
first if only its headers were stored. A writing-help bar at the bottom
asks the agent to write or change the message ("Write a reply", "Make it
shorter", …, or the user's own words). It runs in a session of its own
that can see only the thread being answered; its answer replaces the
body, with Undo. The session is read-only: the core refuses every tool
that would change mail (2026-09-29; it had only refused proposals, which
left tools that need no approval open). Sending stays the user's.

### 14.6 Agent panel

Not a chat window. The prompt bar sits under the thread list; a session
opens an inspector column on the right with: a compact transcript
(assistant text, collapsed tool calls "Searched mail — 18 threads"),
**results rendered as a thread list** (from `mail.present_threads`, under
the agent's title and the count: "Needs a reply · 12 conversations") that
behaves exactly like the main list, pending approval cards with
Review/Reject/Approve, and a Cancel button. Drafts created by the agent open
in the composer for review with a "Created by Claude" badge.

### 14.6a Acknowledgement and undo **(Amendment 2026-09-27)**

Implemented 2026-09-27 (oagc-82v). Every mail action the user takes shows a short acknowledgement
with a way back, instead of any confirmation dialog ("never use a warning
when you mean undo": all these actions are reversible).

- **What it looks like.** A small notice at the bottom of the thread list:
  "Archived 3 conversations — Undo ⌘Z", with a close button. One at a
  time: a new action replaces it. It stays 8 seconds, pausing while the
  pointer is over it, while it has keyboard focus, and while the window is
  inactive. Reduce Motion: it appears without sliding.
- **The undo does not expire with the notice.** ⌘Z is Edit › Undo
  ("Undo Archive"), backed by the window's undo manager, and works on the
  last 50 actions after the notice is gone, as in Mail. Text fields keep
  their own ⌘Z while they are being edited. ⇧⌘Z redoes.
- **VoiceOver** hears the notice as an announcement without focus moving
  (WCAG 4.1.3); the Undo button is reachable by keyboard; nothing needed is
  lost on a timer (WCAG 2.2.1), since ⌘Z remains.
- **Covered actions:** archive, move to Inbox, trash, read/unread, star,
  add/remove label (menus, keys, swipe, drag-to-label, the `l` popover),
  and label creation is not undone (only its application). Agent and
  routine actions are not on this stack: they have the activity log and
  the routine run's Undo.
- **Exactness.** Each action records, per message, the labels it actually
  changed (a thread already archived is not "un-archived" into the Inbox by
  undo). Undo applies the inverse through the outbox like any change; if
  Gmail changed the thread since, undo still only reverses the recorded
  diff. Undo is per account: after switching accounts, ⌘Z undoes that
  account's actions.
- **Undo Send.** Sending waits in the outbox for a delay (Settings ›
  General: off, 5, 10 (default), 20 or 30 seconds); the notice reads
  "Sending… Undo ⌘Z", and undo returns the message to an open composer.
  Agent sends, approved by the user, use the same delay.

*Implementation notes.*
- The core records each user action in the account's store
  (`undo_actions`, last 50) as per-message diffs and returns an
  `UndoToken` (account, action id); `undo_action`/`redo_action` apply the
  exact inverse or the original as outbox ops in the token's own account.
  Trash is undone with Gmail's `messages.untrash` (outbox `Untrash`), then
  the recorded labels. An action that changed nothing returns no token and
  shows no notice. Agent tools use an unrecorded path.
- Swift keeps one `UndoManager` per account; Edit › Undo/Redo is
  replaced by a command group that sends `undo:`/`redo:` down the
  responder chain when a text view is first responder or another window is
  key, so fields and the composer keep their own undo.
- The notice is a glass capsule over the bottom of the thread list.
- Undo Send: held sends sit in the outbox with `next_attempt_at` set; the
  drain claims an op (`in_flight`) before calling Gmail, so cancelling can
  only remove a send that has not started. Undo removes the optimistic
  Sent copy, returns the draft to editing and reopens it; there is no
  redo. The notice lasts as long as the hold. *Decision:* quitting sends
  held messages at once (the app waits up to 5 s for every send not yet
  handed to Gmail, held or not; any not delivered go at the next launch).
  Only a send never attempted and still within its hold can be taken
  back, and its notice does not pause, since the hold does not.
- The outbox runs strictly in order: an op waiting to retry holds back
  the ones after it (an undo must never reach Gmail before the action it
  reverses); held sends alone step aside until their time.
- The demo mailbox sends locally at once, so it offers no Undo Send.
  Agent sends are held for the same delay. *(Amended 2026-09-28,
  implemented:)* an approved agent send or forward that the core holds
  shows "Sending… Undo" on its approval card for the hold
  (`send_held_until`); Undo cancels it (`cancel_send`), the card reads
  "Not sent. The draft is open for you." and the draft opens in the review
  composer, switching accounts if need be. It is not on ⌘Z's stack and
  has no notice; the agent's transcript still says it sent, and the
  activity log keeps the approval.

### 14.6b Agent suggestions **(Amendment 2026-09-27)**

Implemented 2026-09-27 (oagc-gra). The prompt capsule ("Ask Claude…") gives no hint of what an agent
can do; most users will not guess. Suggestions show examples, drawn from
the tools that exist (§10.2), so nothing is promised that a tool cannot do.

- **Where.** (1) The agent column's empty state lists capabilities in
  groups: *Find and summarise* (read tools), *Draft for you* (drafts,
  never sent without approval), *Tidy up* (archive, labels, read state),
  and, in an archive account, no drafting group. Each line is an example
  prompt in the user's voice. (2) When the prompt field is focused and
  empty, up to four **suggestion chips** appear above the capsule; ↑/↓ or
  Tab move between them, Return sends one, Escape hides them. A chip
  fills the field rather than sending when it ends in "…" (needs the
  user's words).
- **Context-aware.** Suggestions follow what the user is looking at:
  - a thread selected: "Summarise this thread", "Draft a reply that…",
    "What is being asked of me here?", "Add the label …";
  - several threads selected: "Archive these", "Which of these need a
    reply?", "Label these …";
  - a mailbox with unread mail: "What's new since yesterday?", "Which
    unread messages need a reply?";
  - a search in progress: "Summarise these results", "Find the one that
    mentions …";
  - an attachment in the selected thread: "What does the attachment say?";
  - an archive account: the same minus drafting.
  Suggestions are generated locally from state; no model call and no
  network to produce them.
- **Honesty.** Every suggestion maps to tools the current agent and
  account allow. Write actions say what happens: "Archive these (you can
  undo)"; sends are never suggested ("draft" only, since sending needs
  approval, §10.4).
- **Learning.** The chips prefer prompts the user has sent before (last
  20, stored per account); the examples rotate so the list does not look
  static. No telemetry.
- **Accessibility.** Chips are buttons with full labels; VoiceOver
  announces "4 suggestions" when they appear; Reduce Motion disables the
  fade.

*Implementation notes.* `AgentSuggestions` (Swift, pure) builds the groups
and chips; up to two recent prompts that fit the context come first (one
mentioning "this"/"here" fits a single selection, "these"/"results" several
or a search, anything else no selection), then the context's examples, the
most relevant fixed and the rest rotated by day. *Decision:* the archive
chip reads "Archive these (reversible)", not "(you can undo)": agent
actions are not on the ⌘Z stack (§14.6a), though they can be moved back.
The empty state replaces the transcript area (header kept); the chips sit
above the capsule. Settings › Agents › Clear Suggestions History forgets
the recent prompts (UserDefaults, per account).

### 14.7 Other native behaviors

Standard menu bar with all commands and shortcuts; `NSUserNotification` via
`UNUserNotificationCenter` for new mail in Inbox (opt-in per sender
category later); Dock badge for unread; full VoiceOver labeling on custom
AppKit rows; Services and Spotlight are deferred.

*(Amended 2026-09-28.)* Settings › Accounts no longer offers "Download
faster over IMAP": it shows how the account downloads ("Over IMAP", or
"Over the Gmail API" and why), with "Sign In Again for IMAP…" only for an
account whose sign-in lacks the full scope.

### 14.7a Sync Debugger **(Amendment 2026-09-28)**

A permanent window (Window › Sync Debugger) for diagnosing sync, per Gmail
account: the transport downloading mail, the IMAP breaker, the last IMAP
error, IMAP bytes today against the budget, the server's IMAP
capabilities, the messages stored, the latest operation for each job and
the recent operations (time, job, IMAP or API, duration, items, and why
the API served it). *Run Comparison* times each job both ways (up to
10,000 ids listed, 500 headers, 500 messages, and the changes: API history
since the last sync against re-reading labels and flags over IMAP for the
last 30 days) and shows which was faster per item. It downloads a sample
but stores nothing, changes no mail, and leaves the breaker and the
operation record alone.

**Amendments 2026-09-29 (maintainer feedback).** In the reader, a draft
in a thread is an outlined, unfilled card marked **Draft**, its time
"Saved …", so it never looks like mail that went. The composer sends with
⌘Return as well as ⇧⌘D, as in Gmail, and a new message or a forward opens
with the cursor in To. While the agent column is open, the Ask bar sits at
the bottom of that column under the conversation, like a chat, and takes
the cursor; closed, it returns under the reader. The agent column's Agent
Settings… opens Settings on the Agents tab, where a Default agent picker
and a Default tag show which agent answers.

### 14.8 Tasks **(Amendment 2026-09-29)**

A task-based way through email that changes the app as little as
possible: Claude reads an email and suggests what the user has to do about
it, by when, and in which category; accepted tasks go in a task list the
user works through, usually by replying once they have gathered what they
need or made a decision.

- **Stored on this Mac,** per account (§6.2). No Google Tasks: it would
  need another Google scope. Gmail sees tasks only as the account's
  **`Task` label**: found by name (any case) or created the first time a
  task is accepted, and remembered by id. A thread carries it while it has
  an open task: accepting adds it, completing or deleting the thread's
  last open task removes it, reopening or restoring adds it back. These
  are ordinary label changes through the outbox, not separate entries on
  the undo stack: undoing a task operation moves the label with it. A
  `Task` label the user puts on a thread by hand, with no task here, is
  never touched. Accepting leaves the email where it is (no archive).
- **Categories:** a starting set (Reply, Decide, Gather Info, Schedule,
  Review, Admin, Follow Up), editable, reorderable and resettable in
  Settings › Tasks; Claude picks from the account's list. A task keeps
  its category's name when the category is removed.
- **A task** has a title, notes, category, due day or none, the action
  that completes it (reply, reply all, forward or none), Claude's one line
  on why, and whether Claude or the user wrote it.
- **Asking Claude.** The core builds the request from stored mail: each
  thread's latest message as plain text (at most 4,000 characters, the
  message downloaded first if only its headers are stored), today's date
  and weekday, and the account's categories; at most 50 threads at once.
  The emails sit in `<email thread_id="…">` blocks, and the request says
  they are data whose instructions are to be ignored. Claude answers with
  a JSON array (thread, title, category, due day or null, action, why);
  the core reads it leniently (prose or a code fence around it, one
  object, `{"tasks": […]}`) but keeps only threads it asked about, the
  first suggestion for each; an unknown category becomes the first, a day
  that is not a date or an unknown action becomes none. The app asks in a
  one-turn **read-only** agent session of its own that sees only those
  threads: the core refuses every tool that would change mail, whatever an
  email tells the agent (the composer's writing help uses the same kind of
  session). Inside the email blocks every `<` becomes `‹`, so no text can
  close or fake a block. It does not appear in the agent column. With no agent ready, the dialog opens empty.
- **One email: `t`.** In a mail list, `t` (also Message › New Task from
  Email… and the list's context menu) opens the **task dialog** for the
  thread being read and asks Claude at once: the email's sender and
  subject, a line saying what Claude is doing, then Claude's guess filled
  in: the title, the category as chips, the due day (a checkbox and a date
  picker, read as "Today", "Tomorrow", …), what finishes it (Reply, Reply
  All, Forward, No Email) and notes, with Claude's why under the heading.
  A title typed before Claude answers is kept. Return adds the task, Escape
  cancels, Ask Again asks once more. Adding closes the dialog and shows
  "Added a task: “…”" in the undo notice; ⌘Z removes it (and the label, if
  it was the thread's only open task), ⇧⌘Z puts it back.
- **The task list.** A **Tasks** entry under Favorites (its badge: open
  tasks due today or overdue) swaps the list column for the tasks; the
  reader shows the chosen task's email. Open and Done tabs sit in the
  column header. Open tasks group under Overdue, Today, This Week, Later
  and No Date; each row shows the title and due day, then the category
  chip and the email's sender and subject; overdue days are orange, today
  in the accent colour. Keys: `↩` edit (the task dialog, Save), `r` `a` `f`
  reply, reply all, forward, `e` done (or open again among Done), `c`
  category (a chooser with number keys), `⌫` delete, `j` `k` move; the
  same in the context menu, and Mark as Done and Category in the list
  column's toolbar. Every change goes on the undo stack with a notice.
  A reply or forward started from the task list answers the task: when it
  is sent, the app asks "Mark the Task Done?" (Y or Return: done, and the
  thread's `Task` label goes with its last open task; N or Escape: it
  stays open). ⌘Z reopens a task marked done; Undo Send takes the message
  back and reopens the task, and sending it again asks again
  (amended 2026-09-29 at the maintainer's request: it had completed the
  task without asking). `⌫` deletes the task only, never its email. Searching while in Tasks shows mail
  results as anywhere else.
- **Many emails: `⇧T`.** In a mail list, `⇧T` (also Message › Create
  Tasks… and the context menu) opens the **bulk sheet** for the
  highlighted threads, or else the latest 20 in the open list, and asks
  Claude about all of them in one request. Each row shows the email and
  Claude's title, category, due day and why, all editable, with a
  checkbox; threads that already have an open task say so and start
  unchecked. "Add N Tasks" (Return) adds the checked rows that have a
  title; one Undo removes them all.
- **Hiding emails with tasks.** View Options in the Inbox has **Hide
  Emails with Tasks**, remembered per account like Important Only (§14.3):
  the Inbox then leaves out threads carrying the account's `Task` label,
  so it shows only what still needs sorting. It combines with Important
  Only, the category tabs (whose counts follow) and the filters, and the
  list's subtitle says "Tasks hidden"; search ignores it, as it ignores
  the tabs. The store's lists take exclusion narrowings for it
  (`INBOX+!Label_7`, "and not labelled", next to `INBOX+IMPORTANT`).

### 14.9 Writing guide **(Amendment 2026-09-29)**

Whenever an AI composes email for the user (a new message, a reply or a
forward, in the composer's writing help, the agent column or a routine), it
follows the account's **writing guide**: a ruleset and a style guide the
user builds from their own sent mail. Decisions: ADR 0011; plan
`docs/plans/writing-guide.md`.

**Scope.** One guide per account, in the account's store (ADR 0004); one
guide for every agent (Claude Code and Codex). Nothing in a guide crosses
accounts except through an explicit merge, and evidence never does.

**Entries.** Each entry is one sentence, imperative, in one category:

- *Rule*: must or must never; no exceptions unless it states them.
- *Guideline*: how the user usually writes; followed unless the message
  calls for something else.
- *Fact*: something true about the user the agent may use (role, calendar
  link, time zone); never inferred silently, always confirmed. Facts now
  live in Facts (§14.11, amended 2026-10-05): the guide's F3 category is a
  pointer there, and existing F3 entries were moved.

An entry has a category, kind, statement, **scope** (any of: audience
groups, people or domains, message types (new, reply, forward), languages;
none = always), **evidence** (quotes from the user's sent mail with their
message ids, and how many analysed messages support and contradict it),
source (*learned* or *you*), status (*proposed*, *accepted*, *rejected*),
an optional **check** the core can test without an agent (a banned phrase,
a pattern, a required sign-off, a spelling variant), and where it came from
when merged. Rejected proposals are remembered and not proposed again.

**Precedence.** Rules beat guidelines; a narrower scope beats a wider one
(a person's entry beats their group's, a group's beats an unscoped one);
among equals the newer wins. Settings › Permissions always wins over the
guide: H4 can make an agent more careful, never less.

**Categories.** The processing function checks every batch against the
whole list, so coverage does not depend on the agent thinking of a
category. *Learned* categories come from sent mail; *asked* ones cannot be
seen in mail and come from the interview.

| Group | Categories |
| --- | --- |
| A Voice and tone | A1 overall voice; A2 tone by situation (no, bad news, apologising, favours, chasing, thanks, disagreeing, congratulating); A3 humour, emoji, exclamation marks; A4 directness; A5 uncertainty in the user's voice; A6 enthusiasm and acknowledgements; A7 personality markers (regionalisms, colloquialisms, lowercase replies) |
| B Structure | B1 greeting; B2 opening line; B3 body (answer first, paragraphs, lists); B4 length by message type; B5 closing line; B6 sign-off and name; B7 signature block (also asked); B8 subject lines; B9 context before the point; B10 calls to action; B11 questions |
| C Language | C1 spelling variant; C2 punctuation; C3 capitalisation; C4 contractions; C5 numbers, dates, times, money; C6 abbreviations and jargon; C7 favoured words and phrases; C8 things never done (banned words, clichés, formatting, AI habits; also asked); C9 sentence style; C10 languages |
| D Audience | D1 audience groups (also asked); D2 particular people and domains; D3 forms of address; D4 first contact vs established; D5 seniority (also asked) |
| E Message types | E1 replies; E2 forwards; E3 introductions; E4 scheduling (also asked); E5 follow-ups; E6 declines; E7 requests and delegating; E8 status updates and hand-offs; E9 thanks; E10 recipients (reply all, Cc, Bcc); E11 attachments and links; E12 disagreeing and negotiating |
| F Content rules (asked) | F1 commitments; F2 confidentiality; F3 facts about the user (in Facts, §14.11); F4 never invent (on by default); F5 AI disclosure; F6 required wording |
| G Format | G1 plain or rich text; G2 quoting |
| H When unsure (asked) | H1 missing information; H2 conflicts and precedence (fixed, as above, shown to the user); H3 model examples (chosen by the user); H4 draft, send or stay silent |

**Audience groups** are inferred from the mail (recipients' domains and
how the user writes to them) and confirmed by the user, who can rename,
merge or reject them. With fewer than five confirmed, the set is filled
from the obvious gaps (colleagues, direct reports, customers, investors,
vendors, candidates, advisers, friends and family, strangers), marked
*suggested* until confirmed. Learning may add members to a suggested
group but never changes a group the user confirmed or rejected. People and
domains map to groups; a recipient's group is what scopes guideline
entries. Renaming or merging a group re-scopes its entries and is
undoable.

**Learning.**
1. *Gather.* Learn from Sent Mail… asks how many of the latest sent
   messages to use (default 1,000; the dialog shows how many the account
   has) and which people or labels to leave out. Skipped: automatic
   replies, calendar responses, messages with none of the user's own text.
   Messages stored with headers only are downloaded first (§7.4).
2. *Prepare* (core, no agent): the user's own text only (quoted replies
   and forwarded originals stripped; the signature detected, removed and
   kept once for B7), with the message type, recipients, length and
   whether it answered someone.
3. *Process*: batches of 20 go to the user's agent in a read-only session
   (ADR 0007), fenced as in task prompts, with the category list and the
   accepted guide. The agent answers in JSON per category: a new entry,
   evidence for an entry, a contradiction, or nothing. The core reads it
   leniently, keeps known categories only, drops any quote that does not
   occur in the cited message, and merges across batches.
4. *No questions until every batch is processed*: proposals merge over the
   whole sample first, so each arrives once with all its evidence.
   Deciding then opens; every decision is saved as it is made, and the
   user can leave and come back.
5. *Decisions*: proposals by category with quotes and counts; accept, edit
   (statement, kind, scope, check) or reject. Decisions are proposed rules,
   reviewed with the daily review's in the Writing Guide's review flow
   (§14.10; amended 2026-10-05 and 2026-10-06). Mail that
   contradicts an
   accepted entry is a decision: narrow it, change it, or keep it.
6. *Interview*: short questions for the asked categories and for any
   category left without evidence; answers become entries with source
   *you*. It needs no agent. One question at a time: a question with set
   answers shows them as large buttons (keys 1 to 9) and choosing one
   saves it and goes on; *Back* shows the question before with its
   answer, and a different answer replaces the entries the earlier one
   added, in one undoable change **(Amendment 2026-10-05)**.
7. *Coverage*: the guide shows every category with its entries or
   "nothing yet".

A run is a background job in the core, recorded batch by batch in the
store: it survives the dialog and window closing, pauses when the app
quits and resumes at the next launch, and can be paused, resumed or
cancelled (what was analysed is kept). Two progress bars, in the Writing
Guide section and compactly in the sidebar's footer: *Learning* (messages
and batches done, with the time left estimated from how long the batches
so far took, once the first is done) and *Decisions* (decided of total, or
"waiting for analysis"). When analysis finishes with decisions waiting, a
sheet in the main window offers *Review Now* or *Later*, and a
notification (when the app is not in front) opens the decisions; both
open the Writing Guide's review flow on the first proposed rule. An
account with sent mail that has never learned is invited once, by a sheet
after its sync; put off, a banner above the Inbox and in the Writing Guide
stays until a run starts or it is dismissed (amended 2026-10-02). **Further analysis**,
any time: newer mail since the last run; further back; improve one
category or audience (the unanalysed messages most likely to show it,
with that category asked for in particular); re-check the guide against a
fresh sample (changes only). A message is never analysed twice.

**Agents only.** Learning, changing, merging and drafting run through the
user's connected Claude Code or Codex CLI; the app makes no model calls
of its own. With no agent ready, each entry point says so instead of
starting ("Connect Claude Code or Codex to learn from your mail"), with the
agent's status, the command to fix it and a button to Settings › Agents; a
run whose agent stops being ready pauses with the same message.

**Changing the guide by prompt.** Ask Claude to change the guide… takes a
request in the user's words. The agent answers with changes (add, edit,
rescope, remove) grouped into questions, each one decision in plain words
with before and after; nothing changes until the user answers, and the
answers apply as one change.

**Merging.** From another account in the app or an exported file. Into an
account without a guide the entries are listed, then saved; into one with
a guide identical entries are skipped, new points are listed to add, and
the agent writes one high-level decision per point of difference (keep
mine, take the incoming one, or keep both with a scope). Facts and content
rules are always a decision. Merged entries say where they came from.

**Following the guide.** The core renders it for the message at hand:
rules and facts always; guidelines by scope (the recipients' groups and
people, the message type); entries scoped to a language are shown with
their scope for the agent to apply, since the draft's language is not known
ahead, and their checks are left to it; up to three model examples of the
type. It goes into every composing path: the writing-help prompt (the
user's own draft stays part of the prompt), agent and routine sessions
through the system prompt, and again in the results of the draft tools so
a long conversation keeps it. Drafts record the guide version they used.

**Checks** run in the core on AI-written drafts only, never on text the
user typed. In writing help a failing draft is sent back once for a
rewrite, then shown with the failure ("Uses 'circle back', which your
rules ban"); on an agent's draft the result shows on its approval card.

**Missing facts.** Writing help never invents facts. When a request needs
facts the agent does not have (about the user, their company, figures,
dates, names) and they are not in the thread or the guide, it asks first:
the composer shows its questions (at most five) as fields, and the agent
writes once they are answered or skipped, leaving a [bracket] for anything
still unknown. Answers are kept as facts (§14.11, source *Writing help*) unless the
user unticks *Keep these facts*; each question carries a category and a
label, and adding them is undoable (amended 2026-10-02, 2026-10-05).

**Drafting with an audience.** An AI draft says who it is written for
("Written for Customers"), chosen from the recipients. Choosing another
audience writes a new draft under that audience's guidelines from the same
request, starting from the text the request was made on; drafts, with
the user's edits to them, are kept per audience while the composer is
open, and Undo goes back one step (the body before the last AI text).

**Undo and versions.** Every change to the guide (a decision, an edit, an
interview answer, a change by prompt, a merge, an audience rename or
merge) is undoable on the account's stack (§14.6a), and the guide keeps its
versions. Undo never removes evidence learned since the change, and an id
is never reused, so undo cannot overwrite a later entry.

**Privacy.** Learning sends the chosen sent mail to the user's own agent
CLI, as the agent column does with mail it reads; the dialog says so, with
the number of messages, before it starts. The guide and its evidence stay
in the account's store on this Mac. Export writes Markdown (readable) and
JSON (for import and merge) without evidence quotes unless the user asks.

### 14.10 Proposed rules and the daily review **(Amendment 2026-10-05)**

The writing guide keeps learning after onboarding. Once a day, while the
app is open, a background review compares what an AI wrote with what the
user actually sent, and proposes changes to the guide where the user's
edits show it is wrong or missing something; an optional second review
gleans facts (§14.11) from mail the user sends. The writing guide is a
set of rules, and every decision is about a rule: proposed rules wait in
the **Writing Guide**, proposed facts in **Facts** (§14.11); there is no
separate Analysis page (amended 2026-10-05, plan
`docs/plans/rules-and-facts-pages.md`). Decisions: ADR 0013 (recording),
ADR 0012 (global facts); plan `docs/plans/analysis.md`.

**When it shows.** Nothing records, runs or shows until the account has
finished its first writing-guide learning run (§14.9). Archived-mailbox
accounts (§7.7) never record or review.

**Recording.** Every AI composition leaves a record in the account's
store with its full text: writing help (each text the agent writes into
the composer; a rewrite for the guide's checks or for another audience
updates the same record), the agent column's draft tools (`create_draft`,
`update_draft`, replies) and routines (the same tools, source *routine*).
The record keeps the source, agent, message type (new, reply, forward),
draft, thread, recipients (To and Cc, normalised), subject, the user's
instruction or the agent's prompt, the AI text (plain and HTML), the guide
version and audience it was written under. On send it copies the draft's
rfc822 Message-ID before the draft row goes; a discarded draft marks its
record *discarded* (counted in the metrics, not compared). Records are
*waiting*, *matched*, *unmatched*, *discarded* or *reviewed*.

**Matching**, at the start of each review, for records still waiting,
strongest first:

1. *Sent from the draft*: the sent message whose Message-ID is the
   record's. Exact.
2. *Reply* (and forwards, first): the first message the user sends in the
   record's thread after the record was made.
3. *New message* (or forward): the first message the user sends after the
   record was made to any of its recipients (To before Cc), whatever the
   subject.

A sent message matches at most one record; the most recent record wins.
Matches by 2 or 3 with under 15% word overlap with the AI text are
dropped (a "Thanks!" after a long AI draft is a different message). A
record not matched within 14 days becomes *unmatched*. The sent text is
the user's own text, prepared as learning prepares it (quotes, forwarded
originals and signature stripped), and each pair records its distance
(normalised word edit distance, 0–1).

**The daily review**, per account:

- *When*: the first chance each calendar day, once the day's first sync
  goes idle after the app opens; the core's scheduler checks open accounts
  hourly. *Run Review Now* (the Writing Guide's *More* menu, Learning
  Settings) runs it on demand.
- *Gate*: a finished learning run and a connected agent. With no agent the
  review waits and the Writing Guide says why over its list ("Connect
  Claude Code or Codex in Settings › Agents").
- *How*: a background job in hidden read-only sessions (ADR 0007), in
  batches, recorded batch by batch so it pauses and resumes like a
  learning run; the same two progress bars and time-left estimate.
- *Compare*: pairs with distance above 0.05, ten to a batch. The prompt
  gives each pair's AI text and sent text (fenced), the instruction, the
  message type and audience, and the guide entries that applied (rendered
  for that message). The agent answers with proposals (add, edit, rescope,
  remove), each with evidence: the pair, a quote from the sent text and
  what it replaced. Quotes that do not occur in the cited text are
  dropped. Changes of substance (a different date, a new paragraph) are
  not style: they go to fact gleaning, never to guide proposals.
- *Unchanged drafts count*: a draft sent as written (distance ≤ 0.05)
  supports the entries that applied to it; an entry the user's edits go
  against in three pairs is proposed for removal, with those pairs as
  its evidence. Content rules (group F) are not compared: they are not
  style.
- *Merge*: the same proposal in the same category across pairs and days
  becomes one proposal whose evidence grows. A rejected proposal is not
  raised again; one that contradicts an accepted entry is marked so.
- *Thresholds*: a guideline needs evidence from two pairs, a rule three,
  before it is shown. Weaker proposals wait under *Watching*, collecting
  evidence.
- *Cost cap*: at most 50 pairs a day (Settings); the rest wait for the
  next day, oldest first.

**Proposed rules and Review mode** (amended 2026-10-07). Deciding is a
different activity from reading mail, so it has a mode of its own. The
Writing Guide's list shows the categories only; under its title a band
says, large, how many proposed rules wait ("3 proposed rules waiting for
you", or "2 patterns collecting evidence" when only those do) with one
button, *Review*. The band, the sidebar badge and the daily notification
are the ways in; nothing enters the mode by itself.

In Review mode the whole window is the decision: sidebar and reader are
gone until *Done* (top left, also Esc). The toolbar shows the title
("Proposed Rules"), the progress ("3 of 7") and *Accept All*. On the
left, the queue (340 points): every decision the mode opened on, the
learning runs' decisions (§14.9) first, the daily reviews' proposed
changes after, patterns short of the threshold last under *Collecting
evidence*; each row a symbol, the statement and a caption (the change:
*new*, a change, *remove*; its category; its strength, "seen in 4
replies"). The current row is highlighted; arrows or j and k move. On
the right, the current decision as a document in large type, with the
same cues on both pages: a kind chip (Rule, Guideline, Fact), a category
chip, a source chip ("From learning your sent mail", "From the daily
review", "Collecting evidence"); the statement in display type, a change
with the old line struck above the new; a caution block for a conflict
("Goes against an entry of yours", with *Use This Instead* and *Keep
Mine*); the evidence as blockquotes, and for a review proposal the
messages behind it, the AI's draft beside what the user sent with the
differing words marked (*Show All N Messages*). The actions sit in a bar
at the bottom (*Accept*, *Edit…*, *Reject*; Return, e, ⌫) with ⌘Z named
beside them. After a decision the next one waiting becomes current.
Decided items stay in the queue, dimmed with their outcome (*Accepted*,
*Left out*, *Used instead of yours*), until the user leaves, so the run
is visible and Undo has somewhere to land: an item Undo puts back waits
again. When nothing is left, the right pane shows the run's summary ("5
added to your writing guide, 2 left out") with *Done* and *Undo*. The
window keeps its size throughout. Before 2026-10-07 the header carried a
*Review N Proposed Rules* button that filled the detail with every card
at once.

The Writing Guide's actions are in the window toolbar when the page is
open, where the mail actions would be: *Learn from Sent Mail*, a *More*
menu (*Ask … to Change the Guide…*, *Answer Questions…*, *Run Review
Now*, merge and export) and *Learning Settings*. The list's subtitle
says how many entries, how many wait and when learning last ran
("Learned today"). Over the list, only what is happening now: the
learning progress bars while a run is going, the review's progress while
it runs, and why a review waits or failed.

Each decision is one change on the account's undo stack (§14.6a),
recorded with the guide's own change so one Undo puts the guide and the
proposal back, and accepted guide changes make a new guide version. A
rejected proposal is never raised again (rejecting is *Don't Suggest This
Again*); any pair offers *Ignore Edits to This Message*, which takes it
out of every open proposal. The sidebar's Writing Guide entry counts the
proposed rules, and a red dot shows while there are ones created since
the user last opened the Writing Guide (an unseen signal, not a count).
Facts has its own count and dot (§14.11); each page clears only its own.

Learning Settings (from either page's toolbar, and Settings › Learning)
says when the review last ran, what it examined (pairs matched,
unmatched), the next run, how much AI drafts get changed (the median
distance over four weeks) and how many were sent as written, with *Run
Review Now* (amended 2026-10-07: these left the Writing Guide's header).
Over the Writing Guide's list, *Pause* and the progress bars while it
runs: first the drafts compared ("3 of 10 compared"), then *Looking for
facts in your sent mail* with no count; a review with no edited drafts to
compare goes straight to the facts step and says so when it finishes. The
Writing Guide shows each entry's health: how often drafts that applied it
were sent unchanged or overridden.

**Settings** (*Learning Settings* from the Writing Guide's and Facts'
headers, and Settings › Learning; per account,
except the notification, which is app-wide):
*Daily review* on or off; *Learn facts from*: *Off* · *Mail written with
AI* · *All mail I send* (default *Mail written with AI*; received mail is
never used); *Pairs a day* (default 50); *Keep AI drafts for* 7, 30 or 90
days (default 30); an opt-in notification, "3 new proposals from your
mail", which opens the page with something new,
once a day and only when the app is not in front (off by default). The
account menu shows a small dot on accounts with unseen proposals.

**Privacy.** Recording is local. The review sends matched pairs, and with
*All mail I send* the day's sent mail, to the user's own agent CLI, the
same disclosure as learning; the Writing Guide's header says so before
the first review.
The full AI and sent texts are kept for the retention period after the
record is reviewed; after that only the distance, status and proposal
links remain. Nothing changes the guide or facts without the user
accepting, and every change is undoable and versioned. Reviews cannot
draft, send or change mail.

### 14.11 Facts **(Amendment 2026-10-05)**

Facts about the user, their work and the people they mention have a place
of their own, used by every AI that composes for the user. They replace
the writing guide's category F3; accepted F3 entries were moved into
Facts, with a guide version noting it. Decisions: ADR 0012; plan
`docs/plans/analysis.md`.

**A fact** has:

| Field | What |
| --- | --- |
| category | a built-in or custom category |
| label | short, unique within its category ("Title", "Calendar link") |
| value | the fact itself |
| scope | *account* (this account) or *global* (every account) |
| use | *Use freely*, *Ask before using* or *Never share*; default *Use freely* (People and the sensitive starter categories default to *Ask before using*) |
| as of | when it was true; facts that age (travel dates, a headcount) are flagged for review when old |
| source | *You*, *Learned* (with evidence quotes) or *Writing help* (an answer to its question) |
| status | *proposed*, *accepted* or *rejected* |

**Built-in categories**, the few that fit any account, personal or for
work. They can be hidden but not renamed or deleted:

| Key | Category | Labels it suggests |
| --- | --- | --- |
| `identity` | Identity | Full name, Preferred name, Pronouns, Name pronunciation |
| `contact` | Contact | Phone, Other email addresses, Mailing address, Website or profiles |
| `availability` | Availability | Time zone, Usual hours, Calendar link, Where I usually am, Away or travel dates |
| `people` | People | Who people the user mentions are to them and how to refer to them; default *Ask before using* |
| `work` | Work | Occupation or role, Organisation, Team; not shown while empty |
| `preferences` | Preferences | How the user likes to be reached or to meet, things to keep in mind when making plans |
| `other` | Other | Anything that fits nowhere else, until it is moved |

**Custom categories** have a name and a one-line description ("Properties
I'm currently selling"); the description tells the gleaning prompt and
the drafting agent what belongs there. They can be renamed, reordered and
deleted (their facts move to Other, undoably). A new name close to an
existing category offers that category instead. A category belongs where
it was made, an account or global; making a fact global makes its custom
category global too.

**Starter sets** add a few custom categories at once, then edited like any
other:

| Set | Categories |
| --- | --- |
| Business | Company; Products and services; Customers and markets; Pricing and terms (*Ask*); Funding and investors (*Ask*); Policies and support; Approved wording (ties to F6); Links and resources |
| Freelance or consulting | Services and rates (*Ask*); Portfolio and references; Availability for new work |
| Household | Home; Family logistics (*Ask*); Health providers (names only, *Ask*) |
| Job search | Experience and skills; Roles I'm looking for; References (*Ask*) |

The daily review proposes a new custom category when three or more facts in Other
look alike, and a starter set when gleaning keeps finding facts that fit
one.

**Never stored**, even when found: passwords; card and bank numbers;
government ids; health details about others; anything about third parties
beyond their name, role and how the user knows them. Gleaning drops these
by pattern before a proposal is made, and its prompt says so.

**Where facts come from**: the user (the Facts editor, and the
interview, whose F3 questions become fact questions in Identity, Contact,
Availability and Work); writing help's questions (§14.9, *Missing
facts*); and *gleaning* in the daily review (§14.10), when *Learn facts
from* is not *Off*: from each day's AI-matched sent mail, or all mail sent
that day, the agent extracts facts about the user and their work, each
with a quote verified against the message. A value that differs from an
accepted fact becomes a proposal to change it; a fact contradicted by
recent mail can be proposed for removal. Every gleaned fact is a proposal
in Facts, shown at once (one message stating a fact is enough, unlike a
habit); a starter set is proposed once three reviews find facts that fit
it. With *All mail I send*, a review reads up to 30 messages sent since
the last one read.

**Scope.** Facts are per account by default. *Make Global* moves a fact
into the global store (ADR 0012), where every account reads it; *Make This
Account's Only* moves it back. An account fact with the same category and
label as a global one overrides it for that account. Settings › Facts
lists the global facts with the same editing.

**In prompts.** Facts in a hidden category are not used. Facts render
into the guide's "Facts about the user you may use" section: *Use freely* facts as facts, *Ask before using* ones
marked "ask the user before using", *Never share* ones left out. Global
facts merge under account ones. Agents can also read them through a
read-only `facts_lookup` tool (by category or words; never-share facts
left out), so a long list need not sit in every prompt; tool names allow
no dots.

**The Facts page** (a sidebar entry under the Writing Guide, laid out like
it; amended 2026-10-07): its actions are in the window toolbar when the
page is open: *Add Fact*, a *Categories* menu (*Add Category…*, *Add
Categories › From a Starter Set…*, export and merge) and *Learning
Settings*. Under the title a band says, large, how many proposed facts
wait, with *Review*, which opens Review mode (§14.10) on them: the
queue of proposed facts, categories and starter sets on the left, the
current one on the right as a document: a kind chip (Fact, Changed fact,
Category, Starter set), the category and source chips, the label small
and the value in display type (a change with the old value struck), the
quote it came from as a blockquote, and *Drafts may* with how freely
drafts may use it (*Use freely*, *Ask before using*, *Never share*;
preset to the category's default, or the fact's own for a change,
applied in the same change) and one line saying what the choice means;
*Accept* and *Reject* in the bar below. Return accepts, ⌫ rejects; the
next one waiting is then current; decided ones stay dimmed with their
outcome until *Done*. The subtitle says how many facts, how many wait
and when the review last ran. Before 2026-10-07 a header over the list
held the buttons and a *Review N Proposed Facts* button that filled the
detail with every card. The list shows facts by category, with a globe on global ones,
custom categories after the built-in ones in the user's order. The
sidebar entry counts proposed facts and shows a red dot while one is new.
On a fact: edit, delete, change *use*, *Make
Global* and *Make This Account's Only*. Each is one undoable change
(§14.6a). The list exports to Markdown and JSON with its custom
categories, and merges into another account's facts, creating categories
that are missing. The Writing Guide's F3 category points to Facts. The
interview's fact questions and writing help's
kept answers (each question carrying a category and label) write facts.
Settings › Facts lists the global facts with the same editing; its
changes go on the open account's undo stack.

### 14.12 Clean Up **(Amendment 2026-10-08)**

A window for clearing a mailbox in bulk, used once a quarter or a year
rather than every day: the account's mail grouped by sender, subject,
time, size and so on; tick groups and archive, move, trash or mark as
spam thousands of messages at once, with one undo. Plan
`docs/plans/overnight-2026-10-08.md`, feature 2.

**Where.** Its own window (`Window("Clean Up", id: "cleanup")`, like
Routines), opened from *Mailbox › Clean Up Mailbox…* and from *Settings ›
Accounts › Clean Up…*. It cleans the open account. A one-time tip in the
Inbox suggests it when the Inbox holds more than 1,000 messages. Nothing
is added to the main sidebar.

**Scope.** *Inbox* (messages carrying `INBOX`) by default; an *All Mail*
toggle widens it to every message except Spam, Trash and drafts.

**Views**, the window's left column. Each groups the messages in scope;
a group shows a title, an "aka" line where the view has one, and its
message count. Groups are ordered by count, largest first; Time and Size
keep their own order.

| View | Groups by | Title, aka |
| --- | --- | --- |
| Sender | `from_email`, ignoring case | the most used name (else the address); the other names as aka; the address below |
| People I've Emailed | as Sender, for senders the user has written to (`contacts.sent_count > 0`) | as Sender |
| Subject | identical subject, as stored (`Re:` kept) | the subject, or "(no subject)" |
| Mailing Lists | `List-Id` | the list's most used name (else its id); other names as aka; the id below |
| Time | Today, Yesterday, This Week, Last Week, then calendar months, newest first | "Today" … "September 2026" |
| Social | sender domain, among messages with `CATEGORY_SOCIAL` | the domain; the senders' names below (not aka: many senders share a domain) |
| Promotions | sender domain, among messages with `CATEGORY_PROMOTIONS` | as Social |
| Size | Tiny < 1 KB, Small 1–10 KB, Medium 10–100 KB, Large 100 KB–1 MB, Extra Large 1–10 MB, Jumbo > 10 MB | the bucket, smallest first |

Time uses the user's calendar: weeks start on Monday; a day belongs to
the first bucket it fits (on a Monday, yesterday is Last Week's); months
hold what is older than Last Week; mail dated in the future is Today's.
Sizes are the provider's (Gmail's `sizeEstimate`, IMAP's `RFC822.SIZE`),
in decimal units as macOS shows them. Social and Promotions use Gmail's
own categories; there is no list of brands. A filter field above the
groups ("Type a sender…") keeps groups whose title, aka, address or key
contains what is typed.

**Messages, not threads.** Groups count messages, and actions change
those messages only: archiving the "Amazon" group leaves the replies of
real people in a mixed thread where they are.

**Actions.** The toolbar's *Archive*, *Move…* (a label), *Trash* and
*Spam* apply to every message in the ticked groups. The set is resolved
when the action runs, so a group that grew since it was shown is acted
on as it is now. One action is one undo entry with per-message diffs
(ADR 0006); the changes go to the provider through the outbox in chunks
of 1,000 (`batchModify`'s limit), with progress shown in the toolbar.
Optimistic local copies of sent mail are left out.

**Loading every header.** Clean Up needs the whole mailbox, so opening it
sets the account's sync window to *Everything* when it is narrower
(headers only; the body window is unchanged, §7.4) and shows the header
load's progress; the window says the setting changed, and Settings ›
Accounts shows it. Without IMAP, where headers cost as much as whole
messages, the window states the message count and how long the download
will take, and asks before starting. *(Decided 2026-10-08: this is the
one exception to changing the setting without asking; the user's *Not
Now* holds until the app quits.)*

**Mailing lists from new mail only.** `List-Id`, `List-Unsubscribe` and
`List-Unsubscribe-Post` are stored from 2026-10-08 on (§6.2), on every
fetch path; old mail is not fetched again for them. The Mailing Lists
view fills as mail arrives and says so while it is empty.

**Progress card.** Under the views: Inbox Zero as a percentage of the
Inbox when Clean Up was first opened (the baseline), a sparkline of the
Inbox's daily count, and four numbers: At Midnight, Received Today,
Removed Today, Now. The daily count is recorded at the first sync after
midnight and when Clean Up opens.

**Not in scope** (2026-10-08): Block, Chill and Expire (standing local
rules; perhaps built-in routines later) and Forward. *Unsubscribe* for
groups whose messages carry `List-Unsubscribe` comes with the Mailing
Lists view: the one-click POST (RFC 8058) after a confirmation naming the
sender and the URL's host, or a `mailto:` opened in the composer for the
user to send; never automatic, never an agent's.

*(Implemented 2026-10-08, store: migration `0018_cleanup`, the list
headers on the IMAP, REST and MIME paths, and `mail_store::cleanup`:
groups, a group's messages (paged), their count and ids, the Inbox's
daily counts and the baseline. Groups for a 131,826-message store answer
in 1–45 ms per view in a release build (docs/performance.md).)*

*(Implemented 2026-10-08, core: `cleanup_groups`, `cleanup_messages`,
`cleanup_count` and `cleanup_apply` take the account's id, since the
window cleans one account whatever the main window shows, and use the
real clock and the Mac's offset from UTC for the Time view. All four are
`async` (§4.2): the apply writes every message in one transaction, about
0.6 s for 20,000 in a release build (docs/performance.md), which must
not run on the main thread. Actions: Archive (out of the Inbox), Move
(the label added and out of the Inbox, as the mail list's Move; only a
user label or the Inbox), Trash, Spam (to Spam and out of the Inbox).
The set is resolved inside the apply's transaction; messages already so
are not counted and not recorded, and an apply that changes nothing
returns no undo token. The result carries the count changed, the token
(undone with `undo_action`/`redo_action` like any mail action), the
notice ("Archived 813 messages from Amazon"; "from N groups" when
several are ticked; "20 large messages" in Size, "with the subject …" in
Subject) and Edit › Undo's name. Provider ops: the messages are grouped
by their exact change and queued as `ModifyLabels` outbox rows of at
most 1,000 ids, one `batchModify` each; a row could hold more (the Gmail
provider chunks), but one row per call means a retry repeats one batch
and a failure rolls back exactly its own messages. Trash and Spam go
the same way, adding `TRASH` or `SPAM` as labels, rather than through
`messages.trash`'s call per message; undo and redo of a Clean Up action
are batched alike (its undo record's kind starts `cleanup_`), while
conversation actions keep the trash endpoints (§14.6a). Progress: the
outbox emits `OutboxStatus` after each batch while more are waiting, so
the toolbar can show what is left to reach Gmail; the local write needs
no progress of its own. `cleanup_progress` (the card's numbers) is left
for the progress card's issue.)*

*(Implemented 2026-10-08, window: `Features/CleanUp/`. Sender, People
I've Emailed, Subject, Time and Size are listed; Mailing Lists, Social
and Promotions join with their issues. Highlighting and ticking are
separate: rows highlight as in the mail lists, the checkbox or Space
ticks, and only ticks fill the messages column and are acted on; ticks
stay while the filter changes and clear when the view changes or an
action succeeds. Keys in the groups list: Space, `e`, `⌫` or `#`, `!`,
`j`/`k`. The menu item has no shortcut. *Settings › Accounts › Clean
Up…* switches the mail window to that account first, since the window
cleans the open account; imported mailboxes, being read-only, have no
Clean Up. The undo notice shows in the window that acted (a notice
carries its origin) and ⌘Z there undoes from the account's stack. The
messages column fetches pages of 200 as rows come into view. While the
window is open, changes from sync or the mail window refresh it at most
every 2 s.)*

*(Implemented 2026-10-08, loading every header: `cleanup_load_status`,
`cleanup_load_every_header` and `cleanup_load_estimate`. On opening, a
Gmail account whose window is narrower than *Everything* and whose
headers are cheap (IMAP granted and not refused just now) is widened at
once; otherwise the window asks with the count (Gmail's
`messagesTotal` less the messages stored) and the time at the rate the
backfill runs over the API (5,000 units a minute, 20 per
`messages.get`: 250 messages a minute), or "all older mail" when Gmail
cannot be asked. Widening keeps bodies where they were: a body window of
"the whole window" becomes the span the old window covered (a new *Last
year* body window exists for that), so the older mail is queued for
headers only and no body outside the body window is fetched (tested
against the IMAP fake and `FakeProvider`). An info band over the groups
shows "Loading headers for all mail — N of M" from `SyncStatus`'s
headers count (over the API, "Loading all mail", counting whole
messages) and says the sync window is now *Everything*; the groups fill
as headers arrive. Imported mailboxes, agent mailboxes and the demo have
no sync window and load nothing. If IMAP is refused part way, the engine
promotes the headers-only tier to whole downloads as for any
*Everything* account (§7.4).* *(Amended 2026-10-08, oagc-merk.8: not
any more. A widening with cheap headers records that the headers-only
tier is Clean Up's (`cleanup_headers_only = ask` in `sync_state`; set
from the account's IMAP grant when widened while not syncing); if IMAP
is then refused, that tier waits (no whole download unasked, also across
a restart while refused), the engine reports progress once, and
`CleanupLoadStatus.headers_paused` is set. The window, on opening or when
the band's header count stops moving, asks the same *Load All Mail* /
*Not Now* question with the count still waiting and its time over the
API (`cleanup_load_estimate` answers with those while paused); the band
reads "Waiting for IMAP — headers for N older messages are still to
load". *Load All Mail* calls `cleanup_load_waiting_headers`, which
promotes the tier to whole downloads and makes it no longer Clean
Up's; *Not Now* holds for the session, and IMAP coming back (after the
refusal's hour, or ⌘R) resumes headers only. Setting the sync window in
Settings makes the tier the user's again (`no`), so §7.4's promotion
applies, as for windows from before Clean Up. Tested against the IMAP
fake (refused logins, then allowed) and `FakeProvider`.)*

*Also 2026-10-08: Size groups show their range as the second line ("Less
than 1 KB" … "More than 10 MB"), and optimistic local copies of sent
mail are out of scope everywhere (groups, counts, messages, actions), so
"N messages in M groups" counts exactly what an action may change; the
result's count and notice still give only the messages that changed.)*

*(Implemented 2026-10-08, progress card: `cleanup_progress(account)`
returns the baseline, the percentage, At Midnight, Received Today,
Removed Today, Now and the last 30 days' counts at midnight (today's
last; the sparkline ends with Now). The day's count is recorded by the
first incremental sync after local midnight once the first listing is
done, and by `cleanup_progress` itself, which the window calls as it
opens and after every change; the first record of a day stands. A count
recorded after midnight is the Inbox then less what arrived since
midnight and is still in it, so the day starts consistent (Removed Today
0). Received Today is the mail that arrived today by `internal_date`
(not the user's own, not drafts, not Spam) wherever it is now, not only
what is still in the Inbox: otherwise archiving today's mail would not
count as removed. Mail a Gmail filter keeps out of the Inbox therefore
counts as received and removed; the percentage, which uses only the
baseline and Now, is unaffected. The baseline is set the first time and
rises when the Inbox outgrows it (older Inbox mail arriving as every
header loads would otherwise pin Inbox Zero at 0 %); it never falls. An
empty Inbox is 100 %. Migration `0019_cleanup_progress` indexes
`internal_date`. The card is `CleanUpProgressCard` at the foot of the
views and refreshes with the groups: after every action, undo and redo,
and when mail changes.)*

*(Implemented 2026-10-08, Social and Promotions: listed between Time and
Size, filtered with "Type a domain…". A domain group's second line names
its senders, most used first, three at most ("Status Alerts, Billing and
2 more"), rather than calling them aka, since many senders share a
domain; the filter still finds a domain by a sender's name. When the
view is empty the window says whether the scope has none ("No promotions
in the Inbox.") or the mailbox has no mail in that category at all, as
IMAP-only, imported and agent mailboxes, which Gmail does not sort.)*

*(Implemented 2026-10-08, Mailing Lists and Unsubscribe: Mailing Lists
sits after Subject; its groups are titled by the list's most used name,
else its id, with the id below (the spec's choice, kept over the domain
alone: the id names the list and ends in its domain); while empty it
says "Mailing lists show here as new mail from them arrives."
*Unsubscribe* is in the toolbar of Mailing Lists, Sender and People (the
other views name no list or sender) and is enabled when a ticked group's
newest message in scope carries `List-Unsubscribe`; an older message's
address may have expired, so it is not used. Each list is asked once
(groups sharing a List-Id, or without one the same address, are one).
`List-Unsubscribe-Post: List-Unsubscribe=One-Click` with an https URI
means one POST from the core with RFC 8058's body, `application/x-www-
form-urlencoded`, after the confirmation: no cookies, no credentials (an
address with a user name is refused), no redirect followed, 5 s to
connect and 10 s in all; a 2xx answer counts as done *(amended below: a
3xx is not)*. The address is read from the store again when the user
confirms, never passed in by the app *(amended below: from the very
messages the confirmation showed)*. Otherwise a `mailto:` URI (RFC 6068: To, Cc, Subject and Body,
`+` kept) opens the composer filled in for the user to send; a web page
alone is not offered. The confirmation names each list and the host (or
the address), says ticked groups without a link are left alone, and
offers *Archive Them Too*, off, which archives the ticked groups as one
undoable action afterwards *(amended below: only the lists left)*. What happened shows over the groups. A
one-click success is remembered (`cleanup_meta`, `unsubscribed:list:<id>`
and, from Sender or People, `unsubscribed:from:<address>`), and the
group's second line then starts "Unsubscribed"; a mailto is not, since
sending it is the user's. Not an agent tool, and never automatic. The
Inbox tip suggesting Clean Up shows when the Inbox holds more than 1,000
conversations (the sidebar's count; so more than 1,000 messages), before
the other tips, until put away or until Clean Up is opened.)*

*(Amended 2026-10-08, review fixes, oagc-merk.9–19.)*

- *The day's count waits for the Inbox.* The first sync of a day, and
  `cleanup_progress`, record the count at midnight (and set or raise the
  baseline) only once the Inbox phases are fetched: while an id queued at
  an Inbox priority (0 or 1, which new mail also uses) has no row yet,
  nothing is recorded and the card works its numbers out live; a later
  poll or opening records them. The listing marks an account bootstrapped
  before the backfill stores what it listed, so a first sync could
  otherwise record a partial Inbox that stood all day.
- *Unsubscribe from what was shown.* Each target carries, per group, the
  id of the newest message its address was read from.
  `cleanup_unsubscribe` takes the confirmed targets, reads those groups'
  newest messages again and posts only if they are still the same
  messages and the address still leads to the host shown; otherwise
  nothing is sent for that list and its line says "New mail arrived from
  this list; review it again". A one-click address is offered only on the
  list's own site: the URL's registrable domain (its last two labels, or
  three under a country's own second level such as `co.uk`; a short list,
  not the Public Suffix List, erring towards "foreign") must be the
  `List-Id`'s or the sender's. Otherwise the list's mailto is offered, or,
  without one, the group counts as having no link Clean Up can use. A
  link the user would open in the browser is not offered.
- *Never to the local network* *(amendment 2026-10-08, oagc-cp3.1)*.
  `List-Id` and `From` are the sender's to write, so the one-click
  address must also be https on port 443 at a host name (not an IP
  literal), with at least two labels and not under `.local`,
  `.localhost`, `.internal`, `.lan`, `.home.arpa`, `.intranet` or
  `.corp`. When posting, the name is looked up once and refused if any
  answer is loopback, unspecified, private (RFC 1918), shared (CGNAT
  100.64/10), link-local, multicast, reserved, unique-local (fc00::/7)
  or an IPv6 form of one of those; the POST then connects to exactly the
  addresses checked (no second lookup, no proxy), so a name cannot
  rebind between the check and the connection. Unit tests' local
  servers are allowed by port through a `cfg(test)`-only list that the
  app does not compile.
- *Only a 2xx is done.* A redirect is not followed and not recorded: the
  line says "the list wants you to open a page at *host* to finish".
- *Archive Them Too* archives only the groups whose one-click succeeded or
  whose message opened in the composer; the sheet says so ("Archives only
  the lists that take you off, and those whose message opens for you to
  send"). Groups that failed or had no link stay, still ticked.
- *Spam leaves the user's own sent mail alone* (`SENT`): it is not spam,
  and whether Gmail accepts `SPAM` on a sent message is a hand-check (a
  refusal would roll back a batch of 1,000). Trash still takes it.
- *A message gone from the server costs only its own change.* A
  `NotFound` for a label change on several messages is split in halves
  and retried down to the missing ids, which are dropped (about 20 calls
  for one missing id among 1,000); per-message trash calls go on past a
  missing one. What `batchModify` answers when one id is gone is a
  hand-check.
- *Widening without asking re-checks.* `cleanup_load_every_header`
  takes `expect_cheap`: the window widens unasked because
  `cleanup_load_status` said headers are cheap; if IMAP was refused in
  between, nothing changes and the answer is `NeedsAsk`, so the window
  asks *Load All Mail* / *Not Now* as without IMAP. *Load All Mail*
  passes false.
- *The body window changes only with cheap headers.* Over the API every
  message comes whole, so the body window is left as the user set it.
  When Clean Up does pin it (`Widened { body_window }`), the band adds
  one sentence: "Full messages still download for the last 6 months only
  (Full messages for, in the same place)."
- *Waiting is idle.* While Clean Up's headers-only tier waits for IMAP and
  nothing else is queued, sync reports the account idle (the header count
  still goes out for the band), so the main window does not show it
  syncing for ever after *Not Now*.
- *The window drops a view's groups when the view or scope changes,* and
  ignores ticks until the new groups load, so a tick cannot name a key of
  the view just left.
- *AgentMail:* moving mail to Trash or Spam (Clean Up's actions, Mark as
  Junk) leaves the Inbox on this Mac only; no `archived` label is added at
  the service (§7.9, ADR 0014).

---

## 15. Security Model and Threat Model

### 15.1 Assets

Mailbox content, OAuth tokens, the ability to send as the user, the agent
CLI's credentials (not ours, but in our process tree), the user's files.
*(Amended 2026-10-08, ADR 0016:* an agent mailbox's guide and shared
facts once published to a rules server (§10.6), and the tokens that reach
them.)

### 15.2 Adversaries

1. **Malicious email author** aiming at the user (phishing, tracking, HTML
   exploits) or at the agent (prompt injection).
2. **Malicious or confused agent** — the model follows injected
   instructions, hallucinates a destructive action, or loops.
3. **Local malware** with the same UID (out of scope beyond not making
   things worse; we cannot defend against it).
4. **The project itself** — must be *unable* to see mail (no backend).
   *(Amended 2026-10-08, ADR 0016: it may run a rules server, which
   still never holds mail.)*
5. **A leaked rules-server agent token** *(amendment 2026-10-08)* — reads
   one mailbox's published rules and shared facts and can file reports,
   until revoked.
6. **The rules server's operator** *(amendment 2026-10-08)* — the user,
   or the project for the hosted server; can read what was published
   during requests.

### 15.3 Controls

| Threat | Control |
|---|---|
| HTML/JS exploitation | Rust sanitization + JS-off WKWebView + CSP + no navigation (§14.4) |
| Tracking pixels | Remote images blocked by default |
| Phishing links | Host mismatch confirmation; links open in system browser only |
| Prompt injection → exfiltration by email | `mail.send`/`forward` always approval-gated; recipients frozen and displayed at approval; agent has no other output channel (no shell, no filesystem, no web). Exception: an agent mailbox set to *Send freely* (§7.9) sends without asking, by the user's choice |
| Prompt injection → destructive bulk actions | `delete` gated; bulk caps; reversible ops are actually reversible (archive not delete; trash not purge) |
| Prompt injection → credential theft | Tokens never reach the agent process; Keychain only touched from Swift; MCP tools cannot read settings |
| Agent escapes tool boundary | Claude: `--tools ""` + `dontAsk` + `--strict-mcp-config`; Codex: shell/exec/web tools disabled, read-only sandbox, `--ignore-user-config`; both are belt-and-braces — the real boundary is that OpenAGC only ever *offers* mail tools |
| Rogue MCP client on the socket | Per-launch random socket path, 0600, peer UID check, per-session token in the shim args. Mailbox mode (§10.1) has no per-session token: any same-user process can open a session on any agent mailbox (accepted, §15.4) |
| Rogue socket for the shim | In mailbox mode the shim connects only to a socket owned by the user, in a folder only the user can write (sticky parent allowed), served by a same-user peer *(amendment 2026-10-08, oagc-cp3.4)* |
| Attachments | Never auto-opened; saved with quarantine xattr (`com.apple.quarantine`) so Gatekeeper applies; agent gets extracted text only |
| Log leakage | `Redacted` newtypes; email bodies never logged above `trace`, which is compiled out in release |
| Supply chain | `cargo deny` (licenses, advisories), `cargo audit` in CI, Swift packages pinned by revision, Sparkle EdDSA-signed updates |
| The project sees mail through the rules server | The server holds no mail, no service key and no OAuth token; reports carry only what the agent wrote; it serves agent mailboxes only *(amendment 2026-10-08, ADR 0016, §10.6)* |
| Leaked rules-server agent token | Scoped to one mailbox; reads only published rules and shared facts; cannot read mail or send; stored as a hash; revocable in the app; reports name the token *(amendment 2026-10-08)* |
| Rules server's operator or a leaked database | Only what the publish sheet listed leaves the Mac; no evidence quotes; audience addresses as salted hashes; facts shared one by one; snapshot encrypted at rest with the key wrapped per agent token (required when project-hosted). Does not stop an operator who changes the code *(amendment 2026-10-08)* |

### 15.4 What the MVP does *not* protect against

Malware running as the user; a compromised agent CLI binary; the user
approving a bad send. These are documented in `docs/security.md`.

*(Amendment 2026-10-08, oagc-cp3.6.)* Mailbox mode trusts the user's own
processes: `openagc-mcp --mailbox` carries no per-session token, so any
process running as the user can drive any agent mailbox (read its mail,
guide and the user's facts; send from it). Under *Ask before each send*
every send still waits for the user in the app (and is refused with the
app closed); under *Send freely* it goes. This is accepted, like the
user's other CLI tools: the boundary is the user account, a token readable
by the same user would not stop such a process, and the shim holds no
secrets. Own accounts are never served, only the six mailbox tools exist,
and every call is logged under the outside agent's name. The injection
suite covers the six tools.

---

## 16. Build, Signing, Distribution **(Verified)**

- **Toolchains**: Rust pinned in `rust-toolchain.toml` (1.98.1; `rust-version`
  1.92, the `rmcp` MSRV); Xcode 27; XcodeGen via Homebrew and
  `uniffi-bindgen-swift` built from the workspace, all installed
  via Homebrew/cargo in `scripts/bootstrap.sh`.
- **Signing**: Developer ID Application certificate; hardened runtime on
  the app, `openagc-mcp`, and Sparkle's XPC services; entitlements:
  `com.apple.security.cs.allow-unsigned-executable-memory` **not** needed
  (WebKit JIT lives in Apple's XPC), `disable-library-validation` **not**
  needed (static Rust). No App Sandbox: the app must spawn the user's
  arbitrary `claude`/`codex` binaries, which a sandboxed process cannot.
- **Notarization**: `xcrun notarytool submit --wait` with an App Store
  Connect API key stored as a CI secret, then `stapler staple`.
- **Packaging**: DMG built with `create-dmg`; the DMG is also notarized.
- **Updates**: Sparkle 2.10+ via SwiftPM, EdDSA key generated once
  (`generate_keys`, private key in the release maintainer's Keychain and as
  a CI secret), appcast generated by `generate_appcast`, hosted on GitHub
  Releases with `SUFeedURL` pointing at the raw appcast. Beta channel via
  Sparkle channels.
- **CI** (`.github/workflows/ci.yml`, macOS 26 runner): `cargo fmt --check`,
  `cargo clippy -D warnings`, `cargo test`, `cargo deny check`, build the
  XCFramework, `xcodebuild test`. Release workflow tags → builds → signs →
  notarizes → uploads DMG and appcast.
- **Bundle identifier**: `ai.actual.openagc` (§20).

---

## 17. Logging and Diagnostics

`tracing` throughout Rust. Two subscribers: a rolling file at
`~/Library/Logs/OpenAGC/core.log` (info and above, 5 × 10 MB) and a
`Layer` that forwards `warn`/`error` events over the `EventListener` so
Swift logs them with `os.Logger(subsystem: "ai.actual.openagc", category:)` —
keeping unified-logging privacy annotations under Swift's control rather
than trusting a third-party bridge crate. Swift uses `os.Logger` directly.
A "Collect diagnostics" button zips both logs with secrets scrubbed. No
crash reporter in MVP.

---

## 18. Testing

| Layer | Tooling | What |
|---|---|---|
| Rust unit | `cargo test` | parsers (search grammar, MIME), sanitizer policy (golden files of nasty HTML), permission engine (table-driven), sync state machine |
| Rust integration | `cargo test` + `wiremock` | Gmail client against recorded fixtures; full bootstrap + incremental sync against a fake Gmail; outbox retry/conflict |
| Store | `cargo test` on temp DBs | migrations forward from every version; FTS trigger consistency; keyset pagination invariants |
| MCP | `rmcp` in-process client | every tool's schema, caps, and gating; injection scenarios (an email asking to forward mail must yield a *pending* send, never a sent one) |
| Agent adapters | fake `claude`/`codex` shell scripts emitting recorded NDJSON / JSON-RPC | event mapping, resume, cancel, auth-failure detection |
| Routines | `cargo test` snapshot (`insta`) + fake agent | generated prompt per runner is byte-stable for the shipped template; RRULE scheduling incl. sleep/wake; inferred-run attribution from recorded history deltas; Undo re-checks current labels |
| Swift unit | XCTest | stores, HTML serializer, keychain wrapper (with a test keychain) |
| UI | XCUITest, small | launch, connect (mocked core), archive via keyboard, compose/send approval |
| Performance | XCTest `measure` + signposts | §1.3 targets against a 100k-message fixture (generated by a `cargo xtask`) |
| Security regression | golden suite | sanitizer and permission tests are the gate for any change under `mail-mime/` or `permissions/` |

Fixtures: a corpus of ~200 real-world-shaped MIME messages (multipart
edge cases, RFC 2047 headers, calendar invites, inline images, HTML-only
newsletters) lives in `crates/mail-mime/fixtures/`; contributors add a
fixture with every parser bug fix.

---

## 19. Milestones

Each milestone is a beads epic; tasks inside are beads issues.

| # | Milestone | Exit criterion |
|---|---|---|
| M0 | **Skeleton** | Cargo workspace + Xcode project build; UniFFI round-trip (`Core::new`, `ping`); CI green; `bd` initialized |
| M1 | **Read-only Gmail** | OAuth (shipped + BYO), bootstrap sync, incremental sync, inbox list, thread view with sanitized HTML, labels sidebar; perf harness reads a 100k fixture at target |
| M2 | **Full mail client** | Archive/read/label with outbox; search; composer (new/reply/forward, attachments); send; drafts sync; notifications; Sparkle updates; first notarized beta DMG |
| M3 | **Agents** | Detection, Claude + Codex adapters, MCP shim + socket, read tools, results-as-thread-list, cancel, transcript persistence |
| M4 | **Actions with approval** | Reversible tools, drafts by agent, send/forward/delete with approval UI, audit view, bulk caps, injection test suite |
| M5 | **Routines** | Routine model + template + prompt generator (snapshot-tested); structured editor; local runner with dry-run preview and scheduler; Claude cloud hand-off with fire-token Run now; inferred attribution and Undo |
| M6 | **Polish and 1.0** | Keyboard completeness, accessibility pass, dark mode, settings, onboarding, docs, Google verification submitted |

M1 and M3 can proceed in parallel after M0 since they share only
`mail-domain`. M5's model, template and generator can start after M0;
its runner depends on M4.

---

## 20. Maintainer Decisions (resolved 2026-09-23)

Tracked in beads as `oagc-45c`, `oagc-hcv`, `oagc-ams`, `oagc-gws`,
`oagc-8xu`, `oagc-igo`; all closed.

| # | Decision | Outcome |
|---|---|---|
| 1 | GitHub organization / repository | `audiojak/openagc` for now, to be transferred to an org later (GitHub redirects after transfer). `OpenAGC/OpenAGC` is an unrelated PlayStation 5 graphics library (Aug 2026); the product name stays OpenAGC and the README states the two are unrelated |
| 2 | Bundle identifier prefix | `ai.actual.openagc` (Actual AI's domain). App `ai.actual.openagc`, Keychain service `ai.actual.openagc.oauth`, MCP shim `ai.actual.openagc.mcp`, unified-logging subsystem `ai.actual.openagc` |
| 3 | Google Cloud project for the shipped OAuth client | Actual AI's Google Cloud org; the consent screen names Actual AI; restricted-scope verification and CASA are Actual AI's cost |
| 4 | Apple Developer Program team | Actual AI's team signs and notarizes |
| 5 | Agent model picker | None in MVP; each CLI uses its own configured default |
| 6 | Claude cloud routine publishing via `RemoteTrigger` | Enabled by default; the paste hand-off appears automatically if the tool is missing or the call fails |

---

## Appendix A — Sources verified 2026-09-23

Claude Code: `code.claude.com/docs/en/headless`, `/cli-reference`, `/mcp`,
`/authentication`, `/agent-sdk/overview`, `/agent-sdk/streaming-output`;
`support.claude.com` article 15036540 (subscription use with the Agent SDK).

Codex: `learn.chatgpt.com/docs/app-server`, `/non-interactive-mode`,
`/cli/reference`, `/extend/mcp`, `/config-file/config-reference`, `/auth`;
`openai/codex` repo: `codex-rs/app-server-protocol/src/protocol/common.rs`,
`codex-rs/exec/src/cli.rs`, `codex-rs/cli/src/login.rs`, PR #42993 (removal
of `codex mcp-server`), issue #16045 (`-c mcp_servers={}` caveat).

Routines: `code.claude.com/docs/en/routines`, `/desktop-scheduled-tasks`,
`/legal-and-compliance` (third-party credential policy), `/deep-links`;
`platform.claude.com/docs/en/api/claude-code/routines-fire` (the only
*documented* routine endpoint); `claude.com/connectors/gmail` (connector
tool list); **local verification 2026-09-23**: `claude -p … --allowedTools
RemoteTrigger` (Claude Code 2.1.267, subscription login) listed the
account's routines via `GET /v1/code/triggers` with HTTP 200, returning the
`job_config`/`mcp_connections` shape reproduced in §11.1 and §11.5;
`learn.chatgpt.com/docs/automations`, `/docs/cloud`,
`/use-cases/manage-your-inbox`; `openai/codex` issues #47660 (no cloud
scheduling), #13967 / #21995 (local `automation.toml`).

Google: `developers.google.com/workspace/gmail/api/reference/quota` (per-minute
quotas effective 2026-05-01), `/guides/sync`, `/guides/push`, `/guides/sending`,
`/guides/batch`, `/auth/scopes`; `developers.google.com/identity/protocols/oauth2/native-app`,
`/production-readiness/restricted-scope-verification`.

Rust: `uniffi` 0.32.2 (futures, foreign traits, `uniffi-bindgen-swift`;
issues #1726, #2811, #2992 on runtimes), `rmcp` 3.4.1, `rusqlite` 0.40.2,
`mail-parser` 0.11.9, `mail-builder` 1.0.0, `ammonia` 4.2.0, `oauth2` 5.0.0,
`security-framework` 3.7.0; `sqlite.org/fts5.html` (external content,
trigram).

Apple: Hardened Runtime and notarization documentation; Sparkle 2.10.0
release notes and documentation.

## Appendix B — Local environment at time of writing

macOS 26.6.2; Xcode 27.0 (Swift 6.4, macOS 27 SDK); Rust 1.98.1 via
Homebrew rustup; XcodeGen 2.46; cargo-deny 0.20; Claude Code
2.1.267; Codex CLI 0.145.0; beads 1.3.0.
