# futures::executor::block_on for Asynchronous Unit Test Execution: Unit Test Routines Invoke Asynchronous Domain

These rules are ALWAYS ACTIVE for all unit test modules verifying asynchronous core domain logic, state machines, and data access layers.

### Rules

- **R-EXEC-001** MUST: Unit test routines MUST invoke asynchronous domain functions via futures::executor::block_on on the current test thread rather than spawning ambient multithreaded runtime reactors.

### Verify

```bash
# Discover repository test manifests and execute unit tests utilizing the synchronous executor pattern
$REPO_TEST_DISCOVERED --unit
```

**Accept when:**
- All unit tests using futures::executor::block_on complete successfully with zero assertion failures or deadlocks.
- Asynchronous core routines successfully execute and return expected domain values without requiring an ambient reactor.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>