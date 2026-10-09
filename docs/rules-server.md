# The rules server (`openagc-rules`)

Cloud agents (a Claude cloud routine, an agent on another machine, a
script) cannot reach OpenAGC on your Mac. `openagc-rules` gives them an
agent mailbox's writing guide and the facts you chose to share, published
from the app. It is one binary and one SQLite file; you run it yourself,
behind your own TLS proxy. Spec §10.6, ADR 0016, plan
`docs/plans/rules-server.md`.

*Status:* the server is built (bearer tokens; `guide_rules` and
`facts_lookup`). The app's *Publish to a Rules Server…* (oagc-gmn7.3),
OAuth for claude.ai connectors (oagc-gmn7.4), *Connect a Cloud Agent…*
(oagc-gmn7.5), `check_draft` and `report_send` (oagc-gmn7.6) and encryption
at rest come next.

## What it holds, and what it never holds

It holds, per registered agent mailbox:

- the **last five snapshots** the app published: the mailbox's address,
  the name it sends as and what an agent is told about it; its accepted
  rules and guidelines with their scope and checks (never the evidence
  quotes, which come from sent mail); its audience groups; and the facts
  you shared with cloud agents;
- the **SHA-256 hash** of the mailbox's publisher token, and of each agent
  token, with the agent token's id, its name ("Weekly outreach routine")
  and when it was made and revoked.

Every address a snapshot names (audience-group members, and the people a
rule or guideline is for) is a **salted hash**: lower-case hex SHA-256 of
the salt, a zero byte and the trimmed, lower-cased address or `@domain`.
The salt travels with the snapshot. When an agent gives recipients, the
server hashes them the same way to find their audiences and the entries
for them; an entry for particular people is shown only for a message to
one of them, naming the recipient it matched. The server refuses a
snapshot with a plain address in it.

It never holds mail, a mail service's key, an OAuth token or a token in
the clear. It cannot read or send mail as anyone. It never logs a token,
an address it was asked about or what a snapshot says: each request is
logged as its method, route pattern (`/v1/m/{address}/guide`), status,
time and the token's id (`agent:3f9c…`, `publisher:1`).

**Not yet:** encryption at rest (spec §10.6). Until it comes, whoever can
read the SQLite file or a backup of it can read what was published (rules,
guidelines, shared facts), though not the addresses.

## Run it

```sh
cargo build --release -p rules-server        # target/release/openagc-rules
openagc-rules --data-dir /var/lib/openagc-rules
```

| Flag | Environment | Default | |
|---|---|---|---|
| `--listen` | `OPENAGC_RULES_LISTEN` | `127.0.0.1:8787` | Plain HTTP. Keep it on loopback behind a proxy. |
| `--data-dir` | `OPENAGC_RULES_DATA_DIR` | `./data` | Holds `rules.sqlite3`. Made mode 0700, the file 0600. |
| `--log` | `OPENAGC_RULES_LOG` | `info` | The server's own log level (`debug` adds health checks). Dependencies log at `warn`. |
| `--rate-limit` | `OPENAGC_RULES_RATE_LIMIT` | `120` | Requests per minute per token, in bursts of a minute's worth; `0` turns it off. Registration has one bucket of its own. |
| | `OPENAGC_RULES_REGISTRATION_TOKEN` | unset | When set, registering a mailbox needs `Authorization: Bearer <it>`. Set it on any server strangers can reach. |

Other commands:

- `openagc-rules forget-mailbox <address> [--data-dir …]` deletes a mailbox,
  its snapshots and its agent tokens: for when the app has lost its
  publisher token and must register again.
- `openagc-rules backup <file> [--data-dir …]` writes a consistent copy
  of the database while the server runs (below).
- `openagc-rules healthcheck [--listen …]` exits 0 if `GET /healthz`
  answers 200 (the image has no curl).

Logs go to standard error. It stops cleanly on Ctrl-C or SIGTERM.

### Docker

```sh
docker build -f crates/rules-server/Dockerfile -t openagc-rules .
docker run -d --name openagc-rules --restart unless-stopped \
  -p 127.0.0.1:8787:8787 -v openagc-rules:/data \
  -e OPENAGC_RULES_REGISTRATION_TOKEN="$(openssl rand -hex 32)" \
  openagc-rules
```

The image runs as a non-root user (65532) on distroless, listens on
`0.0.0.0:8787` inside the container, keeps its file in the `/data` volume
and has a health check. Releases will publish it on GitHub's registry
(ADR 0016).

### TLS with Caddy

The server speaks plain HTTP and expects a proxy to terminate TLS. A
Caddyfile for `rules.example.com`:

```caddyfile
rules.example.com {
	reverse_proxy 127.0.0.1:8787
}
```

Caddy gets and renews the certificate. Do not log request headers at the
proxy: they carry the bearer tokens. Caddy's default access log does not
record `Authorization`; keep it that way.

### Backups

Copy the database with SQLite's online backup, never by copying the file
while the server writes:

```sh
sqlite3 /var/lib/openagc-rules/rules.sqlite3 ".backup '/backups/rules-$(date +%F).sqlite3'"
```

