//! The `living_entity_use_item_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0132

pub trait Hooks {
    /// Each living tick that `entity` may still use `stack` in `hand`.
    /// `entity` is as in `native_living_entity_use_item_start`; `duration`
    /// replaces the remaining-ticks counter, not the original use duration.
    /// The order is the may-still-use check, this event, the per-tick effects
    /// only while the counter is above 0, then the decrement and server finish.
    ///
    /// On the server, canceling (which leaves -1), or setting 0 or below,
    /// finishes the use after the decrement: food is eaten, potions applied,
    /// `native_living_entity_use_item_finish` fires, and the use is cleared.
    /// Setting 1 runs the effects and finishes in this same tick. This path
    /// never calls release-using: a bow fires nothing. A use-on-release item
    /// (a crossbow) does not finish here; its counter goes negative until release.
    /// On the client cancellation skips this tick's effects and ignores its duration
    /// replacement; the existing counter decrements until server completion or release.
    ///
    /// In contrast, canceling Start prevents the use beginning (duration 0
    /// does begin it); canceling Stop skips only release-using. Finish is not
    /// cancellable, but its result stack may be replaced. Both sides: change
    /// the world only on the server, not on a server's client. Runs every tick
    /// of every use; check the item first.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Tick",
        cancellable = true
    )]
    fn living_entity_use_item_tick();
}
