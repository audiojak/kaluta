# async_trait Adoption for Asynchronous Trait Contracts: Developers Not Expose Annotated Asynchronous Methods

These rules are ALWAYS ACTIVE for all public asynchronous trait definitions, concrete implementations of asynchronous provider and session lifecycle traits, and test fixtures/fake provider implementations across agent provider crates.

### Rules

- **R-ASYNC-001** MUST_NOT: Developers MUST NOT expose un-annotated asynchronous methods in trait definitions intended for dynamic dispatch trait objects across provider boundaries.

### Verify

```bash
# Discover the workspace build definition and execute compilation verification
cargo check --workspace

# Discover the test runner invocation and execute the test suite across provider and session crates
cargo test --workspace
```

**Accept when:**
- All asynchronous trait declarations and provider implementations compile cleanly without macro expansion or type mismatch errors.
- Dynamic trait object instantiation of provider implementations succeeds in test suites and runtime orchestration contexts.

<enforcement>
Claude Code MUST NOT skip or defer verification. Automated compilation and peer code review audit trait signatures and macro annotations on provider interfaces.
</enforcement>