//! Native mods' world views (`native_render::world_views`): the frame's world drawn again
//! from a camera of the mod's own into a target of its own, as the water mirror is: the same
//! section lists (culled for every view's frustum too), the same entity geometry, the frame's
//! sky and fog, through the frame's pipelines with the view's globals.

use super::{Cull, Frame, Frustum, Globals, Renderer, frustum, native_render};
use glam::{DVec3, Mat4, Vec3, Vec4};
use std::collections::HashMap;

/// This frame's world views (`native_render::world_views`, none on the title screen) and their
/// frusta, which the frame's section lists take in.
pub(super) struct Asked {
    pub views: Vec<native_render::WorldView>,
    pub frusta: Vec<Frustum>,
}

impl Asked {
    pub fn new(frame: &Frame, max_side: u32) -> Self {
        let views = if frame.title {
            Vec::new()
        } else {
            asked(max_side)
        };
        let frusta = views
            .iter()
            .map(|v| Frustum {
                planes: frustum(ViewCamera::new(v, frame.camera, frame.depth_far).view_proj),
            })
            .collect();
        Self { views, frusta }
    }
    /// While a view is asked for, nothing is left out by what follows the player's camera alone:
    /// the caves, occlusion, faces turned away, GPU culling.
    pub fn on(&self) -> bool {
        !self.views.is_empty()
    }
}

/// `cull` together with the world views' frusta.
pub(super) struct Union<'a, C>(pub C, pub &'a [Frustum]);

impl<C: Cull> Cull for Union<'_, C> {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.0.intersects(min, max) || self.1.iter().any(|f| f.intersects(min, max))
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.0.contains(min, max) || self.1.iter().any(|f| f.contains(min, max))
    }
    fn tight(&self) -> bool {
        self.0.tight()
    }
}

/// This frame's world views (`native_render::world_views`) that can be drawn at most `max_side`
/// pixels a side (`Renderer::world_view_side`); none, and no allocation, while no mod listens.
fn asked(max_side: u32) -> Vec<native_render::WorldView> {
    if !native_render::world_views_hooked() {
        return Vec::new();
    }
    let mut views = Vec::new();
    native_render::world_views(&mut views);
    native_render::drawable_world_views(views, max_side)
}

/// The camera's frustum the game culls entities with (`Renderer::camera_frustum`): one that takes
/// in everything while a mod asks for world views, which see the entities it would leave out.
pub(super) fn open_while_asked(camera: Frustum, max_side: u32) -> Frustum {
    if asked(max_side).is_empty() {
        camera
    } else {
        Frustum {
            planes: [Vec4::W; 6],
        }
    }
}

/// A view's camera in the frame's camera-relative world: the frame's geometry is placed relative
/// to the player's camera, so the view's matrices take the offset to its own camera in.
pub(super) struct ViewCamera {
    /// The frame's camera-relative world to the view's clip space.
    pub view_proj: Mat4,
    /// The view's own camera-relative world to its clip space (`WorldViewDrawn::view_proj`).
    pub own_view_proj: Mat4,
    /// The view's eye, relative to the frame's camera.
    pub offset: Vec3,
    view: Mat4,
    aspect: f32,
}

impl ViewCamera {
    pub fn new(view: &native_render::WorldView, camera: DVec3, far: f32) -> Self {
        let aspect = view.size[0] as f32 / view.size[1] as f32;
        let rotation = crate::camera::view_matrix_rolled(view.yaw, view.pitch, view.roll);
        let projection = super::world_projection(view.fov_degrees, aspect, far);
        let offset = (view.camera - camera).as_vec3();
        let own_view_proj = projection * rotation;
        Self {
            view_proj: own_view_proj * Mat4::from_translation(-offset),
            own_view_proj,
            offset,
            view: rotation * Mat4::from_translation(-offset),
            aspect,
        }
    }
}

