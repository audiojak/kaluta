# futures::executor::block_on for Asynchronous Unit Test Execution: When Testing Asynchronous Foreign Function Interface

These rules are ALWAYS ACTIVE for unit test modules verifying asynchronous core domain logic, state machines, data access layers, and foreign function interface boundaries intended for unmanaged host environments.

### Rules

- **R-AF-001** MUST: When testing asynchronous foreign function interface exports, test suites MUST verify execution safety by invoking futures::executor::block_on from unmanaged OS threads.

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