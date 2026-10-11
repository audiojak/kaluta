# Adopt mail_domain Shared Module for Core Domain Entities and Typed Identifiers: Modules Encapsulate Domain Identifier Sets Standard

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Multiple workspace crates require shared email domain definitions, including thread identifiers, message identifiers, and account addresses, to coordinate messaging operations.
- Exchanging raw strings across service, agent, and persistence boundaries risks transposition errors, invalid formatting, and leaky domain boundaries.
- Centralizing domain identity types within a dedicated domain library establishes consistent validation and serialization across independent components.

## Problem Statement

Passing unvalidated primitive strings across asynchronous agent boundaries, persistence stores, and mailbox synchronization layers leads to type confusion, invalid domain state, and duplicate parsing logic across workspace components.

## Decision

1. SHOULD: Modules SHOULD encapsulate domain identifier sets using standard collection structures wrapping mail_domain types for scoping and filtering operations.

## Policy Block

- SHOULD Modules SHOULD encapsulate domain identifier sets using standard collection structures wrapping mail_domain types for scoping and filtering operations.

In scope:
- All workspace components defining or consuming email domain entities, identifiers, and session contexts.
- Inter-crate communication contracts, event payloads, and agent tool definitions.

Out of scope:
- External protocol transport adapters that serialize or deserialize raw wire formats at the perimeter.
- Generic utility modules that do not model email domain entities.

## Rationale

- Adopting the dedicated mail_domain module provides compile-time guarantees against identifier transposition and malformed entity representations across crate boundaries.
- Centralized serialization and parsing logic ensures consistent wire-format representation across agent event streams and persistence storage layers.
- Shared domain types decouple business identity concepts from the underlying transport mechanisms and specific storage engine implementations.

## Consequences

Positive:
- Eliminates identifier transposition bugs between disparate domain entity types at compile time.
- Establishes a single source of truth for email entity data structures and serialization behavior.
- Simplifies agent session contracts by enabling explicit scope filtering and typed entity references.

Negative:
- Introduces shared dependency coupling across workspace packages to the domain module.
- Requires wrapping and unwrapping primitive values when interfacing with external raw APIs.

## Alternatives

- Use Primitive String Types Across Crate Boundaries (rejected)
  Rejected because: Leaves system vulnerable to identifier transposition errors and lacks compile-time validation.
  When valid: Only in rapid prototyping where domain models are unstable and type safety is deprioritized.
- Duplicate Domain Type Definitions in Each Consuming Crate (rejected)
  Rejected because: Causes domain divergence, incompatible serialization schemas, and maintenance overhead across crates.
  When valid: In fully isolated microservices communicating exclusively via network RPC protocols without shared libraries.

## Risks

- Breaking changes in domain entities necessitate coordinated updates across all dependent workspace crates.
  Mitigation: Maintain backward-compatible constructor and serialization contracts with semantic versioning practices.
  Owner: engineering team
- Unnecessary allocation or conversion overhead between primitive values and strongly typed identifier wrappers.
  Mitigation: Implement transparent newtype representations with zero-cost abstraction patterns where possible.
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
- Construct domain identifiers exclusively via provided constructors such as ThreadId::new or MessageId::new to ensure proper initialization.
- Derive serialization and standard collection traits on domain types to facilitate hashing, ordering, and event payload transport across boundaries.

## Continuation Context


Verify commands:
- # Discover and execute the project verification script across domain-dependent packages
project_verify=$(find . -maxdepth 2 -type f \( -name "*verify*" -o -name "*check*" \) -perm -111 | head -n 1)
[ -n "$project_verify" ] && "$project_verify" || true
- # Verify all workspace references resolve through the authoritative dependency lock configuration
find . -maxdepth 3 -type f \( -name "*lock*" -o -name "*manifest*" \)

Accept when:
- All unit and integration test suites pass across workspace packages consuming the domain module.
- Static type analysis passes without primitive string substitutions for domain identifier types in public contracts.
- The dependency resolution artifact matches the declared manifest ranges and builds cleanly.

## Enforcement

- Verified by: Automated workspace build checks and unit test execution in continuous integration pipelines.
- Verified by: Peer code reviews verifying that new inter-crate interfaces utilize domain module types instead of primitive values.
- Violation handling: Merge requests that introduce primitive string identifiers for domain entities in public APIs are blocked until refactored.
- Violation handling: Compiler errors caused by mismatched domain identifier types prevent code integration.
- Exception process: Architectural exceptions require written rationale approved by the lead architect and documented in tracking tickets.