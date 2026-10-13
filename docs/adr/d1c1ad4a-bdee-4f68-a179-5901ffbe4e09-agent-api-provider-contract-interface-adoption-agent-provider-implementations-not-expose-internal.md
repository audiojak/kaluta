# agent-api Provider Contract Interface Adoption: Agent Provider Implementations Not Expose Internal

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- The application integrates diverse AI agent runtimes to automate unattended tasks, preview routines, and draft communications.
- Different agent providers require distinct execution binaries, environment configurations, and streaming output parsers.
- Decoupling core orchestration from individual agent mechanics requires a centralized interface contract within the workspace.

## Problem Statement

Integrating multiple AI agent backends directly into application core logic risks tight coupling to specific command-line arguments, process lifecycles, and event streaming mechanisms. Without a shared contract boundary, testing agent-dependent workflows requires running actual external agent binaries, leading to brittle and slow test suites.

## Decision

1. MUST_NOT: Agent provider implementations MUST NOT expose provider-internal process handling primitives, argument parsers, or direct process streams outside their respective integration crates.

## Policy Block

- MUST_NOT Agent provider implementations MUST NOT expose provider-internal process handling primitives, argument parsers, or direct process streams outside their respective integration crates.

In scope:
- Development of external agent provider integrations
- Workspace packages defining or consuming agent process lifecycle contracts
- Asynchronous agent session orchestration and approval dispatch

Out of scope:
- Internal mail synchronization protocols and database persistence models
- In-memory secret storage and credential resolution

## Rationale

- The agent-api module establishes a clean boundary between core business logic and external agent runtime environments, decoupling process execution from state management.
- Standardizing provider interfaces around async_trait and Arc ensures asynchronous safety across concurrent tasks while maintaining consistent error handling and cancellation semantics.
- Encapsulating process discovery in Locator and event emission in AgentEvent allows new provider engines to be integrated without altering core orchestration logic.

## Consequences

Positive:
- Agent providers can be developed, tested, and upgraded independently of core application logic.
- Test doubles in agent-api eliminate the need to run live agent subprocesses during test suites.
- Consistent event streaming via AgentEvent enables uniform audit logging and approval handling across all agent providers.

Negative:
- Changes to agent-api traits require coordinated updates across all implementing provider crates.
- Dynamic dispatch with async_trait and Arc introduces minor heap allocation overhead for asynchronous futures.

## Alternatives

- Direct Subprocess Execution in Core (rejected)
  Rejected because: Spawning provider processes directly within core application logic couples provider-specific arguments and process monitoring directly to domain services.
  When valid: Only viable in single-provider prototypes where external process variation does not exist.
- Dynamic Foreign Function Interface Plugin Loading (rejected)
  Rejected because: Shared library foreign function interface plugins introduce binary interface stability constraints and cross-platform loading overhead compared to compile-time workspace contracts.
  When valid: When external providers are distributed as compiled binary plugins by third parties.

## Risks

- Divergence between provider capabilities and the shared contract definitions in agent-api.
  Mitigation: Design contract interfaces around extensible event streams and configurable execution sinks rather than provider-specific flags.
  Owner: Agent Platform Team
- Subprocess deadlocks during external agent session execution.
  Mitigation: Enforce strict execution timeouts and cancellation token monitoring across all provider session runners.
  Owner: Agent Platform Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Implement new agent providers in isolated workspace packages that declare a dependency on the agent-api contract module.
- Ensure all provider event sinks handle AgentEvent streams asynchronously without blocking the parent execution thread.

## Continuation Context


Verify commands:
- Discover and run the workspace test suite targeting agent contract packages and provider integrations.
- Execute static analysis and contract verification checks defined in the repository workflow configuration.

Accept when:
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

## Enforcement

- Verified by: Continuous integration test passes verifying contract implementations across workspace packages.
- Verified by: Architectural code review ensuring no provider-specific process handling primitives leak into core orchestration.
- Violation handling: Pull requests bypassing agent-api contracts or directly invoking provider subprocesses in core will be blocked during code review.
- Exception process: Exceptions require formal architectural review and approval by the platform leads, documented with an explicit rationale.