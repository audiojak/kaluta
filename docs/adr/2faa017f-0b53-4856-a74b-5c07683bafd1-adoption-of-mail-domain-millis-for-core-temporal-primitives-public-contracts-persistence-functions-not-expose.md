# Adoption of mail_domain::Millis for Core Temporal Primitives: Public Contracts Persistence Functions Not Expose

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Storage and search subsystems require consistent point-in-time representations across query AST nodes, background routine execution logs, and task tracking records.
- Using untyped primitives or fragmented datetime representations across module boundaries risks epoch scale confusion, timezone ambiguity, and conversion overhead.
- The mail_domain core library provides unified domain primitives to standardize temporal semantics across database persistence and search execution.

## Problem Statement

Representing points in time across storage, query parsing, and background task management without a standardized domain primitive introduces risks of scale mismatches between seconds and milliseconds, timezone inconsistencies, and redundant parsing logic across subsystem boundaries.

## Decision

1. MUST_NOT: Public contracts and persistence functions MUST NOT expose raw unvalidated integer values or polymorphic datetime representations where domain timestamps are exchanged across storage boundaries.

## Policy Block

- MUST_NOT Public contracts and persistence functions MUST NOT expose raw unvalidated integer values or polymorphic datetime representations where domain timestamps are exchanged across storage boundaries.

In scope:
- Data structures and domain models representing timestamps, execution histories, task due dates, and search filter bounds within storage and query subsystems.
- Public contracts and persistence adapters handling creation, modification, or querying of time-sensitive entity records.

Out of scope:
- User interface presentation formatting components where localized human-readable time strings are rendered.
- External protocol adapters communicating with third-party network services prior to conversion into domain representations.

Exceptions:
- EXC-20-001: Interfacing directly with third-party operating system or runtime APIs that require native clock duration structures.

## Rationale

- Static analysis demonstrates mail_domain::Millis as a shared core dependency across search query parsing, search execution, routine execution, and task tracking modules.
- A standardized domain primitive eliminates ambiguity between seconds, milliseconds, and microseconds across transactional write boundaries and search index lookups.
- Standardizing on epoch millisecond integers ensures deterministic numeric ordering in database records and exact equality comparisons in parsed search filters.

## Consequences

Positive:
- Guarantees uniform timestamp granularity and comparison semantics across storage records and query filters.
- Eliminates unit mismatch defects and conversion ambiguities across transactional boundaries.
- Enables efficient numeric indexing, range scanning, and monotonic ordering within persistence engines.

Negative:
- Requires explicit conversion logic when parsing relative time intervals or external calendar strings into domain primitives.
- Direct human inspection of raw database columns requires epoch timestamp conversion to derive human-readable dates.

## Alternatives

- Raw primitive integer types without semantic domain wrapper (rejected)
  Rejected because: Lacks semantic type safety, allowing unit confusion between seconds, milliseconds, and microseconds across module boundaries.
  When valid: In low-level resource-constrained systems without domain abstraction requirements.
- ISO 8601 formatted text strings in persistence and abstract syntax tree structures (rejected)
  Rejected because: Introduces string parsing and formatting overhead, variable-length storage footprint, and complex collation in search filtering.
  When valid: In external human-readable configuration files or public REST API responses.

## Risks

- Integer overflow or negative timestamp edge cases when performing relative date arithmetic.
  Mitigation: Enforce checked or saturating arithmetic within domain helper routines that manipulate millisecond values.
  Owner: Architecture and Core Infrastructure Team
- Inadvertent loss of timezone metadata when normalizing dates to millisecond epoch timestamps.
  Mitigation: Store dedicated timezone identifiers alongside the millisecond timestamp when timezone preservation is strictly required by domain rules.
  Owner: Data Architecture Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Construct search query abstract syntax tree representations by converting relative time offsets and absolute calendar dates into millisecond representations during the parse phase.
- Ensure all persistence transaction closures receive domain entities utilizing canonical millisecond types for record creation, modification, and execution checkpoints.

## Continuation Context


Verify commands:
- Discover the workspace build and test execution runner to run all unit test suites covering query parsing and domain storage operations.
- Execute the workspace static analysis and linting suites to ensure timestamp fields conform to domain primitive constraints.
- Run database round-trip test suites to confirm millisecond timestamp persistence and retrieval fidelity across routines and tasks.

Accept when:
- All unit test suites covering query parsing, routine execution, and task persistence pass without failures.
- Static analysis confirms all public timestamp fields across storage and query contracts reference the canonical domain primitive.
- Persistence round-trip verification confirms deterministic serialization and ordering of millisecond timestamps.

## Enforcement

- Verified by: Automated continuous integration pipeline running static analysis and unit test suites on every pull request.
- Verified by: Architecture and peer review verifying that public API contracts and persistence layers utilize the canonical domain primitive.
- Violation handling: Continuous integration failure blocking pull request merges when unapproved temporal representations are detected.
- Violation handling: Mandatory code review remediation requiring conversion of raw integer or third-party datetime values to canonical domain types.
- Exception process: Submit an architectural exception request outlining specific technical constraints preventing the use of the canonical domain primitive.
- Exception process: Obtain formal approval from core architecture maintainers prior to merging non-conforming interface types.