# Dual-transport abstraction with IMAP primary and REST fallback: Fallback Imap Rest Endpoints Mediated Automatic

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-TRANSPORT-001** MUST: Fallback from IMAP to REST endpoints MUST be mediated by an automatic circuit breaker rather than unmanaged immediate retries.

### Verify

```bash
cargo test --package mail-sync --lib transport
cargo test --package provider-gmail --lib imap
```

**Accept when:**
- All email sync network operations are routed strictly through the `Transport` interface.
- Bulk backfill and push notification workflows use IMAP while gracefully falling back to REST upon induced IMAP failures.
- Circuit breaker state changes are observable and testable upon IMAP connection failures.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>