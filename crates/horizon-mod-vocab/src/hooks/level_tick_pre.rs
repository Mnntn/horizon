//! The `level_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0010

pub trait Hooks {
    /// Before a level's tick pass on either logical side. `level` identifies the level.
    #[hook(side = "both", contract = "listen", neoforge = "LevelTickEvent.Pre")]
    fn level_tick_pre();
}
