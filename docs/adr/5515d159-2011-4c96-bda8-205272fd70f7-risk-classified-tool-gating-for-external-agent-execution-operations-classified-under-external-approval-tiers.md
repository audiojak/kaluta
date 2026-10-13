# Risk-classified tool gating for external agent execution: Operations Classified Under External Approval Tiers

Status: proposed
Date: 2026-10-05
Deciders: AI (signal conversion)

## Context

- Autonomous LLM agent runtimes (such as Claude and Codex) require access to tools that interact with core application domain logic and mailbox state.
- Without explicit boundary controls, autonomous agents could directly invoke destructive operations—such as sending emails or mutating remote labels—without verification or user consent.
- The introduction of `crates/permissions` and an MCP socket shim boundary (`crates/agent-mcp/src/server.rs`, `crates/agent-mcp/src/client.rs`) establishes structured risk tiers (read, reversible, and external approval) and policy decision limits to gate agent tool invocations.

## Problem Statement

Autonomous agent runtimes can trigger dangerous operations that mutate core mailbox state unless gated behind an immutable risk-classified policy barrier and approval mechanism.

## Decision

1. MUST: Operations classified under external approval tiers MUST generate and await explicit approval card verification before mutating core mailbox state stores.

## Policy Block

- MUST Operations classified under external approval tiers MUST generate and await explicit approval card verification before mutating core mailbox state stores.

In scope:
- crates/permissions
- crates/agent-mcp
- crates/openagc-core/src/agents/approvals.rs
- crates/openagc-core/src/agents/tools.rs

Out of scope:
- Internal direct user mailbox operations invoked directly via UI without agent execution

Exceptions:
- ex-1: Headless integration test harnesses requiring deterministic synthetic approval cards in automated CI test environments

## Rationale

- Categorizing tools into distinct risk tiers establishes a strict privilege barrier isolating LLM runtimes from core mailbox state.
- Requiring approval cards for high-risk operations prevents accidental or unauthorized external actions like sending mail or modifying remote labels.
- Centralizing policy evaluation in `crates/permissions` and the MCP socket shim ensures uniform enforcement across all agent interactions.

## Consequences

Positive:
- Deterministic policy enforcement prevents unverified or dangerous state mutations by external agent models.
- Clear separation of concerns between agent communication via MCP and permission policy gating in `crates/permissions`.
- Traceable user approval flow for destructive external actions.

Negative:
- Introduces latency and blocking workflows when awaiting user interaction on approval cards.
- Additional maintenance overhead for registering new tools into explicit risk tiers.

## Alternatives

- Allow direct unmediated tool execution from the MCP socket interface without risk tier gating (rejected)
  Rejected because: Fails to protect core mailbox data stores against unauthorized or unintended mutations by autonomous LLM agents.
- Rely entirely on dynamic agent self-classification for tool danger levels at runtime (rejected)
  Rejected because: Agent self-classification cannot guarantee security invariant enforcement and is susceptible to prompt injection or model error.

## Risks

- The MCP socket server could be targeted or bypassed by unauthorized processes running within the user session.
  Mitigation: Enforce strict socket transport validation, local session authentication, and boundary verification on all incoming MCP payloads.
  Owner: Security Architecture
- A newly added tool could default to an overly permissive tier if unassigned.
  Mitigation: Ensure policy verification fails closed, rejecting any tool call that lacks an explicitly defined risk tier in `crates/permissions`.
  Owner: Core Platform Team

## Implementation Notes

- Verify tool definitions in `crates/openagc-core/src/agents/tools.rs` bind to `crates/permissions` risk definitions.
- Wire approval card generation in `crates/openagc-core/src/agents/approvals.rs` to policy evaluation limits in `crates/agent-mcp/src/server.rs`.
- Implement fail-closed validation on client and server socket communication in `crates/agent-mcp`.

## Continuation Context


Verify commands:
- Discover and execute test targets covering permissions verification and tool gating in agent crates.
- Discover and execute test targets covering MCP socket boundary authorization and approval card workflows.

Accept when:
- All tools registered in agent execution paths have an explicit risk tier in crates/permissions.
- External approval tools successfully pause execution until approved via approval cards.
- Attempted tool invocations without policy clearance are rejected by the MCP server shim.

## Enforcement

- Verified by: Automated unit and integration test suites validating tool registration and policy checks
- Verified by: Compile-time type verification ensuring all registered tools map to a risk tier enum in `crates/permissions`
- Verified by: Code review inspection of any changes to `crates/agent-mcp` or `crates/openagc-core/src/agents/tools.rs`
- Violation handling: MCP tool invocations lacking valid tier assignment or approval are rejected with policy violation errors
- Violation handling: CI build failure if any tool definition fails to implement risk classification bindings
- Exception process: Security review approval and documented rationale for any custom bypass or temporary tier reclassification

## References

- file:crates/permissions/src/lib.rs
- file:crates/agent-mcp/src/server.rs
- file:crates/agent-mcp/src/client.rs
- file:crates/openagc-core/src/agents/approvals.rs
- file:crates/openagc-core/src/agents/tools.rs
- commit:7267093eaa52b6c4317b1b30a3294d2bdb955922
- commit:2a936bb292bed88f2d9ea93ce6b497e522f08fbf
- commit:3d17779d07ee2bf73adb0ec4f97f446bcdd0937f
- commit:bbef72596727074a1a2d108c10fc386d9a5b6d01
- commit:e361e46d8af5a40a63deee415de05f729e9c3142
- commit:70e030b6fc7f02f68607071d0712018d6f8fdca0