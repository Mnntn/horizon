use super::*;
/// An entity type's spawn rule, declared in code (NeoForge's
/// `RegisterSpawnPlacementsEvent`): the type's id (`mymod:ghoul`) and its rule.
/// The game resolves it once, at load, into the tables its spawn checks read.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnPlacementDeclaration {
    /// The entity type the rule belongs to.
    pub id: String,
    /// Where, how bright and in which biomes the type may spawn.
    pub rule: crate::SpawnPlacementRule,
}

/// The spawn rules mods declared.
pub fn spawn_placement_declarations() -> &'static [SpawnPlacementDeclaration] {
    installed().map_or(&[], |installed| &installed.spawn_placements)
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[cfg(test)]
    fn rule() -> SpawnPlacementRule {
        SpawnPlacementRule {
            placement: SpawnPlacementType::OnGround,
            heightmap: Heightmap::MotionBlockingNoLeaves,
            spawnable_on: Some("minecraft:animals_spawnable_on".to_string()),
            light: SpawnLight::Bright,
            biome_tag: Some("minecraft:is_forest".to_string()),
        }
    }

    /// A mod's registration reaches the table the game resolves once at load.
    #[test]
    pub(crate) fn a_registered_rule_is_installed_for_the_game_to_resolve() {
        struct Ghoul;
        #[mod_api::__subscriptions]
        impl Hooks for Ghoul {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_spawn_placement("ghoul", rule()).unwrap();
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("ruby", Box::new(Ghoul))])
            .unwrap();
        let declared = crate::__private::spawn_placement_declarations();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].id, "ruby:ghoul");
        assert_eq!(declared[0].rule, rule());
    }

    #[test]
    pub(crate) fn spawn_placement_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar.declare_spawn_placement("ghoul", rule()).unwrap();
        // The same type twice is a duplicate, spelled bare or with the namespace.
        assert_eq!(
            registrar.declare_spawn_placement("ruby:ghoul", rule()),
            Err(SpawnPlacementError::Duplicate)
        );
        // The path stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_spawn_placement("foreign:ghoul", rule()),
            Err(SpawnPlacementError::InvalidId)
        );
        // A tag is namespaced and written without `#`.
        for tag in ["#minecraft:is_forest", "forest"] {
            let bad = SpawnPlacementRule {
                biome_tag: Some(tag.to_string()),
                ..rule()
            };
            assert_eq!(
                registrar.declare_spawn_placement("wraith", bad),
                Err(SpawnPlacementError::InvalidTag),
                "{tag}"
            );
        }
        let bad = SpawnPlacementRule {
            spawnable_on: Some("#minecraft:animals_spawnable_on".to_string()),
            ..rule()
        };
        assert_eq!(
            registrar.declare_spawn_placement("wraith", bad),
            Err(SpawnPlacementError::InvalidTag)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.spawn_placements.len(), 1);
        assert_eq!(registrar.spawn_placements[0].id, "ruby:ghoul");
        assert_eq!(registrar.spawn_placements[0].rule, rule());
        assert_eq!(registrar.refused.len(), 5);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }
}

/// Registrar operations for spawn placements.
pub trait SpawnPlacementsRegistrar {
    /// Declare where the mod's entity type `path` may spawn (NeoForge's
    /// `RegisterSpawnPlacementsEvent`): its placement type, heightmap and a
    /// rule of a ground block tag, light and biome tag. The vanilla spawn
    /// rules (`SpawnPlacements.isSpawnPositionOk` and `checkSpawnRules`) then
    /// answer from the rule for the type, wherever the game checks a spawn:
    /// natural spawning, chunk generation, spawners, patrols, sieges and
    /// reinforcements. Code that must decide per check is the separate
    /// `mob_spawn_placement_check` hook, which still hears the rule's answer.
    ///
    /// Resolved once, at load: an id outside the mod's namespace, a tag that is
    /// not a namespaced name or a second declaration of the same type refuses
    /// the mods, naming the mod; a type no content file defines refuses the
    /// resource load, naming the type.
    fn declare_spawn_placement(
        &mut self,
        path: &str,
        rule: crate::SpawnPlacementRule,
    ) -> Result<(), crate::SpawnPlacementError>;
}

impl SpawnPlacementsRegistrar for PartRegistrar {
    fn declare_spawn_placement(
        &mut self,
        path: &str,
        rule: crate::SpawnPlacementRule,
    ) -> Result<(), crate::SpawnPlacementError> {
        let result = (|| {
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            if namespace.is_empty() || name.is_empty() || namespace != self.mod_id {
                return Err(crate::SpawnPlacementError::InvalidId);
            }
            let id = format!("{namespace}:{name}");
            if self.spawn_placements.iter().any(|d| d.id == id) {
                return Err(crate::SpawnPlacementError::Duplicate);
            }
            for tag in [&rule.spawnable_on, &rule.biome_tag].into_iter().flatten() {
                if tag.starts_with('#') || !namespaced(tag) {
                    return Err(crate::SpawnPlacementError::InvalidTag);
                }
            }
            self.spawn_placements
                .push(SpawnPlacementDeclaration { id, rule });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} spawn placement {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}
