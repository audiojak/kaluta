# Dual-transport abstraction with IMAP primary and REST fallback: Synchronizer Network Calls Dispatch Through Unified

These rules are ALWAYS ACTIVE for all email synchronization network operations, specifically targeting files within `crates/mail-sync/`, `crates/provider-api/`, and `crates/provider-gmail/`.

### Rules

- **R-SYNC-001** MUST: All synchronizer network calls MUST dispatch through the unified `Transport` abstraction rather than invoking REST or IMAP provider APIs directly.

### Verify

```bash
# Discover and run the project's integration and unit test suite targeting transport boundary contracts and fallback behavior.
cargo test --package mail-sync --package provider-gmail

# Discover and execute architectural or module dependency linters to ensure no direct provider client usage exists outside transport modules.
cargo check --all-targets
```

**Accept when:**
- All email sync network operations are routed strictly through the `Transport` interface.
- Bulk backfill and push notification workflows use IMAP while gracefully falling back to REST upon induced IMAP failures.
- Circuit breaker state changes are observable and testable upon IMAP connection failures.

<enforcement>
Claude Code MUST NOT skip or defer verification. All synchronizer network changes must be statically checked and tested against the unified transport boundary contract.
</enforcement>