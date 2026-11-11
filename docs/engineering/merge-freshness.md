# Advisory merge freshness

Before each owned merge or rollup, run:

```sh
python3 tools/ci/merge_fresh.py 3065
```

All remote operations are read-only `gh api` requests. The command does not fetch,
checkout, merge, dispatch, or post. Exit 0 means `FRESH`, 1 means `STALE` due to
an intervening conflict, and 2 means `STALE` because evidence is unavailable.
For STALE, merge current main into the owned branch once and obtain a fresh gate.

The newest dyna-ci pull-request run associated with this PR and its current head
must have successful latest-attempt `lint` and `verify` jobs. `launcher` and
`pool-watch` must be completed successfully or deliberately skipped. Missing,
ambiguous, pending, failed, and cancelled records cannot establish freshness.
A main push or manual dispatch cannot substitute. The exact attempt's unique,
unexpired `gate-source-<attempt>` artifact must contain `gate-source.json` with
schema version 1, integer `pr`, `run_id`, and `attempt`, and 40-character lowercase
hexadecimal `base`, `head`, and `merge`. Its identities must match the selected
PR, run, attempt, and current head. No older attempt's artifact is borrowed.

When current main equals the recorded base, valid green evidence is FRESH even
for a PR editing shared contracts. Otherwise main must descend from that base.
The command compares the intervening main files with the PR files. Either side
touching a `slots/` component, Cargo.lock, or the explicit Wire layout fixtures
and generated layout pin files makes it STALE.
Shared files or a shared `crates/<name>/` or `launcher/` scope also make it STALE.
Workflow and CI tooling paths are file-scoped, not directory-wide contracts:
`tools/ci/ignored_tests.py` versus `tools/ci/nightly.py` (#3051 versus #3043)
is disjoint, but changes to the same tooling or workflow file are STALE. Renames count both names; deletions count the removed name.
Disjoint crates without a shared contract are FRESH. Compare file responses at
GitHub's 300-file cap and incomplete PR pagination are unavailable, not FRESH.

This is advisory, not atomic: pointer rereads detect changes during the check,
but main or the PR can still move afterward. Server-side up-to-date protection
would close that race; enabling it is deferred to the owner. This tool does not
replace the automatic main release picker: that picker still requires all four
actual successful jobs, the same-SHA fixture, and matching SDK evidence through
Nightly. No release or nightly semantics change here.

`tests/test_merge_fresh.py` constructs real temporary local git histories with
independent main and PR commits, then mocks only the remote evidence transport.
The miniature fixtures model the slot-row #2989 versus #2972 break and the Wire
pin #3013 versus #2983 break, plus disjoint and same-crate histories. They do not
compile Rust or claim to replay the original full integration failures.
