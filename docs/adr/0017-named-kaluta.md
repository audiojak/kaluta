# ADR 0017: The project is named Kaluta

- Status: Accepted
- Date: 2026-10-09
- Amends: every name in the spec and the ADR 0001 register (bundle ID,
  binaries, crates, data folder, Keychain service); ADR 0009 (the
  isolation guards now cover both names)
- Spec: §1, §4.1, §10.1, §12, §16; plan `docs/plans/kaluta.md`

## Context

The project was called OpenAGC, for *Open Agent Gmail Client*. The name
said Gmail when the app also serves agent mailboxes on other services and
imported mailboxes, it was hard to say, and it was not a name anyone could
own. The maintainer bought kaluta.org and approved a logo: a kaluta, a
small Australian marsupial, in black ink.

The app had no release yet. Its only user with real data was the
maintainer, whose OpenAGC build had a data folder, Keychain items and
preferences under the old names.

## Decision

- **Everything takes the new name**, not only what the user sees: the app
  (Kaluta, bundle ID `org.kaluta.Kaluta`), its data folder
  (`~/Library/Application Support/Kaluta`), Keychain service, log
  subsystem and preferences domain (`org.kaluta.Kaluta`), the crates and
  binaries (`kaluta-core`, `kaluta-mcp`, `kaluta-rules`), the MCP servers
  (`kaluta`, `kaluta-<agent>`), environment variables (`KALUTA_…`), launch
  flags (`-Kaluta…`), HTTP User-Agents, the sanitizer's image schemes and
  quote class, the AgentMail outbox header and the export formats. The
  website is kaluta.org, built in its own repository.
- **The first launch takes over what OpenAGC left, and changes none of
  it.** It clones the data folder (APFS, so instant), copies Keychain items
  Kaluta lacks (macOS asks once per item, since a new bundle ID is a new
  code identity) and the app's own settings but not its window state,
  then writes `migrated-from-openagc.json` so it never runs again. It asks
  to quit OpenAGC first. It never runs under tests or on a scratch data
  directory.
- **What was written under the old name still reads**, each with a test:
  stored bodies' `openagc-cid:`, `openagc-remote:` and `openagc-quote`
  are read as the new names; a send queued under `X-OpenAGC-Outbox-Id` is
  found and never sent twice; `openagc-writing-guide` and `openagc_facts`
  files import; *Connect an Agent…* replaces the `openagc-<agent>` entry
  it wrote; a rules server's `OPENAGC_RULES_*` settings work, with a
  warning.
- **The rules server's encryption labels keep their bytes**
  (`openagc-rules/v1/…`). They are bound into every sealed snapshot,
  report and wrapped key; changing them would make all of them
  unreadable. A known-answer test pins them.
- **Kept on purpose:** the rules server's token prefixes (`oagc_pub_`,
  `oagc_agt_`, `oagc_oat_`, `oagc_ort_`, `oagc_cli_`, `oagc_consent_`),
  which tokens already issued carry; the beads prefix `oagc-`; accepted ADRs and the
  dated plans (they record what happened under the old name), and the
  original product spec at the repository root.
- `cargo xtask check-brand` fails on the old name anywhere else.

## Consequences

- Nothing the maintainer had is lost or changed, and nothing needs signing
  in again unless a Keychain prompt is refused. Notification permission,
  *Open at Login* and outside agents' entries are set again by hand.
- The App ID for provisioning profiles and Developer ID releases is
  `org.kaluta.Kaluta`.
- Old names stay in a short, linted list of places, each there to read
  something the old name wrote.
