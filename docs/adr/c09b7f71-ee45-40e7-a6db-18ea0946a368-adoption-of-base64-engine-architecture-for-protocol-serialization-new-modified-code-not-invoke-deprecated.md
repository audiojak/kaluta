# Adoption of base64 Engine Architecture for Protocol Serialization: New Modified Code Not Invoke Deprecated

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Email synchronization and provider integration require frequent conversions between raw binary payloads and ASCII-safe text representations.
- Different protocol specifications mandate distinct Base64 variants, including standard MIME Base64 with padding for message bodies and URL-safe Base64 without padding for authentication parameters and web tokens.
- Standardizing on a unified engine trait guarantees consistent encoding behavior, explicit configuration choices, and deterministic error handling across all workspace crates.

## Problem Statement

Disparate encoding requirements across email protocols and authentication tokens risk runtime corruption and protocol rejection if implicit or inconsistent Base64 formatting is used. A centralized architectural rule is required to ensure explicit engine selection and uniform trait-based execution across all modules.

## Decision

1. MUST_NOT: New or modified code MUST NOT invoke deprecated global encoding or decoding free functions that bypass the Engine trait.

## Policy Block

- MUST_NOT New or modified code MUST NOT invoke deprecated global encoding or decoding free functions that bypass the Engine trait.

In scope:
- All binary payload encoding and decoding for email body composition and transmission.
- IMAP protocol test harnesses and wire serialization utilities.
- OAuth identity token parsing, PKCE exchange generation, and authentication parameter formatting.

Out of scope:
- Standard UTF-8 string manipulation that does not involve binary-to-text radix transformation.
- Hexadecimal and non-base64 hash digest representations.

## Rationale

- Evidence across mail synchronization, provider serialization, and OAuth authentication demonstrates consistent reliance on base64::Engine and explicit engine instances.
- Email composition requires strict standard padding for RFC MIME compliance, whereas authentication flows require unpadded URL-safe characters for query strings and JWT segments.
- Standardizing on the base64 Engine trait ensures deterministic behavior across transport boundaries without relying on deprecated implicit interfaces.

## Consequences

Positive:
- Explicit separation of standard alphabet padding and URL-safe unpadded configurations prevents protocol-level serialization failures.
- Direct usage of the Engine trait enables predictable, allocation-conscious buffer management in data transfer workflows.
- Standardized encoding abstractions unify data transformations across independent protocol and service providers.

Negative:
- Developers must explicitly import the Engine trait alongside engine constant values, increasing boilerplate.
- Engine configuration mismatches between padded and unpadded variants must be carefully prevented via review and testing.

## Alternatives

- Legacy Free-Function Base64 Routines (rejected)
  Rejected because: Global free-function helpers rely on implicit defaults that obscure alphabet selection and padding rules, leading to protocol incompatibilities between standard MIME and URL-safe token contexts.
  When valid: Legacy modules pinned to historical releases where the Engine trait abstraction is unavailable.
- Custom In-House Base64 Transformations (rejected)
  Rejected because: Hand-rolled bit-shifting routines introduce security vulnerabilities, lack SIMD optimizations, and unnecessarily increase maintenance burden.
  When valid: Minimal bare-metal or standalone targets where external library linking is prohibited.

## Risks

- Accidental use of standard engine with padding in URL-safe web token contexts leading to token parsing rejections.
  Mitigation: Encapsulate URL-safe operations in dedicated domain helpers and mandate comprehensive unit tests verifying padding absence.
  Owner: Engineering Team
- Failure to import the Engine trait resulting in unresolved method compilation errors when calling encode or decode.
  Mitigation: Document trait import requirements in crate module guidelines and enforce via compiler diagnostics in continuous integration.
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
- Bring base64::Engine into module scope whenever calling encode or decode operations on engine instances.
- Encapsulate recurring engine operations within domain-specific wire helpers to ensure consistent error conversion and avoid repeated engine parameter declarations.

## Continuation Context


Verify commands:
- Discover the workspace build script and run static compilation to verify all base64 method invocations resolve against the Engine trait.
- Discover and run the workspace test runner to verify encoding and decoding round-trip fixtures across mail and provider suites.
- Discover and execute repository linting suites to ensure no prohibited free functions or unapproved encoding libraries are introduced.

Accept when:
- All binary-to-text and text-to-binary conversions in synchronization and provider modules compile cleanly using Engine trait methods.
- All unit and integration tests covering MIME header composition, IMAP body fetching, and OAuth token exchanges pass without assertion failures.
- Static analysis detects zero invocations of legacy or unapproved base64 free functions.

## Enforcement

- Verified by: Automated continuous integration checks compiling all workspace targets.
- Verified by: Automated unit and integration test suites validating encoding round-trips.
- Verified by: Peer code review verifying appropriate engine selection for new protocol interactions.
- Violation handling: Continuous integration builds fail upon detecting compiler warnings or deprecated base64 method invocations.
- Violation handling: Pull requests introducing unapproved encoding patterns or free functions are blocked until refactored to approved engine patterns.
- Exception process: Exceptions require documented justification submitted to the architecture review board detailing why standard or URL-safe engines are insufficient.
- Exception process: Approved exceptions must declare custom engine configurations and establish dedicated integration verification suites.