//! Render hook points for native mods (docs/modding/src/how-to/native-mods.md): empty functions the renderer calls at
//! fixed places of a frame, which a mod's mixins hook like those of
//! `horizon-client/src/game/native_hooks.rs`, and declared with them in `hooks/`.

use glam::{DVec3, Mat4};
mod generated {
    include!(concat!(env!("OUT_DIR"), "/native_render_generated.rs"));
}
pub use generated::{
    HandDrawnEvent, RenderNeedsEvent, WorldDrawnEvent, WorldViewDrawnEvent, WorldViewsEvent,
};
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

/// The most world views ([`world_views`]) one frame draws.
pub const MAX_WORLD_VIEWS: usize = 4;
/// The largest side of a world view's target, in pixels.
pub const MAX_WORLD_VIEW_SIDE: u32 = 4096;

/// A view of the world a mod asks for ([`world_views`]): the same world, drawn again from a
/// camera of its own into a target of its own (a camera monitor, a second camera's preview, a
/// still for a film strip).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WorldView {
    /// The mod's name for the view: [`world_view_drawn`] hands its picture back with it, and
    /// the target stays the view's while each frame asks for it again.
    pub id: u64,
    /// Where the camera is in the world.
    pub camera: DVec3,
    /// Degrees, as the player's camera turns: yaw, pitch, and a roll about the view direction
    /// (positive turns the picture counterclockwise).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// The vertical field of view, in degrees (1 to 170).
    pub fov_degrees: f32,
    /// The target's width and height, in pixels (1 to [`MAX_WORLD_VIEW_SIDE`]).
    pub size: [u32; 2],
}

impl WorldView {
    pub fn new(
        id: u64,
        camera: DVec3,
        yaw: f32,
        pitch: f32,
        fov_degrees: f32,
        size: [u32; 2],
    ) -> Self {
        Self {
            id,
            camera,
            yaw,
            pitch,
            roll: 0.0,
            fov_degrees,
            size,
        }
    }

    pub fn with_roll(self, roll: f32) -> Self {
        Self { roll, ..self }
    }

    /// Whether it can be drawn: every value finite and in range, no side over `max_side`.
    fn valid(&self, max_side: u32) -> bool {
        self.camera.is_finite()
            && [self.yaw, self.pitch, self.roll]
                .iter()
                .all(|v| v.is_finite())
            && (1.0..=170.0).contains(&self.fov_degrees)
            && self
                .size
                .iter()
                .all(|&s| (1..=max_side.min(MAX_WORLD_VIEW_SIDE)).contains(&s))
    }
}

/// A world view just drawn ([`world_view_drawn`]): its picture and depth, and what a mod needs
/// to draw into it or copy it out. Drawn after the frame's world and before [`world_drawn`], so
/// a mod can show it in the frame's picture there.
#[non_exhaustive]
pub struct WorldViewDrawn<'a> {
    /// The [`WorldView::id`] it was asked with.
    pub id: u64,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The view's picture (the world's light in `color_format`, as [`WorldDrawn::color`] holds
    /// it, before the tone curve and without the hand or the HUD), and a view of it. The target
    /// is the view's while it is asked for each frame and is drawn over the next frame: copy it
    /// (it can be copied and sampled) to keep a still.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    /// Its depth (Depth32Float, reversed: 0 is the sky), sampleable.
    pub depth: &'a wgpu::TextureView,
    /// World relative to [`WorldViewDrawn::camera`] to clip space.
    pub view_proj: Mat4,
    /// The view's camera.
    pub camera: DVec3,
}

/// Asked as each frame is built and drawn (not on the title screen's panorama), more than once:
/// push the world views your mod wants drawn this frame, the same each time. The first
/// [`MAX_WORLD_VIEWS`] valid ones draw; one with a value out of range, or an `id` already asked
/// for, is left out. Keep it cheap.
#[inline(never)]
pub fn world_views(views: &mut Vec<WorldView>) {
    let _ = views;
}

/// A world view is drawn (see [`WorldViewDrawn`]): once per view asked for this frame.
#[inline(never)]
pub fn world_view_drawn(view: &mut WorldViewDrawn<'_>) {
    let _ = view;
}

/// Whether the world views place has an installed listener: while none has, the renderer does
/// not ask.
pub fn world_views_hooked() -> bool {
    mod_points::__private::TABLE
        .active_point(mod_points::PointId::ModRenderRenderWorldViews as usize)
        || mod_points::__private::TABLE
            .active_point(mod_points::PointId::ModRenderRenderWorldViewsReturn as usize)
}

