//! Render hook points for native mods (docs/modding/src/how-to/native-mods.md): empty functions the renderer calls at
//! fixed places of a frame, which a mod's mixins hook like those of
//! `horizon-client/src/game/native_hooks.rs`, and declared with them in `hooks/`.

use glam::{DVec3, Mat4};
mod generated {
    include!(concat!(env!("OUT_DIR"), "/native_render_generated.rs"));
}
pub use generated::{HandDrawnEvent, RenderNeedsEvent, WorldDrawnEvent};
pub use wgpu;

/// Whether the raw world draw place has an installed listener.
pub fn world_drawn_hooked() -> bool {
    mod_points::__private::TABLE
        .active_point(mod_points::PointId::ModRenderRenderWorldDrawn as usize)
        || mod_points::__private::TABLE
            .active_point(mod_points::PointId::ModRenderRenderWorldDrawnReturn as usize)
}

/// What a mod can draw with once the world is drawn: after the sky, terrain, entities,
/// particles, weather and the world post pass, before the first-person hand ([`HandDrawn`]),
/// the HUD and the screens. `color` holds the world's picture (copy it out before reading it: it is the target
/// drawn into), `depth` the world's depth, reversed (0 is the sky).
#[non_exhaustive]
pub struct WorldDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The world's depth (Depth32Float, sampleable), and its size: smaller than `size` when the
    /// world draws at a lower resolution. It holds the frame's depth only when a mod asked for
    /// it ([`RenderNeeds::world_depth`]) or something else kept it.
    pub depth: &'a wgpu::TextureView,
    pub depth_size: [u32; 2],
    /// Camera-relative world to clip space, without TAA's jitter, and with it (what the world
    /// was drawn with).
    pub view_proj: Mat4,
    pub jittered: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The camera-relative entity geometry of the frame (`Frame::entities`), quads of 4
    /// vertices, and its runs by entity when a mod asked for them
    /// ([`RenderNeeds::entity_spans`]) or a shader pack draws them.
    pub entities: &'a [Vertex],
    pub spans: &'a [EntitySpan],
    /// Which entity each run of `entities` is, recorded every frame (`Frame::entity_motion`):
    /// the run's first vertex and the entity, `(1 << 32) | id` for a mob or a player and
    /// `(2 << 32) | id` for a dropped item (their entity ids), 0 for anything else. A run lasts
    /// until the next. Unlike `spans`, whose ids name a type, these follow one entity from
    /// frame to frame.
    pub motion: &'a [(u32, u64)],
    /// The shared atlas entity UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices), for as many quads as `entities` holds.
    pub quad_indices: &'a wgpu::Buffer,
    /// The block textures the terrain samples, for drawing blocks with the player's own
    /// textures (`block_sprites` says where a block's are).
    pub block_sprites: BlockSpriteArrays<'a>,
}

/// The block sprite arrays ([`WorldDrawn::block_sprites`]): each texture a whole square layer,
/// its UVs 0 to 1 across it, as the world uses them (gamma-encoded, animated layers at the frame
/// shown).
#[non_exhaustive]
pub struct BlockSpriteArrays<'a> {
    /// The arrays as `D2Array` views, terrain.wgsl's eight (`sprites`, `sprites_1`...): a 1-layer
    /// stand-in past the ones in use, each with the mip levels the Mipmap Levels option leaves.
    pub views: &'a [wgpu::TextureView],
    /// Layers per array on this device (2048 on most; fewer where the GPU holds fewer, and
    /// OpenGL pads each array past it): a sprite's `array` and `layer` are cut by it.
    pub split: u32,
    /// A layer's width and height in texels.
    pub size: u32,
    /// Nearest, repeating, the nearest mip level: texels drawn sharp.
    pub nearest: &'a wgpu::Sampler,
    /// The terrain's own (linear, clamped, anisotropic under that Texture Filtering).
    pub filtered: &'a wgpu::Sampler,
}

