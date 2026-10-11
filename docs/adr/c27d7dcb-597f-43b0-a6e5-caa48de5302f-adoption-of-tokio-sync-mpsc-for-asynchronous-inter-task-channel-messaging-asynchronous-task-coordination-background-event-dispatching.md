# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Asynchronous Task Coordination Background Event Dispatching

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Asynchronous services require reliable decoupled mechanisms to exchange data and control signals across independent tasks without introducing data races.
- Direct shared-memory synchronization primitives across asynchronous task boundaries create risk of deadlocks, contention, and unpredictable task suspension.
- Multiple core subsystems including protocol socket handlers, domain event buses, and agent streaming pipelines require multi-producer single-consumer message delivery.

## Problem Statement

Concurrent tasks within the application architecture need to exchange messages, stream protocol frames, and dispatch domain events without coupling execution lifecycles or sharing mutable state across asynchronous boundaries. Without a standardized asynchronous channel primitive, components risk introducing inconsistent concurrency models, thread starvation, or deadlocks from uncoordinated synchronization.

## Decision

1. MUST: Asynchronous task coordination, background event dispatching, and socket message routing MUST use tokio::sync::mpsc channels to pass discrete typed messages between producers and consumers.

## Policy Block

- MUST Asynchronous task coordination, background event dispatching, and socket message routing MUST use tokio::sync::mpsc channels to pass discrete typed messages between producers and consumers.

In scope:
- Asynchronous service actors coordinating background message processing.
- Event broadcasting and coalescing buses dispatching domain events to listeners.
- Socket and protocol handlers routing incoming frames and outbound replies.

Out of scope:
- Synchronous in-memory operations confined to a single thread or local execution scope.
- High-throughput shared-memory read caches requiring read-write lock synchronization.

Exceptions:
- EXC-20-001: A single-threaded synchronous component requires direct callback invocation without background task dispatch.

## Rationale

- Standardizing on tokio::sync::mpsc establishes a unified message-passing concurrency paradigm across socket handlers, event buses, and API streaming.
- Multi-producer single-consumer channels cleanly enforce single-owner message consumption while allowing arbitrary producer handles to queue work concurrently.
- Asynchronous channel primitives integrate directly with runtime polling, eliminating blocking calls and simplifying backpressure and cancellation handling.

## Consequences

Positive:
- Eliminates shared mutable state across asynchronous task boundaries, reducing concurrency defects and deadlock hazards.
- Enforces decoupled producer-consumer lifecycles with natural backpressure signaling through channel capacity limits.
- Standardizes event routing and message reception across diverse subsystem crates.

Negative:
- Requires explicit capacity tuning for bounded channels to balance memory footprint against producer suspension.
- Introduces channel allocation and buffer maintenance overhead compared to direct in-place method invocations.
- Requires robust handling of channel disconnection and receiver task termination to prevent silent message drops.

## Alternatives

- Shared mutable state protected by mutexes and condition variables across asynchronous tasks (rejected)
  Rejected because: Shared-state locking across async yield points introduces severe lock contention, priority inversion, and deadlock hazards.
  When valid: Confined strictly to synchronous read-heavy data caches within a single execution thread.
- Synchronous blocking channel queues bridging worker thread pools (rejected)
  Rejected because: Synchronous blocking channels block runtime worker threads and lack native non-blocking asynchronous await polling integration.
  When valid: Bridging compute-intensive synchronous thread pools to external non-async native libraries.

## Risks

- Channel buffer exhaustion in bounded queues under burst traffic leading to producer stalls or dropped events.
  Mitigation: Implement event coalescing, bounded capacity sizing based on load characteristics, and telemetry alerts on queue saturation.
  Owner: Engineering Team
- Silent message loss if a receiver task terminates unexpectedly while senders remain active.
  Mitigation: Monitor receiver loop heartbeats and handle send error returns to trigger supervisor recovery.
  Owner: Engineering Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Instantiate channels using explicit capacity configurations, passing cloned sender handles to producer tasks while retaining the single receiver handle within the dedicated consumer task loop.
- Pair channel reception with cancellation-safe select blocks and timeout guards to ensure receiver tasks respond promptly to shutdown signals.

## Continuation Context


Verify commands:
- Discover the workspace test runner and execute all asynchronous unit and integration tests covering channel communication and event coalescing.
- Discover the project static analysis and linting scripts to verify asynchronous channel usage complies with concurrency guidelines.

Accept when:
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

## Enforcement

- Verified by: Automated continuous integration test runs executing concurrency test suites and static analysis checks.
- Verified by: Mandatory peer code reviews validating channel lifecycle handling and absence of cross-task mutex locks.
- Violation handling: Pull requests introducing uncoordinated shared-memory locking across tasks or unbounded channel proliferation will be blocked from merging.
- Violation handling: Violations discovered in active development must be refactored to use bounded channel message passing before deployment.
- Exception process: Submit an architectural review request detailing why channel-based message passing cannot meet latency or throughput requirements, including benchmark evidence.
- Exception process: Obtain explicit sign-off from the technical lead before implementing alternative synchronization primitives.