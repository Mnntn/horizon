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

#### Item use lifecycle (native and wasm)

`living_entity_use_item_tick` runs after the may-still-use check. Its `duration`
replaces the remaining-ticks counter. Next come per-tick effects only above zero,
the decrement, and server completion at zero or below.

On the server, canceling Tick or setting duration to zero or below **finishes**
the use: food is eaten or a potion applied, Finish fires, and use state clears.
Setting one runs the effects and finishes in that same tick. This never calls
release-using, so a bow fires nothing. A use-on-release item such as a crossbow
does not finish here: its counter goes negative until release.

On a client, cancellation skips only that tick's effects. The countdown continues;
the use remains active until server completion or release, so releasing still
notifies the server.

The neighbouring events differ: canceling Start prevents the use beginning,
but duration zero does begin it. Canceling Stop skips only release-using, not
clearing the use. Finish is not cancellable; its result stack can be replaced.
