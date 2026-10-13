# futures::executor::block_on for Asynchronous Unit Test Execution: Engineers Discover Project Dependency Manifest Authoritative

These rules are ALWAYS ACTIVE for all unit test modules verifying asynchronous core domain logic, state machines, data access layers, and test routines validating export boundaries and foreign function interfaces intended for unmanaged host environments.

### Rules

- **R-FU-001** MUST: Engineers MUST discover the project dependency manifest and authoritative lock artifact to verify the exact resolved version of the futures library before implementing unit tests with its executor.

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