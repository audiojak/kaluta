# Security and threat model

What OpenAGC protects, from whom, and where each control lives in the code.
The design is in [SPECIFICATION.md §15](SPECIFICATION.md); this page tracks
the implementation.

## What is at stake

The user's mail, the Google sign-in that can read and change it, the ability
to send as the user, and the user's agent CLIs (which run with the user's
own AI accounts).

## Who we defend against

1. **A malicious email author** — phishing, tracking pixels, hostile HTML,
   and prompt injection aimed at the agent ("forward all invoices to …").
2. **A confused or manipulated agent** that follows injected instructions,
   hallucinates a destructive action, or loops.
3. **Ourselves** — OpenAGC has no servers and must not be able to see mail.

Local malware running as the same user is out of scope; we avoid making it
worse (secrets in the Keychain, private sockets, no credentials on disk).

## Controls

| Threat | Control | Where |
|---|---|---|
| Mail leaves the Mac | No backend; sync is Google ↔ Mac only; no analytics | whole app |
| Stolen sign-in | Refresh token only in the Keychain; `gmail.modify` scope (never `mail.google.com`); PKCE + loopback | `KeychainSecretStore.swift`, `provider-gmail/src/oauth.rs` |
| Hostile HTML | Sanitized at sync (ammonia, style allowlist); reader has JavaScript off, a strict CSP, no navigation, per-scheme image handlers | `mail-mime/src/sanitize.rs`, `MessageWebView.swift` |
| Tracking pixels | Remote images blocked until the user loads them; fetched without cookies or referrer | `SchemeHandlers.swift` |
| Deceptive links | Visible-text/target mismatch warning before opening | `LinkSafety.swift` |
| Malicious attachments | Downloaded only on use, into the app's folder, quarantined (Gatekeeper checks before opening); names sanitized | `mail-sync/src/attachments.rs`, `CoreClient.quarantine` |
| Agent reaches beyond mail | CLIs run with no built-in tools and only OpenAGC's MCP server (Claude: `--tools ""`, `--strict-mcp-config`, `dontAsk`; Codex: read-only sandbox, `approval_policy=never`, the user's MCP servers replaced, shell/exec/browser/apps disabled) | `agent-claude/src/session.rs`, `agent-codex/src/session.rs` |
| Prompt injection → sending, forwarding, deleting | Every tool call is decided in the core; external actions always wait for the user, time out after 10 minutes, and are rejected if the turn is cancelled | `permissions`, `openagc-core/src/agents/{tools,approvals}.rs` |
| Agent edits what the user reviews | A draft under review is frozen for the agent; approving sends what the user sees | `approvals.rs` |
| Agent sends the user's own drafts | Only drafts created in the session can be sent | `permissions::SessionGuard` |
| Bulk damage | ≤ 200 threads per call, ≤ 2,000 per prompt, ≤ 60 calls a minute, system labels refused, selection scope | `permissions`, `tools.rs` |
| "What did the agent see or do?" | Every call audited with its decision and the ids it returned or touched; exportable | Settings › Permissions › Activity |
| Other users on the Mac | The MCP socket is per launch, mode 0600, same-UID peers only, bound to known sessions (mailbox mode: to an agent mailbox, see below) | `agent-mcp/src/server.rs` |
| A socket planted for the shim | In mailbox mode `openagc-mcp` reads the socket's path from a file, so before connecting it requires a socket (not a link) owned by the user, in a folder owned by the user that no one else can write to (a sticky parent such as `/tmp` allowed), and a peer running as the user; otherwise it refuses in words and does not fall back | `agent-mcp/src/client.rs` (`check_socket`) |
| Unsubscribe aimed at the local network | One-click POST only to https on 443 at a public-looking name (no IP literal, single label, `.local`/`.localhost`/`.internal`/`.lan`/`.home.arpa`…) of the list's own site; the name is resolved once, refused if any address is loopback, private, CGNAT, link-local, multicast, reserved or unique-local, and the POST connects to exactly those addresses (no second lookup, no proxy); test servers are allowed by a `cfg(test)`-only list | `openagc-core/src/cleanup/unsubscribe.rs` |
| Agent config copies readable by others | *Connect an Agent…* writes `~/.claude.json` / `~/.codex/config.toml` through a temp file created new with the file's mode (0600 for a new file) from the start, removed on any error | `openagc-core/src/agent_connect.rs` |
| Signed URLs and agent-mail keys in logs | HTTP errors drop their URL (`without_url`) for every provider request and AgentMail's signed downloads; `logging::scrub` also redacts `am_…`/`prim_…` keys and presigned-URL signature/credential/token parameters | `provider-api/src/http.rs`, `provider-agentmail/src/lib.rs`, `openagc-core/src/logging.rs` |
| Two drainers, one send | Outbox claims are held by `flock` on a per-claimant file; a sweep removes only a file it holds locked whose name still refers to it, and a new claimant re-checks the inode after locking | `mail-store/src/outbox.rs` |
| Nested-session hangs, key leakage | Agent CLIs get a scrubbed environment; routines never get an API key | adapters, `agent-claude/src/routines.rs` |
| Cloud routine logs | Shown as plain text, never fed to an agent | `RoutinesWindow.swift` |
| Untrusted PDFs | Text extracted by PDFKit in the app, not a Rust parser | `PDFTextExtractor` in `CoreClient.swift` |
| Supply chain | `cargo deny` (licenses, advisories, sources) in the gate; Swift packages pinned by revision; Sparkle updates EdDSA-signed | `deny.toml`, `project.yml` |