/// A view's targets, kept while it is asked for each frame.
pub(super) struct Target {
    size: [u32; 2],
    format: wgpu::TextureFormat,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

#[derive(Default)]
pub(super) struct Targets(HashMap<u64, Target>);

/// What of the frame the views draw.
struct Lists {
    pub globals: Globals,
    pub clear: wgpu::Color,
    pub sky: bool,
    pub flat: bool,
    pub hdr: bool,
    pub solid_draws: u32,
    pub translucent_draws: u32,
    pub entity_quads: u32,
}

impl Renderer {
    /// Draws each of `views` and hands it to `native_world_view_drawn`. The views'
    /// globals go in the frame's own buffer, its bind groups with it, and the frame's are put
    /// back after them, all in `encoder`'s order.
    /// `globals` are the frame's, `flat` whether it drew flat, `counts` its solid and translucent
    /// section draws and entity quads.
    pub(super) fn draw_world_views(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        views: &[native_render::WorldView],
        globals: Globals,
        flat: bool,
        [solid_draws, translucent_draws, entity_quads]: [u32; 3],
    ) {
        let [r, g, b, _] = globals.fog_color.map(f64::from);
        let lists = &Lists {
            globals,
            // as the world pass clears, but for a shader pack's own sky
            clear: if frame.end_sky && !frame.underwater && super::sky_open(frame) {
                wgpu::Color::BLACK
            } else {
                wgpu::Color { r, g, b, a: 1.0 }
            },
            sky: super::sky_visible(frame),
            flat,
            hdr: self.hdr.is_some(),
            solid_draws,
            translucent_draws,
            entity_quads,
        };
        let format = self.output.frame.format;
        let mut targets = std::mem::take(&mut self.world_views.0);
        targets.retain(|id, _| views.iter().any(|v| v.id == *id));
        for view in views {
            let target = targets
                .entry(view.id)
                .and_modify(|t| {
                    if t.size != view.size || t.format != format {
                        *t = Target::new(&self.device, view.size, format);
                    }
                })
                .or_insert_with(|| Target::new(&self.device, view.size, format));
            let camera = ViewCamera::new(view, frame.camera, frame.depth_far);
            let globals = Globals {
                view_proj: camera.view_proj.to_cols_array_2d(),
                inverse_view: camera.view.inverse().to_cols_array_2d(),
                misc: [
                    lists.globals.misc[0],
                    lists.globals.misc[1],
                    camera.aspect,
                    (view.fov_degrees.to_radians() / 2.0).tan(),
                ],
                outline_params: super::outline_params(view.size[0], view.size[1]),
                view_origin: [camera.offset.x, camera.offset.y, camera.offset.z, 0.0],
                ..lists.globals
            };
            self.uploads
                .buffer(encoder, &self.globals, 0, bytemuck::bytes_of(&globals));
            self.draw_world_view(encoder, target, frame, lists);
            native_render::world_view_drawn(&mut native_render::WorldViewDrawn::from(
                mod_render::render::built::WorldViewDrawn {
                    id: view.id,
                    device: &self.device,
                    queue: &self.queue,
                    encoder,
                    color: &target.color,
                    color_view: &target.color_view,
                    color_format: format,
                    size: view.size,
                    depth: &target.depth,
                    view_proj: camera.own_view_proj,
                    camera: view.camera,
                },
            ));
        }
        if !views.is_empty() {
            self.uploads.buffer(
                encoder,
                &self.globals,
                0,
                bytemuck::bytes_of(&lists.globals),
            );
        }
        self.world_views.0 = targets;
    }

