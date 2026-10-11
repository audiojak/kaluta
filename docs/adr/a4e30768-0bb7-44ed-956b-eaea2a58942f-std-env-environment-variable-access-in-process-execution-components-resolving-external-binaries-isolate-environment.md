# std::env Environment Variable Access in Process Execution: Components Resolving External Binaries Isolate Environment

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- The codebase requires mechanisms to locate and invoke external command-line binaries asynchronously.
- Process path resolution requires querying host environment variables while maintaining deterministic behavior in automated testing.
- Direct dependency on unconstrained ambient environment variables introduces risks of unintentional binary hijacking or untracked state dependencies.

## Problem Statement

Querying ambient environment variables directly across subprocess dispatch routines creates tight coupling to host runtime state and risks unvalidated binary discovery. A disciplined boundary is necessary to constrain environment variable inspection, provide explicit directory overrides, and bound child process execution times.

## Decision

1. MUST: Components resolving external binaries MUST isolate environment variable access to dedicated locator abstractions rather than querying host environment state directly throughout call sites.

## Policy Block

- MUST Components resolving external binaries MUST isolate environment variable access to dedicated locator abstractions rather than querying host environment state directly throughout call sites.

In scope:
- Process execution modules and binary locator utilities that query host environment variables.
- Subprocess execution wrappers that spawn external executables.

Out of scope:
- Internal in-memory processing modules that do not interact with host operating system environment variables or child processes.

Exceptions:
- EXC-43-001: Test harnesses requiring fixed synthetic path overrides in temporary directories

## Rationale

- Encapsulating environment variable inspection inside dedicated locator abstractions allows test suites to supply deterministic directory lists without mutating global process state.
- Enforcing explicit timeouts on asynchronous child process executions prevents stalled external commands from exhausting system worker threads.
- Restricting environment variable usage prevents ambient environment pollution from being confused with structured credential and secret management.

## Consequences

Positive:
- Decouples command lookup logic from ambient host environment variables, enabling deterministic unit and integration tests.
- Enforces deterministic process termination through bounded execution timeouts.
- Reduces the attack surface associated with unvalidated path traversal during external binary resolution.

Negative:
- Requires explicit locator configuration and initialization rather than ad-hoc subprocess invocations.
- Adds architectural overhead for simple command executions that would otherwise rely on ambient system paths.

## Alternatives

- Direct ambient environment variable lookup at every command dispatch call site (rejected)
  Rejected because: Scattering ambient environment variable reads hinders deterministic testing and couples execution to unverified external system state.
  When valid: Single-script utilities where test isolation and process governance are unnecessary.
- Hardcoding absolute system binary paths within configuration files (rejected)
  Rejected because: Fails across heterogeneous host environments where binary locations vary across installations.
  When valid: Containerized deployments with strictly immutable filesystem layouts.

## Risks

- Environment variable resolution may locate an unexpected binary if search path precedence is misconfigured.
  Mitigation: Implement strict path ordering and require explicit directory candidate lists in the locator abstraction.
  Owner: Engineering Team
- Subprocess execution may hang indefinitely if timeouts are omitted or misconfigured.
  Mitigation: Enforce mandatory timeout durations across all asynchronous process execution wrappers.
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
- Ensure all locator abstractions expose constructors allowing injected search paths to bypass default environment lookups during automated tests.
- Wrap asynchronous process executions in bounded timeout futures to ensure orderly cleanup of orphaned child processes upon cancellation.

## Continuation Context


Verify commands:
- # Discover the repository build and test script and run test suites covering process execution and path resolution
- # Discover the repository static analysis suite and verify that environment variable queries are restricted to locator modules

Accept when:
- All automated process execution tests pass with explicit timeout verification and isolated locator paths.
- Static analysis confirms no unencapsulated environment variable queries exist outside approved locator modules.

## Enforcement

- Verified by: Continuous integration test pipelines executing automated unit and integration tests.
- Verified by: Peer code reviews auditing subprocess execution patterns and environment variable access.
- Violation handling: Pull requests introducing unencapsulated environment queries or unbounded child process executions must be blocked until refactored.
- Exception process: Exceptions require documented approval from security and architecture leads detailing why ambient environment access cannot be encapsulated.