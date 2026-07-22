#!/usr/bin/env bash
#
# Canonical routine verification for s7s.
#
# Runs the automated contract that every change must pass, in order, failing
# fast and naming the stage that failed. Contains no local paths so it works
# from any clean checkout. Expensive real-data and real-CLI checks are NOT run
# here; they remain explicit opt-in steps documented in docs/testing.md
# (real_data_turn_parity, --usage-probe, --model-probe, manual CLI/PTY checks).
#
# Usage: scripts/check.sh   (run from the repository root)

set -euo pipefail

cd "$(dirname "$0")/.."

stage() {
  printf '\n==> %s\n' "$1"
}

stage "cargo fmt --all -- --check"
cargo fmt --all -- --check

stage "cargo test -q"
cargo test -q

stage "cargo clippy --all-targets --all-features -- -D warnings"
cargo clippy --all-targets --all-features -- -D warnings

stage "cargo build --release"
cargo build --release

printf '\nAll checks passed.\n'