    /// The sky, the solid terrain, the entities and the translucent terrain, in the world
    /// pass's order, without what follows one camera alone (the sunrise's mesh, the sun and
    /// moon, stars, clouds, particles, weather, the block outline, the hand).
    fn draw_world_view(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &Target,
        frame: &Frame,
        lists: &Lists,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Native world view"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(lists.clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.group, &[]);
        if lists.sky {
            pass.set_pipeline(&self.sky);
            pass.draw(0..3, 0..1);
        }
        let flat = self.flat.as_ref().filter(|_| lists.flat);
        let vertices = || super::flat_vertices(&self.arena, self.translucent_arena.as_ref());
        match flat {
            Some(flat) => {
                pass.set_pipeline(flat.solid.get(lists.hdr));
                flat.draw(&mut pass, &flat.calls[..flat.solid_calls], vertices());
                pass.set_bind_group(0, &self.group, &[]);
            }
            None => {
                pass.set_pipeline(self.solid.get(lists.hdr));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_solid_sections(&mut pass, &self.indirect, 0, 0, lists.solid_draws);
            }
        }
        // The game's block entity sections, which take in what the views see while any is asked for.
        let static_entities = frame
            .visible_static_block_entity_sections
            .iter()
            .filter_map(|section| self.static_entity_sections.get(section));
        if !frame.visible_static_block_entity_sections.is_empty() {
            pass.set_pipeline(&self.static_entity_pipeline);
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_bind_group(1, &self.static_entity_group, &[]);
            pass.set_index_buffer(
                self.static_entity_indices.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            for gpu in static_entities {
                let quads = gpu.vertices / 4;
                if quads > 0 {
                    pass.set_vertex_buffer(0, gpu.buffer.slice(..));
                    pass.set_vertex_buffer(1, gpu.origin_buffer.slice(..));
                    pass.draw_indexed(0..quads * 6, 0, 0..1);
                }
            }
        }
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        if lists.entity_quads > 0 {
            pass.set_pipeline(&self.entity_pipeline);
            pass.set_bind_group(0, &self.entity_group, &[]);
            pass.set_vertex_buffer(0, self.entity_buffer.slice(..));
            pass.draw_indexed(0..lists.entity_quads * 6, 0, 0..1);
        }
        pass.set_bind_group(0, &self.group, &[]);
        match flat {
            Some(flat) => {
                pass.set_pipeline(&flat.translucent);
                flat.draw(&mut pass, flat.translucent_calls(), vertices());
            }
            None => {
                pass.set_pipeline(&self.translucent);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                self.draw_sections(
                    &mut pass,
                    &self.indirect,
                    0,
                    lists.solid_draws,
                    lists.translucent_draws,
                );
            }
        }
    }
}

impl Target {
    fn new(device: &wgpu::Device, [width, height]: [u32; 2], format: wgpu::TextureFormat) -> Self {
        let texture = |label, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let sampled = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let color = texture("Native world view", format, sampled);
        let depth = texture(
            "Native world view depth",
            wgpu::TextureFormat::Depth32Float,
            sampled,
        );
        Self {
            size: [width, height],
            format,
            color_view: color.create_view(&Default::default()),
            color,
            depth: depth.create_view(&Default::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view at the frame's camera sees as a screen view would; one elsewhere sees a point
    /// where its own camera-relative matrix does.
    #[test]
    fn a_view_sees_the_world_from_its_own_camera() {
        let frame_camera = DVec3::new(100.5, 70.0, -20.25);
        let at = |camera, yaw, pitch| {
            native_render::WorldView::new(1, camera, yaw, pitch, 70.0, [320, 180])
        };
        let here = ViewCamera::new(&at(frame_camera, 30.0, 10.0), frame_camera, 512.0);
        let screen = super::super::View::screen(30.0, 10.0, 70.0, 320.0 / 180.0, 512.0);
        assert!(
            here.view_proj
                .abs_diff_eq(screen.view_proj(Mat4::IDENTITY), 1e-6)
        );
        // a camera 40 blocks east looking west (yaw 90) sees the frame camera's point dead ahead
        let east = ViewCamera::new(
            &at(frame_camera + DVec3::X * 40.0, 90.0, 0.0),
            frame_camera,
            512.0,
        );
        let clip = east.view_proj * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(
            clip.x.abs() / clip.w < 1e-5 && clip.y.abs() / clip.w < 1e-5,
            "{clip}"
        );
        assert!(clip.w > 39.9 && clip.w < 40.1, "40 blocks in front: {clip}");
        // and the same point relative to its own camera through `own_view_proj`
        let own = east.own_view_proj * glam::Vec4::new(-40.0, 0.0, 0.0, 1.0);
        assert!((own - clip).abs().max_element() < 1e-3);
        // a roll turns the picture about the view direction: 90 counterclockwise puts up on the left
        let rolled = ViewCamera::new(
            &at(frame_camera, 0.0, 0.0).with_roll(90.0),
            frame_camera,
            512.0,
        );
        let up = rolled.view_proj * glam::Vec4::new(0.0, 1.0, 10.0, 1.0);
        assert!(up.x < -0.05 * up.w && (up.y / up.w).abs() < 1e-5, "{up}");
    }
}
