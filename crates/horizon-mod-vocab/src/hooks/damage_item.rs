//! The `damage_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `IItemExtension.damageItem`: the wear `stack` is about to take
    /// (`ItemStack.hurtAndBreak`: a tool that mines or hits, armour that takes
    /// a hit, a shield, a rod, an elytra, a tool a dispenser uses, a mob's
    /// armour), after Unbreaking has had its say, as NeoForge passes it.
    /// `amount` is the points about to be lost; change it (0 or less leaves
    /// the item whole) or set `canceled` for no wear at all. Not asked when
    /// Unbreaking already saved the item, nor for a creative player's tools
    /// (they take no wear). Only the `ItemPart`s attached to `stack`'s item
    /// hear it: an item with none costs one table lookup, and a game with no
    /// such part none at all. On the paths of a player the player's state is
    /// swapped in, so `host.players().acting_player()` answers them. Once for
    /// each wear, on a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        neoforge = "IItemExtension.damageItem"
    )]
    fn damage_item();
}
