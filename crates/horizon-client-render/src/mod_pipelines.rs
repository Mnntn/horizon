//! Render pipelines mods register (`Hooks::register_render_pipelines`, NeoForge's
//! `RegisterRenderPipelinesEvent`): the registrations, kept once at start, and what each
//! resource load makes of them. A load reads every registered pipeline's fragment file
//! (`assets/<namespace>/shaders/pipeline/<path>.wgsl`), puts Horizon's prelude before it and its
//! entry points after it, and checks the whole shader, so the renderer (`gfx::mod_pipelines`) is
//! only ever handed WGSL that compiles. A frame reads none of this by name.
//!
//! Colour follows the mod render colour rule (docs/modding/src/concepts/mod-render-colour.md):
//! the mod's function reads and returns linear light, and the generated entry points convert
//! from the quad's vanilla-space colour and to the frame, which stores vanilla's 2.2 encoding
//! in every format the renderer draws today.
use mod_api::{
    Identifier, PipelineBlend, RenderPipeline, RenderPipelineError, RenderPipelineRef, WorldDepth,
};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::sync::OnceLock;

/// The folder under each namespace's `shaders` the fragment files are read from.
const FOLDER: &str = "shaders/pipeline";

/// The function a pipeline's file defines.
const FUNCTION: &str = "pipeline_fragment";

/// What a pipeline's file can use, before it in the shader. Names starting `horizon_` are the
/// renderer's own.
const PRELUDE_TYPES: &str = "\
// What `pipeline_fragment` is given for each fragment.
struct PipelineInput {
    // The quad's texture coordinate here, as its corners gave it.
    uv: vec2<f32>,
    // The quad's colour in linear light; alpha is coverage.
    color: vec4<f32>,
    // This point in blocks, relative to the camera.
    position: vec3<f32>,
}
// The frame's values, the same for every fragment.
struct PipelineFrame {
    // The world picture's size in pixels.
    viewport: vec2<f32>,
    // Seconds of game time, smooth between ticks, wrapping to 0 every hour.
    time: f32,
}
@group(0) @binding(0) var<uniform> horizon_pipeline_view: mat4x4<f32>;
@group(0) @binding(1) var<uniform> frame: PipelineFrame;
";

/// The entry points, after the pipeline's file: the vertex stage, and a fragment stage that
/// hands the mod linear light and stores its answer as the frame holds colour.
const ENTRIES: &str = "
struct HorizonPipelineVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) position: vec3<f32>,
}
@vertex fn horizon_vs_pipeline(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>) -> HorizonPipelineVertex {
    var out: HorizonPipelineVertex;
    out.clip = horizon_pipeline_view * vec4<f32>(position, 1.0);
    out.uv = uv;
    out.color = color;
    out.position = position;
    return out;
}
@fragment fn horizon_fs_pipeline(in: HorizonPipelineVertex) -> @location(0) vec4<f32> {
    let result = pipeline_fragment(PipelineInput(in.uv, horizon_decode_rgba(in.color), in.position));
    // Coverage only: no light rides in the frame's alpha, and NaN covers nothing.
    let coverage = clamp(select(0.0, result.a, result.a > 0.0), 0.0, 1.0);
    return vec4<f32>(horizon_encode(result.rgb), coverage);
}
";

/// The renderer's entry points in a built pipeline's shader.
pub const VERTEX_ENTRY: &str = "horizon_vs_pipeline";
pub const FRAGMENT_ENTRY: &str = "horizon_fs_pipeline";

static REGISTERED: OnceLock<Vec<RenderPipeline>> = OnceLock::new();

/// Publish the mods' registrations once at startup, as `register_render_pipelines` left them:
/// each id once, in a namespace its mod owns. A pipeline's place in the list is its slot.
pub fn register(pipelines: Vec<RenderPipeline>) {
    if REGISTERED.set(pipelines).is_err() {
        log::warn!("Render pipelines already registered; keeping the first list");
    }
}

/// The registered pipelines; none until registration publishes them.
fn registered() -> &'static [RenderPipeline] {
    REGISTERED.get().map_or(&[], Vec::as_slice)
}

