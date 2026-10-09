# Follow-ups — 2026-10-08

Branch `overnight-8`, created from `overnight-7` (PR #14, open). Its PR
targets `overnight-7` until #14 is merged, then `main`. Commit and push
after every closed issue; never push to `main`.

The open tickets that need no one but the code, in order:

1. **Bugs.** oagc-nta8 and oagc-v41 (flaky agent tests: find the race,
   no retries or sleeps as fixes), oagc-9hf (Facts rows clipped to one
   line), oagc-shc9 (sidebar counts at 24.6 ms against a 3 ms budget).
2. **Cross-process outbox lock** (oagc-uys.3). The app and the headless
   MCP may both send; no message goes out twice. Tests with two writers.
3. **Headless MCP, mailbox mode** (oagc-uys.10), plan
   [headless-mcp.md](headless-mcp.md), *without* the embedded helper:
   - `openagc-mcp --mailbox <address>`: guide, facts and mail read tools;
     `mail_send` / `mail_reply` recorded as AI-written (ADR 0013), checked
     against the guide, honouring the mailbox's send mode.
   - App running: through the core, as the app's own sessions do.
   - App closed: reads open the account's stores read-only; a send takes
     the outbox lock and **queues**, and the tool says it goes out when
     OpenAGC next opens. Sending with the app closed needs the helper,
     which waits on oagc-uys.7 (an App ID and provisioning profile from
     the maintainer). This interim is recorded in the plan and spec.
   - *Connect an Agent…* writes the Claude Code / Codex MCP entry scoped to
     the mailbox, or shows the line to paste. Spec §10 and §12.
4. **Rules server plan** (oagc-ikr): a draft plan with options and open
   questions for the maintainer; no code.
5. **Security pass** over what PR #14 added (unsubscribe, AgentMail, the
   WebSocket, service-account secrets) against §15, notes on oagc-0c8; it
   stays open for the signed build (oagc-qtt).

Waiting on the maintainer: oagc-qtt (Apple Developer enrolment), oagc-7la
(Google verification submission), oagc-uys.7 (App ID and provisioning
profile), oagc-0c8's close (signed build).

Checks, guardrails and handoff as in
[overnight-2026-10-08.md](overnight-2026-10-08.md).
