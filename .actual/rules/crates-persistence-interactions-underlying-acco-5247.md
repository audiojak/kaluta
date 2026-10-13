# Adoption of openagc-core Modular Domain Orchestration Library: Persistence Interactions Underlying Account Stores Execute

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, and agent interaction coordinators.

### Rules

- **R-COR-001** MUST: All persistence interactions with underlying account stores MUST execute within explicit transactional read closures or transactional write closures coordinated through the core engine rather than through direct uncoordinated store access.

### Verify

```bash
# Discover and run the workspace test runner for unit and integration test suites validating core engine operations.
# Discover and run repository static analysis script verifying transactional read-write closures.
# Discover and run multi-account concurrency integration tests to verify account scoping and event isolation.
```

**Accept when:**
- All unit and integration tests covering the core coordinator, account registry, routines, and agent approvals pass without error.
- Static analysis confirms no direct database store access bypasses the core transactional closures.
- Concurrent multi-account test executions demonstrate zero cross-account data leakage and verified event attribution.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration test suites and architectural peer code reviews.
</enforcement>