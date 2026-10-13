# tracing-subscriber Structured Diagnostic Registry and Event Layering: Background Session Engines Provider Workers Attach

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Asynchronous runtime engines and agent integration routines execute concurrent operations that require unified diagnostic visibility across task boundaries.
- Operational telemetry emitted by background synchronization and provider execution contains sensitive tokens and addresses that must be sanitized before ingestion.
- Decoupling diagnostic instrumentation from downstream event consumers requires an extensible registry pattern rather than hard-coded logging sinks.

## Problem Statement

Without a unified, structured telemetry framework, concurrent runtime engines and agent integration routines emit unstructured diagnostics that are difficult to correlate across asynchronous tasks, lack contextual metadata, and risk leaking sensitive credentials or personal addresses into diagnostic sinks.

## Decision

1. MUST: Background session engines and provider workers MUST attach subscriber handles to the active asynchronous executor to preserve diagnostic context across task boundaries.

## Policy Block

- MUST Background session engines and provider workers MUST attach subscriber handles to the active asynchronous executor to preserve diagnostic context across task boundaries.

In scope:
- All background services, runtime engines, and agent provider implementations that produce diagnostic events or telemetry.
- All custom subscriber layers and diagnostic sinks consuming structured events.

Out of scope:
- Test harness mocks and unit verification routines using isolated in-memory no-operation listeners.
- External third-party subprocess binaries executing outside the direct host runtime subscriber context.

Exceptions:
- EX-48-001: A component requires isolated local test output without attaching to the primary event bus.

## Rationale

- The tracing and tracing-subscriber framework provides structured context propagation across asynchronous execution contexts and thread boundaries.
- Layered subscriber composition enables independent filtering, formatting, and forwarding of operational warnings and errors to the event bus without altering instrumented call sites.
- Visitor-based field recording provides deterministic redaction and sanitization of authentication tokens and sensitive user information before events reach persistent storage.

## Consequences

Positive:
- Establishes consistent structured operational telemetry across all asynchronous services and agent providers.
- Guarantees deterministic scrubbing of sensitive credentials and addresses prior to log persistence or event bus distribution.
- Enables dynamic subscriber layer composition for decoupled diagnostic forwarding and localized file rotation.

Negative:
- Introduces allocation and synchronization overhead during structured field formatting and subscriber dispatch.
- Requires strict adherence to structured field schemas and visitor conventions instead of arbitrary string formatting.

## Alternatives

- Direct unstructured standard output and error logging (rejected)
  Rejected because: Lacks contextual field attribution, fails to preserve asynchronous execution boundaries, and prevents centralized redaction of sensitive credentials.
  When valid: Small standalone diagnostic scripts with no asynchronous concurrency or privacy compliance requirements.
- Monolithic single-sink logging framework (rejected)
  Rejected because: Lacks composite subscriber layering capabilities required for simultaneous local rotating persistence and dynamic event bus forwarding.
  When valid: Single-threaded utilities directing all diagnostic records exclusively to an external system daemon.

## Risks

- Unbounded buffer growth or execution blocking if the event bus subscriber sink stalls under heavy warning or error volume.
  Mitigation: Employ non-blocking channel dispatch and bounded queues within event bus subscriber layers.
  Owner: Runtime Infrastructure Team
- Accidental bypass of data scrubbing when developers format sensitive data directly into event message strings rather than structured fields.
  Mitigation: Enforce automated static linting and diagnostic message scrubbing on all raw event strings prior to disk persistence.
  Owner: Security Architecture Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Configure the global or scoped subscriber registry during runtime initialization before spawning background asynchronous tasks.
- Implement field visitor traits to sanitize sensitive token values and email patterns prior to formatting event records.

## Continuation Context


Verify commands:
- Discover and run the project repository validation scripts that verify diagnostic layer composition and test suite compliance.
- Execute the repository linting and static analysis suite to confirm absence of unredacted credential patterns in diagnostic outputs.

Accept when:
- All background service operations emit structured events captured by registered subscriber layers.
- Diagnostic verification tests confirm that sensitive tokens and addresses are scrubbed while warnings and errors reach the event bus.
- Repository verification suites and integration tests pass with zero subscriber dispatch panics or deadlocks.

## Enforcement

- Verified by: Automated continuous integration test suites verifying event listener delivery and field redaction behavior.
- Verified by: Peer code review auditing all new diagnostic call sites for adherence to structured field conventions.
- Violation handling: Merge requests containing unstructured logging or bypassing subscriber redaction are blocked until rectified.
- Exception process: Submit an architectural exception request outlining specific technical constraints that prevent subscriber registry integration.