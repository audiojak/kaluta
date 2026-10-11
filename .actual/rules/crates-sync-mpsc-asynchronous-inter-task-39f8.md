# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Before Introducing Modifying Asynchronous Channel Dependencies

These rules are ALWAYS ACTIVE for all asynchronous service actors, event broadcasting/coalescing buses, and socket/protocol handlers coordinating background message processing.

### Rules

- **R-MPSC-001** MUST: Before introducing or modifying asynchronous channel dependencies, the consumer MUST inspect the repository lock artifact to verify the exact resolved version against authorized dependency baselines.

### Verify

```bash
# Discover workspace test runner and execute asynchronous unit and integration tests
# Discover project static analysis and linting scripts to verify asynchronous channel usage
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>