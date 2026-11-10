//! Keyframe animations (vanilla's `AnimationDefinition`, `AnimationChannel`, `Keyframe`,
//! `KeyframeAnimation` and `AnimationState`) as plain data and pure functions. The game samples its
//! own mobs' animations through [`AnimationChannel::sample`], and a mod plays the files it ships
//! (`RenderAssets::animation`) on its own `ModelPart` with the same code, natively or in wasm.
use crate::ModelPart;

#[cfg(test)]
#[path = "keyframe_animation_tests.rs"]
mod tests;

/// What a channel moves (`AnimationChannel.Targets`): its sampled value is added to the part's
/// position, rotation or scale (`ModelPart.offsetPos`, `offsetRotation`, `offsetScale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the three built-in targets; registered ones come with #880
#[derive(horizon_mod_api_macros::Wire)]
pub enum AnimationTarget {
    /// The part's position, in texels (`minecraft:position`).
    Position,
    /// The part's rotation, in radians (`minecraft:rotation`).
    Rotation,
    /// The part's scale, as an offset from 1 (`minecraft:scale`).
    Scale,
}

/// How a channel reaches a keyframe from the one before (`AnimationChannel.Interpolations`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two built-in interpolations; registered ones come with #880
#[derive(horizon_mod_api_macros::Wire)]
pub enum Interpolation {
    /// A straight line from the previous keyframe (`minecraft:linear`).
    Linear,
    /// A Catmull-Rom spline through the neighbouring keyframes (`minecraft:catmullrom`).
    CatmullRom,
}

/// One keyframe (`Keyframe`): at `time` seconds, the value arriving (`pre`) and the value leaving
/// (`post`), and how the channel moves into it.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
#[derive(horizon_mod_api_macros::Wire)]
pub struct Keyframe {
    /// When, in seconds from the animation's start.
    pub time: f32,
    /// The value the channel arrives at.
    pub pre: [f32; 3],
    /// The value it leaves from.
    pub post: [f32; 3],
    /// How the channel moves into this keyframe from the one before.
    pub interpolation: Interpolation,
}

/// One bone's keyframes for one target (`AnimationChannel`), in time order.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnimationChannel {
    /// What it moves.
    pub target: AnimationTarget,
    /// Its keyframes, in time order.
    pub keyframes: Vec<Keyframe>,
}

impl AnimationChannel {
    /// `KeyframeAnimation.Entry.apply`'s interpolation: the channel's value at `seconds`, times
    /// `scale`. Before the first keyframe it holds the first one, after the last the last.
    pub fn sample(&self, seconds: f32, scale: f32) -> [f32; 3] {
        let k = &self.keyframes;
        if k.is_empty() {
            return [0.0; 3];
        }
        // Mth.binarySearch(0, len, i -> seconds <= k[i].time) - 1
        let first = k.iter().position(|f| seconds <= f.time).unwrap_or(k.len());
        let prev = first.saturating_sub(1);
        let next = (prev + 1).min(k.len() - 1);
        let delta = seconds - k[prev].time;
        let alpha = if next != prev {
            (delta / (k[next].time - k[prev].time)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        match k[next].interpolation {
            Interpolation::CatmullRom => {
                let p0 = k[prev.saturating_sub(1)].post;
                let p1 = k[prev].post;
                let p2 = k[next].post;
                let p3 = k[(next + 1).min(k.len() - 1)].post;
                std::array::from_fn(|i| catmullrom(alpha, p0[i], p1[i], p2[i], p3[i]) * scale)
            }
            Interpolation::Linear => {
                let a = k[prev].post;
                let b = k[next].pre;
                std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * alpha) * scale)
            }
        }
    }
}

