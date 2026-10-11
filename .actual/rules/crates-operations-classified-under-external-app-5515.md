# Risk-classified tool gating for external agent execution: Operations Classified Under External Approval Tiers

These rules are ALWAYS ACTIVE for all files matching the configured scope (crates/permissions, crates/agent-mcp, crates/openagc-core/src/agents/approvals.rs, crates/openagc-core/src/agents/tools.rs).

### Rules

- **R-TOOL-001** MUST: Operations classified under external approval tiers MUST generate and await explicit approval card verification before mutating core mailbox state stores.

### Verify

```bash
cargo test --workspace --lib --bins
```

**Accept when:**
- All tools registered in agent execution paths have an explicit risk tier in crates/permissions.
- External approval tools successfully pause execution until approved via approval cards.
- Attempted tool invocations without policy clearance are rejected by the MCP server shim.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>