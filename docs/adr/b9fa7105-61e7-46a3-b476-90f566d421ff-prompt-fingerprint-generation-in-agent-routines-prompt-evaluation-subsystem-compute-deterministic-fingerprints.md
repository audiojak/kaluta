# Prompt Fingerprint Generation in Agent Routines: Prompt Evaluation Subsystem Compute Deterministic Fingerprints

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- The routine subsystem compiles prompts across multiple execution runner targets and dry-run modes.
- Prompt determinism and drift detection require a compact comparable identifier to verify that prompt contents remain stable unless intentionally modified.
- An eight-byte sequence formatted into a sixteen-character hexadecimal string was implemented within prompt routine tests to validate fingerprint consistency.
- The pattern was detected under the data protection taxonomy facet although its operational scope is localized prompt regression verification rather than cryptographic security.

## Problem Statement

Prompt generation across diverse execution runners requires verification of deterministic output and drift detection without incurring high runtime overhead or complex cryptographic dependencies. However, localized byte-formatting implementations can be mistaken for robust cryptographic data protection mechanisms if architectural boundaries and guarantees are not explicitly defined.

## Decision

1. MUST: The prompt evaluation subsystem MUST compute deterministic prompt fingerprints using the public prompt_fingerprint function to establish digest identity across prompt generation cycles.

## Policy Block

- MUST The prompt evaluation subsystem MUST compute deterministic prompt fingerprints using the public prompt_fingerprint function to establish digest identity across prompt generation cycles.

In scope:
- Prompt generation and evaluation routines within the agent API component.
- Deterministic verification of prompt template outputs and test snapshot harnesses.

Out of scope:
- Cryptographic hashing, data encryption at rest, and secret masking operations.
- Cross-service authentication tokens and tamper-resistant audit logs.

## Rationale

- The sixteen-character hexadecimal fingerprint provides a minimal fixed-length identifier suitable for asserting prompt consistency across target configurations.
- Limiting the scope of the fingerprinting function to routine prompt verification prevents conflating structural change detection with cryptographic data protection.
- Retaining deterministic fingerprint assertions ensures unintended prompt mutations are caught during automated test validation.

## Consequences

Positive:
- Provides deterministic lightweight verification of prompt output changes across execution runners.
- Avoids introducing external cryptographic dependencies for simple prompt drift checking.
- Clarifies that localized byte formatting is intended solely for prompt identification rather than secure data protection.

Negative:
- Sixteen-character digests offer low entropy and provide no defense against deliberate preimage attacks or collisions.
- Engineers may mistakenly assume the fingerprinting logic satisfies compliance or cryptographic data protection requirements.

## Alternatives

- Adopt a cryptographic hashing library for prompt fingerprinting (rejected)
  Rejected because: Cryptographic hashing introduces unnecessary dependency complexity and runtime overhead for internal prompt stability verification
  When valid: When prompt fingerprints are used as tamper-evident integrity seals across untrusted network boundaries
- Compare full raw prompt text strings instead of fingerprints (rejected)
  Rejected because: Full string comparisons in test logs and telemetry consume excessive memory and obscure concise differential identification
  When valid: When full diffs are required for interactive debugging or snapshot approvals

## Risks

- Developers might treat prompt_fingerprint as a secure digest for sensitive payload integrity
  Mitigation: Document that the fingerprinting routine is restricted to internal prompt identification and fails cryptographic security standards
  Owner: engineering team
- Digest collisions occurring between distinct prompt templates due to the short sixteen-character hexadecimal length
  Mitigation: Supplement fingerprint equality checks with full snapshot and safety rule assertions in test suites
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
- Ensure prompt fingerprint generation logic remains purely deterministic and isolated from external runtime state such as timestamps or random seeds.
- Validate that safety rules and bucket descriptions are included in prompt inputs prior to computing verification fingerprints.

## Continuation Context


Verify commands:
- Discover the project test runner configuration and execute the unit test suite targeting prompt routine verification.
- Discover and run the snapshot review suite to confirm prompt outputs match approved baselines across all runner targets.

Accept when:
- All prompt generation tests pass and assert equal fingerprints for identical routine inputs.
- Modified routine descriptions produce distinct fingerprint values of exactly sixteen hexadecimal characters.

## Enforcement

- Verified by: Automated unit test execution in continuous integration pipelines.
- Verified by: Peer code review for any modifications to prompt routine contracts or fingerprint formatting.
- Violation handling: Pull requests altering fingerprint format or breaking determinism assertions are rejected automatically by continuous integration.
- Exception process: Architectural review and written approval from security and engineering leads before using the fingerprint function in security-sensitive contexts.