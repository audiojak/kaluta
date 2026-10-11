# futures::executor::block_on for Asynchronous Unit Test Execution: Engineers Use Dedicated Runtime Test Macros

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Core application modules expose asynchronous interfaces for storage operations, routine workflows, and agent interactions.
- Target embedding environments invoke core exports across foreign function interface boundaries from unmanaged host threads without an ambient asynchronous reactor.
- Asynchronous unit testing traditionally defaults to multi-threaded runtime macros that introduce non-deterministic task scheduling and hide reactor dependency coupling.
- Codebase evidence across core packages demonstrates consistent adoption of futures::executor::block_on to drive asynchronous futures synchronously on isolated test threads.

## Problem Statement

Asynchronous core domain logic requires fast, deterministic unit test verification. Relying on multi-threaded asynchronous runtime test harnesses introduces scheduler non-determinism, increases test execution overhead, and obscures whether exported asynchronous interfaces can safely execute within non-reactor host environments. A standardized testing mechanism is required to execute asynchronous unit tests deterministically on dedicated threads while verifying that domain logic remains decoupled from specific runtime engines.

## Decision

1. MAY: Engineers MAY use dedicated runtime test macros only when the test subject explicitly requires asynchronous timers, physical process supervision, or cross-task channels that cannot advance on a single-threaded executor.

## Policy Block

- MAY Engineers MAY use dedicated runtime test macros only when the test subject explicitly requires asynchronous timers, physical process supervision, or cross-task channels that cannot advance on a single-threaded executor.

In scope:
- Unit test modules verifying asynchronous core domain logic, state machines, and data access layers.
- Test routines validating export boundaries and foreign function interfaces intended for unmanaged host environments.

Out of scope:
- End-to-end integration tests that exercise real network transport protocols or live background daemons.
- Asynchronous tests with strict requirements for timer facilities, child process supervision, or multi-threaded task stealing.

Exceptions:
- EXC-39-001: A unit test specifically exercises timer-driven timeouts, external process execution, or event channels requiring an active asynchronous reactor loop.

## Rationale

- Adopting futures::executor::block_on enables direct synchronous driving of asynchronous futures on the caller thread, eliminating scheduling races and thread synchronization delays.
- Executing asynchronous exports on plain threads mirrors host embedding constraints where external callers invoke core routines without an ambient runtime context.
- Evidence across twenty-one files shows consistent usage of futures::executor::block_on alongside isolated temporary fixtures, proving test determinism and high execution velocity.

## Consequences

Positive:
- Unit test execution runs deterministically on single threads without scheduler jitter or multi-threaded race conditions.
- Asynchronous domain APIs remain decoupled from specific runtime engines, ensuring compatibility with host foreign function interface callers.
- Test execution overhead decreases by avoiding reactor startup, background thread pool allocation, and teardown cycles.

Negative:
- Tests cannot exercise operations that rely on active reactor facilities such as runtime timers or cooperative task preemption without custom mocking.
- Blocking execution on a single thread risks test process deadlocks if a future yields indefinitely on unhandled external events.

## Alternatives

- Mandating multi-threaded runtime test attributes for all asynchronous unit tests (rejected)
  Rejected because: Spawning multi-threaded runtime reactors for every unit test introduces significant thread allocation overhead, non-deterministic scheduler timing, and fails to prove that APIs operate without ambient reactor dependencies.
  When valid: When testing complex multi-task coordination, active socket polling, or real-time process management.
- Refactoring all domain operations into synchronous blocking variants exclusively for testing (rejected)
  Rejected because: Maintaining duplicate synchronous and asynchronous implementations creates codebase divergence, maintenance burden, and testing gap risks.
  When valid: When a library has no asynchronous requirements and exposes purely synchronous public APIs.

## Risks

- Unbounded future awaiting causing test runner thread deadlock when a future does not complete
  Mitigation: Enforce test timeouts at the test execution runner configuration layer and utilize explicit wait loops with timeout thresholds in polling test helpers
  Owner: engineering team
- Inadvertently calling timer-dependent runtime APIs inside synchronous executor test contexts
  Mitigation: Structure core business logic around injectable abstract time providers and clock abstractions rather than concrete runtime timers
  Owner: engineering team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Construct test fixtures with isolated temporary directory paths incorporating process identifiers to guarantee parallel test workspace isolation.
- Implement polling helper closures with explicit bounded iteration limits and sleep intervals when waiting for asynchronous background state reconciliation.

## Continuation Context


Verify commands:
- # Discover repository test manifests and execute unit tests utilizing the synchronous executor pattern
- $REPO_TEST_DISCOVERED --unit

Accept when:
- All unit tests using futures::executor::block_on complete successfully with zero assertion failures or deadlocks.
- Asynchronous core routines successfully execute and return expected domain values without requiring an ambient reactor.

## Enforcement

- Verified by: Automated continuous integration test execution gates.
- Verified by: Peer code review verification of new test modules to ensure adherence to single-thread synchronous executor conventions.
- Violation handling: Pull requests introducing unnecessary ambient multi-threaded runtime test attributes for pure domain logic will be blocked during code review.
- Exception process: Submit an architectural review request outlining why the specific test case requires ambient reactor services rather than synchronous executor driving.