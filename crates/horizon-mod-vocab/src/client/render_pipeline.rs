//! Render pipelines a client mod registers as plain data, and the quads it draws with them.
use crate::{Identifier, WorldDepth};

/// A pipeline registered by `Hooks::register_render_pipelines`: a fragment shader from the
/// mod's assets and the fixed state it draws with. This is a restricted form: the mod supplies
/// the fragment stage and the host fixes the rest. Vanilla requires a vertex shader and a
/// primitive type; this API fixes both. The engine builds every GPU object.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct RenderPipeline {
    /// The pipeline's own name (`mymod:glow`), in a namespace the mod owns.
    pub id: Identifier,
    /// The fragment shader (`mymod:glow` is `assets/mymod/shaders/pipeline/glow.wgsl`), as the
    /// mod's assets or a resource pack supply it.
    pub fragment: Identifier,
    /// What the pipeline's draws are made of.
    pub vertex_format: PipelineVertexFormat,
    /// How the shader's result joins the frame.
    pub blend: PipelineBlend,
    /// Whether nearer world surfaces hide the draw. No pipeline writes depth.
    pub depth: WorldDepth,
}
impl RenderPipeline {
    /// Describe a pipeline that draws [`PipelineVertexFormat::PositionTexColor`] geometry.
    pub fn new(
        id: Identifier,
        fragment: Identifier,
        blend: PipelineBlend,
        depth: WorldDepth,
    ) -> Self {
        Self {
            id,
            fragment,
            vertex_format: PipelineVertexFormat::PositionTexColor,
            blend,
            depth,
        }
    }
}

/// The vertex data a pipeline's draws carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PipelineVertexFormat {
    /// A position, a texture coordinate and a colour: what [`PipelineQuad`] gives.
    PositionTexColor,
}

/// How a pipeline's fragments join the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PipelineBlend {
    /// Laid over the frame by the fragment's alpha, as [`crate::WorldQuad`] is.
    Translucent,
    /// Added to the frame, scaled by the fragment's alpha.
    Additive,
}

/// A registered pipeline as the current resources built it, resolved through
/// [`crate::RenderAssets::render_pipeline`]. Retain it between frames; resolve it again after a
/// successful resource reload.
// no wire: decoding from_parts rejects generation zero (an unpublished pipeline handle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: private fields; only the engine resolves pipeline handles
pub struct RenderPipelineRef(pub(crate) u64, pub(crate) u32);

impl RenderPipelineRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished pipeline generation");
        }
        Ok(Self(generation, slot))
    }
}

/// Why a pipeline could not be resolved or drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RenderPipelineError {
    /// No mod registered a pipeline of this id.
    Unknown,
    /// The pipeline is registered, but the current resources give it no usable shader. The log
    /// names the pipeline and the reason.
    Invalid,
    /// A successful resource reload superseded the handle's generation.
    Stale,
}

/// A two-sided quad drawn by a mod pipeline. Corners run top left, bottom left, bottom right,
/// top right as seen from the front.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PipelineQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Each corner's texture coordinate, handed to the shader as given.
    pub uv: [[f32; 2]; 4],
    /// Packed 0xAARRGGBB colour in vanilla's space; the shader reads it as linear light.
    pub color: u32,
}
impl PipelineQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], uv: [[f32; 2]; 4], color: u32) -> Self {
        Self { corners, uv, color }
    }
    /// A quad whose texture coordinates span 0 to 1: (0, 0) at the top left corner and (1, 1)
    /// at the bottom right.
    pub fn spanning(corners: [[f64; 3]; 4], color: u32) -> Self {
        Self::new(
            corners,
            [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]],
            color,
        )
    }
}

/// Why a whole pipeline batch was refused; nothing from that batch was appended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PipelineDrawError {
    /// The handle is stale, or names no usable pipeline.
    Pipeline(RenderPipelineError),
    /// A coordinate is not finite, or a camera-relative coordinate overflows f32.
    NonFinite,
    /// The batch would exceed this view's quad capacity, which pipeline quads share with
    /// [`crate::WorldDrawContext::quads`].
    CapacityExceeded,
}

/// What `owner` added to `pipelines` in `register_render_pipelines`, checked as it returns: a
/// pipeline outside the mod's namespaces or whose id an earlier registration took is logged
/// with the mod and dropped.
pub(crate) fn validate_render_pipelines(
    owner: &str,
    previous: &[RenderPipeline],
    pipelines: &mut Vec<RenderPipeline>,
) {
    validate_in(owner, previous, pipelines, &|namespace| {
        crate::parts::owns_namespace(owner, namespace)
    });
}

fn validate_in(
    owner: &str,
    previous: &[RenderPipeline],
    pipelines: &mut Vec<RenderPipeline>,
    owns: &dyn Fn(&str) -> bool,
) {
    let added = pipelines.split_off(previous.len().min(pipelines.len()));
    for pipeline in added {
        let refusal = if !owns(pipeline.id.namespace()) {
            Some("its id is outside the mod's namespaces")
        } else if pipelines.iter().any(|kept| kept.id == pipeline.id) {
            Some("an earlier registration has that id")
        } else {
            None
        };
        match refusal {
            Some(reason) => log::warn!(
                "register_render_pipelines: mod {owner}: pipeline {} refused: {reason}",
                pipeline.id
            ),
            None => {
                pipelines.push(pipeline);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(id: &str) -> RenderPipeline {
        let id = Identifier::parse(id).unwrap();
        RenderPipeline::new(
            id.clone(),
            id,
            PipelineBlend::Translucent,
            WorldDepth::Tested,
        )
    }
    fn ids(pipelines: &[RenderPipeline]) -> Vec<String> {
        pipelines.iter().map(|p| p.id.to_string()).collect()
    }

    #[test]
    fn a_mod_keeps_only_new_pipelines_in_its_own_namespace() {
        let previous = vec![pipeline("other:glow")];
        let mut pipelines = previous.clone();
        pipelines.extend([
            pipeline("mine:glow"),
            pipeline("other:beam"),
            pipeline("mine:glow"),
            pipeline("mine:beam"),
        ]);
        validate_in("mine", &previous, &mut pipelines, &|namespace| {
            namespace == "mine"
        });
        assert_eq!(ids(&pipelines), ["other:glow", "mine:glow", "mine:beam"]);
    }

    #[test]
    fn a_mod_cannot_take_an_id_registered_before_it() {
        let previous = vec![pipeline("shared:glow")];
        let mut pipelines = previous.clone();
        pipelines.push(pipeline("shared:glow"));
        validate_in("late", &previous, &mut pipelines, &|_| true);
        assert_eq!(pipelines, previous);
    }

    #[test]
    fn a_mod_can_register_more_than_vanillas_pipeline_count() {
        let mut pipelines: Vec<_> = (0..131).map(|i| pipeline(&format!("mine:p{i}"))).collect();
        validate_in("mine", &[], &mut pipelines, &|_| true);
        assert_eq!(pipelines.len(), 131);
        assert_eq!(pipelines.last().unwrap().id.to_string(), "mine:p130");

        let previous = pipelines.clone();
        pipelines.push(pipeline("next:glow"));
        validate_in("next", &previous, &mut pipelines, &|_| true);
        assert_eq!(pipelines.len(), 132);
    }
}