/// The views of those asked for ([`world_views`]) that can be drawn on a device whose textures
/// are at most `max_side` pixels a side (`max_texture_dimension_2d`; never above
/// [`MAX_WORLD_VIEW_SIDE`]).
pub fn drawable_world_views(mut views: Vec<WorldView>, max_side: u32) -> Vec<WorldView> {
    let mut ids = Vec::new();
    views.retain(|v| {
        v.valid(max_side) && !ids.contains(&v.id) && {
            ids.push(v.id);
            true
        }
    });
    views.truncate(MAX_WORLD_VIEWS);
    views
}

/// An entity, item and effect vertex (the entity pipelines), with a face
/// normal for `entity.wgsl`'s directional light.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub layer: u32,
    /// RGBA8: tint multiplied by directional shade and ambient occlusion.
    pub color: u32,
    /// Smooth light, 16x fixed point: sky in bits 0..8, block in bits 8..16.
    pub light: u32,
    /// RGBA8_SNORM face normal; w = -127 for two-sided lighting, 0 to keep baked/unlit colour,
    /// `CULL_BACK` to keep it and also drop back-facing pixels.
    pub normal: [i8; 4],
    /// The `GLINT_TEXTURING` atlas UV for a foil item quad; zero elsewhere.
    pub glint_uv: [f32; 2],
}
/// Where a run of `Frame::entities` begins, and what it is (slice 7): the run lasts until the
/// next span. Recorded only while a shader pack draws entities, so Horizon's own frames carry
/// none; Horizon itself still draws the whole list at once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct EntitySpan {
    /// The first vertex.
    pub start: u32,
    /// The render stage (5.4: `ENTITIES`, `BLOCK_ENTITIES`, `PARTICLES`, `DESTROY`), which picks
    /// the program.
    pub stage: u32,
    /// `entityId` for entities, `blockEntityId` for block entities (5.3), else 0.
    pub id: i32,
    /// An entity part that blends (a name tag, a shadow): `gbuffers_entities_translucent`.
    pub translucent: bool,
    /// Hidden from the camera (Horizon's entity culling, the player in first person): only the
    /// shadow pass draws it.
    pub hidden: bool,
    /// The local player (`shadowPlayer`).
    pub player: bool,
    /// A block entity whose block emits light (`shadowLightBlockEntities`).
    pub emissive: bool,
}
impl EntitySpan {
    /// Marks vertices `start..end` of `spans`' list (one entity) as hidden: the span covering
    /// `start` is split there, the spans inside are hidden, and the geometry after `end` goes
    /// on in the last span's kind. False when no span covers `start`.
    pub fn hide(spans: &mut Vec<EntitySpan>, start: usize, end: usize) -> bool {
        let Some(i) = spans.iter().rposition(|s| s.start as usize <= start) else {
            return false;
        };
        let after = spans[spans.len() - 1];
        if spans[i].start as usize != start {
            let split = EntitySpan {
                start: start as u32,
                ..spans[i]
            };
            spans.insert(i + 1, split);
        }
        for s in spans.iter_mut().skip(i) {
            if s.start as usize >= start {
                s.hidden = true;
            }
        }
        if end > start {
            spans.push(EntitySpan {
                start: end as u32,
                ..after
            });
        }
        true
    }
}

