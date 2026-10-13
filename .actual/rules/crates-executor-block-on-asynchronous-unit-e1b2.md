# futures::executor::block_on for Asynchronous Unit Test Execution: Test Suites Not Initialize Persistent Background

These rules are ALWAYS ACTIVE for unit test modules verifying asynchronous core domain logic, state machines, and data access layers, and test routines validating export boundaries and foreign function interfaces intended for unmanaged host environments.

### Rules

- **R-TEST-001** MUST_NOT: Test suites MUST NOT initialize persistent background runtime reactors when testing pure state transitions, input validations, or transactional database writes.

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