Where there is no `sqlite3` (the image has none), the server makes the
same consistent copy itself (`VACUUM INTO`), to a file that must not exist
yet:

```sh
openagc-rules backup /backups/rules-$(date +%F).sqlite3 --data-dir /var/lib/openagc-rules
# Docker:
docker exec openagc-rules /openagc-rules backup /tmp/backup.sqlite3
docker cp openagc-rules:/tmp/backup.sqlite3 ./rules-$(date +%F).sqlite3
```

A backup holds what the live file holds (above); keep it as private. To
restore, stop the server and put the copy in place as `rules.sqlite3`. Losing
the database loses nothing that matters: the app publishes again, though
agents need new tokens.

## The trust model

- **The app is the only writer.** It registers each agent mailbox it
  publishes and keeps the mailbox's **publisher token** in the Keychain
  (`rules.publish_token.<server>.<account>`, spec §12). Only that token can
  push a snapshot, mint or revoke agent tokens, or forget the mailbox.
- **Registration: the first one wins.** `POST /v1/mailboxes` answers a
  publisher token once; registering the same address again is refused
  (409). Nobody can take over a mailbox the app has registered. On a
  server strangers can reach, someone could register your address first
  and keep the app from publishing there (they could publish nothing in
  your name to your agents, whose tokens come from the app); set
  `OPENAGC_RULES_REGISTRATION_TOKEN` to close registration to everyone who
  does not have it, and `forget-mailbox` clears a squatted address.
- **Agent tokens** are minted by the app through the publisher's API,
  named, scoped to one mailbox, shown once and revocable; revoking takes
  effect at the agent's next request. A leaked agent token reads that
  mailbox's published guide and shared facts until revoked; it cannot
  publish, read mail or send.
- **The operator** (you, or the project for a hosted server) can read what
  was published, now from the file and, once encryption at rest comes,
  still during requests. Only what the publish sheet lists leaves the Mac.

## The API

All bodies are JSON. Errors are `{"error": "<code>", "message": "…"}`; a
401 carries `WWW-Authenticate: Bearer realm="openagc-rules"` (with
`error="invalid_token"` for a token that is unknown or revoked), a 429
carries `Retry-After`. Times are RFC 3339 in UTC.

### For the app (publisher token)

| Call | Body | Answer |
|---|---|---|
| `POST /v1/mailboxes` | `{"address"}` | 201 `{"address", "publisher_token"}`, once; 409 `already_registered`. Needs the registration token when one is set |
| `PUT /v1/mailboxes/{address}/snapshot` | The snapshot (`writing_guide::Snapshot`, `schema_version` 1) | 200 `{"version", "published_at", "versions_kept"}` and `ETag: "<version>"` |
| `GET /v1/mailboxes/{address}/snapshot/version` | | The same, `version` null before the first push |
| `POST /v1/mailboxes/{address}/agent-tokens` | `{"name"}` | 201 `{"id", "name", "created_at", "revoked_at", "token"}`; the token is shown only here |
| `GET /v1/mailboxes/{address}/agent-tokens` | | `{"agent_tokens": [{"id", "name", "created_at", "revoked_at"}]}` |
| `DELETE /v1/mailboxes/{address}/agent-tokens/{id}` | | 204 |
| `DELETE /v1/mailboxes/{address}` | | 204; the mailbox, its snapshots and its tokens are gone |

Publishing: the first push sends no `If-Match` (or `If-Match: 0`); every
later one sends `If-Match` with the current version (`5` or `"5"`) and a
higher `version` in the body. Answers: 428 `if_match_required` and 412
`version_mismatch` carry `current_version` (read it and push again), 409
`version_not_newer`, 422 `invalid_snapshot` (unknown `schema_version`, a
plain address, not JSON) or `mailbox_mismatch` (the snapshot's
`mailbox.address` is not the path's). The newest five versions are kept.

### For agents (agent token)

- **MCP** at `/mcp`, Streamable HTTP, stateless:
  `claude mcp add --transport http openagc-scout https://rules.example.com/mcp --header "Authorization: Bearer oagc_agt_…"`.
- **REST**, for scripts:
  `GET /v1/m/{address}/guide?to=ann@acme.com&to=…&message_type=reply`
  (`to` may repeat or be comma-separated) and
  `GET /v1/m/{address}/facts?category=Work&query=calendar`.

| Tool | Input | Answer |
|---|---|---|
| `guide_rules` | `to` (array of addresses), `message_type` (`new`, `reply`, `forward`) | `{"mailbox", "sends_as", "about", "writing_guide", "guide_version", "version", "published_at"}` |
| `facts_lookup` | `category`, `query` | `{"facts": [{"category", "label", "value", "ask_before_using"}], "version", "published_at"}` |

The names, arguments and answers are mailbox mode's (`docs/mcp.md`), so an
agent's instructions work with either; there is no `send_mode`, since a
cloud agent sends through the mailbox's service. `version` and
`published_at` say which snapshot answered and when the app published it:
with the app closed nothing changes, and the guide is "as of" that time.
Before anything is published both answer `not_published`.

`GET /healthz` answers `ok` when the database answers.
