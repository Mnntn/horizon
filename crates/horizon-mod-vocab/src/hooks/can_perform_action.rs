//! The `can_perform_action` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canPerformAction`: can `stack` perform the item
    /// ability `ability`, a namespaced id such as `minecraft:shield_block`,
    /// `minecraft:shears_harvest` or `minecraft:shears_carve` (a mod's own
    /// abilities are its own, `my_mod:chop`)? `can_perform` arrives with
    /// vanilla's answer for that ability: the item kind's (shears shear) or the
    /// stack's component (`blocks_attacks`, a shield's, blocks); set it either
    /// way and the stack grants or loses the ability. Asked where vanilla checks
    /// the item for an action: a raised item blocking attacks
    /// (`minecraft:shield_block`: a granted stack blocks as a bare
    /// `blocks_attacks` component does, a denied one never blocks nor is raised);
    /// shearing a sheep, mooshroom, snow golem or bogged, a dispenser's shears
    /// and a bee nest's honeycombs (`minecraft:shears_harvest`); carving a pumpkin
    /// (`minecraft:shears_carve`). It decides whether the stack may do the action,
    /// not what the action does to the world or the tool (`block_tool_modification`
    /// is the block's side of a tool action; item wear is its own). Only the
    /// `ItemPart`s attached to `stack`'s item are asked, on both sides, so the
    /// client's prediction agrees with the server: answer from the stack and
    /// `ability` alone, and fast. An item with no such part costs one table lookup.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.canPerformAction"
    )]
    fn can_perform_action();
}
