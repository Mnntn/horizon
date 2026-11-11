#!/usr/bin/env bash
# Cargo nests RUSTC_WORKSPACE_WRAPPER (clippy-driver) inside RUSTC_WRAPPER.
# Keep hand-written workspace linting direct and cache ordinary rustc invocations.
set -euo pipefail
case "${1##*/}" in
  clippy-driver | clippy-driver.exe)
    exec "$@"
    ;;
        -Cincremental=*) shift; continue ;;
      esac
      args+=("$1")
      shift
    done
    # Incremental units cannot be cached by sccache. Only this generated crate
    # drops its incremental flag; hand-written workspace crates keep theirs.
    set -- "${args[@]}" -D warnings
    ;;
esac
# Cargo has already selected incremental compilation in rustc's arguments.
# sccache refuses CARGO_INCREMENTAL=1 even for non-incremental dependencies.
unset CARGO_INCREMENTAL
exec sccache "$@"
