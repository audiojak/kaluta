# async_trait Adoption for Asynchronous Trait Contracts: Purely Synchronous Trait Definitions Private Asynchronous

These rules are ALWAYS ACTIVE for all public asynchronous trait definitions, concrete implementations, and test fixtures across agent provider and session crates.

### Rules

- **R-ASYNC-001** MAY: Purely synchronous trait definitions or private asynchronous traits restricted strictly to static dispatch within a single internal module MAY omit the async_trait::async_trait macro.
- **R-ASYNC-002** MUST: Apply the attribute macro consistently to both the asynchronous trait declaration and every corresponding trait implementation block when dynamic dispatch is required.
- **R-ASYNC-003** MUST: Ensure that future return types adhere to thread-safe concurrency bounds required by provider runtimes when dynamic dispatch is utilized across execution contexts.
- **R-ASYNC-004** MUST: Execute the workspace lock-version grounding process to identify resolved versions prior to using versioned dependency APIs.

### Verify

```bash
# Discover workspace build definition and execute standard compilation verification
cargo check --workspace --all-targets

# Discover test runner invocation from project manifest and execute test suite
cargo test --workspace
```

**Accept when:**
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated compilation, test execution pipelines, and peer code reviews enforce compliance.
</enforcement>