# Adopt mail_domain Shared Module for Core Domain Entities and Typed Identifiers: Developers Inspect Project Dependency Manifest Authoritative

These rules are ALWAYS ACTIVE for all workspace components defining or consuming email domain entities, identifiers, and session contexts, inter-crate communication contracts, event payloads, and agent tool definitions.

### Rules

- **R-MAIL-001** MUST: Developers MUST inspect the project dependency manifest and authoritative repository lock artifact to determine the resolved dependency version before writing code that integrates with the module.
- **R-MAIL-002** MUST: Construct domain identifiers exclusively via provided constructors such as ThreadId::new or MessageId::new to ensure proper initialization.
- **R-MAIL-003** MUST: Derive serialization and standard collection traits on domain types to facilitate hashing, ordering, and event payload transport across boundaries.

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
Claude Code MUST NOT skip or defer verification. Verified by automated workspace build checks, unit test execution, and peer code reviews.
</enforcement>