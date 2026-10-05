#!/usr/bin/env bash
# Release build and run. This is the build to actually use.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo run --release -- "$@"
