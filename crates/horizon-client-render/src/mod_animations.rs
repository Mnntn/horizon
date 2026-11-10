//! Keyframe animations mods and packs ship (NeoForge's
//! `assets/<namespace>/neoforge/animations/entity/<path>.json`), read on every resource load into
//! the published render assets, where `RenderAssets::animation` finds them by id. The game's own
//! exported animations keep their reader (`mobs::keyframes`); both play through `mod_api`'s
//! sampling.
use mod_api::{
    AnimationChannel, AnimationHolder, AnimationTarget, Identifier, Interpolation, Keyframe,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// The folder under each namespace the files are read from.
const FOLDER: &str = "neoforge/animations/entity";

/// The animations of one resource load, by id (`mymod:golem/attack` for
/// `assets/mymod/neoforge/animations/entity/golem/attack.json`).
#[derive(Clone, Debug, Default)]
pub struct Animations(BTreeMap<Identifier, Arc<AnimationHolder>>);

impl Animations {
    /// Every animation file of `source`. A file that doesn't parse is logged with its reason and
    /// left out; the others load.
    pub fn load(source: &crate::resource_source::ResourceSource) -> Self {
        let mut out = BTreeMap::new();
        for file in source.list_under("assets", &[FOLDER]) {
            let Some((namespace, path)) = file
                .strip_prefix("assets/")
                .and_then(|rest| rest.split_once(&format!("/{FOLDER}/")))
            else {
                continue;
            };
            let Some(path) = path.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = Identifier::from_parts(namespace, path) else {
                log::warn!("Horizon: animation {file}: not a valid id; skipped");
                continue;
            };
            match source
                .read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| parse(&text))
            {
                Ok(holder) => {
                    out.insert(id, Arc::new(holder));
                }
                Err(error) => log::warn!("Horizon: animation {id} not loaded: {error}"),
            }
        }
        Self(out)
    }
    /// The animation `id`, if a file gave it.
    pub fn get(&self, id: &Identifier) -> Option<&AnimationHolder> {
        self.0.get(id).map(Arc::as_ref)
    }
}

