//! The `world_view_drawn` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0350

pub trait Hooks {
    /// A world view is drawn (see [`WorldViewDrawn`]): once per view asked for this frame, after
    /// the frame's world and before [`world_drawn`].
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::world_view_drawn",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn world_view_drawn();
}
