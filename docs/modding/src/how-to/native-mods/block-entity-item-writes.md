<!-- chapter: Dropped items during block-entity ticks -->

## Dropped items during block-entity ticks

The overlay below is the port's own transactional requirement, not vanilla or NeoForge behavior. NeoForge adds no event, deferral or drop capture around a block entity's tick; new item entities join immediately. Block-break and entity death/shear captures are separate mechanisms.

During a mod's `BlockEntityPart::block_entity_tick`, existing dropped-item stack writes and removals are proposals. Use `ItemEntityRef::set_stack` to replace a complete nonempty stack and `EntityRef::discard` or `EntityRef::remove` to remove it. Item-place writes join the same scope. Native and wasm mods use the same calls; there is no separate batch method or transaction seal to finish the tick.

Later Parts see earlier proposed stacks in registration order. A proposed removal hides the item from handle resolution, item narrowing, entity selectors and item-place reads. Explicit nested host transactions can abort their own proposals; committing one still waits for final tick acceptance.

The engine validates the final block-entity snapshot and every item's original stack and residency incarnation before publishing either side. Rejected revisions, invalid stack conversions, malformed guest responses, stale item preconditions, and removal or replacement of the ticking block entity discard the proposals. A successful host setter means the proposal was accepted into the scope, not that final writeback has already committed. A numeric entity ID never retargets a replacement or another level. Removal notifications run after publication and after detaching the completed overlay.

This is deliberately not a whole-world transaction. Pickup delay and target changes, spawning, block changes, motion and teleportation, non-item removals, players, attachments, neighboring block entities, weather and world effects remain deliberately immediate. They are not undone when the snapshot is rejected. An immediate operation that destroys a staged item's precondition can reject the pending proposal while retaining its own effects.

With no scope open, the engine behaves exactly as vanilla for these item operations: changes are immediate, the overlay remains `None`, and nothing is staged. The prepared-resource adapter regression `native_tick_publishes_or_rejects_item_proposals_with_writeback` checks that its outside-scope setter changes the item immediately and leaves both overlay and tick key `None`. Vanilla block-entity tick item changes are immediate too; only the port's mod block-entity host operations gain the stronger atomicity above.
