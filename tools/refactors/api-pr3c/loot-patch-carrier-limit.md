# Loot consumer cut: the current patch carrier is lossy

Part of #1608. This is a stop-condition report, not a completed consumer cut.
No API, engine, dispatch or particle behavior is changed by this report.

## Representation mismatch

At integration base `a819320057`, `horizon-world/src/loot.rs` declares
`LootStack.components: Vec<(String, serde_json::Value)>`. JSON null means removal.
The proposed API snapshot in `horizon-mod-api/src/world/snapshots.rs` instead carries
`Vec<(String, Option<Tag>)>`. Unknown component values must keep their NBT types.

The existing production functions `scoreboard::tag_to_json` and
`scoreboard::json_to_tag` are not a lossless bridge:

| Input Tag | Intermediate JSON | Returned Tag |
| --- | --- | --- |
| `ByteArray([7])` | `[7]` | `List([Int(7)])` |
| `List([Int(7)])` | `[7]` | `List([Int(7)])` |
| `Byte(7)` | `true` | `Byte(1)` |

Consequently, simply using that codec at the game boundary would silently lose
unknown component data. A map, wrapper or alternate JSON encoding would be a
second representation rather than the requested clean break. Refusing all such
NBT values would not provide the requested unknown-Tag round trip either.

## Required prerequisite

Change the engine's loot patch storage and materialization to ordered
`(String, Option<Tag>)` entries, not just the API snapshot. Decode the JSON loot
function input once where it owns that source, and materialize the resulting NBT
without converting back through JSON. Preserve the order of repeated entries and
represent removal separately. Keep the special trim handling equivalent to the
existing `ItemData::set_loot_component` implementation.

Audited consumers that need coordinated changes include:

- `horizon-world/src/loot.rs`: `set_components`, `set_instrument`, and the horn test.
- `horizon-items/src/item_data.rs`: the JSON-based `set_loot_component` materializer.
- `horizon-client/src/structure_blocks.rs`: `loot_item_with_components`.
- `horizon-client/src/game/mobs/creatures/mod.rs`: `from_loot`.
- `horizon-client/src/mobs/drops.rs`: the generated trim patch.

After that prerequisite, implement the guarded engine-to-API context/stack
snapshots, stage complete validated writeback before mutating the roll, and run
production mutation tests for unknown NBT, removals and repeated ordering.
Signed counts must not go through occupied inventory stack validation.

## Focused diagnostic

A temporary test appended to the production scoreboard module calls the actual
codec, not a duplicate implementation:

```rust
#[cfg(test)]
mod loot_patch_cut_probe {
    use super::{json_to_tag, tag_to_json};
    use horizon_core::nbt::Tag;

    #[test]
    fn loot_patch_codec_is_not_injective() {
        let array = Tag::ByteArray(vec![7]);
        let list = Tag::List(vec![Tag::Int(7)]);
        assert_ne!(array, list);
        assert_eq!(tag_to_json(&array), tag_to_json(&list));
        assert_eq!(json_to_tag(&tag_to_json(&array)), list);
        assert_ne!(json_to_tag(&tag_to_json(&array)), array);
        assert_eq!(json_to_tag(&tag_to_json(&Tag::Byte(7))), Tag::Byte(1));
        assert_ne!(json_to_tag(&tag_to_json(&Tag::Byte(7))), Tag::Byte(7));
    }
}
```

Command:

```sh
cargo test -p horizon-client-core --lib loot_patch_cut_probe::loot_patch_codec_is_not_injective -- --nocapture
```

Observed result: **1 passed, 0 failed, 91 filtered out** at this worktree's
integration base plus the temporary test. The boundary property check
(`python3 -m unittest tests.test_mod_api_boundary`) also passed all 5 tests. The earlier attempt with a
120 second timeout expired while waiting for the shared build lock; the retry
used a 1,800 second timeout and completed successfully. The temporary test was
then removed and the production scoreboard source verified unchanged.

The diagnostic is not retained as a regression test: pinning this loss as desired
behavior would obstruct the actual fix. No production-conversion mutant is
claimed, since there is no new production converter in this report.

## Scope left open

The loot consumer cut, particle consumer cut, export removal, catalog and book
migration changes, WIT/host-manifest comparison, subscription counters, mutation
tests and allocation/latency benchmarks remain unimplemented. In particular,
this report does not remove the engine exports before consumers are ready.
