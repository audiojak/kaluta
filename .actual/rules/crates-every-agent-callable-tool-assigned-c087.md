# Risk-classified tool gating for external agent execution: Every Agent Callable Tool Assigned Immutable

These rules are ALWAYS ACTIVE for all agent tool definitions, permission configurations, and MCP socket shim implementations matching the configured scope.

### Rules

- **R-TOOL-001** MUST: Every agent-callable tool MUST be assigned an immutable risk tier within `crates/permissions` categorizing it into read, reversible, or external approval tiers.
- **R-TOOL-002** MUST: Policy verification MUST fail closed, rejecting any tool call that lacks an explicitly defined risk tier in `crates/permissions`.

### Verify

```bash
cargo test -p crates-permissions
cargo test -p crates-agent-mcp
cargo test -p openagc-core --agents
```

**Accept when:**
- All tools registered in agent execution paths have an explicit risk tier in crates/permissions.
- External approval tools successfully pause execution until approved via approval cards.
- Attempted tool invocations without policy clearance are rejected by the MCP server shim.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>