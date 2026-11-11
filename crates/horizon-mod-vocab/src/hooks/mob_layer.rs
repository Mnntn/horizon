//! The `mob_layer` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0160

pub trait Hooks {
    /// A layer on every mob and player in view, once a frame after its
    /// vanilla model and layers (`EntityRenderersEvent.AddLayers`, with
    /// `RenderLivingEvent.Post` and `RenderPlayerEvent.Post`): add geometry
    /// in the entity's frame, which starts at its feet turned with the body
    /// (`mob.head_yaw`, `mob.pitch` and the walk cycle place a part, the eyes
    /// being at the entity type's eye height). `who` is set for a player.
    /// `parts` is a humanoid model's parts as vanilla posed them this frame
    /// (`HumanoidModel.setupAnim`), taken from the body's own draw, so a part
    /// `native_player_submit` hid reads as not visible; read-only: attach
    /// geometry to an arm or the head without animating it again; None for a
    /// model that isn't humanoid. Not asked for what a mod drew itself (`native_mob_submit`,
    /// `native_player_submit`) nor for a mob that isn't drawn at all.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "EntityRenderersEvent.AddLayers",
        cancellable = false
    )]
    fn mob_layer();
}
