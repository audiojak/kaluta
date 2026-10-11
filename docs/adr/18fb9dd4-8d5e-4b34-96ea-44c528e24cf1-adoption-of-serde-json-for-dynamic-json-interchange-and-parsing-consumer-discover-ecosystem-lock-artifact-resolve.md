# Adoption of serde_json for Dynamic JSON Interchange and Parsing: Consumer Discover Ecosystem Lock Artifact Resolve

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- The system coordinates interactions across external agent transcripts, streaming event protocols, and dynamic guide definition interchange formats.
- Incoming payloads exhibit polymorphic structures, optional fields, and variable schemas that require resilient deserialization without breaking client workflows.
- A unified serialization library is required across core modules to prevent fragmented ad-hoc parsing logic and inconsistent error handling.

## Problem Statement

Core components and agent integrations must process variable, streaming, and partially structured JSON payloads from external sources. Without a unified dynamic data representation strategy, modules risk duplicating parsing logic, failing on unexpected fields, or enforcing brittle compile-time schemas on evolving agent protocols.

## Decision

1. MUST: The consumer MUST discover the ecosystem lock artifact and resolve the exact locked version of the library before introducing or modifying serialization calls.

## Policy Block

- MUST The consumer MUST discover the ecosystem lock artifact and resolve the exact locked version of the library before introducing or modifying serialization calls.

In scope:
- Serialization and deserialization of external agent communication transcripts, streams, and tool call parameters.
- Parsing and transformation of semi-structured guide import and change documents.

Out of scope:
- Internal domain structures with static compile-time schemas that do not interface with external JSON streams.
- Binary protocols or non-JSON data transport layers.

## Rationale

- Adopting serde_json provides a standard mechanism for both strongly typed data structures and flexible untyped representations via serde_json::Value.
- The library is consistently present across guide management and streaming agent processing components, demonstrating an established architectural precedent.
- Dynamic document handling decouples stream decoders and merge planners from strict external schema changes while maintaining deterministic error reporting.

## Consequences

Positive:
- Enables resilient ingestion of dynamic and evolving JSON structures from external agent interactions.
- Eliminates redundant parsing implementations across distinct core and agent integration modules.
- Facilitates concise summary generation and streaming event mapping from unstructured inputs.

Negative:
- Shifts field existence and type validation from compile-time checking to explicit runtime evaluation.
- Materializing full dynamic value trees incurs memory and allocation overhead compared to zero-allocation parsers.

## Alternatives

- Strict Static Schema Deserialization for All Ingestion Paths (rejected)
  Rejected because: Predefined static schemas fail when incoming agent payloads or external guide formats contain unpredictable or evolving intermediate fields, requiring excessive maintenance overhead.
  When valid: Valid when protocol schemas are strictly frozen, fully specified, and controlled entirely by internal contracts.
- Custom String Slicing and Ad-Hoc Parsers for Transcripts and Guide Payloads (rejected)
  Rejected because: Custom tokenization introduces parsing edge-case defects, brittle escape handling, and redundant maintenance burdens across domain components.
  When valid: Valid only in constrained environments with strict zero-allocation boundaries that prohibit general-purpose serialization libraries.

## Risks

- Dynamic JSON manipulation using untyped structures can lead to runtime parse errors if expected keys are absent.
  Mitigation: Enforce pattern matching and explicit error propagation on all dynamic value extractions, accompanied by comprehensive unit testing.
  Owner: Core Engineering Team
- Deeply nested or excessively large JSON payloads could cause high memory allocations during value tree materialization.
  Mitigation: Impose payload size limits at boundary interfaces and generate compact single-line summaries for large nested objects.
  Owner: Core Engineering Team

## Implementation Notes

- DISCOVERY POLICY (MANDATORY): This ADR omits all tool names, file names, commands, package managers, and version numbers. The consumer MUST derive them from the project repository.

LOCK-VERSION GROUNDING (MANDATORY) — before writing code that uses a versioned library, execute in order:
1. Find the dependency manifest in the repo. It declares ranges, not installed versions.
2. Identify the build tool from the manifest.
3. Inspect the repository lock or resolution artifact to determine the exact resolved version. This artifact is authoritative; build-tool output only verifies the active environment matches it.
4. Look up the official documentation, changelog, or public API reference for that exact version. Do not use training-data recall — fetch or search the public internet for version-specific docs.
5. Confirm every API, class, or function you will call exists in that exact version's documentation before using it.
6. For version-sensitive behavior, re-run steps 3-5 per dependency at point of use.
- Apply defensive accessors and pattern matching when traversing dynamic value objects to safely handle missing or mismatched types.
- Provide concise single-line summarization when converting multi-line or expansive payload structures into audit and event streams.

## Continuation Context


Verify commands:
- Discover the project test runner configuration and execute the unit test suite governing core guide transformations and agent stream decoding.
- Locate and run the static analysis and linter checks to verify correct error handling on JSON parsing routines.

Accept when:
- All unit test suites verifying guide change proposal, guide merge planning, and streaming agent event decoding execute successfully.
- Dynamic JSON payload decoders handle malformed input structures gracefully by emitting typed errors without runtime panics.

## Enforcement

- Verified by: Automated continuous integration test suites verifying serialization, merge planning, and stream decoding fixtures.
- Verified by: Peer code review auditing error propagation and dynamic value traversal across boundary interfaces.
- Violation handling: Pull requests introducing ad-hoc JSON parsers or unhandled unwraps on dynamic data trees will be blocked until corrected.
- Exception process: Deviations requiring alternate serialization mechanisms require architectural review and formal approval documented in an updated decision record.