//! Biome modifiers declared in code: the operations the data-file modifiers
//! (`data/<ns>/neoforge/biome_modifier/`) already do, as data a mod registers with
//! [`Registrar::declare_biome_modifier`](crate::BiomeModifiersRegistrar::declare_biome_modifier).
use serde_json::{Value, json};

/// Vanilla's generation steps, by name, in order (`GenerationStep.Decoration`).
const STEPS: [&str; 11] = [
    "raw_generation",
    "lakes",
    "local_modifications",
    "underground_structures",
    "surface_structures",
    "strongholds",
    "underground_ores",
    "underground_decoration",
    "fluid_springs",
    "vegetal_decoration",
    "top_layer_modification",
];

/// One mob spawner entry of a [`BiomeModifierOperation::AddSpawns`] (`SpawnerData`).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifierSpawner {
    /// The entity type (`mymod:ghoul`).
    pub entity: String,
    /// Relative weight in the spawn category; zero is allowed.
    pub weight: u32,
    /// Smallest group size.
    pub min_count: u32,
    /// Largest group size; at least `min_count`.
    pub max_count: u32,
}

/// What a biome modifier does to its biomes.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeModifierOperation {
    /// `add_features`: append placed features to one generation step.
    AddFeatures {
        /// Placed feature ids (`mymod:ore`) or tags (`#mymod:tag`).
        features: Vec<String>,
        /// The generation step (`underground_ores`, ...).
        step: String,
    },
    /// `remove_features`: take placed features out of generation steps.
    RemoveFeatures {
        /// Placed feature ids or tags.
        features: Vec<String>,
        /// The steps to take them out of; none means every step.
        steps: Vec<String>,
    },
    /// `add_spawns`: append spawners to their entity types' spawn categories.
    AddSpawns {
        /// The spawners to add.
        spawners: Vec<BiomeModifierSpawner>,
    },
    /// `remove_spawns`: take entity types out of every spawn category.
    RemoveSpawns {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
    },
    /// `add_spawn_costs`: set the spawn cost of entity types.
    AddSpawnCosts {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
        /// The `energy_budget` of the cost.
        energy_budget: f64,
        /// The `charge` of the cost.
        charge: f64,
    },
    /// `remove_spawn_costs`: take entity types out of `spawn_costs`.
    RemoveSpawnCosts {
        /// Entity type ids or tags.
        entity_types: Vec<String>,
    },
}

/// A biome modifier: the biomes it applies to and what it does to them.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a mod builds it with a literal; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifier {
    /// Biome ids (`minecraft:plains`) or biome tags (`#minecraft:is_forest`).
    pub biomes: Vec<String>,
    /// The change.
    pub operation: BiomeModifierOperation,
}

/// Why a biome modifier was refused
/// ([`Registrar::declare_biome_modifier`](crate::BiomeModifiersRegistrar::declare_biome_modifier)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BiomeModifierError {
    /// The modifier's own id is empty or in a namespace the declaring mod does not own.
    InvalidId,
    /// Another modifier of the mod has the id.
    Duplicate,
    /// A list that must name something is empty.
    EmptyList,
    /// An id or tag is not namespaced (`mymod:thing`, a tag with a leading `#`).
    InvalidName,
    /// A generation step name vanilla does not have.
    UnknownStep,
    /// A spawner outside signed integer bounds, with an empty group, or inverted group bounds.
    InvalidSpawner,
    /// A spawn cost that is not a finite number.
    InvalidCost,
}

fn name_ok(name: &str, tags: bool) -> bool {
    let id = match name.strip_prefix('#') {
        Some(tag) if tags => tag,
        Some(_) => return false,
        None => name,
    };
    id.split_once(':')
        .is_some_and(|(namespace, path)| !namespace.is_empty() && !path.is_empty())
}

fn names_ok(names: &[String], tags: bool) -> Result<(), BiomeModifierError> {
    if names.is_empty() {
        return Err(BiomeModifierError::EmptyList);
    }
    match names.iter().all(|n| name_ok(n, tags)) {
        true => Ok(()),
        false => Err(BiomeModifierError::InvalidName),
    }
}

fn step_ok(step: &str) -> Result<(), BiomeModifierError> {
    match STEPS.contains(&step) {
        true => Ok(()),
        false => Err(BiomeModifierError::UnknownStep),
    }
}

