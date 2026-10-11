# Adoption of tokio::sync::mpsc for Asynchronous Inter-Task Channel Messaging: Event Dispatchers Coalesce Merge High Frequency

These rules are ALWAYS ACTIVE for all asynchronous service actors, event broadcasting and coalescing buses, and socket/protocol handlers coordinating background message processing.

### Rules

- **R-MPSC-001** MAY: Event dispatchers MAY coalesce or merge high-frequency messages within time windows before queuing them into consumer channels to minimize transmission overhead.

### Verify

```bash
# Discover workspace test runner and execute asynchronous unit/integration tests
# Discover project static analysis and linting scripts to verify asynchronous channel usage
```

**Accept when:**
- All asynchronous test suites pass with zero concurrency panics, task deadlocks, or leaked channel handles.
- Static analysis and concurrency checks complete without warnings or lint violations regarding channel error handling.

<enforcement>
Claude Code MUST NOT skip or defer verification. All pull requests introducing uncoordinated shared-memory locking across tasks or unbounded channel proliferation will be blocked from merging.
</enforcement>