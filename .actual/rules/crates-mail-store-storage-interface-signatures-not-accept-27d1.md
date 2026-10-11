# Adoption of mail_domain Module for Core Domain Primitives: Storage Interface Signatures Not Accept Return

These rules are ALWAYS ACTIVE for all persistence, storage, and queue modules defining public interfaces for entity storage, retrieval, or routine scheduling.

### Rules

- **R-STOR-001** MUST_NOT: Storage interface signatures MUST_NOT accept or return raw unbounded strings or primitive integer values in place of designated mail_domain types for domain identity or domain epoch timestamps.

### Verify

```bash
# Discover the project build tool and run the test suite and linter across all persistence modules
# (Tool names omitted per discovery policy)
```

**Accept when:**
- All persistence module test suites execute successfully without compilation errors or type mismatches.
- No public storage interface accepts or returns raw unvalidated strings or integer primitives where mail_domain domain types are required.

<enforcement>
Claude Code MUST NOT skip or defer verification. All storage interfaces must strictly use mail_domain types for entity identifiers and timestamps.
</enforcement>