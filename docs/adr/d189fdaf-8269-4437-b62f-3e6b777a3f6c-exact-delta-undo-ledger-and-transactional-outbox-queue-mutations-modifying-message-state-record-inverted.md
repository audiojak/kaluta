# Exact-delta undo ledger and transactional outbox queue: Mutations Modifying Message State Record Inverted

Status: proposed
Date: 2026-10-05
Deciders: AI (signal conversion)

## Context

- Mutations initiated by user actions and agent tool calls require responsive, optimistic UI updates without sacrificing data integrity or the ability to revert state changes.
- Previously, outbox queue entries were scheduled directly without logging reversal deltas, preventing operations such as label updates from being undone or queued outbound email transmissions from being cancelled prior to remote synchronization.

## Problem Statement

Direct, irreversible outbox scheduling prevents rolling back optimistic message mutations and cancelling delayed outgoing mail sends before network dispatch occurs.

## Decision

1. MUST: Mutations modifying message state MUST record inverted deltas in the undo ledger prior to scheduling external sync outbox jobs.

## Policy Block

- MUST Mutations modifying message state MUST record inverted deltas in the undo ledger prior to scheduling external sync outbox jobs.

In scope:
- crates/mail-store database migrations and storage routines
- crates/mail-sync outbox processing loops
- crates/openagc-core mutation and compose logic

Out of scope:
- Read-only message queries and search indexing
- External mail synchronization ingest pipelines receiving inbound messages

## Rationale

- Recording inverted deltas atomically with state changes guarantees that every staged mutation can be reversed exactly without relying on lossy full snapshots.
- Providing a cancellable outbox dispatch buffer allows delayed email dispatches and agent batch mutations to be intercepted before external side effects take place.

## Consequences

Positive:
- Optimistic UI state changes can be immediately reflected and safely rolled back upon user or agent request.
- Outgoing emails held in the dispatch buffer can be cancelled before transmission across the network.
- Per-message label changes can be accurately reversed using exact diffs without overwriting unrelated concurrent updates.

Negative:
- Additional database write overhead and storage consumption for maintaining the undo ledger.
- Increased state management complexity across the mutation and outbox synchronization pipelines.

## Alternatives

- Direct, irreversible outbox queuing without delta logging or undo capability (rejected)
  Rejected because: Precludes optimistic UI rollbacks and cannot cancel queued sends or revert label modifications prior to external network dispatch.
- Snapshot-based full message state restoration for undo history (rejected)
  Rejected because: Storing full message snapshots incurs significant storage overhead and risks overwriting concurrent mutations to unrelated fields during reversal.

## Risks

- Rapid agent tool batch actions could trigger race conditions between outbox dispatch and undo cancellations.
  Mitigation: Enforce a dispatch timeout window and verify cancellation state within the outbox worker loop immediately before initiating network requests.
  Owner: mail-sync team

## Implementation Notes

- Schema migration 0007_undo.sql introduces the ledger schema required for delta persistence.
- The outbox dispatch loop in crates/mail-sync must check cancellation status prior to executing network calls.

## Continuation Context


Verify commands:
- Discover and run the project integration tests for mail storage mutations, undo ledger rollbacks, and outbox queue dispatch.

Accept when:
- Message mutations write exact inverted deltas into the undo ledger within the same database transaction.
- Executing an undo operation correctly restores prior message label diffs.
- Delayed outbox queue entries are successfully cancelled and excluded from network dispatch when undone.

## Enforcement

- Verified by: Automated transactional integration tests covering mutation, delta generation, and undo rollbacks.
- Verified by: Code review verification on all database mutation handlers touching mail store state.
- Violation handling: Pull requests scheduling outbox jobs without corresponding inverted delta ledger records will be blocked at code review.
- Exception process: Submit an architectural waiver detailing why a state mutation is strictly idempotent or cannot support reversal.

## References

- file:crates/mail-store/migrations/0007_undo.sql
- file:crates/mail-store/src/undo.rs
- file:crates/mail-store/src/outbox.rs
- file:crates/mail-sync/src/outbox.rs
- file:crates/openagc-core/src/mutations.rs
- file:crates/openagc-core/src/compose.rs
- commit:89031ca4f7c5cf7f81ec3555f2b14a69e855beb8
- commit:a11c928fb9ed2e156ad981411813c3057ee8cfcc
- commit:705978943ac9082067597598c685d976dc022811
- commit:e196ca4970110b5b9f1b75cb6227e7088b8924cc
- commit:0233b934150682d1d8e7868eed701ad0196d6d39
- commit:5667c37b120850b0be6ca0f6921a0c636386764b