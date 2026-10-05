#!/usr/bin/env bash
# Debug build and run. Faster to recompile than scripts/run.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run -- "$@"
