# Adoption of mail_domain Module for Core Domain Primitives: Persistence Operations Consuming Mail Domain Types

These rules are ALWAYS ACTIVE for all persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling, as well as storage contracts that manipulate message identifiers or millisecond-level timestamps.

### Rules

- **R-MAIL-001** SHOULD: Persistence operations consuming mail_domain types validate or propagate domain representations through standard store error handling abstractions.

### Verify

```bash
# Discover the repository build tool from the project manifest and execute the test suite across all persistence modules.
# Discover and run the project code linter and type checker to verify that all storage interfaces adhere to mail_domain type constraints.
```

**Accept when:**
- All persistence module test suites execute successfully without compilation errors or type mismatches.
- No public storage interface accepts or returns raw unvalidated strings or integer primitives where mail_domain domain types are required.

<enforcement>
Claude Code MUST NOT skip or defer verification. Verified by automated continuous integration build checks enforcing strict type checking across workspace boundaries and peer code review confirming that new persistence interfaces utilize mail_domain primitives.
</enforcement>