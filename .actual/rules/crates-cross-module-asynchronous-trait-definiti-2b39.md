# async_trait Adoption for Asynchronous Trait Contracts: Cross Module Asynchronous Trait Definitions Their

These rules are ALWAYS ACTIVE for all public asynchronous trait definitions, provider implementations, test fixtures, and session lifecycle traits across agent provider crates intended for dynamic dispatch.

### Rules

- **R-ASYNC-001** MUST: All cross-module asynchronous trait definitions and their concrete provider implementations intended for dynamic dispatch MUST be annotated with the async_trait::async_trait macro.
- **R-ASYNC-002** MUST: Apply the attribute macro consistently to both the asynchronous trait declaration and every corresponding trait implementation block.
- **R-ASYNC-003** MUST: Ensure that future return types adhere to thread-safe concurrency bounds required by provider runtimes when dynamic dispatch is utilized across execution contexts.

### Verify

```bash
# Discover the workspace build definition and execute compilation verification
cargo check --workspace
# Discover the test runner invocation and execute the test suite across provider and session crates
cargo test
```

**Accept when:**
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated compilation and test execution pipelines across workspace crates and peer code reviews.
</enforcement>