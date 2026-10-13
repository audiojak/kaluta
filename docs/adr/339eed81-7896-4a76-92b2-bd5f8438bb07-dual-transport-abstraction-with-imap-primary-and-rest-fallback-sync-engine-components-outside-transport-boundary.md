# Dual-transport abstraction with IMAP primary and REST fallback: Sync Engine Components Outside Transport Boundary

Status: proposed
Date: 2026-10-05
Deciders: AI (signal conversion)

## Context

- Email synchronization in the sync engine was previously coupled directly to the Gmail REST API with client-side rate-limit throttling. Under this model, operations were constrained by Gmail REST quota ceilings and high polling latency.
- To resolve quota exhaustion and improve latency, message backfill, folder operations (such as Spam, Trash, and Drafts), and push notifications have been transitioned to persistent IMAP connections utilizing `async-imap`. REST endpoints are retained as an automatic circuit-broken fallback.
- Introducing persistent connection state alongside automatic fallback creates operational complexity at the sync boundary, raising requirements to ensure connection stalls are monitored and that direct provider API calls do not bypass the unified transport.

## Problem Statement

Direct Gmail REST API integration suffers from rate-limiting quotas and polling latency, whereas moving to IMAP introduces protocol connection state and the need for circuit-broken fallback to REST without leaking transport details into the sync engine.

## Decision

1. MUST_NOT: Sync engine components outside the transport boundary MUST NOT hold or manipulate protocol connection state directly.

## Policy Block

- MUST_NOT Sync engine components outside the transport boundary MUST NOT hold or manipulate protocol connection state directly.

In scope:
- crates/mail-sync/src/transport.rs
- crates/mail-sync/src/engine.rs
- crates/provider-api/src/lib.rs
- crates/provider-gmail/src/imap.rs

Out of scope:
- crates/provider-gmail/src/imap_fake.rs
- Non-synchronization administrative API workflows

Exceptions:
- ex-1: Test harnesses and mock implementations require simulated direct socket or protocol state control.

## Rationale

- Encapsulating dual transports behind a unified `Transport` boundary insulates the sync engine (`crates/mail-sync/src/engine.rs`) from protocol-level state and failure switching logic.
- Using IMAP as the primary transport bypasses aggressive REST quota exhaustion and enables low-latency IDLE push notifications.
- Maintaining REST endpoints behind an automatic circuit breaker preserves service availability during IMAP connectivity stalls without requiring ad-hoc failover handling in core sync workflows.

## Consequences

Positive:
- Significantly decreases API quota consumption against the Gmail REST API during bulk backfill.
- Reduces sync latency by leveraging persistent IMAP IDLE push notifications.
- Decouples sync engine logic from specific transport protocols and fallback mechanisms.

Negative:
- Increases architecture complexity through persistent connection management and stateful IMAP lifecycle handling.
- Circuit breakers can potentially mask chronic IMAP connection degradation if fallback metrics are not properly monitored.

## Alternatives

- Direct synchronization against the Gmail REST API with API rate-limit throttling (rejected)
  Rejected because: Subject to Gmail REST quota limits, polling latency penalties, and an inability to support persistent push notifications efficiently.
- Pure IMAP-only transport without REST fallback (rejected)
  Rejected because: Fails to provide fault tolerance if IMAP connections experience protocol stalls or transient authentication/network failures where REST endpoints remain reachable.

## Risks

- Automatic circuit breaking to REST may silently mask chronic IMAP connection stalls and exhaust REST quotas unexpectedly in production.
  Mitigation: Expose transport metrics and circuit breaker transition state alerts to track IMAP failure frequency and fallback duration.
  Owner: Sync Infrastructure Team
- Connection state synchronization issues between IMAP IDLE and REST fallback could cause duplicate or missed message sync events.
  Mitigation: Enforce strict deduplication and state verification using backfill source markers (`BackfillSource`) within the engine boundary.
  Owner: Sync Engine Team

## Implementation Notes

- The `Transport` abstraction is implemented in `crates/mail-sync/src/transport.rs`.
- The IMAP connection implementation is encapsulated within `crates/provider-gmail/src/imap.rs` using `async-imap`.
- Backfill strategy tracking utilizes `BackfillSource` to coordinate sync states across transport transitions.

## Continuation Context


Verify commands:
- Discover and run the project's integration and unit test suite targeting transport boundary contracts and fallback behavior.
- Discover and execute architectural or module dependency linters to ensure no direct provider client usage exists outside transport modules.

Accept when:
- All email sync network operations are routed strictly through the `Transport` interface.
- Bulk backfill and push notification workflows use IMAP while gracefully falling back to REST upon induced IMAP failures.
- Circuit breaker state changes are observable and testable upon IMAP connection failures.

## Enforcement

- Verified by: Static analysis and module boundary checks ensuring sync modules only import `transport::Transport`.
- Verified by: Unit and integration test suites validating circuit breaker transition and fallback invocation.
- Violation handling: Build failures triggered by forbidden direct imports of REST or IMAP provider clients in sync engine code.
- Violation handling: Rejection during code review of PRs bypassing the transport abstraction.
- Exception process: Architecture review sign-off documented via ADR amendment when provider-specific protocol access is required outside the unified transport.

## References

- file:crates/mail-sync/src/transport.rs
- file:crates/mail-sync/src/engine.rs
- file:crates/provider-api/src/lib.rs
- file:crates/provider-gmail/src/imap.rs
- file:crates/provider-gmail/src/imap_fake.rs
- file:crates/openagc-core/src/account.rs
- commit:60c207ade70bf8b29ec0b972d8a8ac467661dbc4
- commit:8657ef06d31a8397b653bda00e8b0e1dfe4fc9d2
- commit:f9a68081382cbe0dac59d6085e404835f2bea0d6
- commit:d133cb25e3ca62169042eb9d3cf8526259992b97
- commit:2a8f60922d2ee015377f3f494223646f3653de37
- commit:5adeb53499b283d7d2e09f028c509af0d10cd7ab
- commit:6393cf36dc0212f2273cc58c0f5ff24d3af98684
- commit:ba7d44a2847a918a1e089d240e488519d18a98ef
- commit:9cce263494558c2094e00106ab38c0c4e5610554
- commit:bd4ae8df7d71dee3f785f0142b62353fe275e587