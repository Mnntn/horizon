<!-- chapter: A menu's slots -->
<!-- separator: 1 -->

#### A menu's slots

##### Mod-created recipe input sets

`Hooks::recipe_property_sets` runs once after recipes resolve for each load and again after
`/reload`. Add `RecipePropertyAddition::new("mymod:cooking_pot_input", items)` to its
`event.additions`: a key in the listener's manifest-owned namespaces creates a set. Ownership
uses the package ID plus the manifest's `namespaces`, so a package named
`ported_farmersdelight` with `namespaces = ["farmersdelight"]` can create
`farmersdelight:cooking_pot_input`. A later listener may extend an existing set but cannot
create a new foreign key. Invalid keys, unknown vanilla keys and air are skipped with a
warning; unregistered items do not participate in acceptance or network sync.

Attach a `MenuPart` to the registered menu type and filter its input slot with the same set:

```rust,ignore
struct PotInput;
#[mod_api::horizon_part]
impl mod_api::MenuPart for PotInput {
    fn menu_slot_rules(
        &self,
        context: &dyn mod_api::LoopContext,
        event: &mut mod_api::MenuSlotRulesEvent<'_>,
    ) {
        if event.slot == 0 {
            event.may_place &= context.mod_recipe_property_contains(
                "farmersdelight:cooking_pot_input", event.stack.item(),
            );
        }
    }
}
// In Hooks::register:
// registrar.menu_part("farmersdelight:cooking_pot", PotInput);
```

The Host exposes the same `mod_recipe_property_contains(set, ItemKey)` query outside
restricted menu callbacks. This query covers mod-created sets only: unknown sets, unknown
items and vanilla set keys return false. Existing vanilla station acceptance is unchanged.
The server reads its owning load; the client reads local preload sets before its first
`UpdateRecipes`, then the authoritative server generation. A packet replaces the entire
generation, including an empty one. Closing a screen or changing dimension retains it;
disconnecting discards it. Reads allocate nothing and do not rescan recipes or registries.
There is no second hook and no per-slot load callback.


Client hooks can read the local player's menu with `host.local_player()?.container_menu(host)`,
including the inventory when no container is open. `MenuRef::slot_count`, `slot` and `carried`
read on either side; `slot` and `carried` return an outer `None` when the handle or slot is
unavailable and `Some(None)` for an empty slot. Each read checks the world epoch and menu generation,
so a handle cannot read a replacement, reopened or respawned menu. These methods do not click or
change slots.

Mutable client hooks can submit a vanilla click with
`menu.click(host, &[mod_api::MenuClick::QuickMove { slot }])`. The slot is the same menu-order
index used by `slot`, not a hotbar index or a GUI slot id. For example:

```rust,ignore
{{#include ../../../examples/src/lib.rs:menu_quick_move}}
```

The typed actions are `Pickup` (with `MouseButton::Left` or `Right`), `QuickMove`,
`Swap` (with `SwapWith::Hotbar(0..=8)` or `Offhand`), `Clone`, `Throw`,
`Drag` and `PickupAll`. `Pickup` accepts `None` for outside, dropping the carried stack.
A `Drag { kind, slots }` owns its menu-order slot list and always submits a complete gesture;
`DragKind::Split` divides evenly, `One` places one per slot and `Clone` fills slots in creative mode.
Every action in a batch is validated before any prediction or sending; out-of-range slots/hotbar
indices, creative-only actions in survival and empty batches refuse without effects.
Stale/replaced/respawned menus, spectators, non-client hosts and the creative inventory screen refuse with `false`.
`true` means submitted, not server acceptance: prediction uses the normal menu engine and sends
`ContainerClick` with its state id, changed slots and carried item. Server corrections still win.
Inventory reads and direct server-side item transactions keep their existing contracts; a click does
not expose direct client inventory writes or enforce a mod's locked-slot policy. Call from an explicit
input action rather than submitting every render/tick. Diverges from upstream: new helper.

`menu.close(host)` closes the menu's screen as Escape does and tells the server (`ContainerClose`);
it returns `false` while that screen is not open, so the inventory menu closes only while the inventory
screen shows. `menu.click_button(host, button)` presses one of the open menu's buttons
(`ContainerButtonClick`, as a loom's pattern list does), for input the screen's own
buttons don't take; the server's menu decides what it does, and the inventory menu has none.

`ItemPlace::MenuSlot { menu, slot }` is slot `slot` of a player's open menu, a `MenuRef` (`player.container_menu(host)`). It is
a fork addition: NeoForge 26.3's `ItemAccess` reaches a player's inventory slot (`forPlayerSlot`) and cursor
(`forPlayerCursor`), not a menu's slot. It is the storage the slot shows, named by `menu.slot_backing(host, slot)`
(`MenuSlotBacking`), and a write goes to that storage through its own handler, so it is kept once by the transaction
and the storage's rules hold: the player's inventory, armour and off hand by their
[`ENTITY_ITEMS`](013-an-entitys-items.md#an-entitys-items) index, a chest's or a furnace's slot through its `ITEMS` handler, a chest minecart's
or a chested mount's through its `ENTITY_ITEMS`, and the player's ender chest, inventory crafting grid and a mod's chest
menu (`open_menu`), kept by the game. The slot's own rules come first, as a click has them:
`menu.slot_holds(host, slot, &stack)` is how many of a stack it holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`: a
furnace's fuel slot one bucket, an armour slot one piece that fits it, a shulker box's slot no shulker box, a result
slot none), and `menu.slot_gives(host, slot)` whether it gives up what it holds (`Slot.mayPickup`: bound armour stays
on outside creative). An insert fills that slot alone, and what doesn't fit stays out. A crafting grid a transaction
changed has its result made again at the outermost commit, once for each player however many writes it took
(`InventoryMenu.slotsChanged`, with `native_prepare_item_craft`); the result slot takes nothing and gives nothing,
since taking it is a craft. A menu closed, replaced or reopened since the `MenuRef` was made, or one whose player left,
reads and writes nothing. `menu.cursor()` is that menu's cursor (`ItemPlace::PlayerCursor { menu }`,
`forPlayerCursor`), which reaches nothing once the menu is gone, so a kept cursor never reaches the next menu's. The player's screen follows with the tick's container sync.

```rust
// Put four of the menu's offered emeralds into slot 0 of the open chest, or nothing.
let (slot, cursor) = (ItemPlace::MenuSlot { menu, slot: 0 }, menu.cursor());
host.transaction(|host, tx| {
    let took = cursor.extract(host, tx, &emerald, 4);
    if took == 4 && slot.insert(host, tx, &emerald, 4) == 4 { Ok(()) } else { Err(()) }
})
```
