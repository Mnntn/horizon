# Frozen fixture discovery

From the repository root:

```sh
cargo run --manifest-path tools/frozen-fixtures/Cargo.toml > inventory.json
cargo run --manifest-path tools/frozen-fixtures/Cargo.toml -- --apply > before.json
```

The syn parser resolves explicit absolute imports (including aliases/groups) and absolute calls to the standard game fixture. Relative imports, custom roots, local wrappers, shadowed names and calls through local helpers are skipped. File-wide resource, reload, filesystem and macro signals conservatively retain the baseline helper; this intentionally produces false positives rather than admitting mutable fixtures. Ignore attribute text naming HORIZON_TEST_RESOURCES is not a mutation signal. Existing frozen calls are reported but not rewritten. No registry of eligible files is kept.

The tool changes only the called path, to a fully qualified shared frozen helper. Review the machine-readable eligible/skipped inventory and generated diff. Run again after applying: no additional edits should occur. This is conservative syntax analysis, not a proof about arbitrary method bodies or macros; mutable live game state remains fresh in either fixture.
