#!/usr/bin/env bash
set -eo pipefail
if [ "$SAME_REPO" = true ]; then
  ENABLED=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_SELF_HOSTED" -q .value 2>/dev/null || true)
  LEASE=$(gh api "repos/$GITHUB_REPOSITORY/actions/variables/DYNA_CI_POOL_UNTIL" -q .value 2>/dev/null || true)
fi
# A refused read prints GitHub's error JSON: keep only a well-formed answer, else use the snapshot.
case "${LEASE:-}" in '' | *[!0-9]*) LEASE= ;; esac
case "${ENABLED:-}" in true | false) ;; *) ENABLED= ;; esac
echo "pool lease: live=${LEASE:-unreadable} snapshot=${VARS_LEASE:-none}"
SOURCE=live
CREATED=
if [ -z "${LEASE:-}" ]; then
  SOURCE=snapshot
  # Actions read is sufficient for run metadata, unlike the Variables endpoint.
  CREATED_AT=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" -q .created_at 2>/dev/null || true)
  if [ -n "$CREATED_AT" ]; then
    CREATED=$(date -d "$CREATED_AT" +%s 2>/dev/null || true)
  fi
fi
ENABLED=${ENABLED:-$VARS_ENABLED} LEASE=${LEASE:-$VARS_LEASE} LEASE_SOURCE=$SOURCE CREATED=$CREATED bash tools/ci/dyna_ci_runner.sh >> "$GITHUB_OUTPUT"
