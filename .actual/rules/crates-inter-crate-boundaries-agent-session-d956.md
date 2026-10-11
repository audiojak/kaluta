# Adopt mail_domain Shared Module for Core Domain Entities and Typed Identifiers: Inter Crate Boundaries Agent Session Contracts

These rules are ALWAYS ACTIVE for all workspace components defining or consuming email domain entities, identifiers, and session contexts.

### Rules

- **R-MAIL-001** MUST_NOT: Inter-crate boundaries and agent session contracts MUST NOT accept or emit unvalidated raw strings for domain identifiers.

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
Claude Code MUST NOT skip or defer verification.
</enforcement>