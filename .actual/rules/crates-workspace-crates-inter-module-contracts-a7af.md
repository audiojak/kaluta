# Adopt mail_domain Shared Module for Core Domain Entities and Typed Identifiers: Workspace Crates Inter Module Contracts Adopt

These rules are ALWAYS ACTIVE for all workspace components defining or consuming email domain entities, identifiers, and session contexts, as well as inter-crate communication contracts, event payloads, and agent tool definitions.

### Rules

- **R-MAIL-001** MUST: All workspace crates and inter-module contracts MUST adopt the shared mail_domain module for representing core domain entities and identifiers rather than implementing duplicate or primitive types.

### Verify

```bash
# Discover and execute the project verification script across domain-dependent packages
project_verify=$(find . -maxdepth 2 -type f \( -name "*verify*" -o -name "*check*" \) -perm -111 | head -n 1)
[ -n "$project_verify" ] && "$project_verify" || true
# Verify all workspace references resolve through the authoritative dependency lock configuration
find . -maxdepth 3 -type f \( -name "*lock*" -o -name "*manifest*" \)
```

**Accept when:**
- All unit and integration test suites pass across workspace packages consuming the domain module.
- Static type analysis passes without primitive string substitutions for domain identifier types in public contracts.
- The dependency resolution artifact matches the declared manifest ranges and builds cleanly.

<enforcement>
Claude Code MUST NOT skip or defer verification. Violation handling: Merge requests that introduce primitive string identifiers for domain entities in public APIs are blocked until refactored. Compiler errors caused by mismatched domain identifier types prevent code integration.
</enforcement>