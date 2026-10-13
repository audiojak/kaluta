# async_trait Adoption for Asynchronous Trait Contracts: Test Doubles Fake Provider Implementations Apply

These rules are ALWAYS ACTIVE for all public asynchronous trait definitions, concrete implementations, test fixtures, and fake provider implementations across agent provider crates.

### Rules

- **R-ASYNC-001** SHOULD: Test doubles and fake provider implementations SHOULD apply async_trait::async_trait identical to production provider contracts to maintain behavioral parity under dynamic dispatch.

### Verify

```bash
# Discover the workspace build definition and execute the standard compilation verification directive to validate macro expansion.
cargo check --workspace --all-targets

# Discover the test runner invocation from the project manifest and execute the test suite across provider and session crates.
cargo test --workspace
```

**Accept when:**
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated compilation, test execution pipelines, and peer code review auditing trait signatures and macro annotations on provider interfaces.
</enforcement>