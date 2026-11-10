use super::*;
use crate::{ModelPart, PartPose};

fn key(time: f32, value: [f32; 3], interpolation: Interpolation) -> Keyframe {
    Keyframe {
        time,
        pre: value,
        post: value,
        interpolation,
    }
}
fn channel(target: AnimationTarget, keyframes: Vec<Keyframe>) -> AnimationChannel {
    AnimationChannel { target, keyframes }
}
fn part(children: Vec<(&str, ModelPart)>) -> ModelPart {
    ModelPart::new(
        Vec::new(),
        children
            .into_iter()
            .map(|(name, child)| (name.to_owned(), child))
            .collect(),
        PartPose::ZERO,
    )
}

#[test]
fn linear_keyframes_interpolate_post_to_pre() {
    let c = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            key(1.0, [2.0, 4.0, 6.0], Interpolation::Linear),
        ],
    );
    assert_eq!(c.sample(0.5, 1.0), [1.0, 2.0, 3.0]);
    // Past the end it holds the last value, times the scale.
    assert_eq!(c.sample(3.0, 0.5), [1.0, 2.0, 3.0]);
    // Before the first, the first.
    assert_eq!(c.sample(-1.0, 1.0), [0.0; 3]);
    // A linear segment ends at the next keyframe's arriving value, not its leaving one.
    let jump = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            Keyframe {
                time: 1.0,
                pre: [2.0; 3],
                post: [10.0; 3],
                interpolation: Interpolation::Linear,
            },
        ],
    );
    assert_eq!(jump.sample(0.5, 1.0), [1.0; 3]);
}

#[test]
fn catmullrom_passes_through_keyframes() {
    let c = channel(
        AnimationTarget::Rotation,
        vec![
            key(0.0, [0.0; 3], Interpolation::CatmullRom),
            key(1.0, [1.0; 3], Interpolation::CatmullRom),
            key(2.0, [0.0; 3], Interpolation::CatmullRom),
        ],
    );
    assert!((c.sample(1.0, 1.0)[0] - 1.0).abs() < 1e-6);
    // Mth.catmullrom(0.5, 0, 0, 1, 0) = 0.5 * (1 + (0 + 4 - 0) * 0.25 + (0 - 0 - 3 + 0) * 0.125)
    assert_eq!(c.sample(0.5, 1.0)[0], 0.5625);
}

/// A bone resolves to the first part of its name, each part's children before its
/// grandchildren; a bone the model lacks is left out, and the rest still plays.
#[test]
fn bake_finds_the_first_part_of_a_name_and_drops_a_missing_bone() {
    let root = part(vec![
        ("body", part(vec![("arm", part(vec![]))])),
        ("arm", part(vec![])),
    ]);
    let holder = AnimationHolder {
        length: 1.0,
        looping: false,
        bones: vec![
            (
                "arm".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
                )],
            ),
            (
                "tail".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [9.0; 3], Interpolation::Linear)],
                )],
            ),
        ],
    };
    let baked = holder.bake(&root);
    assert_eq!(baked.entries.len(), 1, "the missing tail is dropped");
    let mut posed = root.clone();
    baked.apply_static(&mut posed);
    // The root's own `arm` (index 1) is found before body's.
    let arm = posed.get_child("arm").unwrap();
    assert_eq!((arm.x, arm.y, arm.z), (1.0, 2.0, 3.0));
    let inner = posed.get_child("body").unwrap().get_child("arm").unwrap();
    assert_eq!((inner.x, inner.y, inner.z), (0.0, 0.0, 0.0));
}

