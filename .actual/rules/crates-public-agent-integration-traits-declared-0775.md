# agent-api Provider Contract Interface Adoption: Public Agent Integration Traits Declared Async

These rules are ALWAYS ACTIVE for development of external agent provider integrations, workspace packages defining or consuming agent process lifecycle contracts, and asynchronous agent session orchestration and approval dispatch.

### Rules

- **R-AGNT-001** MUST: Public agent integration traits MUST be declared using the async_trait macro and shared via Arc references across asynchronous tasks to ensure thread-safe lifecycle and session execution.

### Verify

```bash
# Discover and run the workspace test suite targeting agent contract packages and provider integrations.
# Execute static analysis and contract verification checks defined in the repository workflow configuration.
cargo test --package agent-api
cargo test --workspace --tests
```

**Accept when:**
- All provider implementations satisfy agent-api trait contracts and pass contract compliance tests.
- Fake provider unit tests confirm expected event dispatch and status responses without external network or subprocess calls.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by continuous integration test passes verifying contract implementations across workspace packages and architectural code review.
</enforcement>