/// One animation file: `length` (seconds), optional `loop`, and `animations`, entries of a
/// `bone`, a `target` and its `keyframes` (`timestamp`, `target` values, `interpolation`). Values
/// go through vanilla's builder helpers (`KeyframeAnimations`): a position's y is negated
/// (`posVec`), a rotation is in degrees (`degreeVec`), a scale is a factor (`scaleVec` keeps its
/// offset from 1). An unknown target or interpolation fails the file, naming it.
pub fn parse(text: &str) -> Result<AnimationHolder, String> {
    let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let length = json["length"].as_f64().ok_or("no numeric `length`")? as f32;
    let looping = match json.get("loop") {
        None => false,
        Some(value) => value.as_bool().ok_or("`loop` is not a boolean")?,
    };
    let entries = json["animations"]
        .as_array()
        .ok_or("no `animations` list")?;
    let mut bones: Vec<(String, Vec<AnimationChannel>)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let bone = entry["bone"]
            .as_str()
            .ok_or_else(|| format!("animations[{index}]: no `bone`"))?;
        let target = match id(&entry["target"]).as_deref() {
            Some("minecraft:position") => AnimationTarget::Position,
            Some("minecraft:rotation") => AnimationTarget::Rotation,
            Some("minecraft:scale") => AnimationTarget::Scale,
            other => {
                return Err(format!(
                    "animations[{index}]: unknown target {}",
                    other.unwrap_or("(none)")
                ));
            }
        };
        let keyframes = entry["keyframes"]
            .as_array()
            .ok_or_else(|| format!("animations[{index}]: no `keyframes` list"))?
            .iter()
            .enumerate()
            .map(|(at, keyframe)| {
                keyframe_of(keyframe, target)
                    .map_err(|e| format!("animations[{index}].keyframes[{at}]: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let channel = AnimationChannel { target, keyframes };
        match bones.iter_mut().find(|(name, _)| name == bone) {
            Some((_, channels)) => channels.push(channel),
            None => bones.push((bone.to_owned(), vec![channel])),
        }
    }
    Ok(AnimationHolder {
        length,
        looping,
        bones,
    })
}

/// A namespaced id, `minecraft:` when it names none (`Identifier.parse`).
fn id(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    Some(if text.contains(':') {
        text.to_owned()
    } else {
        format!("minecraft:{text}")
    })
}

fn keyframe_of(keyframe: &Value, target: AnimationTarget) -> Result<Keyframe, String> {
    let time = keyframe["timestamp"]
        .as_f64()
        .ok_or("no numeric `timestamp`")? as f32;
    let values = keyframe["target"]
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or("`target` is not three numbers")?;
    let mut v = [0.0f64; 3];
    for (slot, value) in v.iter_mut().zip(values) {
        *slot = value.as_f64().ok_or("`target` is not three numbers")?;
    }
    let value = match target {
        // KeyframeAnimations.posVec
        AnimationTarget::Position => [v[0] as f32, -(v[1] as f32), v[2] as f32],
        // KeyframeAnimations.degreeVec
        AnimationTarget::Rotation => v.map(|d| d as f32 * 0.017_453_292),
        // KeyframeAnimations.scaleVec
        AnimationTarget::Scale => v.map(|d| (d - 1.0) as f32),
    };
    let interpolation = match id(&keyframe["interpolation"]).as_deref() {
        Some("minecraft:linear") => Interpolation::Linear,
        Some("minecraft:catmullrom") => Interpolation::CatmullRom,
        other => {
            return Err(format!(
                "unknown interpolation {}",
                other.unwrap_or("(none)")
            ));
        }
    };
    Ok(Keyframe {
        time,
        pre: value,
        post: value,
        interpolation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_source::{Resource, ResourceSource};
    use mod_api::{AnimationState, ModelPart, PartPose};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    const SWING: &str = r#"{"length": 2.0, "loop": true, "animations": [
        {"bone": "arm", "target": "minecraft:rotation", "keyframes": [
            {"timestamp": 0.0, "target": [0, 0, 0], "interpolation": "minecraft:linear"},
            {"timestamp": 1.0, "target": [90, 0, -45], "interpolation": "linear"}]},
        {"bone": "arm", "target": "minecraft:position", "keyframes": [
            {"timestamp": 0.0, "target": [1, 2, 3], "interpolation": "minecraft:catmullrom"}]},
        {"bone": "body", "target": "minecraft:scale", "keyframes": [
            {"timestamp": 0.0, "target": [2, 1, 0.5], "interpolation": "minecraft:linear"}]}
    ]}"#;

    fn key(holder: &AnimationHolder, bone: usize, channel: usize, at: usize) -> Keyframe {
        holder.bones[bone].1[channel].keyframes[at]
    }

    /// Values go through vanilla's builder helpers: degrees to radians, y negated, scale as an
    /// offset from 1. 90 and -45 degrees times `degreeVec`'s factor round to the f32 quarter and
    /// eighth turns exactly (worked out apart from this code).
    #[test]
    fn a_file_parses_with_vanillas_units_and_groups_its_bones() {
        let holder = parse(SWING).unwrap();
        assert_eq!((holder.length, holder.looping), (2.0, true));
        let bones: Vec<&str> = holder.bones.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(
            bones,
            ["arm", "body"],
            "two entries of `arm` share its bone"
        );
        let swing = key(&holder, 0, 0, 1);
        assert_eq!(swing.post, [FRAC_PI_2, 0.0, -FRAC_PI_4]);
        assert_eq!(swing.pre, swing.post);
        assert_eq!(
            swing.interpolation,
            Interpolation::Linear,
            "an unnamespaced id is minecraft's"
        );
        let at = key(&holder, 0, 1, 0);
        assert_eq!(at.post, [1.0, -2.0, 3.0]);
        assert_eq!(at.interpolation, Interpolation::CatmullRom);
        assert_eq!(key(&holder, 1, 0, 0).post, [1.0, 0.0, -0.5]);
        assert!(
            !parse(r#"{"length":1,"animations":[]}"#).unwrap().looping,
            "loop defaults to false"
        );
    }

    #[test]
    fn an_unknown_target_or_interpolation_fails_the_file_by_name() {
        let bad_target =
            r#"{"length":1,"animations":[{"bone":"a","target":"mymod:glow","keyframes":[]}]}"#;
        assert_eq!(
            parse(bad_target).unwrap_err(),
            "animations[0]: unknown target mymod:glow"
        );
        let bad_interpolation = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"mymod:bounce"}]}]}"#;
        assert_eq!(
            parse(bad_interpolation).unwrap_err(),
            "animations[0].keyframes[0]: unknown interpolation mymod:bounce"
        );
        assert!(parse(r#"{"animations":[]}"#).is_err(), "no length");
        let short = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert!(parse(short).is_err(), "two values");
    }

    /// Every load reads the files again: a nested path is part of the id, a broken file is left
    /// out while the others load, and an edited file plays its new pose on a mod's own model.
    #[test]
    fn a_mod_animation_loads_by_id_and_a_reload_reads_the_edited_file() {
        let source = |swing: &str| {
            ResourceSource::indexed(
                [
                    (
                        "assets/golems/neoforge/animations/entity/golem/swing.json",
                        swing.to_owned(),
                    ),
                    (
                        "assets/golems/neoforge/animations/entity/broken.json",
                        "{".to_owned(),
                    ),
                    ("assets/golems/other/swing.json", SWING.to_owned()),
                ]
                .into_iter()
                .map(|(name, text)| (name.to_owned(), Resource::bytes(text.into_bytes())))
                .collect(),
            )
        };
        let id = Identifier::parse("golems:golem/swing").unwrap();
        let model = ModelPart::new(
            Vec::new(),
            vec![(
                "arm".to_owned(),
                ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO),
            )],
            PartPose::ZERO,
        );
        let arm_x_rot = |animations: &Animations| {
            let holder = animations.get(&id).expect("the file loads by its id");
            let mut posed = model.clone();
            let mut state = AnimationState::new();
            state.start(0);
            // 10 ticks: 500 ms, halfway to the second keyframe.
            holder.bake(&model).apply(&mut posed, &state, 10.0);
            posed.get_child("arm").unwrap().x_rot
        };
        let first = Animations::load(&source(SWING));
        assert_eq!(first.0.len(), 1, "only the good file under the folder");
        assert_eq!(arm_x_rot(&first), FRAC_PI_4);
        let edited = SWING.replace("[90, 0, -45]", "[180, 0, 0]");
        assert_eq!(arm_x_rot(&Animations::load(&source(&edited))), FRAC_PI_2);
    }
}
