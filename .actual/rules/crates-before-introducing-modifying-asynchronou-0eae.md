# async_trait Adoption for Asynchronous Trait Contracts: Before Introducing Modifying Asynchronous Trait Definitions

These rules are ALWAYS ACTIVE for all public asynchronous trait definitions across agent provider crates, concrete implementations of asynchronous provider and session lifecycle traits, and test fixtures/fake provider implementations implementing asynchronous provider traits.

### Rules

- **R-ASYNC-001** MUST: Before introducing or modifying asynchronous trait definitions relying on external macros, developers MUST discover the workspace dependency manifest and verify the exact resolved version recorded in the repository lock artifact against official documentation.
- **R-ASYNC-002** MUST: Apply the attribute macro consistently to both the asynchronous trait declaration and every corresponding trait implementation block.
- **R-ASYNC-003** MUST: Ensure that future return types adhere to thread-safe concurrency bounds required by provider runtimes when dynamic dispatch is utilized across execution contexts.

### Verify

```bash
# Discover workspace dependency manifest, verify lock artifact, and compile code to validate macro expansion
cargo check --workspace --all-targets

# Execute test suite across provider and session crates
cargo test --workspace
```

**Accept when:**
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

<enforcement>
Claude Code MUST NOT skip or defer verification. All asynchronous trait changes must be verified against workspace compilation and test suites.
</enforcement>