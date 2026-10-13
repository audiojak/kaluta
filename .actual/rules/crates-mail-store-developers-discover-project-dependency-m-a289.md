# Adoption of mail_domain Module for Core Domain Primitives: Developers Discover Project Dependency Manifest Resolve

These rules are ALWAYS ACTIVE for all persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling that manipulate message identifiers or millisecond-level timestamps.

### Rules

- **R-MAIL-001** MUST: Developers MUST discover the project dependency manifest and resolve the authoritative lock artifact before introducing or updating dependencies on mail_domain to guarantee API consistency.

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