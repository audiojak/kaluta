# futures::executor::block_on for Asynchronous Unit Test Execution: Unit Test Fixtures Reset Temporary Storage

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-BLKI-001** SHOULD: Unit test fixtures SHOULD reset temporary storage state synchronously before invoking futures::executor::block_on against freshly initialized core instances.

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