/// A registered pipeline one resource load built: a shader that compiles and its fixed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The whole shader: prelude, the mod's file, entry points.
    pub shader: String,
    pub blend: PipelineBlend,
    pub depth: WorldDepth,
}

/// The registered pipelines as one resource load built them, by slot. A pipeline the load
/// refused has no shader here and resolves to [`RenderPipelineError::Invalid`].
#[derive(Clone, Debug, Default)]
pub struct Pipelines {
    ids: Vec<Identifier>,
    built: Vec<Option<Built>>,
    /// The render assets' generation this set was loaded for; 0 for none.
    generation: u64,
    invalid: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
}

impl Pipelines {
    /// Build every registered pipeline from `source`, for the load of `generation`. A pipeline
    /// whose file is missing or refused is logged with its reason; the others build.
    pub fn load(source: &crate::resource_source::ResourceSource, generation: u64) -> Self {
        Self::build(registered(), generation, |file| {
            source.read_to_string(file).map_err(|e| e.to_string())
        })
    }

    pub(crate) fn build(
        registered: &[RenderPipeline],
        generation: u64,
        read: impl Fn(&str) -> Result<String, String>,
    ) -> Self {
        let built = registered
            .iter()
            .map(|pipeline| {
                let file = fragment_file(&pipeline.fragment);
                match read(&file).and_then(|text| compose(&text)) {
                    Ok(shader) => Some(Built {
                        shader,
                        blend: pipeline.blend,
                        depth: pipeline.depth,
                    }),
                    Err(reason) => {
                        log::warn!(
                            "Horizon: render pipeline {} refused: {file}: {reason}",
                            pipeline.id
                        );
                        None
                    }
                }
            })
            .collect();
        Self {
            ids: registered.iter().map(|p| p.id.clone()).collect(),
            built,
            generation,
            invalid: Default::default(),
        }
    }

    pub(crate) fn invalidate(&self, slot: usize) {
        self.invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(slot);
    }

    /// The load this set belongs to: a renderer holding another builds its pipelines again.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every slot in order, with what the load built for it.
    pub fn built(&self) -> &[Option<Built>] {
        &self.built
    }

    /// The handle of the registered pipeline `id` in this load.
    pub fn resolve(&self, id: &Identifier) -> Result<RenderPipelineRef, RenderPipelineError> {
        let slot = self
            .ids
            .iter()
            .position(|registered| registered == id)
            .ok_or(RenderPipelineError::Unknown)?;
        if self.built[slot].is_none()
            || self
                .invalid
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&slot)
        {
            return Err(RenderPipelineError::Invalid);
        }
        Ok(mod_api::__private::built::render_pipeline_ref(
            self.generation,
            slot as u32,
        ))
    }

    /// The slot a handle draws with, checked once when a batch is submitted.
    pub fn slot(&self, pipeline: RenderPipelineRef) -> Result<u32, RenderPipelineError> {
        use mod_api::__private::game::Fields;
        let fields = pipeline.fields();
        if fields.generation != self.generation {
            return Err(RenderPipelineError::Stale);
        }
        if self
            .invalid
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(fields.slot as usize))
        {
            return Err(RenderPipelineError::Invalid);
        }
        match self.built.get(fields.slot as usize) {
            Some(Some(_)) => Ok(fields.slot),
            Some(None) => Err(RenderPipelineError::Invalid),
            None => Err(RenderPipelineError::Unknown),
        }
    }
}

/// `mymod:beam/glow` is `assets/mymod/shaders/pipeline/beam/glow.wgsl`.
fn fragment_file(fragment: &Identifier) -> String {
    format!(
        "assets/{}/{FOLDER}/{}.wgsl",
        fragment.namespace(),
        fragment.path()
    )
}

/// The prelude a pipeline's file follows.
fn prelude() -> String {
    format!(
        "{PRELUDE_TYPES}{}",
        horizon_shader_api::stages::COLOR_HELPERS
    )
}

