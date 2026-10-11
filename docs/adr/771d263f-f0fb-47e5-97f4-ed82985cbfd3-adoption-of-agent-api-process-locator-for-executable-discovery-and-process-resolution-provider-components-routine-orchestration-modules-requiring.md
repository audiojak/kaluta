# Adoption of agent_api::process::Locator for Executable Discovery and Process Resolution: Provider Components Routine Orchestration Modules Requiring

Status: proposed
Date: 2025-02-14
Deciders: Detection Pipeline (automated)

## Context

- Multiple agent provider implementations interact with external command-line executables to perform provider capability detection and interactive session management.
- Direct invocation of host system executables using ambient system environment variables couples provider implementations to specific host environments and obstructs automated test isolation.
- Core routine workflows require the capability to route execution to sandboxed mock executables or specific execution targets during testing without invoking production agent binaries.

## Problem Statement

External agent providers and routine runners depend on locating and invoking command-line interfaces across differing operating system environments and test harnesses. Embedding ambient path resolution or hardcoded executable lookup inside individual provider modules creates brittle runtime couplings, complicates security auditing of spawned binaries, and prevents hermetic integration testing where simulated executables must be injected.

## Decision

1. MUST: All provider components and routine orchestration modules requiring interaction with external command-line executables MUST encapsulate executable lookup and resolution through agent_api::process::Locator.

## Policy Block

- MUST All provider components and routine orchestration modules requiring interaction with external command-line executables MUST encapsulate executable lookup and resolution through agent_api::process::Locator.

In scope:
- Modules responsible for agent provider integrations that invoke or detect external command-line processes.
- Core service and routine orchestration workflows that coordinate external agent runners or cloud routine proxies.

Out of scope:
- Purely internal library computations and in-memory data transformation modules that do not spawn external processes.
- Web service connectors operating exclusively over network application protocols without local process dependencies.

Exceptions:
- EXC-20-001: Direct process spawning is performed by an isolated operating system sandbox wrapper that supersedes standard locator discovery.

## Rationale

- Centralizing process discovery behind agent_api::process::Locator guarantees uniform executable resolution rules and eliminates divergent path parsing across independent provider modules.
- Injecting Locator instances enables hermetic automated testing, permitting test suites to swap production binaries for mock scripts without modifying provider runtime logic.
- Standardizing on Locator references across session and detection workflows establishes a clear architectural contract when coordinating asynchronous process lifecycles.

## Consequences

Positive:
- Decouples provider domain logic from host-specific filesystem layouts and ambient environment variable state.
- Enables hermetic automated testing by allowing test fixtures to substitute simulated test binaries in isolated temporary directories.
- Establishes a centralized architectural boundary for auditing and restricting external executable invocations.

Negative:
- Increases constructor signature verbosity across provider implementations by requiring explicit locator dependency injection.
- Requires synchronization primitives when sharing mutable locator state across asynchronous task boundaries in multi-account or multi-provider runtimes.

## Alternatives

- Ambient environment path lookup using standard library process command resolution (rejected)
  Rejected because: Ambient resolution couples process execution to global host environment variables, prevents deterministic test sandboxing, and risks binary hijacking from untrusted search directories.
  When valid: Isolated command-line utilities without multi-provider pluggable interfaces or integration test isolation requirements.
- Static configuration strings specifying absolute binary paths per provider (rejected)
  Rejected because: Requires fragile host-specific configuration files across differing deployment targets and breaks automated execution in dynamic or ephemeral environments.
  When valid: Single-tenant deployments where executable installation locations are strictly immutable and standardized across all machines.

## Risks

- Misconfigured Locator search paths may cause provider initialization failure if expected executables are not discovered.
  Mitigation: Implement clear diagnostic error reporting during the provider detection phase to articulate search boundaries and missing binaries.
  Owner: Agent Integration Engineering
- Concurrent modification of shared locator state in test harnesses could lead to test pollution across parallel test executions.
  Mitigation: Construct isolated locator instances and scoped temporary directories per test case rather than mutating shared global state.
  Owner: Core Platform Engineering

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Construct Locator instances during application bootstrap and inject shared references into provider factories and detection routines.
- In test suites, instantiate scoped directory trees containing mock execution binaries and configure Locator instances to target solely those directories.

## Continuation Context


Verify commands:
- Discover the workspace dependency manifest and execute the test runner to verify all provider detection and session startup routines pass.
- Execute the project verification test suite for cloud routine runners to confirm isolated locator behavior with simulated executables.
- Run static linting and dependency analysis checks to verify no direct process spawning occurs outside the locator abstraction.

Accept when:
- All unit and integration tests for agent providers and cloud routine runners pass without invoking unmanaged host system executables.
- Executable discovery correctly resolves targets from configured locator directories and fails with structured errors when targets are missing.
- Provider detection and session startup workflows successfully receive and utilize injected locator instances.

## Enforcement

- Verified by: Continuous integration test pipelines executing automated unit and integration suites.
- Verified by: Peer code review verifying that all external process interactions utilize the locator module abstraction.
- Violation handling: Pull requests introducing direct ambient process execution or bypassing locator injection will be blocked during review and automated verification.
- Exception process: Exceptions require formal architectural review and submission of an approved exception request detailing underlying process isolation guarantees.