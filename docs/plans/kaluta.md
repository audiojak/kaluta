# OpenAGC becomes Kaluta — 2026-10-09

The app, its binaries, crates, identifiers, docs and copy take the name
**Kaluta** (domain kaluta.org). A kaluta is a small Australian marsupial;
the logo is one, in black ink.

Branch `kaluta`, from `main` at 97821fe (PRs #14, #15 and #16 merged),
run during the day rather than overnight (maintainer, 2026-10-09). A
rename touches nearly every file, so nothing else should land on `main`
while it runs. Commit and push after every closed issue; never push to
`main`; PR to `main` at the end.

## Decisions (maintainer, 2026-10-08)

- **Rename everything, and migrate.** New bundle ID `org.kaluta.Kaluta`;
  data folder `~/Library/Application Support/Kaluta`; Keychain service,
  log subsystem and defaults domain `org.kaluta.Kaluta`. On first launch
  the app takes over what the OpenAGC build left, and never changes or
  deletes the originals (see *Migration*). The app is unreleased, so the
  maintainer is the only person who migrates.
- **The website is out of scope.** It lives in another repo and the
  maintainer builds it separately. This run links to `https://kaluta.org`
  and adds no site, pages or hosting.
- **App icon: option A, ink on white.** The logo in black on a white
  tile with a faint warm-grey gradient (`#FFFFFF` → `#E9E6E1`), the mark
  about 600 of the 1024 canvas wide, nudged to sit optically centred.
  `scripts/make-icon.sh` renders it, so a designer's final icon is a
  one-file swap.
- **The GitHub repo is renamed later**, by the maintainer, to
  `audiojak/kaluta`. The run writes the new URLs everywhere (Cargo
  `repository`, README, docs, `SUFeedURL`). They 404 until the rename,
  so **rename the repo before merging the PR**; GitHub then redirects
  the old URLs.
- **Kept on purpose:** the beads prefix `oagc-` (issue ids stay valid),
  the local checkout path `~/Code/OpenAGC`, historical plan docs in
  `docs/plans/` and accepted ADRs (they record what happened; ADR 0017
  records the rename), and the original spec file at the repo root.

### Names

| What | Before | After |
|---|---|---|
| App, display name | OpenAGC | Kaluta |
| Tagline | Open Agent Gmail Client | *An open-source Mac mail client for your AI agents* (proposed; the maintainer may change it) |
| Bundle ID / Keychain service / log subsystem | `ai.actual.openagc` | `org.kaluta.Kaluta` |
| Test and scratch Keychain services | `ai.actual.openagc.tests`, `.scratch` | `org.kaluta.Kaluta.tests`, `.scratch` |
| Core framework / bundle ID | `OpenAGCCore`, `ai.actual.openagc.core` | `KalutaCore`, `org.kaluta.Kaluta.core` |
| Data folder | `…/Application Support/OpenAGC` | `…/Application Support/Kaluta` |
| Crates | `openagc-core`, `openagc-mcp` | `kaluta-core`, `kaluta-mcp` |
| UniFFI module | `openagc_core` | `kaluta_core` |
| Rules server package / binary / realm | `openagc-rules` | `kaluta-rules` |
| MCP server name (in-app agents) | `openagc` (tools `mcp__openagc__*`) | `kaluta` (`mcp__kaluta__*`) |
| Outside-agent config entries | `openagc-<agent>` | `kaluta-<agent>` |
| Env vars | `OPENAGC_LOG`, `OPENAGC_RULES_*` (incl. `TRUSTED_PROXY`, `REQUIRE_ENCRYPTION`), `OPENAGC_CLAIMS_*`, `OPENAGC_ENTITLEMENTS` | `KALUTA_…` |
| Rules server copy | consent page ("Connect code from OpenAGC"), `resource_name`, MCP instructions, Basic realm, usage and error text, Dockerfile image and volume | Kaluta / `kaluta-rules` |
| Short socket fallback | `/tmp/openagc-<uid>` | `/tmp/kaluta-<uid>` |
| Outgoing Message-ID | `<token>.openagc@<domain>` | `<token>.kaluta@<domain>` |
| AgentMail sign-up `source` | `openagc` | `kaluta` |
| Launch flags | `-OpenAGCSnapshot…`, `-OpenAGCFakeAgents`, `-OpenAGCDataDirectory`, `-OpenAGCDemo` | `-Kaluta…` |
| HTTP User-Agent | `OpenAGC/<version>` | `Kaluta/<version>` |
| Sanitizer schemes, quote class | `openagc-cid:`, `openagc-remote:`, `openagc-quote` | `kaluta-cid:`, `kaluta-remote:`, `kaluta-quote` |
| AgentMail outbox header | `X-OpenAGC-Outbox-Id` | `X-Kaluta-Outbox-Id` |
| Export formats | `openagc-writing-guide`, `openagc_facts` | `kaluta-writing-guide`, `kaluta_facts` |
| Config backups | `<file>.openagc-backup-<time>` | `<file>.kaluta-backup-<time>` |
| Xcode | `macos/OpenAGC`, `OpenAGCTests`, `OpenAGC.xcodeproj`, `OpenAGCApp` | `macos/Kaluta`, `KalutaTests`, `Kaluta.xcodeproj`, `KalutaApp` |
| Dev signing identity | "OpenAGC Dev" | "Kaluta Dev" (the script still finds an existing "OpenAGC Dev") |
| DMG | `OpenAGC-<v>.dmg` | `Kaluta-<v>.dmg` |
| Copyright | Actual AI and the OpenAGC contributors | Actual AI and the Kaluta contributors |

### Old names that stay readable

Stored data and files outside the repo still carry the old names. Each
of these is read (never written) under the old name, with a test that
holds it:

1. **Sanitized HTML in `mail.sqlite`** holds `openagc-cid:` and
   `openagc-remote:` URLs and the `openagc-quote` class. The message view
   registers both schemes and its stylesheet styles both classes.
2. **AgentMail outbox:** reconciliation after a timeout looks for either
   header among sent messages, so a send queued before the rename is
   never sent twice.
3. **Imports** accept `openagc-writing-guide` and `openagc_facts` files.
4. **Outside-agent configs** (`~/.claude.json`, `~/.codex/config.toml`):
   an `openagc-<agent>` entry, or one whose command is an `openagc-mcp`
   path, is recognised as ours. Settings shows it as *Needs updating*,
   and *Update* rewrites it as `kaluta-<agent>` through the existing
   preview, approve and backup flow. Nothing is rewritten unasked.
5. **Rules server env vars:** `OPENAGC_RULES_*` is read when the
   `KALUTA_RULES_*` variable is unset, with a warning in the log naming
   the new one.
6. **Backups:** cleanup and listing find `.openagc-backup-` files too.
7. **Rules encryption labels are not renamed.** `rules-crypto`'s
   domain-separation labels (`openagc-rules/v1/<label>` and the HKDF
   salt `openagc-rules/v1/credential`) are protocol constants: changing
   them would make every sealed snapshot, wrapped key and report on a
   server unreadable. They stay byte for byte, with a comment saying
   why, and a test pins them.

## Migration (first launch of the Kaluta build)

Runs once, before the core opens, only when the data directory is the
real default one: never under tests, `-KalutaDataDirectory` scratch runs
or snapshots. Every step is injected (paths, Keychain services, defaults
domains) so tests run it on scratch folders, the test Keychain services
and throwaway suites. Nothing the OpenAGC build left is changed or
deleted.

1. **Data folder.** If `Kaluta/` is missing and `OpenAGC/` exists, and no
   OpenAGC process or `openagc-mcp` holds the store (the existing
   cross-process lock and the `run/` socket), clone `OpenAGC/` into a
   temporary sibling with APFS `clonefile` (instant, no extra space),
   then rename that to `Kaluta/`. If OpenAGC is running, say so and ask
   to quit it. If both folders exist, use `Kaluta/` and touch nothing.
2. **Keychain.** Copy every generic password under `ai.actual.openagc` to
   `org.kaluta.Kaluta` (same account names, same accessibility). The new
   bundle ID is a new code identity, so macOS asks once per item
   whether Kaluta may read the OpenAGC item: the maintainer chooses
   *Always Allow*. An item that cannot be read is left alone, and the
   account says it needs signing in again, in words.
3. **Defaults.** Copy the `ai.actual.openagc` persistent domain into the
   app's defaults, skipping window and split-view frames (they belong to
   the old window sizes). Keys whose names contain the old name are
   renamed.
