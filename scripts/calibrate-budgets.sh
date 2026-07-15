#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:---check}"

if [[ "$mode" != "--check" ]]; then
  echo "The U7 pre-deletion calibration is frozen; only --check is supported." >&2
  echo "usage: scripts/calibrate-budgets.sh --check" >&2
  exit 2
fi

cd "$repo_root"
python3 scripts/verify-budgets.py
cargo +1.85.0 nextest run -p mdstream-conformance --test budget_contract