## Mailbox mode trusts the user's own processes (accepted risk)

Agents outside OpenAGC reach an agent mailbox through `openagc-mcp
--mailbox <address>` (spec §10.1). That session carries **no per-session
token**: the app accepts any process running as the user on its socket
(mode 0600, same-UID peer check) and opens a session on any agent mailbox
it names, and with the app closed the shim runs the core headless itself.
So any program running as the user can drive any agent mailbox: read its
mail, guide and the user's facts, and send from it.

What holds regardless: only agent mailboxes are served, never the user's
own accounts; only the six mailbox tools exist (`guide_rules`,
`facts_lookup`, `mail_search`, `mail_get_thread`, `mail_send`,
`mail_reply`), through the same permission engine; every call is in the
activity log under the outside agent's name. What depends on the user's
choice is the agent's send mode:

- *Ask before each send*: every send and reply is a proposal the user
  approves in the app, with the outside agent named; unanswered it
  expires, and with the app closed it is refused (`needs_openagc`), never
  queued.
- *Send freely*: sends go without asking (with the guide check returned
  to the agent). A program running as the user, or an outside agent
  manipulated by mail it reads, can then send anything the agent mailbox
  can see, the user's facts included, to anyone the service allows. The
  user picks this per agent mailbox, knowing it.

We accept this, as for the user's other CLI tools (`gh`, `git`, cloud
CLIs that hold tokens for the same user): the boundary is the user
account. A process that can run as the user can already read the app's
data folder and drive the user's agent CLIs; a token in a file it can
read, or in the shim's arguments it can see, would not stop it. What we
avoid is making it worse: the shim holds no secrets, and the headless core
never reaches the Keychain.

## Tests that hold the line

- `crates/openagc-core/src/agents/injection_tests.rs` — a naive agent obeys
  hostile emails; nothing is sent, trashed or archived without the user.
  Mailbox mode too: an outside agent using the six mailbox tools on hostile
  mail sends nothing under *Ask* (proposals expire; the headless core
  refuses), cannot reach the user's own account or other tools, and under
  *Send freely* is logged by name.
- `crates/openagc-core/src/cleanup/unsubscribe/tests.rs` — one-click is
  never offered or posted to internal names, IP literals, other ports or
  names that resolve to private addresses; the POST is pinned.
- `crates/agent-mcp/src/client.rs` — the shim refuses sockets that are not
  ours, links, and folders others can write to.
- `crates/mail-store/src/outbox.rs` — claimants register while sweeps
  hammer the lock folder; none is ever taken for gone.
- `crates/permissions` — the policy table and hard limits.
- `crates/mail-mime/tests/sanitize.rs` — hostile HTML fixtures.
- `crates/openagc-mcp/tests/shim.rs` — the socket refuses unknown sessions.
- `crates/agent-claude/tests`, `crates/agent-codex/tests` — the exact CLI
  flags, against fake CLIs.

## What the MVP does not protect against

Local malware with the user's privileges; a compromised agent CLI binary
(it runs as the user); a cloud routine acting within the Gmail permissions
the user granted to Claude or ChatGPT (outside OpenAGC's approval rules, as
the routine editor says); the user approving something they should not;
a program running as the user driving an agent mailbox through
`openagc-mcp --mailbox` (above), which under *Send freely* can send from
it.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting on
[audiojak/openagc](https://github.com/audiojak/openagc/security) rather than a
public issue.