4. **Marker.** `Kaluta/migrated-from-openagc.json` records when, what was
   copied and what was skipped. Its presence means the migration never
   runs again.

What the maintainer redoes by hand: notification permission (macOS asks
again), *Open at Login* if it was on, and outside-agent entries
(*Update* in Settings).

## Issues, in order

Each closed issue is committed and pushed before the next starts.

1. **K1 Brand assets.** `brand/` holds the approved files from the Drive
   folder (`Kaluta-logo.svg`, the editable PDF, the 4096 px transparent
   and white PNGs, the approved original). `macos/Icon/kaluta-icon.svg`
   is option A built from `Kaluta-logo.svg`; `make-icon.sh` renders the
   AppIcon set from it. The asset catalog gains `KalutaMark` (vector,
   template rendering) for the places the app shows its own mark: the
   onboarding welcome and the About panel. `openagc-icon.svg` goes.
2. **K2 Rust rename.** Crates, packages, binaries, the UniFFI module,
   `xtask` check-deps and MCP-docs text, `deny.toml`, `Cargo.toml`
   `repository`, `build-core.sh` outputs, MCP server names (`kaluta`,
   `kaluta-<agent>`, `kaluta-rules`), env vars, User-Agents, Message-ID
   domain, temp-dir prefixes in tests, every user-facing string in the
   core (errors, routine notes such as "Kaluta was not running."), and
   the rules server's realm, Dockerfile and binary. Gate passes.
