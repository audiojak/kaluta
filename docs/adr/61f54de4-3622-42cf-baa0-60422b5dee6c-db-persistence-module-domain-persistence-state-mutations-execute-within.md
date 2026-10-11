# Db Persistence Module: Domain Persistence State Mutations Execute Within

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Domain features across the storage crate require persistent entity management, transaction safety, and consistent error handling.
- Data access logic across separate domain models relies on a shared Db abstraction that exposes transactional write closures and isolated read closures.
- Direct database access without structured transaction boundaries creates risks of uncommitted mutations, lock contention, and divergent error reporting.

## Problem Statement

Managing persistent state across diverse domain entities requires a unified access pattern that enforces transaction boundaries and prevents partial writes. Without a standardized persistence abstraction, individual storage modules could bypass transactional controls, leading to race conditions, incomplete rollback handling, and inconsistent error signatures.

## Decision

1. MUST: All domain persistence state mutations MUST execute within transaction closures dispatched through write_blocking on the Db instance.

## Policy Block

- MUST All domain persistence state mutations MUST execute within transaction closures dispatched through write_blocking on the Db instance.

In scope:
- Domain persistence operations and data store modules requiring read or write access to persistent storage.
- Transaction management and state transitions for stored domain records.

Out of scope:
- Pure in-memory data transformation logic that does not interact with persistent storage.
- Transient network payloads and external protocol serialization outside the storage boundary.

## Rationale

- Standardizing on Db with write_blocking and read_blocking guarantees that all mutating operations occur within atomic transaction closures while reads remain isolated.
- Enforcing StoreResult return types across all public storage functions establishes predictable, uniform error propagation throughout the domain layer.
- Centralizing database handle access ensures consistent connection pooling, file handling, and isolation levels across all domain repositories.

## Consequences

Positive:
- Guarantees atomic rollback and consistency across all entity mutation workflows.
- Prevents accidental write operations within read query paths through separate closure signatures.
- Normalizes error handling across domain boundaries using standard store error types.

Negative:
- Synchronous blocking operations can introduce latency bottlenecks under high write concurrency.
- Enforcing closure-based execution requires passing transactional contexts explicitly through nested helper functions.

## Alternatives

- Unencapsulated Direct Database Connection Access (rejected)
  Rejected because: Direct connection access allows modules to execute queries without transactional safety and leads to inconsistent error handling.
  When valid: Small prototype scripts with single-threaded, non-critical data persistence requirements.
- Asynchronous Connection Pool Abstraction (deferred)
  Rejected because: Asynchronous execution adds runtime complexity and dependency overhead that is unnecessary for local blocking storage workloads.
  When valid: High-throughput networked database services requiring asynchronous non-blocking connection multiplexing.

## Risks

- Write transaction contention could lead to blocking delays during long-running write operations.
  Mitigation: Keep write transaction closures minimal, performing data preparation and transformation before entering write_blocking.
  Owner: engineering team
- Uncaught panics within transaction closures could leave scratch databases or resources in an inconsistent state during testing.
  Mitigation: Encapsulate test database lifecycles in RAII scratch structures that clean up directories upon drop.
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
- Structure all store submodule functions as standalone public operations that accept a transaction or connection reference passed by Db closures.
- Ensure domain types stored in or retrieved from the persistence layer implement standard serialization, deserialization, and equality traits.

## Continuation Context


Verify commands:
- Inspect repository configuration to identify the test runner and execute the automated test suite covering storage modules.
- Discover the workspace static analysis tool and run verification checks against data access code to confirm transaction closure compliance.

Accept when:
- All unit and integration tests executing queries and updates through Db transaction closures pass successfully.
- Persistence methods consistently propagate errors using StoreResult without panicking or leaking underlying database connections.

## Enforcement

- Verified by: Automated test runs in the continuous integration pipeline verifying transactional persistence and rollback behavior.
- Verified by: Peer code reviews confirming that new persistence access logic uses Db closures instead of raw connection management.
- Violation handling: Pull requests attempting direct unencapsulated storage access or omitting transaction boundaries are blocked from merging.
- Violation handling: Violations identified during code review must be refactored to use standard Db closures before approval.
- Exception process: Exceptions require approval from the architecture review board accompanied by documented performance or initialization justifications.