#!/usr/bin/env bash
# Headless unit tests for listing, sorting, and filesystem operations.
set -euo pipefail
cd "$(dirname "$0")/.."
exec cargo test "$@"