/// The whole shader of a pipeline whose file is `text`, or why the file is refused, with the
/// file's own line where there is one.
pub fn compose(text: &str) -> Result<String, String> {
    let prelude = prelude();
    let mut shader = format!("{prelude}{text}\n");
    let located = |at: Option<usize>, message: String| match at
        .and_then(|at| at.checked_sub(prelude.len()))
        .filter(|at| *at <= text.len())
    {
        Some(at) => format!(
            "line {}: {message}",
            text.as_bytes()[..at]
                .iter()
                .filter(|b| **b == b'\n')
                .count()
                + 1
        ),
        None => message,
    };
    // First without the entry points, so a missing or mistyped function is reported as such
    // rather than as a call that doesn't resolve.
    let module = naga::front::wgsl::parse_str(&shader).map_err(|e| {
        let at = e
            .labels()
            .next()
            .and_then(|(span, _)| span.to_range())
            .map(|range| range.start);
        located(at, e.message().to_owned())
    })?;
    for (_, function) in module.functions.iter() {
        if function.expressions.iter().any(|(_, expression)| matches!(expression,
            naga::Expression::GlobalVariable(handle) if module.global_variables[*handle].name.as_deref() == Some("horizon_pipeline_view"))) {
            return Err("horizon_pipeline_view is vertex-only".into());
        }
    }
    if !module.overrides.is_empty() {
        return Err("override declarations are not supported".into());
    }
    signature(&module)?;
    if !module.entry_points.is_empty() {
        return Err("it declares an entry point; the game supplies both stages".into());
    }
    if module
        .global_variables
        .iter()
        .filter(|(_, variable)| variable.binding.is_some())
        .count()
        != 2
    {
        return Err("it declares a binding; a pipeline reads only `frame` and its input".into());
    }
    shader.push_str(ENTRIES);
    let module = naga::front::wgsl::parse_str(&shader).map_err(|e| e.message().to_owned())?;
    Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|e| {
            let at = e
                .spans()
                .next()
                .and_then(|(span, _)| span.to_range())
                .map(|range| range.start);
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(inner) = source {
                message += &format!(": {inner}");
                source = inner.source();
            }
            located(at, message)
        })?;
    Ok(shader)
}

