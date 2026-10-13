# AgentManager Realtime Session Event Routing and Coordination Boundary: Agentmanager Guard Internal Session Registries Provider

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Multi-agent runtime environments require coordinating prompt turn dispatches across diverse agent providers while isolating individual session states.
- Client consumers require realtime streaming updates of text deltas and execution lifecycle events rather than blocking on full turn completion.
- Concurrent session initialization and status queries necessitate synchronized access to provider registries without blocking runtime worker loops.

## Problem Statement

Direct invocation of heterogeneous agent providers from application endpoints tightly couples caller logic to provider-specific lifecycles, risks race conditions in uncoordinated session registries, and prevents unified realtime event multiplexing across concurrent execution streams.

## Decision

1. MUST: AgentManager MUST guard internal session registries and provider detection status tables using asynchronous mutual-exclusion locks to guarantee safe concurrent access.

## Policy Block

- MUST AgentManager MUST guard internal session registries and provider detection status tables using asynchronous mutual-exclusion locks to guarantee safe concurrent access.

In scope:
- Modules and runtime boundaries coordinating agent provider sessions, turn inputs, and realtime execution event streams.

Out of scope:
- Static prompt template parsers and offline batch data processors operating without realtime interactive agent providers.

## Rationale

- Encapsulating provider operations behind AgentManager decouples client callers from downstream protocol variances and installation checks.
- Multiplexing events onto an asynchronous channel tagged by session identifier provides responsive, ordered streaming telemetry across concurrent sessions.
- Protecting internal session registries with asynchronous locks guarantees safe concurrent lifecycle transitions without blocking event loops.

## Consequences

Positive:
- Decouples consuming modules from disparate provider implementations through a unified session management boundary.
- Guarantees correlated realtime streaming events tagged by session identifier across concurrent execution streams.
- Ensures thread-safe session tracking, cached provider detection, and graceful resource cleanup upon cancellation.

Negative:
- Channel-based event streaming requires consumers to drain receivers diligently to avoid buffering accumulation.
- Shared lock boundaries for session registries introduce potential serialization bottlenecks under high-volume concurrent session creation.

## Alternatives

- Direct synchronous agent invocation per client request (rejected)
  Rejected because: Synchronous invocation blocks caller threads, prevents realtime incremental event streaming, and tightly couples client interfaces to provider-specific execution protocols.
  When valid: Single-turn command-line scripts with no requirement for concurrent execution, background polling, or streaming telemetry.
- Individual dedicated event channels per session instance (rejected)
  Rejected because: Proliferates channel lifecycle management, increases resource footprint, and complicates centralized event multiplexing and observability.
  When valid: Distributed architectures where individual sessions execute across physically isolated computing nodes.

## Risks

- Unbounded event channels could accumulate unconsumed events if client receivers stall or lag behind provider output.
  Mitigation: Implement consumer backpressure monitoring and establish bounded channel buffer limits where applicable.
  Owner: engineering team
- Lock contention on shared session registries during burst session creation may increase request latency.
  Mitigation: Utilize read-optimized synchronization structures and minimize lock retention duration during session queries.
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
- Ensure consuming routines read from the realtime event channel concurrently with turn dispatch calls to prevent channel consumer stalls.
- Maintain session identifiers as consistent correlation attributes across all outbound events, logs, and state transition telemetry.

## Continuation Context


Verify commands:
- Discover the project test runner and execute the integration suite covering session lifecycle management and concurrent event streaming.
- Execute the repository static analysis and linting suite to confirm concurrency lock scopes and channel sender patterns comply with project standards.

Accept when:
- All unit and integration tests covering session creation, turn dispatching, event reception, and cancellation pass without error.
- Realtime events received on the channel receiver match expected event sequences and correlate accurately with initiating session identifiers.
- Session registries accurately reflect zero active sessions following explicit session closure and cancellation operations.

## Enforcement

- Verified by: Automated test suites executing concurrent session lifecycle and event routing scenarios.
- Verified by: Static analysis and architectural review gates verifying adherence to session coordination boundaries.
- Violation handling: Pull requests introducing uncoordinated direct provider invocations will be blocked by automated checks.
- Violation handling: Code introducing unbuffered channel leaks or unsynchronized shared mutable state will be rejected during peer review.
- Exception process: Submit an architectural exception request detailing concurrency benchmarks and demonstrating alternative isolation mechanisms to the technical lead.