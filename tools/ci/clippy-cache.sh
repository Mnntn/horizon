#!/usr/bin/env bash
# Cargo nests clippy-driver inside RUSTC_WRAPPER. Lint workspace crates directly.
set -euo pipefail
case "${1##*/}" in
  clippy-driver | clippy-driver.exe) exec "$@" ;;
esac
# Cargo has already selected incremental compilation in rustc's arguments.
unset CARGO_INCREMENTAL
exec sccache "$@"