/// `apply` only plays a started state, at `(age - start) * 50` ms; a looping animation wraps
/// by its length; `apply_walk` times by `pos * 50 * speed_factor` and scales by
/// `min(speed * scale_factor, 1)`; every apply adds to the pose.
#[test]
fn apply_times_by_the_state_and_wraps_a_loop() {
    let root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        length: 2.0,
        looping: true,
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Rotation,
                vec![
                    key(0.0, [0.0; 3], Interpolation::Linear),
                    key(2.0, [4.0, 0.0, 0.0], Interpolation::Linear),
                ],
            )],
        )],
    };
    let baked = holder.bake(&root);
    let head_x = |root: &ModelPart| root.get_child("head").unwrap().x_rot;

    // Not looping, any time past the start would hold the last value, 4.0.
    let once = AnimationHolder {
        looping: false,
        ..holder.clone()
    }
    .bake(&root);
    let mut stopped = root.clone();
    once.apply(&mut stopped, &AnimationState::new(), 30.0);
    assert_eq!(head_x(&stopped), 0.0, "a stopped state plays nothing");

    let mut state = AnimationState::new();
    state.start(10);
    // 10 ticks after the start: 500 ms, a quarter of the way, 1.0.
    let mut posed = root.clone();
    baked.apply(&mut posed, &state, 20.0);
    assert_eq!(head_x(&posed), 1.0);
    // 50 ticks after: 2.5 s, wrapped to 0.5 s.
    let mut wrapped = root.clone();
    baked.apply(&mut wrapped, &state, 60.0);
    assert_eq!(head_x(&wrapped), 1.0);
    // Twice as fast: 1 s, halfway.
    let mut fast = root.clone();
    baked.apply_scaled(&mut fast, &state, 20.0, 2.0);
    assert_eq!(head_x(&fast), 2.0);

    // Walk position 20 at speed factor 1: 1000 ms, halfway; strength min(0.25 * 2, 1).
    let mut walk = root.clone();
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 1.0);
    // Applying again adds on top, as offsetRotation does.
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 2.0);
    // A strength over 1 is held at 1: min(1.5 * 2, 1) at halfway is 2.0.
    let mut strong = root.clone();
    baked.apply_walk(&mut strong, 20.0, 1.5, 1.0, 2.0);
    assert_eq!(head_x(&strong), 2.0);
}

/// Vanilla's transitions: `STOPPED` is `Integer.MIN_VALUE`, `animateWhen` starts once and stops,
/// `fastForward` moves the start back by `(int)(ticks * speed)` and wraps as Java's int does.
#[test]
fn animation_state_follows_vanillas_transitions() {
    let mut state = AnimationState::new();
    assert!(!state.is_started());
    state.animate_when(true, 5);
    state.animate_when(true, 9);
    assert_eq!(state.time_in_millis(7.0), 100, "started once, at 5");
    state.fast_forward(4, 0.5);
    assert_eq!(state.time_in_millis(7.0), 200, "now as if started at 3");
    state.animate_when(false, 9);
    assert!(!state.is_started());
    state.fast_forward(100, 1.0);
    assert!(!state.is_started(), "a stopped state doesn't move");
    state.start(i32::MIN + 1);
    state.fast_forward(2, 1.0);
    assert_eq!(
        state,
        AnimationState {
            start_tick: i32::MAX
        },
        "wraps, never panics"
    );
}

#[test]
fn oracle_root_bone_targets_actual_root_without_children() {
    let mut root = part(vec![]);
    let holder = AnimationHolder {
        length: 1.0,
        looping: false,
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).apply_static(&mut root);
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_root_bone_takes_precedence_over_child_named_root() {
    let mut root = part(vec![("root", part(vec![]))]);
    let holder = AnimationHolder {
        length: 1.0,
        looping: false,
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).apply_static(&mut root);
    let child = root.get_child("root").unwrap();
    assert_eq!((child.x, child.y, child.z), (0.0, 0.0, 0.0));
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_walk_nan_strength_propagates_like_java_math_min() {
    let mut root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        length: 1.0,
        looping: false,
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder
        .bake(&root)
        .apply_walk(&mut root, 0.0, f32::NAN, 1.0, 1.0);
    let head = root.get_child("head").unwrap();
    assert!(
        head.x.is_nan() && head.y.is_nan() && head.z.is_nan(),
        "Java Math.min(NaN, 1) propagates NaN, got {:?}",
        (head.x, head.y, head.z)
    );
}
