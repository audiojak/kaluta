# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Channel Senders Not Hold Cross Task

These rules are ALWAYS ACTIVE for asynchronous services, event broadcasting and coalescing buses, and socket and protocol handlers coordinating background message processing.

### Rules

- **R-MPSC-001** SHOULD_NOT: Channel senders SHOULD_NOT hold cross-task synchronization locks across channel dispatch points to prevent task deadlocks.

### Verify

```bash
# Discover the workspace test runner and execute all asynchronous unit and integration tests covering channel communication and event coalescing.
# Discover the project static analysis and linting scripts to verify asynchronous channel usage complies with concurrency guidelines.
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
```

**Accept when:**
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration test runs executing concurrency test suites and static analysis checks, as well as mandatory peer code reviews validating channel lifecycle handling and absence of cross-task mutex locks.
</enforcement>