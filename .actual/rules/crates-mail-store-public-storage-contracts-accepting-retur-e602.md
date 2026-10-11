# Adoption of mail_domain Module for Core Domain Primitives: Public Storage Contracts Accepting Returning Millisecond

These rules are ALWAYS ACTIVE for all persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling.

### Rules

- **R-MAIL-001** MUST: Public storage contracts accepting or returning millisecond-based temporal timestamps MUST declare parameter and return types as mail_domain::Millis.

### Verify

```bash
# Discover the repository build tool from the project manifest and execute the test suite across all persistence modules.
# Discover and run the project code linter and type checker to verify that all storage interfaces adhere to mail_domain type constraints.
```

**Accept when:**
- All persistence module test suites execute successfully without compilation errors or type mismatches.
- No public storage interface accepts or returns raw unvalidated strings or integer primitives where mail_domain domain types are required.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>