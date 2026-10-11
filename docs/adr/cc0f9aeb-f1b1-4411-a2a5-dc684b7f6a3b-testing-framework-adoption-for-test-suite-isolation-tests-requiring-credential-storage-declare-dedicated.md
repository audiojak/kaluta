# Testing Framework Adoption for Test Suite Isolation: Tests Requiring Credential Storage Declare Dedicated

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Automated testing across desktop core clients, UI shell windows, and background agent panels requires strict separation between test execution environments and local user state.
- Interaction with production directories, persistent keychain services, or real external agent endpoints during automated test runs introduces non-deterministic test results and risks local data corruption.
- Adopting the Testing framework establishes structured test suites with standardized isolation barriers across data storage, credential management, and asynchronous runtime operations.

## Problem Statement

Without rigorous test execution isolation, automated test runs risk mutating local user data directories, polluting user keychain storage, and hanging on live external agent dependencies. A structured testing strategy is required to enforce reproducible test execution across core services, user interface windows, and agent controllers without leaking side effects into the host environment.

## Decision

1. MUST: Tests requiring credential storage MUST declare dedicated test keychain services and MUST NOT interact with host user keychain services.

## Policy Block

- MUST Tests requiring credential storage MUST declare dedicated test keychain services and MUST NOT interact with host user keychain services.

In scope:
- Unit and integration test suites covering core client routines, UI shell components, and agent coordination panels.
- Test harness configurations managing temporary storage, mock keychain services, and simulated agent runtimes.

Out of scope:
- End-to-end acceptance runs against designated staging environments that explicitly require production credential validation.
- Production application runtime paths executing outside automated test runner contexts.

## Rationale

- The Testing framework provides native support for structured asynchronous testing, isolated execution contexts, and fine-grained test organization.
- Enforcing distinct scratch directories and test-specific keychain service namespaces guarantees hermetic test runs that can execute in parallel without cross-test state leakage.
- Decoupling agent panels and UI host windows from live runtime services prevents flaky network calls and persistent display state side effects.

## Consequences

Positive:
- Guarantees test hermeticity by preventing test suites from reading or writing host user state and real data stores.
- Eliminates test suite flakiness caused by environmental factors, live external agent timeouts, or shared keychain state.
- Enables safe parallel and headless test execution in continuous integration environments.

Negative:
- Requires maintaining explicit mock fixtures, scratch directory lifecycles, and isolated keychain configurations in the test harness.
- May fail to catch subtle integration issues that only emerge when communicating with real host subsystem persistence layers.

## Alternatives

- Direct test execution against shared host environment directories and default user keychain services (rejected)
  Rejected because: Introduces cross-test interference, non-deterministic execution, and risks destructive modification of local user data.
  When valid: Only valid for quick manual diagnostic scripts executed in throwaway virtualized environments.
- Mocking all filesystem and keychain APIs via third-party mocking libraries (rejected)
  Rejected because: Introduces heavy mocking maintenance overhead and couples tests to internal API implementation details rather than behavioral contracts.
  When valid: Valid when underlying operating system storage APIs cannot be configured with custom directory paths or namespaces.

## Risks

- Scratch test directories may accumulate and exhaust disk storage if teardown routines fail.
  Mitigation: Implement automated cleanup hooks tied to test lifecycle events and ensure scratch locations utilize temporary system partitions.
  Owner: engineering team
- Discrepancies between isolated test harnesses and production runtime environments could mask configuration errors.
  Mitigation: Supplement unit and component test suites with targeted end-to-end integration verifications in isolated containers.
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
- Initialize scratch directory allocations during test setup and ensure isolation paths are completely decoupled from production configuration paths.
- Verify that test keychain service identifiers utilize distinct domain prefixes to avoid collision with production credentials.

## Continuation Context


Verify commands:
- Discover the test execution command from the project manifest and execute the complete automated test suite to ensure all test suites pass.
- Inspect test execution logs to verify that all filesystem and credential operations target temporary test scratch locations.

Accept when:
- All automated test suites execute successfully within isolated scratch environments without accessing real data directories.
- No test cases fail due to state contamination from prior test runs or concurrent test executions.
- All credential operations in test suites resolve against dedicated test keychain namespaces without modifying host user credentials.

## Enforcement

- Verified by: Automated continuous integration pipeline running test suites on every pull request.
- Verified by: Peer code review auditing new test suites for adherence to test isolation standards.
- Violation handling: Pull requests containing tests that access real data directories or omit scratch isolation MUST be blocked from merging.
- Violation handling: Test failures attributable to environment pollution or missing isolation triggers immediate remediation.
- Exception process: Exceptions for dedicated end-to-end testing requiring real service interaction must be documented and approved by the engineering architecture team.