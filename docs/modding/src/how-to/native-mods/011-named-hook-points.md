<!-- chapter: Named hook points -->
<!-- separator: 1 -->

### Named hook points

Implement catalog hooks as methods of `mod_api::Hooks`. An injection does not select a hook by
`hook` or `hook_version`: the manifest schema accepts `[[inject]]` with a target in `at`,
a crate-root listener in `with`, and the pinned function hash. See [Injections](../../injections.md).

Raw frame functions are native escape hatches, not methods on `Hooks`. For example, an
internals injection at `horizon_mod_render::render::world_drawn` names its listener with
`with`; use the current catalog hash and `kind = "head"`. The launcher supplies the
checked raw listener signature, and the ordinary registry owns its priority and lifetime.
The renderer owns the frame geometry and dispatch slots; the mod host installs adapters before
startup. `mod_render::render::{Vertex, EntitySpan}` re-exports the engine's geometry without
copying buffers. `RenderNeeds` requests are copied back into the engine's frame requirements;
world and hand callbacks borrow the current GPU resources without allocating per frame.

#### Teleport hooks

`ender_pearl_teleport` version **2** covers player-owned and mob-owned pearls. Its `owner` is an
`EntityRef`; listeners can cancel or redirect `target`. Player owners retain the
acting-player context and mutable fall `damage`. Mob owners take no pearl damage
and make no endermite roll. Pearl placement does not emit the command-only
`entity_teleport` notification. The pearl is consumed even when teleportation is canceled.

`item_consumption_teleport` version **2** covers players and foxes with the random-teleport
consume effect. Its `consumer` is a `LivingRef`, and `consumed` is the stack captured
before use, including count and components. Cancellation ends further teleport
attempts, not consumption. Fox callbacks run before that fox's living AI/movement
and before the next mob ticks, with the full server host. Listener changes to an
actor's item, position or eligibility survive rather than being overwritten by a
stale teleport decision. Command and Ender-entity teleport hooks are unchanged.

Both version-2 contracts replace version 1's player UUID/name fields: use `owner`
or `consumer.entity()` to address the actor, and the acting-player context for
player-only operations. Named injections must request `hook_version = 2` and bind
the new parameter names; version 1 is not an alias for these contracts.
