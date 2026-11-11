use super::LIVING_TEMPLATES;
use super::entity_sounds::EntitySounds;
use serde::Deserialize;
use std::collections::BTreeMap;

/// an entity type the mod adds: a copy of a vanilla template's. A thrown one
/// is thrown from `item` and drawn as it; a living one is its template's mob
/// with other `attributes`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntityType {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    /// one of [`super::ENTITY_TEMPLATES`].
    pub template: String,
    /// a thrown type's: one of the mod's own items, which throws it. A
    /// living type has none.
    #[serde(default)]
    pub item: Option<String>,
    /// a thrown type's: what a hit deals to an entity, as `thrown` damage.
    #[serde(default)]
    pub damage: f32,
    /// a living type's default attribute values that differ from its
    /// template's, by attribute id (`minecraft:max_health`): the template's
    /// own attributes only, checked against its row in `entity-types.json`
    /// when the content root builds.
    #[serde(default)]
    pub attributes: BTreeMap<String, f64>,
    /// passenger attachment points (`passenger_attachments`).
    #[serde(default, deserialize_with = "deserialize_passenger_attachments")]
    pub passenger_attachments: Vec<[f32; 3]>,
    /// Omitted slots inherit the template selection.
    #[serde(default)]
    pub sounds: EntitySounds,
}

fn deserialize_passenger_attachments<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<[f32; 3]>, D::Error> {
    use serde_json::Value;
    match Value::deserialize(d)? {
        Value::Number(n) => Ok(vec![[0.0, n.as_f64().unwrap_or(0.0) as f32, 0.0]]),
        Value::Array(a) if a.len() == 3 && a.iter().all(|v| v.is_number()) => {
            let p = [
                a[0].as_f64().unwrap_or(0.0) as f32,
                a[1].as_f64().unwrap_or(0.0) as f32,
                a[2].as_f64().unwrap_or(0.0) as f32,
            ];
            Ok(vec![p])
        }
        Value::Array(a) => a
            .into_iter()
            .map(|v| match v {
                Value::Number(n) => Ok([0.0, n.as_f64().unwrap_or(0.0) as f32, 0.0]),
                Value::Array(coords) if coords.len() == 3 => Ok([
                    coords[0]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                    coords[1]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                    coords[2]
                        .as_f64()
                        .ok_or_else(|| serde::de::Error::custom("expected float"))?
                        as f32,
                ]),
                _ => Err(serde::de::Error::custom(
                    "expected attachment point [x, y, z] or height y",
                )),
            })
            .collect(),
        _ => Err(serde::de::Error::custom(
            "expected attachment point or list",
        )),
    }
}

impl EntityType {
    /// whether the type copies a living mob ([`LIVING_TEMPLATES`]).
    pub fn is_living(&self) -> bool {
        LIVING_TEMPLATES.contains(&self.template.as_str())
    }
}

impl EntityType {
    pub(super) fn validate_thrown_attachments_and_sounds(&self, name: &str) -> Result<(), String> {
        if !self.passenger_attachments.is_empty() {
            return Err(format!(
                "{name}: only a living type has passenger attachments (its template is {})",
                self.template
            ));
        }
        if self.sounds != EntitySounds::default() {
            return Err(format!(
                "{name}: only a living type has sounds (its template is {})",
                self.template
            ));
        }
        Ok(())
    }
}