impl BiomeModifier {
    /// Whether every field is something the game can apply; the first problem found.
    pub fn check(&self) -> Result<(), BiomeModifierError> {
        names_ok(&self.biomes, true)?;
        match &self.operation {
            BiomeModifierOperation::AddFeatures { features, step } => {
                names_ok(features, true)?;
                step_ok(step)
            }
            BiomeModifierOperation::RemoveFeatures { features, steps } => {
                names_ok(features, true)?;
                steps.iter().map(String::as_str).try_for_each(step_ok)
            }
            BiomeModifierOperation::AddSpawns { spawners } => {
                if spawners.is_empty() {
                    return Err(BiomeModifierError::EmptyList);
                }
                for spawner in spawners {
                    names_ok(std::slice::from_ref(&spawner.entity), false)?;
                    if spawner.weight > i32::MAX as u32
                        || spawner.min_count == 0
                        || spawner.min_count > spawner.max_count
                        || spawner.max_count > i32::MAX as u32
                    {
                        return Err(BiomeModifierError::InvalidSpawner);
                    }
                }
                Ok(())
            }
            BiomeModifierOperation::RemoveSpawns { entity_types }
            | BiomeModifierOperation::RemoveSpawnCosts { entity_types } => {
                names_ok(entity_types, true)
            }
            BiomeModifierOperation::AddSpawnCosts {
                entity_types,
                energy_budget,
                charge,
            } => {
                names_ok(entity_types, true)?;
                match energy_budget.is_finite() && charge.is_finite() {
                    true => Ok(()),
                    false => Err(BiomeModifierError::InvalidCost),
                }
            }
        }
    }

