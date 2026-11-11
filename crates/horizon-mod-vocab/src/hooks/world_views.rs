//! The `world_views` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0349

pub trait Hooks {
    /// Asked as each frame is built and drawn (not on the title screen's panorama), more than
    /// once: push the world views (see [`WorldView`]) your mod wants drawn this frame, the same
    /// each time. The world is drawn again from each one's camera into a target of its own.
    #[notes = r#"## World views

A client mod can have the world drawn again from cameras of its own, into targets of its own: a camera monitor, a
second camera's preview beside the player's view, stills for a film strip. As each frame is built and drawn (not on
the title screen's panorama), `world_views` asks for them, more than once a frame, so keep it cheap: push a
`WorldView::new(id, camera, yaw, pitch, fov_degrees, size)` (`with_roll(degrees)` turns it about the view
direction) for each one wanted this frame. Up to `MAX_WORLD_VIEWS` (4) draw, each at most `MAX_WORLD_VIEW_SIDE`
(4096) pixels a side, or the device's largest texture side when that is smaller; one with a value out of range or an `id` already asked for is left out. Push the same views
each time within a frame.

Each view is drawn after the frame's world with the frame's terrain (solid and translucent), sky colour, fog and
entities (block entities too), from its own camera and with its own frustum; then `world_view_drawn` hands it back
as a `WorldViewDrawn`: its `id`, `color` (`color_format`, the world's light as `WorldDrawn::color` holds it, before
the tone curve, without the hand, the HUD or the frame's post passes), `depth` (Depth32Float, reversed), `view_proj`
(the world relative to the view's `camera`, to its clip space), and the device, queue and the frame's encoder. Draw
your own geometry into it there, sample it (both textures can be bound) or copy it out: `copy_texture_to_buffer`
into a buffer of your own and map that after the frame, for a still. The target stays the view's while each frame
asks for it, so a texture view of it can be kept and shown, and is drawn over every frame; one no frame asks for any
more is freed. Views draw before `world_drawn`, which can put them into the frame's picture.

A view sees what lies in its own frustum within the render distance: the sections the frame draws and the entities
it was given are the player camera's and the views' together. While any view is asked for, nothing is left out by
the player's camera alone (its frustum for entities, the caves, occlusion, faces turned away) and GPU culling is
off, so the frame's own picture stays the same but costs more; the views' sorting of translucent terrain is the
player's camera's. A view draws no sun, moon, stars, clouds, particles, weather, block outline or hand, and none of
a shader pack's passes. A frame without views costs nothing more: while no mod listens to `world_views` it is not
called.

`python3 tools/run_world_views.py <dir>` (throwaway homes) draws views through the renderer offscreen and checks
that the frame's picture is unchanged; `--bench` times a frame with and without one."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "views",
        target = "mod_render::render::world_views",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn world_views();
}
