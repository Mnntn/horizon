//! The `mob_submit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0113

pub trait Hooks {
    /// Draw the mob `mob` (its id, kind, custom name and the frame's body
    /// and head turns and walk cycle, `LivingEntityRenderState`) on this
    /// client with a model of your own, once a frame while it is in view
    /// (vanilla's `EntityRenderer.submit`), and set `drawn` to leave out its
    /// vanilla model, armour and held items (NeoForge's `RenderLivingEvent.Pre`,
    /// cancelled). Add geometry with `native_draw_cube`, `native_draw_quad`,
    /// `native_draw_block` and `native_draw_item` under `draw`'s pose stack,
    /// which starts at the mob's feet turned with its body: +z is the way it
    /// faces, +x its left, +y up, in blocks. `draw` also has the partial tick,
    /// the game time and the mob's light. Its name tag, shadow, flames and
    /// glowing outline draw as for any mob. Asked for every mob in view but
    /// players; a mob your server half spawned (`native_spawn_mob`) is told
    /// apart by its id, sent with mod messages, or by its custom name.
    /// A mob is drawn afresh each frame: `draw.keep()` does nothing here.
    /// `hidden.hide_named(name)` leaves only the part of that name out of the
    /// vanilla model this frame (a villager's `nose`), drawn on the CPU or the
    /// GPU: a hidden part takes its children with it, while its armour, held
    /// items and other layers keep every part.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "drawn",
        neoforge = "RenderLivingEvent.Pre",
        cancellable = true
    )]
    fn mob_submit();
}
