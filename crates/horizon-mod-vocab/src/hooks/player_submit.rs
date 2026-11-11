//! The `player_submit` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0159

pub trait Hooks {
    /// Draw the player `player` (`who`: its profile name, and whether it is
    /// this window's own player) with a model of your own, once a frame while
    /// it is in view (`RenderPlayerEvent.Pre`), and set `drawn` to leave out its
    /// vanilla model, armour, held items, cape and the other layers. This
    /// window's player is drawn only in third person (and F5 front view); an
    /// invisible player is not asked. `draw` is the frame of `native_mob_submit`:
    /// at the player's feet, turned with the body. The name tag, shadow,
    /// flames and glowing outline draw as before. `hidden.hide(part)` (or
    /// `hidden.hide_named(name)`) leaves only that part out of the vanilla model this frame (after Skin
    /// Customization), as a mod drawing its own outer skin layer does: a
    /// hidden part takes its children with it (the hat goes with the head),
    /// while its armour, held items and other layers, the cape among them,
    /// keep every part.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "drawn",
        neoforge = "RenderPlayerEvent.Pre",
        cancellable = true
    )]
    fn player_submit();
}
