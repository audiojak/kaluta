# Performance

Spec §1.3 sets the targets; §13 lists the rules that meet them. This page
records how they are measured and the latest numbers.

## How to measure

```bash
cargo xtask fixture   # once: synthetic mailbox, ~130k messages, cached in build/fixtures
cargo run --release -p xtask -- perf   # store operations, fails if a budget is missed
                                       # (`cargo xtask perf` is the same in a debug build)
scripts/test-macos.sh test -only-testing:OpenAGCTests/PerformanceTests   # through the FFI
```

The Swift tests skip themselves when the fixture is absent.

## Latest results

2026-09-24, MacBook (Apple Silicon), fixture of 131,731 messages in 57,143
threads (39,503 archived). p95 over 40–60 runs after a warm-up.

**Store (Rust, release):**

| Operation | p95 | Budget |
|---|---|---|
| Open store + first inbox page (cold launch share) | 2.35 ms | 60 ms |
| Sidebar mailboxes with counts | 0.01 ms | 3 ms |
| Inbox first page, 150 rows | 0.14 ms | 8 ms |
| Archive page at any depth, 150 rows | 0.12 ms | 8 ms |
| Open thread: detail + all bodies | 0.03 ms | 5 ms |

**Clean Up (Rust, release; 2026-10-08, Apple Silicon).** The fixture
rebuilt with schema v18 (131,826 messages; its services now carry
`List-Id`). Groups for every view (spec §14.12), budget 200 ms each, p95
over 10 runs:

| View | Inbox | All Mail |
|---|---|---|
| Sender (965 groups) | 19.0 ms | 12.7 ms |
| People I've Emailed (960) | 26.0 ms | 26.1 ms |
| Subject (32) | 12.8 ms | 6.0 ms |
| Mailing Lists (3) | 10.4 ms | 0.6 ms |
| Time (64) | 15.2 ms | 45.4 ms |
| Social (3) | 8.9 ms | 3.7 ms |
| Promotions (3) | 9.4 ms | 4.6 ms |
| Size (1) | 12.3 ms | 20.3 ms |

The largest sender's first 200 messages: 6.0 ms; its count: 1.7 ms
(budget 50 ms each). All Mail scans the views' covering indexes and
leaves out Spam, Trash and drafts through one small set of ids; the
Inbox is driven from its label. A debug build takes about three times as
long, still under budget. Leaving optimistic local copies of sent mail
out of scope (oagc-merk.4, a range on the `gmail_id` index) left every
figure within run-to-run noise (rerun 2026-10-08: Sender 19.1/14.4 ms,
Time 15.5/41.3 ms).

**Clean Up's apply (Rust, release; 2026-10-08).** Trashing 20,000
messages (the biggest senders that fit, in All Mail) in one transaction,
as `cleanup_apply` does: the labels, every touched thread's mailboxes and
counts, 20 outbox batches of 1,000 and the one undo record: p50 593 ms,
p95 866 ms over 3 runs (budget 2 s); 44,685 messages took 1.15 s. The
bench puts the messages back after each run.

The same run found two older measures over budget, unchanged with or
without Clean Up's indexes: sidebar mailboxes with counts, 24.6 ms
(budget 3 ms; the Inbox categories' unread count added since 2026-09-24),
and the structured search `is:unread in:inbox`, 18–20 ms (budget 20 ms).

**Through the FFI (Swift, debug build):**

| Operation | p95 | Budget |
|---|---|---|
| Inbox page via FFI, 150 rows | 1.94 ms | 20 ms |
| Select thread → reader data ready | 0.45 ms | 30 ms |
| Build reader HTML | 0.02 ms | 3 ms |
| Configure + lay out 1,000 list rows | 30 ms | 120 ms |

## Not yet measured

- WebKit paint time after `loadHTMLString` (the rest of "body visible in
  < 50 ms").
- Scroll frame rate in the thread list; needs an interactive session with
  Instruments (Animation Hitches).
- Cold launch end to end (process start → first frame).
- A self-hosted Apple Silicon CI runner to track these over time (spec §13).
