#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output="$repo_root/conformance/budgets/streaming.json"
mode="${1:-write}"

if [[ "$mode" != "write" && "$mode" != "--check" ]]; then
  echo "usage: scripts/calibrate-budgets.sh [--check]" >&2
  exit 2
fi

cd "$repo_root"

if [[ "$mode" == "--check" ]]; then
  cargo +1.85.0 run -p mdstream --example u7_calibration --release -- --check "$output"
else
  temporary="$(mktemp "${TMPDIR:-/tmp}/mdstream-u7-calibration.XXXXXX")"
  trap 'rm -f "$temporary"' EXIT
  cargo +1.85.0 run -p mdstream --example u7_calibration --release -- --output "$temporary"
  mv "$temporary" "$output"
  echo "Updated $output"
fi