/// `fn pipeline_fragment(input: PipelineInput) -> vec4<f32>`, or what the file has instead.
fn signature(module: &naga::Module) -> Result<(), String> {
    let expected = format!("expected `fn {FUNCTION}(input: PipelineInput) -> vec4<f32>`");
    let (_, function) = module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some(FUNCTION))
        .ok_or_else(|| format!("no function `{FUNCTION}`: {expected}"))?;
    let takes_input = matches!(
        function.arguments.as_slice(),
        [argument] if module.types[argument.ty].name.as_deref() == Some("PipelineInput")
    );
    let returns_color = function.result.as_ref().is_some_and(|result| {
        matches!(
            module.types[result.ty].inner,
            naga::TypeInner::Vector {
                size: naga::VectorSize::Quad,
                scalar: naga::Scalar::F32,
            }
        )
    });
    if takes_input && returns_color {
        Ok(())
    } else {
        Err(format!("`{FUNCTION}` has another signature: {expected}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_cannot_read_vertex_uniform() {
        assert!(compose("fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return horizon_pipeline_view * input.color; }").unwrap_err().contains("horizon_pipeline_view"));
    }

    #[test]
    fn overrides_without_defaults_are_refused() {
        assert!(compose("override strength: f32; fn pipeline_fragment(input: PipelineInput) -> vec4<f32> { return input.color * strength; }").unwrap_err().contains("override"));
    }

    const GLOW: &str = "\
fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {
    return vec4<f32>(input.color.rgb * (1.0 + sin(frame.time)), input.color.a * input.uv.y);
}
";

    fn pipeline(id: &str, fragment: &str) -> RenderPipeline {
        RenderPipeline::new(
            Identifier::parse(id).unwrap(),
            Identifier::parse(fragment).unwrap(),
            PipelineBlend::Additive,
            WorldDepth::ThroughWalls,
        )
    }
    fn id(text: &str) -> Identifier {
        Identifier::parse(text).unwrap()
    }

    #[test]
    fn a_file_with_the_function_composes_into_a_shader_with_both_entry_points() {
        let shader = compose(GLOW).unwrap();
        let module = naga::front::wgsl::parse_str(&shader).unwrap();
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(entries, [VERTEX_ENTRY, FRAGMENT_ENTRY]);
    }

    #[test]
    fn a_refused_file_says_what_is_wrong_and_where() {
        let refused = |text: &str| compose(text).unwrap_err();
        assert!(refused("").starts_with("no function `pipeline_fragment`"));
        assert!(
            refused("fn pipeline_fragment(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(1.0); }")
                .contains("another signature")
        );
        assert!(
            refused(
                "fn pipeline_fragment(input: PipelineInput) -> vec3<f32> { return input.position; }"
            )
            .contains("another signature")
        );
        // A syntax error is reported on the file's own line, not the composed shader's.
        let broken = "// a comment\nfn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    return vec4<f32>(1.0)\n}\n";
        let message = refused(broken);
        assert!(
            message.starts_with("line 3:") || message.starts_with("line 4:"),
            "{message}"
        );
        // A type error is found by validation, with the file's line too.
        let mistyped = "fn pipeline_fragment(input: PipelineInput) -> vec4<f32> {\n    let x: f32 = input.uv;\n    return vec4<f32>(x);\n}\n";
        assert!(
            refused(mistyped).starts_with("line 2:"),
            "{}",
            refused(mistyped)
        );
        assert!(
            refused(&format!(
                "@group(0) @binding(2) var<uniform> mine: vec4<f32>;\n{GLOW}"
            ))
            .contains("declares a binding")
        );
        assert!(
            refused(&format!(
                "{GLOW}@fragment fn mine() -> @location(0) vec4<f32> {{ return vec4<f32>(1.0); }}\n"
            ))
            .contains("declares an entry point")
        );
    }

    #[test]
    fn a_load_builds_what_compiles_and_names_the_rest_invalid() {
        let registered = [
            pipeline("example:glow", "example:glow"),
            pipeline("example:broken", "example:broken"),
            pipeline("example:missing", "example:nested/missing"),
        ];
        let read = |file: &str| match file {
            "assets/example/shaders/pipeline/glow.wgsl" => Ok(GLOW.to_owned()),
            "assets/example/shaders/pipeline/broken.wgsl" => Ok("fn nothing() {}".to_owned()),
            "assets/example/shaders/pipeline/nested/missing.wgsl" => Err("not found".to_owned()),
            other => panic!("read {other}"),
        };
        let set = Pipelines::build(&registered, 7, read);

        let glow = set.resolve(&id("example:glow")).unwrap();
        assert_eq!(set.slot(glow), Ok(0));
        let built = set.built()[0].as_ref().unwrap();
        assert_eq!(
            (built.blend, built.depth),
            (PipelineBlend::Additive, WorldDepth::ThroughWalls)
        );
        assert_eq!(
            set.resolve(&id("example:broken")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:missing")),
            Err(RenderPipelineError::Invalid)
        );
        assert_eq!(
            set.resolve(&id("example:never")),
            Err(RenderPipelineError::Unknown)
        );
    }

    #[test]
    fn a_reload_makes_old_handles_stale_and_can_fix_or_break_a_pipeline() {
        let registered = [pipeline("example:glow", "example:glow")];
        let before = Pipelines::build(&registered, 7, |_| Ok(GLOW.to_owned()));
        let handle = before.resolve(&id("example:glow")).unwrap();

        // A pack replaced the file with one that doesn't compile: the pipeline is invalid in
        // the new load, and the old handle is stale there, not silently redirected.
        let broken = Pipelines::build(&registered, 8, |_| Ok("garbage".to_owned()));
        assert_eq!(broken.slot(handle), Err(RenderPipelineError::Stale));
        assert_eq!(
            broken.resolve(&id("example:glow")),
            Err(RenderPipelineError::Invalid)
        );

        let fixed = Pipelines::build(&registered, 9, |_| Ok(GLOW.to_owned()));
        assert_eq!(fixed.slot(handle), Err(RenderPipelineError::Stale));
        let again = fixed.resolve(&id("example:glow")).unwrap();
        assert_eq!(fixed.slot(again), Ok(0));
    }
}
