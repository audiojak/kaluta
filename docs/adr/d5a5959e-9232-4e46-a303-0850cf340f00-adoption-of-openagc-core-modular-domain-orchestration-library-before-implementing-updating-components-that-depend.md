# Adoption of openagc-core Modular Domain Orchestration Library: Before Implementing Updating Components That Depend

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Multiple accounts and background synchronization tasks operate concurrently, requiring centralized coordination to prevent race conditions and cross-account data leakage.
- Autonomous agent interactions and scheduled background routines execute state modifications requiring systematic auditing, timeout handling, and rollback capabilities.
- Direct ad-hoc database mutations across concurrent threads cause transactional deadlocks and inconsistent data models without structured boundaries.
- Downstream components and user interfaces require decoupled, account-tagged event notifications regarding synchronization milestones and mailbox updates.

## Problem Statement

Managing concurrent multi-account synchronization, autonomous agent tool executions, and scheduled routine tasks without a unified domain orchestrator leads to fragmented state management, race conditions, accidental data leakage between accounts, and untracked side effects. The architecture requires a consolidated core engine that encapsulates database transactions, lifecycle events, scoped execution contexts, and approval mechanisms behind a coherent domain facade.

## Decision

1. MUST: Before implementing or updating components that depend on openagc-core or related workspace modules, the consumer MUST inspect the repository dependency lock artifact to determine and adhere to the exact resolved versions.

## Policy Block

- MUST Before implementing or updating components that depend on openagc-core or related workspace modules, the consumer MUST inspect the repository dependency lock artifact to determine and adhere to the exact resolved versions.

In scope:
- All domain services, account synchronization engines, background routine executors, and agent interaction coordinators.
- Internal modules requiring access to account-scoped data stores and transactional persistence.

Out of scope:
- Stateless utility helpers that do not interact with domain data stores, account states, or background routines.
- Standalone protocol mock implementations operating exclusively in memory without lifecycle orchestration.

Exceptions:
- EXC-20-001: Isolated unit testing of standalone message formatting components that have no account or database dependencies

## Rationale

- Centralizing domain operations within the Core abstraction ensures uniform lifecycle handling and consistent account isolation across concurrent background operations.
- Funneling database operations through read and write closures prevents concurrent transaction conflicts and guarantees data integrity.
- Enforcing explicit approval pipelines for routine runs and agent interactions establishes an audit trail and provides a mechanism for undoing unintended state modifications.

## Consequences

Positive:
- Provides strict multi-account isolation via scoped execution contexts across concurrent background operations.
- Enforces consistent transactional database access patterns across all reading and writing components.
- Centralizes background routine scheduling and autonomous agent governance with built-in auditability and rollback mechanisms.
- Decouples internal domain state transitions from external consumers via a structured event notification system.

Negative:
- Introduces synchronization overhead through shared coordinator state and lock management across concurrent tasks.
- Requires domain state interactions to pass through asynchronous runtime closures, increasing call stack depth.
- Mandates mock coordinator implementations and event listeners for isolated unit testing of downstream components.

## Alternatives

- Direct database store access from decoupled domain modules (rejected)
  Rejected because: Eliminates centralized concurrency control, risking cross-account store contamination and uncoordinated concurrent writes.
  When valid: In simple single-user utilities where only a single database file exists and no background concurrency occurs.
- Distributed multi-process architecture with independent service processes per account (rejected)
  Rejected because: Incurs significant memory footprint and inter-process communication overhead unsuitable for an embedded application core.
  When valid: In distributed multi-tenant server infrastructure operating across compute clusters.

## Risks

- Contention on shared coordinator locks and store reference handles under high concurrency.
  Mitigation: Ensure store access closures execute minimal work and offload CPU-intensive operations to dedicated blocking worker pools.
  Owner: core architecture team
- Failure to wrap account operations in scoped contexts leading to default-account state pollution.
  Mitigation: Enforce scoped execution through helper wrappers and automated integration test validations.
  Owner: core platform team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Wrap instances of the core coordinator in shared reference handles and pass them to background service workers to ensure single-instance lifecycle management per application session.
- When initiating background routines or provider synchronization tasks, establish scoped execution contexts to guarantee proper account routing and event attribution.

## Continuation Context


Verify commands:
- Discover the workspace test runner from the project repository configuration and execute the unit and integration test suites validating core engine operations.
- Discover the repository static analysis script and execute verification to confirm compliance with transactional read-write closures.
- Discover and run the multi-account concurrency integration tests to verify account scoping and event isolation.

Accept when:
- All unit and integration tests covering the core coordinator, account registry, routines, and agent approvals pass without error.
- Static analysis confirms no direct database store access bypasses the core transactional closures.
- Concurrent multi-account test executions demonstrate zero cross-account data leakage and verified event attribution.

## Enforcement

- Verified by: Automated continuous integration test suites validating core concurrency, transaction safety, and account isolation.
- Verified by: Architectural and peer code reviews for all pull requests introducing or modifying domain persistence or background routines.
- Violation handling: Code changes bypassing the core coordinator or omitting transactional closures must be rejected in pull request review.
- Violation handling: Uncoordinated direct database queries detected in active branches must be refactored to use coordinator access closures before merging.
- Exception process: Submit an architectural exception request detailing technical necessity to the core platform team for formal review.
- Exception process: Document temporary exemptions with an associated tracking issue and a targeted remediation timeline.