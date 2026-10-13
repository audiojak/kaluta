# Prompt Fingerprint Generation in Agent Routines: Routine Modifications That Alter Generated Prompt

These rules are ALWAYS ACTIVE for prompt generation and evaluation routines within the agent API component, and deterministic verification of prompt template outputs and test snapshot harnesses.

### Rules

- **R-PROMPT-001** SHOULD: Routine modifications that alter generated prompt contents verify that the fingerprint changes relative to the baseline generation.

### Verify

```bash
# Discover the project test runner configuration and execute the unit test suite targeting prompt routine verification.
# Discover and run the snapshot review suite to confirm prompt outputs match approved baselines across all runner targets.
```

**Accept when:**
- All prompt generation tests pass and assert equal fingerprints for identical routine inputs.
- Modified routine descriptions produce distinct fingerprint values of exactly sixteen hexadecimal characters.

<enforcement>
Claude Code MUST NOT skip or defer verification.
</enforcement>