/// `Mth.catmullrom`.
fn catmullrom(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

/// An animation as its file gives it, bones by name (vanilla's `AnimationDefinition`; NeoForge's
/// `AnimationHolder`): what `RenderAssets::animation` answers. Bake it against a model with
/// [`AnimationHolder::bake`] before playing it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds
#[derive(horizon_mod_api_macros::Wire)]
pub struct AnimationHolder {
    /// The animation's length in seconds.
    pub length: f32,
    /// Whether it starts over at the end (`loop`).
    pub looping: bool,
    /// Each bone's channels, by the part's name, in file order.
    pub bones: Vec<(String, Vec<AnimationChannel>)>,
}

impl AnimationHolder {
    /// `AnimationDefinition.bake(root)`: each bone resolved once through the model's part lookup
    /// (`ModelPart.createPartLookup`). The name `root` always selects the actual root; other names
    /// select the first match, visiting each part's immediate children before their descendants.
    /// Bake once, as a renderer does in its constructor, and keep the result: a bone the model
    /// lacks is logged here and left out.
    pub fn bake(&self, root: &ModelPart) -> KeyframeAnimation {
        let mut entries = Vec::new();
        for (bone, channels) in &self.bones {
            match root.descendant_path(bone) {
                Some(path) => entries.extend(channels.iter().map(|c| (path.clone(), c.clone()))),
                None => log::warn!("keyframe animation: the model has no part {bone:?}; skipped"),
            }
        }
        KeyframeAnimation {
            length: self.length,
            looping: self.looping,
            entries,
        }
    }
}

/// An animation baked against a model (`KeyframeAnimation`): each channel holds the path to its
/// part, so playing it walks no names.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: built by `AnimationHolder::bake` only; its fields are private
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct KeyframeAnimation {
    pub(crate) length: f32,
    pub(crate) looping: bool,
    pub(crate) entries: Vec<(Vec<u32>, AnimationChannel)>,
}

impl KeyframeAnimation {
    /// `KeyframeAnimation.apply(state, ageInTicks)`: while `state` is started, the pose
    /// `age_in_ticks` gives, added to `root`'s parts.
    pub fn apply(&self, root: &mut ModelPart, state: &AnimationState, age_in_ticks: f32) {
        self.apply_scaled(root, state, age_in_ticks, 1.0);
    }
    /// `KeyframeAnimation.apply(state, ageInTicks, speedFactor)`: as [`Self::apply`], played
    /// `speed_factor` times as fast.
    pub fn apply_scaled(
        &self,
        root: &mut ModelPart,
        state: &AnimationState,
        age_in_ticks: f32,
        speed_factor: f32,
    ) {
        if state.is_started() {
            let millis = (state.time_in_millis(age_in_ticks) as f32 * speed_factor) as i64;
            self.apply_at(root, millis, 1.0);
        }
    }
    /// `KeyframeAnimation.applyWalk(walkAnimationPos, walkAnimationSpeed, speedFactor,
    /// scaleFactor)`: the animation timed by a walk cycle, its strength by the walk speed.
    pub fn apply_walk(
        &self,
        root: &mut ModelPart,
        walk_pos: f32,
        walk_speed: f32,
        speed_factor: f32,
        scale_factor: f32,
    ) {
        let millis = (walk_pos * 50.0 * speed_factor) as i64;
        let strength = walk_speed * scale_factor;
        // Java Math.min propagates NaN; Rust's f32::min would turn it into full strength.
        let scale = if strength.is_nan() {
            strength
        } else {
            strength.min(1.0)
        };
        self.apply_at(root, millis, scale);
    }
    /// `KeyframeAnimation.applyStatic()`: the first frame only.
    pub fn apply_static(&self, root: &mut ModelPart) {
        self.apply_at(root, 0, 1.0);
    }
    /// `KeyframeAnimation.apply(millis, scale)`.
    fn apply_at(&self, root: &mut ModelPart, millis: i64, scale: f32) {
        // getElapsedSeconds: a looping animation wraps by its length.
        let mut seconds = millis as f32 / 1000.0;
        if self.looping {
            seconds %= self.length;
        }
        for (path, channel) in &self.entries {
            let Some(part) = root.descendant_mut(path) else {
                continue;
            };
            let value = channel.sample(seconds, scale);
            match channel.target {
                AnimationTarget::Position => part.offset_pos(value),
                AnimationTarget::Rotation => part.offset_rotation(value),
                AnimationTarget::Scale => part.offset_scale(value),
            }
        }
    }
}

/// When an animation started (vanilla's `AnimationState`): a start tick, or stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its field is private; it changes through vanilla's transitions
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct AnimationState {
    pub(crate) start_tick: i32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            start_tick: Self::STOPPED,
        }
    }
}

impl AnimationState {
    /// `AnimationState.STOPPED`.
    const STOPPED: i32 = i32::MIN;
    /// `new AnimationState()`: stopped.
    pub fn new() -> Self {
        Self::default()
    }
    /// `AnimationState.start(tickCount)`.
    pub fn start(&mut self, tick_count: i32) {
        self.start_tick = tick_count;
    }
    /// `AnimationState.startIfStopped(tickCount)`.
    pub fn start_if_stopped(&mut self, tick_count: i32) {
        if !self.is_started() {
            self.start(tick_count);
        }
    }
    /// `AnimationState.animateWhen(condition, tickCount)`: started while `condition` holds,
    /// stopped otherwise.
    pub fn animate_when(&mut self, condition: bool, tick_count: i32) {
        if condition {
            self.start_if_stopped(tick_count);
        } else {
            self.stop();
        }
    }
    /// `AnimationState.stop()`.
    pub fn stop(&mut self) {
        self.start_tick = Self::STOPPED;
    }
    /// `AnimationState.fastForward(ticks, speed)`: a started animation is `ticks * speed` further.
    pub fn fast_forward(&mut self, ticks: i32, speed: f32) {
        if self.is_started() {
            // Java's int subtraction wraps; so does this, and it never panics.
            self.start_tick = self.start_tick.wrapping_sub((ticks as f32 * speed) as i32);
        }
    }
    /// `AnimationState.getTimeInMillis(ageInTicks)`: how long it has run, 50 ms a tick.
    pub fn time_in_millis(&self, age_in_ticks: f32) -> i64 {
        ((age_in_ticks - self.start_tick as f32) * 50.0) as i64
    }
    /// `AnimationState.isStarted()`.
    pub fn is_started(&self) -> bool {
        self.start_tick != Self::STOPPED
    }
}
