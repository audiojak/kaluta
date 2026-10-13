# futures::executor::block_on for Asynchronous Unit Test Execution: Engineering Team Use Futures Executor Block

These rules are ALWAYS ACTIVE for all unit test modules verifying asynchronous core domain logic, state machines, and data access layers.

### Rules

- **R-EXEC-001** MUST: The engineering team MUST use futures::executor::block_on as the primary execution mechanism for driving asynchronous domain operations within unit tests.

### Verify

```bash
# Discover repository test manifests and execute unit tests utilizing the synchronous executor pattern
$REPO_TEST_DISCOVERED --unit
```

**Accept when:**
- All unit tests using futures::executor::block_on complete successfully with zero assertion failures or deadlocks.
- Asynchronous core routines successfully execute and return expected domain values without requiring an ambient reactor.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory.
</enforcement>