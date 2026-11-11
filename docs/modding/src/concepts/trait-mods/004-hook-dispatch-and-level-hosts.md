<!-- chapter: Hook dispatch and level hosts -->
<!-- separator: 1 -->

To save from inside a hook, use `level.request_save(host)` so the save runs at the next settled
start-of-tick boundary. A synchronous `save-all` while an entity pass or held-out row is borrowed
fails with vanilla's `commands.save.failed` feedback and a failed command result, without saving.

To emit a vibration-producing game event, use `level.game_event(host, event, at, context)`.
`GameEventContext` carries an optional source entity and affected block state; see
[game events with context](../../how-to/game-events.md) for listener semantics and refusals.

`#[listener]` on a hook method sets both, as NeoForge's `EventPriority` and `receiveCanceled` do. Two mods, each
its own crate:

```rust
#[mod_api::horizon_mod]
impl Hooks for Protection {
    // Runs before mods at the default priority, 0; higher runs first.
    #[listener(priority = 100)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if self.protected(host, event.pos) { Flow::Cancel } else { Flow::Continue }
    }
}

#[mod_api::horizon_mod]
impl Hooks for Override {
    // Still told after a cancel, and may undo it.
    #[listener(priority = -100, receive_cancelled)]
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        if event.cancelled && self.allowed(host, event.pos) {
            event.cancelled = false;
        }
        Flow::Continue
    }
}
```

Native and wasm trait mods use the same listener ordering and cancellation rules. Wasm subscription records carry the hook name, signed priority, and receive-cancelled flag through the shared-memory Wire boundary at load time. The event's `cancelled` field crosses with its other writable fields. No component-model adapter is involved.

Each hook method takes its host, where it has one, and one event:

```rust
fn living_incoming_damage(
    &self,
    _level: &mut dyn LevelRandomHost,
    event: &mut LivingIncomingDamageEvent<'_>,
) -> Flow {
    if event.kind == "minecraft:pig" {
        let halved = event.container.new_damage() * 0.5;
        event.container.set_new_damage(halved);
    }
    Flow::Continue
}
```

The event is a generated struct named after the catalog hook, in CamelCase plus `Event`, with
the hook point's arguments as public fields: values as values and borrowed ones as references (`event.kind`). A
number or `bool` a mod may change is a plain field, written back once every mod has run, even after a `Cancel`;
anything else a mod may change, a game struct or a list, is a `&mut` field. A cancellation or replacement is still
the return value; a collection hook's list is a field (`event.lines`). One event goes through every mod, so each
sees the changes before it. Because no mod can build or match the whole
struct, a later version can add a field without breaking any mod, as NeoForge's event objects do; the cost is that
a mod's own tests drive a hook through its dispatch function ([Testing a trait mod](026-testing-a-trait-mod.md#testing-a-trait-mod)).

Hook methods take `&self` and require `Send + Sync`. The process-wide mod list is installed once, like mixin module statics. One mod instance serves both
the window's game and the server-thread game. A hook can also fire again inside
the same mod: for example, `LevelRef::set_block` fires `block_changed`. Mods keep mutable
state with interior mutability so these calls can share the instance safely. Data-load
hooks can run on any loading thread; item default changes must be identical on every side.