    /// The modifier as the NeoForge-shaped JSON file the data-file reader takes, so a modifier
    /// declared in code and one shipped as a file are applied by the same code.
    pub fn json(&self) -> String {
        // One entry is the plain id, as NeoForge writes a single holder.
        let set = |names: &[String]| match names {
            [one] => json!(one),
            many => json!(many),
        };
        let mut file = match &self.operation {
            BiomeModifierOperation::AddFeatures { features, step } => json!({
                "type": "neoforge:add_features",
                "features": set(features),
                "step": step,
            }),
            BiomeModifierOperation::RemoveFeatures { features, steps } => {
                let mut file = json!({
                    "type": "neoforge:remove_features",
                    "features": set(features),
                });
                if !steps.is_empty() {
                    file["steps"] = json!(steps);
                }
                file
            }
            BiomeModifierOperation::AddSpawns { spawners } => json!({
                "type": "neoforge:add_spawns",
                "spawners": spawners.iter().map(|s| json!({
                    "type": s.entity,
                    "weight": s.weight,
                    "minCount": s.min_count,
                    "maxCount": s.max_count,
                })).collect::<Vec<Value>>(),
            }),
            BiomeModifierOperation::RemoveSpawns { entity_types } => json!({
                "type": "neoforge:remove_spawns",
                "entity_types": set(entity_types),
            }),
            BiomeModifierOperation::AddSpawnCosts {
                entity_types,
                energy_budget,
                charge,
            } => json!({
                "type": "neoforge:add_spawn_costs",
                "entity_types": set(entity_types),
                "spawn_cost": {"energy_budget": energy_budget, "charge": charge},
            }),
            BiomeModifierOperation::RemoveSpawnCosts { entity_types } => json!({
                "type": "neoforge:remove_spawn_costs",
                "entity_types": set(entity_types),
            }),
        };
        file["biomes"] = set(&self.biomes);
        file.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn modifier(operation: BiomeModifierOperation) -> BiomeModifier {
        BiomeModifier {
            biomes: strings(&["#minecraft:is_forest"]),
            operation,
        }
    }

    #[test]
    fn a_valid_modifier_checks_and_renders_the_data_file_shape() {
        let m = modifier(BiomeModifierOperation::AddFeatures {
            features: strings(&["ruby:ruby_ore"]),
            step: "underground_ores".into(),
        });
        assert_eq!(m.check(), Ok(()));
        let file: Value = serde_json::from_str(&m.json()).unwrap();
        assert_eq!(
            file,
            json!({
                "type": "neoforge:add_features",
                "biomes": "#minecraft:is_forest",
                "features": "ruby:ruby_ore",
                "step": "underground_ores",
            })
        );
    }

    #[test]
    fn spawn_operations_render_neoforges_field_names() {
        let spawns = modifier(BiomeModifierOperation::AddSpawns {
            spawners: vec![BiomeModifierSpawner {
                entity: "ruby:ghoul".into(),
                weight: 5,
                min_count: 1,
                max_count: 3,
            }],
        });
        assert_eq!(spawns.check(), Ok(()));
        let file: Value = serde_json::from_str(&spawns.json()).unwrap();
        assert_eq!(
            file["spawners"],
            json!([{"type": "ruby:ghoul", "weight": 5, "minCount": 1, "maxCount": 3}])
        );
        let cost = modifier(BiomeModifierOperation::AddSpawnCosts {
            entity_types: strings(&["ruby:ghoul", "minecraft:zombie"]),
            energy_budget: 0.5,
            charge: 0.25,
        });
        let file: Value = serde_json::from_str(&cost.json()).unwrap();
        assert_eq!(
            file["entity_types"],
            json!(["ruby:ghoul", "minecraft:zombie"])
        );
        assert_eq!(
            file["spawn_cost"],
            json!({"energy_budget": 0.5, "charge": 0.25})
        );
        // No steps means every step: the file leaves `steps` out.
        let remove = modifier(BiomeModifierOperation::RemoveFeatures {
            features: strings(&["minecraft:ore_coal_upper"]),
            steps: Vec::new(),
        });
        let file: Value = serde_json::from_str(&remove.json()).unwrap();
        assert!(file.get("steps").is_none());
    }

    #[test]
    fn spawn_boundaries_match_the_reader() {
        for (weight, min_count, max_count, valid) in [
            (0, 1, 1, true),
            (i32::MAX as u32, 1, i32::MAX as u32, true),
            (1, 0, 1, false),
            (u32::MAX, 1, 1, false),
            (1, u32::MAX, u32::MAX, false),
            (1, 1, u32::MAX, false),
        ] {
            let m = modifier(BiomeModifierOperation::AddSpawns {
                spawners: vec![BiomeModifierSpawner {
                    entity: "minecraft:zombie".into(),
                    weight,
                    min_count,
                    max_count,
                }],
            });
            assert_eq!(m.check().is_ok(), valid);
        }
    }

    #[test]
    fn invalid_modifiers_are_refused_by_reason() {
        let bad = |op| modifier(op).check();
        assert_eq!(
            bad(BiomeModifierOperation::AddFeatures {
                features: strings(&["ore"]),
                step: "underground_ores".into(),
            }),
            Err(BiomeModifierError::InvalidName)
        );
        assert_eq!(
            bad(BiomeModifierOperation::AddFeatures {
                features: strings(&["ruby:ore"]),
                step: "underground".into(),
            }),
            Err(BiomeModifierError::UnknownStep)
        );
        assert_eq!(
            bad(BiomeModifierOperation::RemoveSpawns {
                entity_types: Vec::new()
            }),
            Err(BiomeModifierError::EmptyList)
        );
        // A spawner names one entity type, never a tag.
        let spawner = |entity: &str, weight, min_count, max_count| BiomeModifierSpawner {
            entity: entity.into(),
            weight,
            min_count,
            max_count,
        };
        for (spawner, error) in [
            (
                spawner("#ruby:ghouls", 1, 1, 1),
                BiomeModifierError::InvalidName,
            ),
            (
                spawner("ruby:ghoul", 1, 0, 1),
                BiomeModifierError::InvalidSpawner,
            ),
            (
                spawner("ruby:ghoul", 1, 4, 2),
                BiomeModifierError::InvalidSpawner,
            ),
        ] {
            assert_eq!(
                bad(BiomeModifierOperation::AddSpawns {
                    spawners: vec![spawner]
                }),
                Err(error)
            );
        }
        assert_eq!(
            bad(BiomeModifierOperation::AddSpawnCosts {
                entity_types: strings(&["ruby:ghoul"]),
                energy_budget: f64::NAN,
                charge: 1.0,
            }),
            Err(BiomeModifierError::InvalidCost)
        );
        // The biomes are checked too.
        let mut m = modifier(BiomeModifierOperation::RemoveSpawns {
            entity_types: strings(&["minecraft:zombie"]),
        });
        m.biomes = strings(&["plains"]);
        assert_eq!(m.check(), Err(BiomeModifierError::InvalidName));
    }
}
