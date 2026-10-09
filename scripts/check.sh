#!/usr/bin/env bash
# Every Rust check CI runs. Exits non-zero on the first failure, so it can
# gate a commit: scripts/check.sh && git commit ...
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '== %s\n' "$*"; }
step fmt;        cargo fmt --all --check
step clippy;     cargo clippy --workspace --all-targets --locked --quiet -- -D warnings
step test
# Tests make scratch data directories under the temp dir and leave them
# (a crashed test cannot clean up); give the run its own, removed after.
SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/kaluta-tests.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT
if ! out=$(TMPDIR="$SCRATCH/" cargo test --workspace --locked 2>&1); then
  echo "$out" | grep -E 'FAILED|panicked|^error|left:|right:' | head -40
  exit 1
fi
echo "$out" | awk '/^test result/ { passed += $4 } END { print passed " tests passed" }'
step check-deps; cargo xtask check-deps
step check-brand; cargo xtask check-brand
step mcp-docs; cargo xtask mcp-docs --check
step deny;       cargo deny check --hide-inclusion-graph 2>&1 | tail -1
echo "all checks passed"
