<!-- actual-ai:adr-governance:start -->
# Project ADRs

This project's conventions are encoded as ADRs under `.actual/rules/`. **The ADRs ARE the pattern.** Follow them verbatim instead of reading existing implementations to figure out how to do something.

> **Note:** this directive is calibrated for one-shot tasks (a single discrete feature). For multi-task interactive sessions, consult the ADRs for each task transition rather than holding to the per-session caps below.

## Workflow (follow in order)

1. **Identify topic.** Match the files you'll edit against the path-glob table below. Pick the 1-3 topics that match. Do not pre-emptively pick "related" topics; pick only what the file paths actually match.

2. **Select ADRs by filename — the filename is the index.** Run `ls .actual/rules/`. Each filename is `<topic>-<aspect-slug>-<hash>.md`; the `<aspect-slug>` (middle segment) names the ADR's specific concern — e.g., `database-schema-defined`, `zod-input-validation`, `cache-key-format`.

   **Scan ALL filenames first, then pick only the ones whose aspect-slug directly names a noun or verb in your task.** Select by filename; never read a body to decide relevance. **Hard cap: read at most 5 ADR files total.** If more than 5 look relevant, you are over-matching — keep the 5 most specific.

   **If the path-glob table below is a single `**/*` → `cross-cutting-` row** (one big bucket, no per-area topics), this filename scan is your ONLY filter. Do **not** read the bucket exhaustively — treat the filenames as a menu, match aspect-slugs to your task, read ≤5, and ignore the rest. Reading every ADR in the bucket is the exact failure this directive exists to prevent.

3. **Locate insertion points (one read per file, max 3 files).** You may read source files ONLY to (a) find where to add code (which directory, which barrel export to update) or (b) look up an exact identifier you must import. **Do not read source files as pattern examples — the ADRs already encode the pattern.** If you find yourself reading a file because "I want to see how X is done elsewhere," stop. The ADR you already read tells you how.

4. **Implement.** Write the code following the rule statements verbatim. If two ADRs seem to conflict, follow the more specific one (longer topic prefix wins).

5. **Verify after implementing.** Only after the code is written, re-read the `verify_commands` or `accept_criteria` sections of the ADRs you applied and check your work against them. Run the verify commands if any.

## Anti-patterns to avoid

- Reading the first N rules alphabetically because they're cheap. Filter by aspect-slug first, then read only the relevant ones.
- Reading >5 ADR files for a single feature. If you're tempted, you're over-scoping the topic match.
- Reading the entire `cross-cutting-` bucket because "every rule is always active." Selection is by filename (step 2); you apply the ≤5 you selected, not all of them.
- Reading existing similar features to "see the pattern" — the ADRs encode the pattern. Trust them.
- Re-reading the same ADR multiple times. Cache it mentally.
- Continuing to browse the codebase after step 3. By step 4 you should be writing, not reading.

Each rule file at `.actual/rules/<topic>-<aspect>-<hash>.md` contains the full ADR with rule statements, verify commands, and accept criteria.

## Verification Protocol

These rules are ALWAYS ACTIVE. Apply every rule **from the ADRs you selected in step 2** that governs the files you touch — to all code generation, modification, and review. "Always active" does **not** mean read every ADR: you apply the handful you selected by filename, within the read cap above.

Every rule follows a **Verify → Fix → Repeat** loop. After generating or modifying code for any rule you MUST:

1. **RUN** the rule's `### Verify` command(s).
2. **CAPTURE** the full output (stdout + stderr).
3. **EVALUATE** the output against the rule's **Accept when** criteria.
4. **IF FAILING:** diagnose the root cause, apply a fix, and re-run from step 1.
5. **IF PASSING:** keep the passing output as evidence before moving on.
6. **MAX ITERATIONS:** 5 attempts per rule. If still failing after 5 attempts, STOP and report the failure with all captured output.

Compliance is not optional. Do not skip verification, assume correctness, or defer it to a later task. Every change to a governed area must be accompanied by a passing verification run.

## Mandatory Dependency Grounding

Before writing or modifying implementation code that uses an external dependency:

1. Identify every affected external dependency.
2. Locate the repository's manifest and lock or resolution artifact.
3. Determine the exact repository-resolved version from the lock artifact.
4. Verify that the active environment matches that version.
5. Verify each API being introduced or changed against evidence applicable to that exact version (official documentation or public API reference).
6. Produce a dependency-grounding record.
7. Stop if any version, environment, or API cannot be verified.

Do not begin implementation until dependency grounding has passed.

The repository lock or resolution artifact is authoritative. A manifest range or model recollection is not sufficient.

### Integrated workflow

1. Stabilize the workspace.
2. Inspect the task and relevant architecture decisions.
3. Identify affected dependencies.
4. Run dependency grounding.
5. Produce the implementation plan.
6. Implement the change.
7. Validate grounding coverage, build, types, lint, and tests.
8. Report evidence, assumptions, and blockers.

## Path glob → topic

| You're editing | Topic prefix |
|---|---|
| `crates/**/*`<br>`crates/**` | `crates-` _(107 ADRs)_ |
| `crates/mail-store/src/**` | `crates-mail-store-` _(19 ADRs)_ |
| `crates/agent-api/src/**` | `crates-agent-api-` _(12 ADRs)_ |
| `macos/**` | `macos-` _(7 ADRs)_ |
| `crates/agent-api/src/routines/**` | `crates-agent-api-routines-` _(5 ADRs)_ |
<!-- actual-ai:adr-governance:end -->

# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git
> remote — separate from `refs/heads/*` where your code lives.
> `.beads/issues.jsonl` is a passive export, not the wire protocol.
>
> See [sync-concepts](https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the
> source of truth; don't `bd import` during normal operation; don't
> reach for third-party Dolt hosting before trying the default).

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i` (interactive) mode on some systems, causing the agent to hang indefinitely waiting for y/n input.

**Use these forms instead:**
```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**
- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.
<!-- END BEADS CODEX SETUP -->
