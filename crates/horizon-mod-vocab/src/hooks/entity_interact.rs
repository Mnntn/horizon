//! The `entity_interact` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0057

pub trait Hooks {
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `hand` (`Hand::Main` or `Hand::Off`), holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Set `claimed` to take
    /// the click: the hand swings and neither runs. Asked once per hand, as
    /// vanilla tries the main hand then the off hand. Server-side, for every
    /// player of the world (the helpers read the acting one). Diverges from upstream: renamed from
    /// `interact_entity`, to NeoForge's `PlayerInteractEvent.EntityInteract`.
    /// `hand` corresponds to NeoForge's `PlayerInteractEvent.EntityInteract#getHand`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        neoforge = "PlayerInteractEvent.EntityInteract",
        paper = "PlayerInteractEntityEvent"
    )]
    fn entity_interact();
}
