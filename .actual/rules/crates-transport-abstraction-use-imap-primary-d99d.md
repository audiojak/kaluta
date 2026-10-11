# Dual-transport abstraction with IMAP primary and REST fallback: Transport Abstraction Use Imap Primary Channel

These rules are ALWAYS ACTIVE for all files matching the configured scope.

### Rules

- **R-TRAN-001** MUST: The `Transport` abstraction MUST use IMAP as the primary channel for bulk message backfill, folder operations (Spam, Trash, Drafts), and IDLE push notifications.

### Verify

```bash
# Discover and run the project's integration and unit test suite targeting transport boundary contracts and fallback behavior.
cargo test --test transport_tests

# Discover and execute architectural or module dependency linters to ensure no direct provider client usage exists outside transport modules.
cargo check --workspace
```

**Accept when:**
- All email sync network operations are routed strictly through the `Transport` interface.
- Bulk backfill and push notification workflows use IMAP while gracefully falling back to REST upon induced IMAP failures.
- Circuit breaker state changes are observable and testable upon IMAP connection failures.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>