3. **K3 Compat readers** (*Old names that stay readable*, 1–6), each with
   its test.
4. **K4 macOS rename.** Folders, `project.yml` and the regenerated
   project, bundle IDs, display name, copyright, entitlements file
   names, `OpenAGCApp` → `KalutaApp`, launch flags, log subsystems,
   Keychain services. Test isolation keeps its guards under the new
   names and **also** guards the old ones: `KeychainSecretStore` refuses
   to empty either real service, and `test-macos.sh` fails when either
   real prefs plist (`ai.actual.openagc.plist`, `org.kaluta.Kaluta.plist`)
   changes. `clean-test-scratch.sh` sweeps both old and new prefixes.
   `dev-signing.sh` writes `KALUTA_ENTITLEMENTS` and the new path into
   `Local.xcconfig` and regenerates it during the run. All scripts
   (`snapshot.sh`, `test-macos.sh`, `release.sh`, `make-appcast.sh`,
   `help-lint.py`, `design-lint.sh`, `bootstrap.sh`, `check.sh`) and both
   workflows. `test-macos.sh` passes.
5. **K5 Migration** (*Migration*, 1–4) with tests on scratch folders,
   the test Keychain services and throwaway defaults suites, including:
   both folders present, OpenAGC still running, an unreadable Keychain
   item, a second launch (marker present), and a scratch run (never
   migrates). Never run against the real folder, Keychain or defaults.
6. **K6 Words.** Every user-visible string (menus, onboarding, alerts,
   notifications, Settings, help, the agent system prompt), README with
   the new tagline and a link to kaluta.org, `CONTRIBUTING.md`,
   `AGENTS.md`, `CLAUDE.md` (new paths, flags, services; the rule never
   to delete under either `Application Support/OpenAGC` or `Kaluta`),
   spec, architecture, design system, design inventory, keyboard, MCP,
   security, performance, releasing, rules-server and Google OAuth
   client docs. ADR 0017 *The project is named Kaluta* records the
   decisions, the compat list and the migration. `help-lint.py` and
   `design-lint.sh` pass.
7. **K7 Snapshots.** Re-take the light and dark snapshots in
   `docs/design/` that show the name or the icon (onboarding, About,
   Settings panes naming the app), with the demo account and
   `-KalutaFakeAgents YES`. Surfaces the self-snapshot leaves blank
   (macOS 26: Form/List content, onboarding scroll view) are checked
   with `-KalutaSnapshotDumpViews YES` instead.
8. **K8 Brand lint.** `cargo xtask check-brand`, run by `check.sh`: fails
   on `openagc` (any case) outside an allowlist of the compat sites (including the `rules-crypto` labels), the
   historical plans and ADRs, the root spec file and this plan. Then
   `scripts/gate.sh` and `scripts/test-macos.sh` pass, and `bd remember`
   updates the memories that name old paths, flags or services
   (`test-host-isolation`, the snapshot-limits note).

## For the maintainer, in the morning

- Rename the GitHub repo to `audiojak/kaluta` **before** merging, then
  `git remote set-url origin https://github.com/audiojak/kaluta.git`.
- First launch of the Kaluta build: choose *Always Allow* for each
  Keychain prompt; check mail, agent mailboxes and the writing guide are
  there. `Application Support/OpenAGC` stays as it was; delete it, and
  `defaults delete ai.actual.openagc`, only once you are satisfied.
- In Settings, *Update* the outside-agent entries; turn *Open at Login*
  back on if you used it; allow notifications.
- Google Cloud console: rename the OAuth consent screen to Kaluta, with
  kaluta.org as the homepage and its privacy page once the website has
  one (that also helps oagc-7la).
- Apple Developer: register the App ID `org.kaluta.Kaluta` for team
  Y5W2BTVS33. The headless MCP provisioning profile (oagc-zq3) and the
  Developer ID release (oagc-qtt) both use it.
- Any deployed rules server: rename `OPENAGC_RULES_*` to
  `KALUTA_RULES_*` (the old names still work, with a warning).

## Guardrails

- Never launch the app against the real account or the real data
  folder, Keychain or defaults; the migration runs only in tests on
  scratch copies. Snapshots use the demo account and fake agents.
- Never delete anything under `~/Library/Application Support/OpenAGC`
  or `~/Library/Application Support/Kaluta`.
- Never touch real Gmail, Google or Apple accounts, the real
  `~/.claude.json` or `~/.codex/config.toml`, or create cloud routines.
- The crate rename forces a full rebuild: about 67 GiB is free and
  `target/` is 24 GiB, enough without cleaning.
- Gate with `if scripts/gate.sh >log 2>&1; then …; fi`, never piped.
