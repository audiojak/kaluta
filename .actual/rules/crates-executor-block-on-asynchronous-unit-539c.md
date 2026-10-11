# futures::executor::block_on for Asynchronous Unit Test Execution: Engineers Use Dedicated Runtime Test Macros

These rules are ALWAYS ACTIVE for unit test modules verifying asynchronous core domain logic, state machines, and data access layers.

### Rules

- **R-39-001** MAY: Engineers MAY use dedicated runtime test macros only when the test subject explicitly requires asynchronous timers, physical process supervision, or cross-task channels that cannot advance on a single-threaded executor.

### Verify

```bash
# Discover repository test manifests and execute unit tests utilizing the synchronous executor pattern
$REPO_TEST_DISCOVERED --unit
```

**Accept when:**
- All unit tests using futures::executor::block_on complete successfully with zero assertion failures or deadlocks.
- Asynchronous core routines successfully execute and return expected domain values without requiring an ambient reactor.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verification is mandatory via automated continuous integration test execution gates and peer code review.
</enforcement>