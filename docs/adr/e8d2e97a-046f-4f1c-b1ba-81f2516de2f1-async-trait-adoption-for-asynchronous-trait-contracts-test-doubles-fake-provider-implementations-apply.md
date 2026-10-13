# async_trait Adoption for Asynchronous Trait Contracts: Test Doubles Fake Provider Implementations Apply

Status: proposed
Date: 2025-05-18
Deciders: Detection Pipeline (automated)

## Context

- Multiple provider crates and session modules define polymorphic asynchronous capabilities that must be shared and dispatched across module boundaries.
- Rust trait definitions require specialized transformation to support dynamic dispatch and object safety for asynchronous routines across crate boundaries.
- The codebase exhibits recurring usage of the async_trait procedural macro across agent provider implementations, session handlers, and mock fixtures to standardize asynchronous interfaces.

## Problem Statement

Asynchronous interface contracts across agent provider implementations require dynamic dispatch and uniform polymorphic handling. Native asynchronous trait declarations without boxed transformation cannot be used as trait objects across module boundaries, risking inconsistent trait declarations, fractured concurrency abstractions, and incompatible mock implementations across provider crates.

## Decision

1. SHOULD: Test doubles and fake provider implementations SHOULD apply async_trait::async_trait identical to production provider contracts to maintain behavioral parity under dynamic dispatch.

## Policy Block

- SHOULD Test doubles and fake provider implementations SHOULD apply async_trait::async_trait identical to production provider contracts to maintain behavioral parity under dynamic dispatch.

In scope:
- All public asynchronous trait definitions across agent provider crates.
- All concrete implementations of asynchronous provider and session lifecycle traits.
- Test fixtures and fake provider implementations implementing asynchronous provider traits.

Out of scope:
- Internal module traits that only expose synchronous signatures.
- Traits evaluated exclusively through static dispatch and monomorphization within a single private scope.

Exceptions:
- EXC-20-001: A trait is strictly consumed via monomorphic generic parameters and never instantiated as a dynamic trait object.

## Rationale

- Annotating trait definitions with async_trait transforms asynchronous method return types into pinned, heap-allocated futures, enabling object-safe trait representations for polymorphic providers.
- Standardizing on a single macro pattern across provider implementations, session controllers, and test doubles ensures consistent interface ergonomics and simplifies dependency injection.
- Dynamic dispatch across provider crates decouples higher-level agent coordination logic from provider-specific runtime and transport implementations.

## Consequences

Positive:
- Provides uniform dynamic dispatch capabilities for asynchronous provider interfaces across crate boundaries.
- Enables straightforward mocking and substitution of agent providers during automated integration testing.
- Maintains compiler-enforced asynchronous contract compatibility across independently developed provider modules.

Negative:
- Introduces minor heap allocation and pointer indirection overhead for every invoked asynchronous trait method.
- Adds a procedural macro dependency that increases interface expansion overhead during compilation.
- Requires consistent macro annotation across all implementors to avoid contract divergence.

## Alternatives

- Manual return-type desugaring using pinned boxed dynamic futures (rejected)
  Rejected because: Manual signature expansion is verbose, error-prone, and clutters trait signatures across provider codebases.
  When valid: In minimal zero-dependency environments where procedural macro dependencies are strictly prohibited.
- Native asynchronous trait methods without macro transformation (rejected)
  Rejected because: Native asynchronous trait declarations without boxed transformation lack object safety for dynamic trait objects across provider boundaries.
  When valid: When dynamic dispatch is unnecessary and all consumers use static monomorphization.

## Risks

- Micro-allocation overhead from boxed future allocation in performance-critical invocation loops.
  Mitigation: Profile high-throughput execution paths and restrict dynamic trait boundaries to coarse-grained orchestration steps.
  Owner: engineering team
- Macro expansion errors or version mismatch across multi-crate workspace boundaries.
  Mitigation: Ensure workspace-wide dependency convergence on the resolved macro version recorded in the repository lock artifact.
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
- Apply the attribute macro consistently to both the asynchronous trait declaration and every corresponding trait implementation block.
- Ensure that future return types adhere to thread-safe concurrency bounds required by provider runtimes when dynamic dispatch is utilized across execution contexts.

## Continuation Context


Verify commands:
- Discover the workspace build definition and execute the standard compilation verification directive to validate macro expansion.
- Discover the test runner invocation from the project manifest and execute the test suite across provider and session crates.

Accept when:
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

## Enforcement

- Verified by: Automated compilation and test execution pipelines across workspace crates.
- Verified by: Peer code review auditing trait signatures and macro annotations on provider interfaces.
- Violation handling: Pull requests omitting required macro annotations on public asynchronous provider traits will fail automated compilation or be blocked during code review.
- Exception process: Submit an architectural review request detailing the performance necessity and static dispatch design that warrants omitting the macro.