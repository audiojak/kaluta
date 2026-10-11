# Adoption of openagc-core Modular Domain Orchestration Library: Concurrent Multi Account Operations Execute Within

These rules are ALWAYS ACTIVE for all domain services, account synchronization engines, background routine executors, agent interaction coordinators, and internal modules requiring access to account-scoped data stores and transactional persistence.

### Rules

- **R-OPENAGC-001** MUST: Concurrent multi-account operations MUST execute within scoped context boundaries to enforce account isolation and prevent cross-account data leakage across asynchronous tasks.

### Verify

```bash
# Discover the workspace test runner and static analysis from the project repository configuration and execute them:
# 1. Run unit and integration test suites validating core engine operations.
# 2. Run static analysis verification to confirm compliance with transactional read-write closures.
# 3. Run multi-account concurrency integration tests to verify account scoping and event isolation.
```

**Accept when:**
- All unit and integration tests covering the core coordinator, account registry, routines, and agent approvals pass without error.
- Static analysis confirms no direct database store access bypasses the core transactional closures.
- Concurrent multi-account test executions demonstrate zero cross-account data leakage and verified event attribution.

<enforcement>
Claude Code MUST NOT skip or defer verification. Code changes bypassing the core coordinator or omitting transactional closures must be rejected.
</enforcement>