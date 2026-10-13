# Adoption of mail_domain Core Domain Identity Types: Domain Modules Derive Standard Serialization Formatting

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- The mail subsystem consists of modular crates handling distinct responsibilities including queue management, persistent mail storage, drafting, and analytical sampling.
- Without a shared domain modeling library, independent crates risk passing raw primitive strings for message identifiers, leading to primitive obsession, parameter transposition bugs, and leaky abstraction boundaries.
- The mail_domain module provides centralized, strongly typed domain primitives including mail_domain::MessageId, which are consumed by storage queues and core orchestration layers to enforce invariant checks at compile time.

## Problem Statement

Passing message identifiers as primitive string types across decoupled storage, queueing, and workflow crates causes type-safety degradation, permits invalid identifier propagation across crate boundaries, and increases the likelihood of subtle runtime errors such as parameter transposition.

## Decision

1. MAY: Domain modules MAY derive standard serialization, formatting, and comparison traits on domain types to support seamless diagnostic logging and structured testing fixtures.

## Policy Block

- MAY Domain modules MAY derive standard serialization, formatting, and comparison traits on domain types to support seamless diagnostic logging and structured testing fixtures.

In scope:
- All workspace crates and modules defining interfaces for message queuing, persistent storage retrieval, message drafting, and analytical sampling.
- Internal service boundaries passing email entity references between storage and orchestration components.

Out of scope:
- External transport and serialization adapters where protocol formats necessitate native raw string representations prior to validation.
- Transient, non-mail identifier handling that does not represent email entity identity.

Exceptions:
- EXC-20-001: Interfacing directly with third-party protocol payloads that have not yet undergone validation into domain entities.

## Rationale

- Strongly typed domain identifiers from mail_domain enforce compile-time safety and prevent accidental substitution of unrelated identifiers across module boundaries.
- Centralizing identity primitives in mail_domain establishes a single source of truth for email entity identity semantics across both storage and core orchestration modules.
- Static analysis evidence across queue interfaces, draft handling, and message preparation confirms widespread reliance on mail_domain::MessageId across multiple crates.

## Consequences

Positive:
- Compile-time enforcement prevents identity transposition errors across queueing and storage retrieval operations.
- Uniform identity semantics are preserved across decoupled workspace crates.
- Domain logic remains isolated from primitive string manipulation and protocol-specific representation quirks.

Negative:
- Requires explicit wrapping and unwrapping conversions between raw strings and domain types at persistence and protocol boundaries.
- Changes to domain identity types in the shared library necessitate coordinated recompilation across all consuming workspace modules.

## Alternatives

- Use primitive string representations for message identifiers across all module contracts. (rejected)
  Rejected because: Primitive obsession increases susceptibility to parameter confusion, eliminates compile-time type safety, and scatters identifier validation across disparate crates.
  When valid: Lightweight single-file scripts or isolated prototypes where multi-crate modularity is absent.
- Define separate message identifier types independently within each consuming crate. (rejected)
  Rejected because: Redundant definitions create inter-crate conversion overhead and risk semantic divergence in identifier validation.
  When valid: Decoupled microservices operating across network boundaries without a shared workspace.

## Risks

- Excessive allocation or cloning overhead when wrapping or unwrapping domain identifiers in high-throughput hot paths.
  Mitigation: Ensure identifier types implement efficient borrowing semantics, pass references where possible, and support zero-cost transparent newtypes.
  Owner: Core Engineering Team
- Version drift or diamond dependency conflicts across workspace modules depending on mail_domain.
  Mitigation: Enforce unified workspace dependency resolution through repository lock artifacts.
  Owner: Release and Architecture Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Construct domain identifiers via their designated constructors or parsing functions at boundary ingress, propagating mail_domain::MessageId through all internal processing pipelines.
- Implement transparent newtype representations and reference-based lookups to minimize performance overhead during database queries and queue inspections.

## Continuation Context


Verify commands:
- Discover the workspace build harness and execute the complete test suite to confirm type checking and interface compatibility across all crates consuming the domain library.
- Discover the repository static analysis and linting configuration to verify that public contracts adhere to domain identifier typing standards without primitive fallback.

Accept when:
- All workspace crates compile cleanly with zero type errors when passing mail_domain::MessageId across module boundaries.
- Unit and integration test suites succeed, confirming that queueing and storage retrieval operations correctly round-trip domain identifiers.
- Static analysis checks pass with no unapproved primitive string identifier representations in public contract signatures.

## Enforcement

- Verified by: Continuous integration automated build and test pipeline executions.
- Verified by: Compiler type checking and workspace static analysis rules.
- Verified by: Peer code review for all interface changes crossing crate boundaries.
- Violation handling: Continuous integration blocks merge on compile errors or lint violations where raw strings are substituted for domain types.
- Violation handling: Pull requests violating domain typing conventions must be revised to adopt mail_domain types before merge.
- Exception process: Submit an exception request to the Architecture Review Board detailing the external protocol constraints requiring raw primitive representations.
- Exception process: If approved, record the exception in the relevant module documentation alongside an approved exception tracking identifier.