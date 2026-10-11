# openagc-core Internal Library Module Adoption for Core Application Orchestration: Domain Consumers Not Bypass Core Contract

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Multiple domain capabilities including mail synchronization, routine execution, undo histories, and external agent sessions require coordinated access to shared account state.
- Direct interactions with backend persistence layers and communication providers across multiple modules lead to duplicate business logic and uncoordinated state transitions.
- External agents and cloud routines execute automated actions that require strict isolation, read-only verification modes, and explicit approval mechanisms.
- A unified domain abstraction is necessary to provide deterministic event notifications and reversible mutation tokens across distinct mailbox sessions.

## Problem Statement

Directly coupling user interfaces, automated routine schedulers, and external agent integrations to raw database stores and disparate network synchronization transports creates fragmented state transitions, inconsistent mutation histories, and unverified privilege escalation. A centralized architectural core module is required to mediate all domain transactions, enforce approval policies, and coordinate event dissemination.

## Decision

1. MUST_NOT: Domain consumers MUST_NOT bypass the Core API contract to directly execute transactional writes on backend storage engines.

## Policy Block

- MUST_NOT Domain consumers MUST_NOT bypass the Core API contract to directly execute transactional writes on backend storage engines.

In scope:
- All domain business logic, mailbox mutation workflows, and scheduled routine orchestrations.
- External agent connectors and approval workflows interacting with user accounts and mailboxes.
- Multi-account synchronization lifecycle management and background event distribution.

Out of scope:
- Low-level transport protocol parsing that does not mutate account state.
- Pure utility functions and independent mathematical data formatting.

## Rationale

- Centralizing state mutations and scheduling behind the openagc-core API ensures invariant enforcement, consistent event distribution, and atomic outbox processing across accounts.
- Providing reversible action tokens within the core module enables uniform undo and redo semantics regardless of the triggering interface or synchronization transport.
- Isolating agent routines within dedicated session contexts and gating destructive operations behind approval tickets prevents unauthorized account modifications.

## Consequences

Positive:
- Unified domain boundary simplifies multi-account lifecycle management and provides a single point of enforcement for security policies.
- Reversible action tokens allow consistent undo capabilities across all user-facing and automated mailbox mutations.
- Structured event listener contracts decouple asynchronous background synchronization from user interface state updates.

Negative:
- High centralization increases coupling of peripheral modules to the core lifecycle contract.
- Modifications to the shared domain interfaces require coordinated updates across all calling agent adapters and service extensions.

## Alternatives

- Direct storage access from individual agent integrations and synchronization services (rejected)
  Rejected because: Bypassing a unified core domain module causes state divergence, bypasses mutation logging, and makes consistent undo mechanics impossible.
  When valid: Valid only in trivial single-purpose command utilities that do not share state or require event propagation.
- Microservice decomposition communicating exclusively over remote network RPCs (rejected)
  Rejected because: Introduces substantial serialization overhead, operational deployment complexity, and distributed transaction challenges for local desktop application contexts.
  When valid: Valid in distributed multi-tenant cloud architectures where services scale independently across different infrastructure clusters.

## Risks

- Bottlenecks in the core domain actor or event dispatcher under high volumes of concurrent mailbox updates.
  Mitigation: Employ asynchronous database transaction queues, batched event notifications, and non-blocking background workers.
  Owner: Core Platform Engineering
- Interface bloat on the central Core struct as new capabilities and agent tools are introduced.
  Mitigation: Decompose domain capabilities into modular traits and delegated internal subsystem controllers while preserving a cohesive external facade.
  Owner: Architecture Review Board

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Instantiate the central Core struct with dedicated configuration, secret store references, and an EventListener implementation during application bootstrap.
- Wrap destructive agent operations with approval tickets that require explicit resolution before committing changes to storage.

## Continuation Context


Verify commands:
- Locate the primary build configuration manifest and run the test suite for the core domain module.
- Run the workspace unit and integration test harnesses to verify routine execution, mutation undo cycles, and account synchronization.
- Execute the project static analysis and linting checks to ensure no unauthorized direct references bypass the core library boundary.

Accept when:
- All mailbox mutations executed via the core module successfully record reversible action tokens that restore state upon invocation.
- Scheduled routines run within isolated sessions and correctly record execution outcomes without leaking session state.
- The project test suite passes with complete test coverage over multi-account synchronization and event propagation.

## Enforcement

- Verified by: Continuous integration test pipelines executing automated unit and integration suites.
- Verified by: Architecture boundary linters preventing direct imports of internal database storage crates from consumer adapters.
- Verified by: Peer code review verifying adherence to core domain mutation and approval patterns.
- Violation handling: Pull requests bypassing the core module or directly writing to persistence layers will be blocked during automated verification.
- Violation handling: Architectural violations must be refactored to consume the public Core interface before merging.
- Exception process: Exceptions for direct database access or custom synchronization bridges require submission of an architectural waiver and explicit approval from the principal engineering team.