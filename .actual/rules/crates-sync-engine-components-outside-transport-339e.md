# Dual-transport abstraction with IMAP primary and REST fallback: Sync Engine Components Outside Transport Boundary

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-SYNC-001** MUST_NOT: Sync engine components outside the transport boundary MUST NOT hold or manipulate protocol connection state directly.

### Verify

```bash
# Discover and run the project's integration and unit test suite targeting transport boundary contracts and fallback behavior.
cargo test --package mail-sync --lib --tests

# Discover and execute architectural or module dependency linters to ensure no direct provider client usage exists outside transport modules.
cargo check --workspace --all-targets
```

**Accept when:**
- All email sync network operations are routed strictly through the `Transport` interface.
- Bulk backfill and push notification workflows use IMAP while gracefully falling back to REST upon induced IMAP failures.
- Circuit breaker state changes are observable and testable upon IMAP connection failures.

<enforcement>
Claude Code MUST NOT skip or defer verification. All email sync components must strictly adhere to the transport boundary.
</enforcement>