# The rules server (`openagc-rules`)

Cloud agents (a Claude cloud routine, an agent on another machine, a
script) cannot reach OpenAGC on your Mac. `openagc-rules` gives them an
agent mailbox's writing guide and the facts you chose to share, published
from the app, check their drafts against the guide, and take their
reports of what they sent, for the app to record. It is one binary and one SQLite file; you run it yourself,
behind your own TLS proxy. Spec §10.6, ADR 0016, plan
`docs/plans/rules-server.md`.

*Status:* the server is built (bearer tokens; `guide_rules`,
`facts_lookup`, `check_draft` and `report_send`; OAuth sign-in with
one-time connect codes for claude.ai connectors and cloud routines), the
app publishes to it (an agent mailbox's Settings, *Rules server* ›
*Publish to a Rules Server…*), connects agents to it (*Cloud agents* ›
*Connect a Cloud Agent…*) and pulls their reports at each sync. Encryption
at rest comes next.

## What it holds, and what it never holds

It holds, per registered agent mailbox:

- the **last five snapshots** the app published: the mailbox's address,
  the name it sends as and what an agent is told about it (the service it
  sends through and its limits, never your own email or any address but
  the mailbox's); its accepted
  rules and guidelines with their scope and checks (never the evidence
  quotes, which come from sent mail); its audience groups; and the facts
  you shared with cloud agents;
- the **SHA-256 hash** of the mailbox's publisher token, and of each agent
  token, with the agent token's id, its name ("Weekly outreach routine"),
  when it was made and revoked, and when it was last used (to the minute);
- for OAuth: the **clients** that registered (an id, the name they gave,
  their return addresses), each **agent grant** a connect code made (beside
  the agent tokens, with the same id, name and times), and the hashes of
  connect codes, authorization codes and access and refresh tokens until
  they expire;
- the **reports** agents filed with `report_send` and the app has not
  pulled yet: what the agent says it sent (the Message-ID, recipients,
  subject, time and body, in its own words), which agent, the version it
  checked against and what the server's check found. Each is deleted once
  the app has pulled it, and after 30 days regardless (an hourly sweep); a
  mailbox keeps at most 10,000, dropping the oldest (counted, and the app
  logs it).

Every address a snapshot names (audience-group members, and the people a
rule or guideline is for) is a **salted hash**: lower-case hex SHA-256 of
the salt, a zero byte and the trimmed, lower-cased address or `@domain`.
The salt travels with the snapshot. When an agent gives recipients, the
server hashes them the same way to find their audiences and the entries
for them; an entry for particular people is shown only for a message to
one of them, naming the recipient it matched. The server refuses a
snapshot with a plain address in it.

It never holds mail (only what agents say they sent, in their reports, until
the app pulls them), a mail service's key, an OAuth token or a token in the
clear. It cannot read or send mail as anyone. It never logs a token,
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
| `--public-url` | `OPENAGC_RULES_PUBLIC_URL` | unset | The server's address as agents reach it, an origin alone (`https://rules.example.com`; `http://` only to `127.0.0.1` or `localhost`). Turns on OAuth sign-in with connect codes, which claude.ai connectors and cloud routines need; the OAuth issuer and the `/mcp` resource are made from it. |
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
  -e OPENAGC_RULES_PUBLIC_URL=https://rules.example.com \
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
proxy: they carry the bearer tokens (and, on the sign-in page, the CSRF
cookie). Caddy's default access log does not record `Authorization` or
`Cookie`; keep it that way. It does record query strings, which on
`/oauth/authorize` hold a client's `state` and PKCE challenge, and on the
client's return address its authorization code (single use, two
minutes, and useless without the client's PKCE verifier). Serve the
server at the root of its host: OAuth's `/.well-known/` addresses must be
reachable there.

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

### Publish from the app

In OpenAGC, Settings › Accounts, an agent mailbox's row has *Rules
server*: *Publish to a Rules Server…* asks for the server's address
(`https://rules.example.com`; plain `http://127.0.0.1:8787` works for a
server on the same Mac) and, if you set one, the registration token. The
sheet lists exactly what goes before anything does. Publishing registers
the mailbox, keeps the publisher token in the Keychain and pushes; after
that every change to the mailbox's writing guide or shared facts is
pushed a few seconds later while OpenAGC is open, and the row says which
version the server has and when it was published. Which facts go is each
fact's *Share with cloud agents* switch, in the mailbox's Facts.
*Stop Publishing…* either leaves the last version on the server or removes
the mailbox from it.

### Connect a claude.ai connector or a cloud routine

Cloud routines reach MCP servers only through the claude.ai connectors on
the account, and those sign in with OAuth (a fixed `Authorization` header
is a beta few organisations have). The server is its own minimal
authorization server: there are no accounts on it and no passwords; the
sign-in page asks for a **connect code** from OpenAGC. The server needs
`OPENAGC_RULES_PUBLIC_URL`; without it the app says so and offers only a
token.

1. In OpenAGC, the agent mailbox's row in Settings › Accounts, *Cloud
   agents* › *Connect a Cloud Agent…*: name the agent ("Weekly outreach
   routine"), choose *A claude.ai connector or cloud routine
   (recommended)* and *Make Code*. The sheet shows the server's MCP URL,
   the code (`ABCDE-FGHJK`; it works once, for 10 minutes, counted down;
   *New Code* makes another) and instructions for the routine.
2. In claude.ai, *Customize › Connectors › Add custom connector*: the URL
   is the one the sheet shows (`https://rules.example.com/mcp`) exactly;
   authentication *Sign in now*; OAuth client *Register automatically*
   (Claude's published identity, a Client ID Metadata Document, is not
   supported yet). Leave the client ID and secret empty.
3. *Connect* opens the server's sign-in page. It names the app ("Claude")
   and where it goes back to (`claude.ai`). Enter the code and choose
   *Connect*. Within a few seconds the sheet in OpenAGC says "Connected:
   Weekly outreach routine, from Claude."; *Done* closes it, and the code
   is gone.
4. Add the connector to the routine's connections and paste the
   instructions from step 1 into its prompt (call `guide_rules` before
   writing, `facts_lookup` for facts, `check_draft` on each draft before
   sending and `report_send` after). The agent is listed under *Cloud
   agents* with the name from step 1, as *Connector · Claude*, with when
   it was last used; *Revoke…* there ends its sessions at its next
   request.

### Reports in the app

At each sync of a publishing agent mailbox (at most once a minute) and on
*Publish Now*, OpenAGC pulls the reports, keeps each in the mailbox's
store, records it as an AI composition written by `cloud:<agent name>`
(once the mailbox has finished a learning run, as for every AI
composition, ADR 0013), and only then acknowledges them, which deletes
them on the server; pulled twice, a report is recorded once. Each is
matched to the mailbox's sent mail by the Message-ID it names (brackets
and spaces ignored), else to a message to one of its recipients with its
subject sent within 10 minutes of the time it gives; one whose mail has
not synced yet is matched when it does, and after a day says "Reported,
not seen in the mailbox". The daily review pairs a matched report with
its sent copy as it pairs a draft with its own. *Cloud agents* counts the
week's reports ("12 reports this week") and *Show Reports…* lists them.
A report's text is the agent's own: shown as text, never followed, and
fenced like every AI text when the review shows it to an agent. A send
with no report is reviewed as any other.

### Connect Claude Code, the Agent SDK or a script

*Connect a Cloud Agent…* with *Claude Code, the Agent SDK or a script* and
*Make Token* shows a token once (OpenAGC does not keep it: close the sheet
and it is gone), with what to do with it:

```sh
claude mcp add --transport http openagc-scout-rules https://rules.example.com/mcp \
  --header "Authorization: Bearer oagc_agt_…"
curl -H "Authorization: Bearer oagc_agt_…" \
  "https://rules.example.com/v1/m/scout@agents.example/guide?message_type=new"
```

and the same instructions for the agent's prompt. Whoever holds the token
can read this mailbox's published guide and shared facts, check drafts and
file reports, and nothing else, until *Revoke…*. Tokens work on a server without a public URL; the URL is
then the address OpenAGC publishes to.

Claude Code should also be able to sign in with a connect code
(`claude mcp add --transport http openagc-scout-rules
https://rules.example.com/mcp`, then `/mcp` to authenticate; not yet tried
by hand): it registers itself, returns to a loopback address on a port of
its own, which the server matches without the port, and the page warns
that it goes back to a program on this computer. A token is simpler there.

Limits: a connect code is ten characters from 31 without look-alikes (no
0, 1, I, L or O; case, spaces and the dash do not matter), single use,
10 minutes, kept as a hash; a mailbox may have 10 unused at once. A
sign-in page closes after 5 wrong codes; an app (OAuth client) that sends
5 wrong codes is locked out for an hour; the whole server checks at most
30 codes a minute. Access tokens last an hour; refresh tokens 30 days and
change at every refresh, and a refresh token or authorization code used
twice revokes the agent (someone else has a copy). Clients that never
connect an agent are forgotten after a week.

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
  mailbox's published guide and shared facts, and can check drafts and
  file reports (which the app shows as that agent's, matched against the
  mailbox's real sent mail), until revoked; it cannot publish, read
  reports, read mail or send.
- **OAuth agents** are made only with a connect code the app minted for
  one mailbox, so a connector reaches only that mailbox, with the same
  reach as an agent token, and is listed and revoked with the tokens. Its
  access tokens are bound to this server's `/mcp` (they work nowhere else,
  not even on the REST `GET`s). The sign-in page shows the app's name as
  the app gave it, which anyone registering can choose, and the host it
  returns to, which the server checks exactly against what the app
  registered; it cannot be framed, runs no script and sets one cookie, for
  its CSRF check. A stolen connect code is worth one agent until revoked,
  and only within its 10 minutes.
- **The operator** (you, or the project for a hosted server) can read what
  was published, now from the file and, once encryption at rest comes,
  still during requests. Only what the publish sheet lists leaves the Mac.

## The API

All bodies are JSON. Errors are `{"error": "<code>", "message": "…"}`; a
401 carries `WWW-Authenticate: Bearer realm="openagc-rules"` (with
`error="invalid_token"` for a token that is unknown or revoked, and at
`/mcp` with OAuth on `resource_metadata="<public URL>/.well-known/oauth-protected-resource", scope="rules"`),
a 429 carries `Retry-After`. Times are RFC 3339 in UTC.

### For the app (publisher token)

| Call | Body | Answer |
|---|---|---|
| `POST /v1/mailboxes` | `{"address"}` | 201 `{"address", "publisher_token"}`, once; 409 `already_registered`. Needs the registration token when one is set |
| `PUT /v1/mailboxes/{address}/snapshot` | The snapshot (`writing_guide::Snapshot`, `schema_version` 1) | 200 `{"version", "published_at", "versions_kept"}` and `ETag: "<version>"` |
| `GET /v1/mailboxes/{address}/snapshot/version` | | The same, `version` null before the first push |
| `POST /v1/mailboxes/{address}/agent-tokens` | `{"name"}` | 201 `{"id", "name", "created_at", "revoked_at", "token"}`; the token is shown only here |
| `GET /v1/mailboxes/{address}/agent-tokens` | | `{"agent_tokens": [{"id", "name", "kind", "created_at", "revoked_at", "last_used_at"}]}`: every agent, `kind` `token` or `oauth` (a grant made with a connect code, which adds `client_name`); `last_used_at` is when it was last let in, to the minute, or null |
| `DELETE /v1/mailboxes/{address}/agent-tokens/{id}` | | 204; revokes a token or a grant (and its OAuth tokens) |
| `POST /v1/mailboxes/{address}/connect-codes` | `{"name"}` | 201 `{"id", "name", "code", "expires_at"}`; the code is shown only here. 409 `oauth_off` without a public URL, 429 `too_many_codes` with 10 unused |
| `GET /v1/mailboxes/{address}/reports?after=<id>&limit=<n>` | | `{"reports": [{"id", "agent_id", "agent_name", "agent_kind", "received_at", "message_id", "to", "subject", "sent_at", "body_markdown", "checked_version", "check": {"version", "guide_check"}}], "pending", "dropped", "more"}`, oldest first, after the cursor (0 for all), at most `limit` (100 by default, 500 at most) |
| `POST /v1/mailboxes/{address}/reports/ack` | `{"up_to_id"}` | `{"deleted"}`; the reports up to that id are gone |
| `DELETE /v1/mailboxes/{address}` | | 204; the mailbox, its snapshots, its tokens and its reports are gone |

Publishing: the first push sends no `If-Match` (or `If-Match: 0`); every
later one sends `If-Match` with the current version (`5` or `"5"`) and a
higher `version` in the body. Answers: 428 `if_match_required` and 412
`version_mismatch` carry `current_version` (read it and push again), 409
`version_not_newer`, 422 `invalid_snapshot` (unknown `schema_version`, a
plain address, not JSON) or `mailbox_mismatch` (the snapshot's
`mailbox.address` is not the path's). The newest five versions are kept.

### For agents (agent token)

- **MCP** at `/mcp`, Streamable HTTP, stateless:
  `claude mcp add --transport http openagc-scout-rules https://rules.example.com/mcp --header "Authorization: Bearer oagc_agt_…"`.
- **REST**, for scripts:
  `GET /v1/m/{address}/guide?to=ann@acme.com&to=…&message_type=reply`
  (`to` may repeat or be comma-separated),
  `GET /v1/m/{address}/facts?category=Work&query=calendar`, and
  `POST /v1/m/{address}/check` and `POST /v1/m/{address}/reports` with
  the tools' arguments as JSON (200 and 202; 422 `invalid_arguments`, 413
  `too_large`).

| Tool | Input | Answer |
|---|---|---|
| `guide_rules` | `to` (array of addresses), `message_type` (`new`, `reply`, `forward`) | `{"mailbox", "sends_as", "about", "writing_guide", "guide_version", "version", "published_at"}` |
| `facts_lookup` | `category`, `query` | `{"facts": [{"category", "label", "value", "ask_before_using"}], "version", "published_at"}` |
| `check_draft` | `to`, `message_type` (read from a `Re:` or `Fwd:` subject when left out), `subject`, `body_markdown` (required, at most 256 KB) | `{"guide_check": ["Uses “circle back”, which your rules ban"], "guide_version", "version", "published_at"}`: what the draft breaks, empty when nothing |
| `report_send` | `message_id` (as the service answered it), `to`, `subject`, `body_markdown` (required, at most 256 KB), `sent_at` (RFC 3339), `checked_version` | `{"queued": true, "report_id", "guide_check", "version", "message"}`; the server checks the body again and keeps the report for the app |

The names, arguments and answers are mailbox mode's (`docs/mcp.md`), so an
agent's instructions work with either; there is no `send_mode`, since a
cloud agent sends through the mailbox's service. `version` and
`published_at` say which snapshot answered and when the app published it:
with the app closed nothing changes, and the guide is "as of" that time.
`check_draft` is mailbox mode's guide check, word for word: the body's
text (Markdown read as mailbox mode renders it) against the banned and
required phrases and length limits of the entries that apply to those
recipients and that type; no model is asked. Before anything is published
the reading tools and `check_draft` answer `not_published`; `report_send`
still queues (with an empty check). Every call takes one request from the
agent's rate limit.

`GET /healthz` answers `ok` when the database answers.

### OAuth (with `OPENAGC_RULES_PUBLIC_URL`)

The MCP authorization spec's shape (2025-06-18 and 2025-11-25): the
server is the resource server for `<public URL>/mcp` and its own
authorization server, issuer `<public URL>`.

| Call | |
|---|---|
| `GET /.well-known/oauth-protected-resource` (also `…/mcp`) | RFC 9728: `resource`, `authorization_servers`, `scopes_supported: ["rules"]` |
| `GET /.well-known/oauth-authorization-server` | RFC 8414: the endpoints below, `code_challenge_methods_supported: ["S256"]`, `token_endpoint_auth_methods_supported: ["none"]`, `grant_types_supported: ["authorization_code", "refresh_token"]` |
| `POST /oauth/register` | RFC 7591, JSON: `redirect_uris` (1 to 10; `https://`, or `http://` to a loopback address), `client_name`. Every client is public (`token_endpoint_auth_method` `none`). 201 with `client_id`; 400 `invalid_redirect_uri` or `invalid_client_metadata`; 30 a minute across the server |
| `GET /oauth/authorize` | `response_type=code`, `client_id`, `redirect_uri` (exactly as registered; a loopback one with any port), `code_challenge` with `code_challenge_method=S256` (required), `state`, `resource` (this server's `/mcp`, if given). An unknown client or return address is shown on the page, never redirected to; other errors go back with `error` and `state`. Otherwise the sign-in page |
| `POST /oauth/authorize` | The sign-in page's form (its CSRF cookie, its Origin). 303 to the return address with `code`, `state` and `iss`, or `error=access_denied` |
| `POST /oauth/token` | Form-encoded. `grant_type=authorization_code` with `code`, `redirect_uri`, `client_id`, `code_verifier` (and `resource`); `grant_type=refresh_token` with `refresh_token`, `client_id`. 200 `{"access_token", "token_type": "Bearer", "expires_in": 3600, "refresh_token", "scope": "rules"}`; errors are RFC 6749's (`invalid_grant`, `invalid_client`, `invalid_request`, `unsupported_grant_type`, `invalid_target`) |
