<!-- chapter: Testing a trait mod -->

### Testing a trait mod

Enable the `testing` feature on the mod's `horizon-mod-api` test dependency. Then
`testing::with_mods` takes an owned `Mods` list and runs a closure with it installed on the current thread.
It restores the previous list even if the closure panics. The test allocation is retained because dispatch exposes
a static slice. Call `dispatch_<hook>(&mut host, args…)` inside the closure to build the event as the game does.

`testing::FakeHost` is a server and client host built from one small fake per service, so a test runs without
the game: `registries` (item reports, items with slots of their own, the loaded mods), `players` (who is online,
their inventories by slot, what was given to them and shown on their action bar), `level` (blocks, block entities
with item slots, fluid tanks and energy storage as the game answers them, and entities), `commands` and
`messages` (what ran and what was said), and `journal`, which keeps the players and the level on each transaction
and puts them back when it aborts. Set the world up through those fields, run the code under test with
`&mut host`, then read the fakes back:

```rust
use mod_api::testing::{FakeHost, with_mods};
use mod_api::{self, ClientHost, ClientTickPreEvent, Hooks, ModEntry, dispatch_client_tick_pre};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

#[derive(Default)]
struct Counter(Arc<AtomicUsize>);
#[mod_api::horizon_mod]
impl Hooks for Counter {
    fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}
let count = Arc::new(AtomicUsize::new(0));
let mods = [ModEntry::new("counter", Box::new(Counter(count.clone())))];
with_mods(mods.into(), || dispatch_client_tick_pre(&mut FakeHost::new()));
assert_eq!(count.load(Ordering::Relaxed), 1);
```

A test of item moves puts two chests in the level and reads back what went where; the transaction's
journal is the fake's:

```rust
use mod_api::testing::FakeHost;
use mod_api::{HostTransaction, ITEMS, ItemResource, ItemStack, ServerHost, move_resources};

let mut host = FakeHost::new();
let level = host.level.level();
host.level.block_entity([0, 64, 0]).with_items(1);
host.level.block_entity([1, 64, 0]).with_items(1);
let stone = ItemStack { item: "minecraft:stone".into(), count: 5, damage: 0, extra: None };
host.level.block_entity([0, 64, 0]).items.as_mut().unwrap().slots[0] = Some(stone);

let server: &mut dyn ServerHost = &mut host;
let from = level.capability(server, &ITEMS, [0, 64, 0], None).unwrap();
let into = level.capability(server, &ITEMS, [1, 64, 0], None).unwrap();
let moved = server.transaction(|host, tx| {
    move_resources(host, &from, &into, |_: &ItemResource| true, 3, tx)
});
assert_eq!(moved.ok(), Some(3));
let into = host.level.block_entity_at([1, 64, 0]).unwrap().items.as_ref().unwrap();
assert_eq!(into.count("minecraft:stone"), 3);
```

A `Host` method no fake answers panics naming it, as every `Host` method defaults to, so an unexpected call
fails loudly. A test can still write its own double for what the fakes don't model, implementing only what its
hook calls; `testing::FakeJournal` gives such a host the same transaction journal over its own state.

If a `Host` method returns a game value a test can't build, a double can return `None` instead, as with
`Players::acting_player() -> Option<PlayerRef>`. If the dispatch function itself takes an unconstructible value,
such as `dispatch_perform_command`'s `&ServerPlayer`, test the logic beneath the hook instead. The level
hosts are small enough to implement whole. NeoForge's GameTest is the in-game counterpart.
