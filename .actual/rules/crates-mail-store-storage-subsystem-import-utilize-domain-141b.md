# Adoption of mail_domain Module for Core Domain Primitives: Storage Subsystem Import Utilize Domain Primitives

These rules are ALWAYS ACTIVE for all persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling.

### Rules

- **R-MAIL-STORE-001** MUST: The storage subsystem MUST import and utilize domain primitives from mail_domain for all entity identifiers and timestamp representations in public contracts rather than defining localized scalar primitives.

### Verify

```bash
# Discover the repository build tool from the project manifest and execute the test suite across all persistence modules.
# Discover and run the project code linter and type checker to verify that all storage interfaces adhere to mail_domain type constraints.
```

**Accept when:**
- All persistence module test suites execute successfully without compilation errors or type mismatches.
- No public storage interface accepts or returns raw unvalidated strings or integer primitives where mail_domain domain types are required.

<enforcement>
Claude Code MUST NOT skip or defer verification. Violation handling: Pull requests introducing raw primitive identifiers or timestamps in public storage APIs are blocked until updated to mail_domain types.
</enforcement>