#[doc(hidden)]
pub mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct WorldDrawn<'a> {
        pub device: &'a wgpu::Device,
        pub queue: &'a wgpu::Queue,
        pub encoder: &'a mut wgpu::CommandEncoder,
        pub color: &'a wgpu::Texture,
        pub color_view: &'a wgpu::TextureView,
        pub color_format: wgpu::TextureFormat,
        pub size: [u32; 2],
        pub depth: &'a wgpu::TextureView,
        pub depth_size: [u32; 2],
        pub view_proj: Mat4,
        pub jittered: Mat4,
        pub camera: DVec3,
        pub entities: &'a [super::Vertex],
        pub spans: &'a [super::EntitySpan],
        pub motion: &'a [(u32, u64)],
        pub atlas: Option<&'a wgpu::Texture>,
        pub quad_indices: &'a wgpu::Buffer,
        pub block_sprites: super::BlockSpriteArrays<'a>,
    }
    impl<'a> From<WorldDrawn<'a>> for super::WorldDrawn<'a> {
        fn from(v: WorldDrawn<'a>) -> Self {
            let WorldDrawn {
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                depth,
                depth_size,
                view_proj,
                jittered,
                camera,
                entities,
                spans,
                motion,
                atlas,
                quad_indices,
                block_sprites,
            } = v;
            Self {
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                depth,
                depth_size,
                view_proj,
                jittered,
                camera,
                entities,
                spans,
                motion,
                atlas,
                quad_indices,
                block_sprites,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct BlockSpriteArrays<'a> {
        pub views: &'a [wgpu::TextureView],
        pub split: u32,
        pub size: u32,
        pub nearest: &'a wgpu::Sampler,
        pub filtered: &'a wgpu::Sampler,
    }
    impl<'a> From<BlockSpriteArrays<'a>> for super::BlockSpriteArrays<'a> {
        fn from(v: BlockSpriteArrays<'a>) -> Self {
            let BlockSpriteArrays {
                views,
                split,
                size,
                nearest,
                filtered,
            } = v;
            Self {
                views,
                split,
                size,
                nearest,
                filtered,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct WorldViewDrawn<'a> {
        pub id: u64,
        pub device: &'a wgpu::Device,
        pub queue: &'a wgpu::Queue,
        pub encoder: &'a mut wgpu::CommandEncoder,
        pub color: &'a wgpu::Texture,
        pub color_view: &'a wgpu::TextureView,
        pub color_format: wgpu::TextureFormat,
        pub size: [u32; 2],
        pub depth: &'a wgpu::TextureView,
        pub view_proj: Mat4,
        pub camera: DVec3,
    }
    impl<'a> From<WorldViewDrawn<'a>> for super::WorldViewDrawn<'a> {
        fn from(v: WorldViewDrawn<'a>) -> Self {
            let WorldViewDrawn {
                id,
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                depth,
                view_proj,
                camera,
            } = v;
            Self {
                id,
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                depth,
                view_proj,
                camera,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    pub struct HandDrawn<'a> {
        pub device: &'a wgpu::Device,
        pub queue: &'a wgpu::Queue,
        pub encoder: &'a mut wgpu::CommandEncoder,
        pub color: &'a wgpu::Texture,
        pub color_view: &'a wgpu::TextureView,
        pub color_format: wgpu::TextureFormat,
        pub size: [u32; 2],
        pub hand: &'a [super::Vertex],
        pub view_proj: Mat4,
        pub camera: DVec3,
        pub atlas: Option<&'a wgpu::Texture>,
        pub quad_indices: &'a wgpu::Buffer,
    }
    impl<'a> From<HandDrawn<'a>> for super::HandDrawn<'a> {
        fn from(v: HandDrawn<'a>) -> Self {
            let HandDrawn {
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                hand,
                view_proj,
                camera,
                atlas,
                quad_indices,
            } = v;
            Self {
                device,
                queue,
                encoder,
                color,
                color_view,
                color_format,
                size,
                hand,
                view_proj,
                camera,
                atlas,
                quad_indices,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_drawable_views_draw() {
        let view =
            |id, fov, size| WorldView::new(id, DVec3::new(1.0, 70.0, -3.0), 30.0, 10.0, fov, size);
        let mut asked = vec![
            view(1, 70.0, [320, 180]),
            view(1, 70.0, [64, 64]),
            view(2, 0.5, [64, 64]),
            view(3, 70.0, [0, 64]),
            view(4, 70.0, [MAX_WORLD_VIEW_SIDE + 1, 64]),
            view(5, 70.0, [64, 64]).with_roll(f32::NAN),
            WorldView::new(
                6,
                DVec3::new(f64::INFINITY, 0.0, 0.0),
                0.0,
                0.0,
                70.0,
                [64, 64],
            ),
        ];
        asked.extend((10..20).map(|id| view(id, 170.0, [1, MAX_WORLD_VIEW_SIDE])));
        let drawn: Vec<u64> = drawable_world_views(asked, MAX_WORLD_VIEW_SIDE)
            .iter()
            .map(|v| v.id)
            .collect();
        // the first id 1 only; the out-of-range ones left out; at most four in all
        assert_eq!(drawn, [1, 10, 11, 12]);
        // A device with smaller textures (downlevel, GL: 2048) leaves out what it cannot hold.
        let small = vec![view(20, 70.0, [2049, 64]), view(21, 70.0, [2048, 2048])];
        let drawn: Vec<u64> = drawable_world_views(small, 2048)
            .iter()
            .map(|v| v.id)
            .collect();
        assert_eq!(drawn, [21]);
    }
}
