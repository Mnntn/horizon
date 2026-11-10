#!/usr/bin/env bash
# rustup toolchain install with a bounded retry: a reset connection to static.rust-lang.org is transient and hits
# boat and GitHub's runners alike. Four attempts, 10, 20 and 40 s apart; the last failure fails the step.
set -u
for delay in 10 20 40 0; do
  rustup toolchain install "$@" && exit 0
  status=$?
  [ "$delay" -eq 0 ] && exit "$status"
  echo "rustup toolchain install failed ($status); retrying in ${delay}s" >&2
  sleep "$delay"
done