/// What a mod can draw with once the first-person hand is drawn ([`hand_drawn`]): after
/// the world and the hand, before entity outlines, post effects, screen effects and the HUD.
/// `color` holds the frame's picture with the hand in it (copy it out before reading it).
#[non_exhaustive]
pub struct HandDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The hand and held item just drawn, camera-relative quads of 4 vertices: empty when no
    /// hand drew (third person, F1, a sleeping or spectating player) or a shader pack drew it.
    pub hand: &'a [Vertex],
    /// Camera-relative world to clip space for the hand: its own field of view
    /// (`hand_fov_degrees`) and the view bob, without TAA's jitter (the hand has none). Its
    /// depth is the hand's own: the hand's pass clears depth and keeps none.
    pub view_proj: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The shared atlas the hand's UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices).
    pub quad_indices: &'a wgpu::Buffer,
}

/// What the mods need from a frame that the game doesn't make for itself. Each mod sets what it
/// needs and leaves the rest, so several can ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderNeeds {
    /// Record `Frame::entity_spans` without a shader pack: entity runs get a stable id from
    /// their type's name, block entity runs their block state.
    pub entity_spans: bool,
    /// Keep the world pass's depth for [`world_drawn`] when nothing else does.
    pub world_depth: bool,
}

/// The world is drawn (see [`WorldDrawn`]). Not called on the title screen's panorama.
#[inline(never)]
pub fn world_drawn(world: &mut WorldDrawn<'_>) {
    let _ = world;
}

/// The first-person hand is drawn (see [`HandDrawn`]), on every frame [`world_drawn`]
/// runs, with no hand too.
#[inline(never)]
pub fn hand_drawn(hand: &mut HandDrawn<'_>) {
    let _ = hand;
}

/// Asked as a frame is built and drawn: set what your mod needs.
#[inline(never)]
pub fn render_needs(needs: &mut RenderNeeds) {
    let _ = needs;
}

/// What the mods need this frame ([`render_needs`]).
pub fn needs() -> RenderNeeds {
    let mut needs = RenderNeeds::default();
    render_needs(&mut needs);
    needs
}

pub use horizon_client_render::render_geometry::{EntitySpan, Vertex};

/// Install the native render adapters before engine startup.
pub fn install() {
    use horizon_client_render::render_slots as engine;
    engine::install(engine::Slots {
        world_drawn: |v| {
            world_drawn(&mut WorldDrawn {
                device: v.device,
                queue: v.queue,
                encoder: &mut *v.encoder,
                color: v.color,
                color_view: v.color_view,
                color_format: v.color_format,
                size: v.size,
                depth: v.depth,
                depth_size: v.depth_size,
                view_proj: v.view_proj,
                jittered: v.jittered,
                camera: v.camera,
                entities: v.entities,
                spans: v.spans,
                motion: v.motion,
                atlas: v.atlas,
                quad_indices: v.quad_indices,
                block_sprites: BlockSpriteArrays {
                    views: v.block_sprites.views,
                    split: v.block_sprites.split,
                    size: v.block_sprites.size,
                    nearest: v.block_sprites.nearest,
                    filtered: v.block_sprites.filtered,
                },
            })
        },
        hand_drawn: |v| {
            hand_drawn(&mut HandDrawn {
                device: v.device,
                queue: v.queue,
                encoder: &mut *v.encoder,
                color: v.color,
                color_view: v.color_view,
                color_format: v.color_format,
                size: v.size,
                hand: v.hand,
                view_proj: v.view_proj,
                camera: v.camera,
                atlas: v.atlas,
                quad_indices: v.quad_indices,
            })
        },
        render_needs: |v| {
            let mut needs = RenderNeeds {
                entity_spans: v.entity_spans,
                world_depth: v.world_depth,
            };
            render_needs(&mut needs);
            v.entity_spans = needs.entity_spans;
            v.world_depth = needs.world_depth;
        },
        world_drawn_hooked,
    });
}
