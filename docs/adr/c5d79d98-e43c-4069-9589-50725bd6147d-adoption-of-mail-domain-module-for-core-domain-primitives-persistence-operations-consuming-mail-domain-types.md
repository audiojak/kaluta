# Adoption of mail_domain Module for Core Domain Primitives: Persistence Operations Consuming Mail Domain Types

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Persistence and storage modules manage records that represent domain concepts, including message identifiers and temporal tracking points.
- Representing core identifiers and timestamps with raw primitive types introduces primitive obsession, risks parameter transposition, and causes semantic divergence between persistence boundaries and domain logic.
- The codebase establishes the internal mail_domain module to centralize domain-specific primitive types across service boundaries.
- Multiple storage and queue implementations import mail_domain primitives to define their public query parameters, mutation inputs, and result schemas.

## Problem Statement

Storage layer abstractions frequently exchange domain identifiers and timestamp values across database query boundaries, exposing persistence interfaces to type-confusion errors when primitives such as raw strings or generic integers are used. Without standardized domain types, separate storage modules risk defining divergent identifier representations and inconsistent temporal precision, weakening type guarantees between core domain business rules and persistence operations.

## Decision

1. SHOULD: Persistence operations consuming mail_domain types SHOULD validate or propagate domain representations through standard store error handling abstractions.

## Policy Block

- SHOULD Persistence operations consuming mail_domain types SHOULD validate or propagate domain representations through standard store error handling abstractions.

In scope:
- All persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling.
- Storage contracts that manipulate message identifiers or millisecond-level timestamps.

Out of scope:
- Low-level database connection management and migration drivers that operate directly on raw database protocol drivers.
- External transport serialization adapters that require unmapped string or integer protocols at network boundaries.

## Rationale

- Adopting mail_domain domain primitives in persistence boundaries prevents primitive obsession and ensures compile-time validation for entity identifiers.
- Standardizing on mail_domain::Millis ensures consistent time resolution across storage routines, task tracking, and queue operations.
- Sharing core domain types between domain logic and persistence contracts eliminates redundant conversion layers and guarantees semantic consistency across the codebase.

## Consequences

Positive:
- Compile-time type checking eliminates parameter transposition errors between entity identifiers and temporal values.
- Centralizes identity and timestamp representations into a single source of truth shared across persistence and business logic.
- Public persistence interfaces provide self-documenting and semantically rigorous API contracts.

Negative:
- Storage module interfaces become coupled to the mail_domain package, requiring synchronized updates when domain primitives evolve.
- Persistence layer mapping code must explicitly translate between underlying database driver values and mail_domain wrapper types.

## Alternatives

- Use raw scalar primitives for identifiers and epoch timestamps in storage contracts (rejected)
  Rejected because: Permits type confusion between distinct entities, lacks compile-time safety, and risks inconsistent timestamp units across storage tables.
  When valid: Valid in minimal standalone scripts or low-level database engine bindings without domain semantics.
- Define separate storage-specific identifier and timestamp wrapper types within the persistence crate (rejected)
  Rejected because: Introduces redundant type duplication of domain concepts and requires continuous conversion boilerplate between storage and domain boundaries.
  When valid: Valid when persistence models must maintain strict hexagonal isolation from core domain types.

## Risks

- Breaking changes in mail_domain primitive definitions propagate into all persistence interfaces.
  Mitigation: Maintain strict backward compatibility in mail_domain public types and follow lock resolution procedures during updates.
  Owner: engineering team
- Conversion overhead between database driver scalar types and mail_domain wrapper structures.
  Mitigation: Implement zero-cost newtype wrappers and standard conversion traits in storage mapping layers.
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
- Inspect the workspace configuration to discover internal package dependencies and link mail_domain to persistence modules.
- Ensure row-mapping functions implement conversion traits between raw database columns and mail_domain domain types.

## Continuation Context


Verify commands:
- Discover the repository build tool from the project manifest and execute the test suite across all persistence modules.
- Discover and run the project code linter and type checker to verify that all storage interfaces adhere to mail_domain type constraints.

Accept when:
- All persistence module test suites execute successfully without compilation errors or type mismatches.
- No public storage interface accepts or returns raw unvalidated strings or integer primitives where mail_domain domain types are required.

## Enforcement

- Verified by: Automated continuous integration build checks enforcing strict type checking across workspace boundaries.
- Verified by: Peer code review confirming that new persistence interfaces utilize mail_domain primitives.
- Violation handling: Pull requests introducing raw primitive identifiers or timestamps in public storage APIs are blocked until updated to mail_domain types.
- Violation handling: Compilation and type-check failures prevent merging of non-compliant code.
- Exception process: Submit an architectural review request outlining why a raw primitive is strictly required, subject to approval by the persistence module lead.