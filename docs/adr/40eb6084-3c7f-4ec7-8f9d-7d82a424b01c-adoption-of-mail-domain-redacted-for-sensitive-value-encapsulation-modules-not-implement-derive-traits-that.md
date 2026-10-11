# Adoption of mail_domain::Redacted for Sensitive Value Encapsulation: Modules Not Implement Derive Traits That

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Authentication flows and provider integrations handle sensitive credentials such as access tokens, refresh tokens, client secrets, and authorization codes.
- Unchecked propagation of raw string primitives creates risks of credential exposure in diagnostic logging, traces, and serialized error payloads.
- The codebase establishes a shared module providing dedicated encapsulation types to control access to sensitive values.

## Problem Statement

Handling secret values and authentication tokens as standard string primitives risks unintentional credential exposure through logging, error reporting, and serialized structures. An architectural boundary constraint is required to enforce intentional value extraction only at network or persistence edges.

## Decision

1. MUST_NOT: Modules MUST_NOT implement or derive traits that expose the inner value of mail_domain::Redacted instances in telemetry, standard error output, or persistent debug traces.

## Policy Block

- MUST_NOT Modules MUST_NOT implement or derive traits that expose the inner value of mail_domain::Redacted instances in telemetry, standard error output, or persistent debug traces.

In scope:
- Module boundaries handling authentication credentials, tokens, or cryptographic verifiers
- Public contracts defined across core engine and provider interface crates

Out of scope:
- Non-sensitive domain identifiers such as public account names and non-secret message identifiers
- Internal transport protocol payloads that do not contain authorization material

## Rationale

- Encapsulating secrets in mail_domain::Redacted prevents inadvertent inspection or formatting by requiring explicit invocation of expose.
- Unifying credential handling under a shared domain module eliminates fragmented sanitization approaches across separate provider implementations.
- Enforcing wrapper types at public API boundaries establishes compile-time safety against accidental disclosure in logging and tracing subscribers.

## Consequences

Positive:
- Eliminates accidental disclosure of authentication tokens in log records and diagnostic telemetry.
- Clarifies security expectations across provider integration contracts through explicit wrapper types.
- Establishes a single auditable unwrap pattern at external network boundaries.

Negative:
- Requires explicit ceremony to wrap and unwrap values at transport edges.
- Creates an internal dependency on the shared domain module across provider crates.

## Alternatives

- Plain Primitive Strings with Logging Sanitizers (rejected)
  Rejected because: Sanitizers rely on error-prone heuristic filtering and do not provide compile-time guarantees against credential disclosure.
  When valid: Valid only in rapid prototypes with no persistent logging or external provider integrations.
- Independent Secret Types per Integration Crate (rejected)
  Rejected because: Per-crate custom secret wrappers duplicate behavior and prevent consistent contract definitions across core interfaces.
  When valid: Valid in decoupled multi-repository projects where shared domain crates cannot be referenced.

## Risks

- Premature or excessive invocation of expose in application code could bypass encapsulation protection.
  Mitigation: Enforce code review checks and static search directives preventing expose outside of authorized transport adapters.
  Owner: engineering team
- Serialization implementations could attempt to bypass encapsulation to persist credentials.
  Mitigation: Restrict serialization traits on credential structures and require explicit transformation routines.
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
- Expose calls must remain localized to transport adapters during outbound network transmission or credential persistence.
- Constructors and contract factories must accept mail_domain::Redacted directly instead of unwrapped primitives to avoid accidental early disclosure.

## Continuation Context


Verify commands:
- Discover the project test runner and execute the automated test suite covering secret storage and authentication workflows.
- Run repository static analysis tools to verify that credential structures wrap sensitive fields within mail_domain::Redacted.
- Execute repository search directives to verify that invocations of expose remain confined to transport adapters.

Accept when:
- All unit and integration tests across core engine and provider modules pass without errors.
- Public interfaces accepting tokens or secrets strictly require mail_domain::Redacted rather than primitive strings.
- No unauthorized unwrap calls are detected outside designated transport adapters.

## Enforcement

- Verified by: Automated continuous integration checks executing repository test suites.
- Verified by: Peer code review verification of new provider contracts and secret access points.
- Violation handling: Pull requests passing raw secret strings across contract boundaries are blocked until updated with wrapper types.
- Violation handling: Unauthorized expose calls outside transport layers are flagged for refactoring.
- Exception process: Exceptions require documented architecture review and explicit sign-off from the core engineering team.