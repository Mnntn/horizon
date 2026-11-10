use super::*;
/// A biome modifier declared in code: its id (`mymod:ruby_ore`) and what it does. The game
/// applies every declared modifier once, when the world's biome tables build.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifierDeclaration {
    /// The modifier's id, in the declaring mod's namespace.
    pub id: String,
    /// The biomes it applies to and the change.
    pub modifier: crate::BiomeModifier,
}

impl BiomeModifierDeclaration {
    /// The path the data-file reader knows the modifier by, as if the mod shipped it.
    pub fn file(&self) -> String {
        let (namespace, name) = self.id.split_once(':').unwrap_or_default();
        format!("data/{namespace}/neoforge/biome_modifier/{name}.json")
    }
}

/// The biome modifiers mods declared, in registration order (mods in dispatch order).
pub fn biome_modifier_declarations() -> &'static [BiomeModifierDeclaration] {
    installed().map_or(&[], |installed| &installed.biome_modifiers)
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[cfg(test)]
    fn modifier() -> BiomeModifier {
        BiomeModifier {
            biomes: vec!["#minecraft:is_forest".to_string()],
            operation: BiomeModifierOperation::RemoveSpawns {
                entity_types: vec!["minecraft:zombie".to_string()],
            },
        }
    }

    /// A mod's registration reaches the table the game applies once at load.
    #[test]
    pub(crate) fn a_declared_modifier_is_installed_for_the_game_to_apply() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.declare_biome_modifier("no_zombies", modifier()).unwrap();
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("ruby", Box::new(Quiet))])
            .unwrap();
        let declared = crate::__private::biome_modifier_declarations();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].id, "ruby:no_zombies");
        assert_eq!(declared[0].modifier, modifier());
        assert_eq!(
            declared[0].file(),
            "data/ruby/neoforge/biome_modifier/no_zombies.json"
        );
    }

    #[test]
    fn nonfinite_climate_is_refused_directly_and_at_registration() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for temperature in [true, false] {
                let modifier = BiomeModifier {
                    biomes: vec!["minecraft:plains".into()],
                    operation: BiomeModifierOperation::ModifyBiome {
                        temperature: temperature.then_some(value),
                        downfall: (!temperature).then_some(value),
                        has_precipitation: None,
                        temperature_modifier: None,
                        sky_color: None,
                        fog_color: None,
                        water_color: None,
                        water_fog_color: None,
                        foliage_color: None,
                        grass_color: None,
                        dry_foliage_color: None,
                        ambient_particles: None,
                        ambient_loop: None,
                        ambient_additions: None,
                        ambient_mood: None,
                        background_music: None,
                        music_volume: None,
                    },
                };
                assert_eq!(modifier.check(), Err(BiomeModifierError::InvalidValue));
                let mut registrar = super::PartRegistrar::new("ruby");
                assert_eq!(
                    registrar.declare_biome_modifier("climate", modifier),
                    Err(BiomeModifierError::InvalidValue)
                );
            }
        }
    }

    #[test]
    pub(crate) fn biome_modifier_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar
            .declare_biome_modifier("no_zombies", modifier())
            .unwrap();
        // The same id twice is a duplicate, spelled bare or with the namespace.
        assert_eq!(
            registrar.declare_biome_modifier("ruby:no_zombies", modifier()),
            Err(BiomeModifierError::Duplicate)
        );
        // The id stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_biome_modifier("foreign:no_zombies", modifier()),
            Err(BiomeModifierError::InvalidId)
        );
        // The modifier itself is checked.
        let bad = BiomeModifier {
            biomes: Vec::new(),
            ..modifier()
        };
        assert_eq!(
            registrar.declare_biome_modifier("nothing", bad),
            Err(BiomeModifierError::EmptyList)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.biome_modifiers.len(), 1);
        assert_eq!(registrar.refused.len(), 3);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }
}

/// Registrar operations for biome modifiers.
pub trait BiomeModifiersRegistrar {
    /// Declare a biome modifier `path` (`no_zombies` or `mymod:no_zombies`) in code: the same
    /// operations as the data files in `data/<ns>/neoforge/biome_modifier/` (add and remove
    /// features, add and remove spawns, add and remove spawn costs) on the biomes it names.
    /// The game applies it with the data-file modifiers, once, when the world's tables build:
    /// every addition first, then every removal, then every `ModifyBiome`, each in mod and
    /// registration order. A world
    /// no modifier names generates exactly as before.
    ///
    /// `ModifyBiome` is NeoForge's MODIFY phase: it sets a biome's climate values and effects
    /// colours (`horizon:modify_biome` in a file).
    /// A modifier naming a biome, feature or entity type the world lacks (another mod's, a
    /// convention tag) is skipped with a line in the log, as a data file is.
    ///
    /// An id outside the mod's namespace, a repeated id, an empty list, a name that is not
    /// namespaced, an unknown generation step, a spawner of weight 0 or with its smallest group
    /// above its largest, a spawn cost that is not finite, and a climate number that is not finite, a colour above
    /// 0xFFFFFF or a temperature modifier other than `none` or `frozen` refuse the mods, naming the mod.
    fn declare_biome_modifier(
        &mut self,
        path: &str,
        modifier: crate::BiomeModifier,
    ) -> Result<(), crate::BiomeModifierError>;
}

impl BiomeModifiersRegistrar for PartRegistrar {
    fn declare_biome_modifier(
        &mut self,
        path: &str,
        modifier: crate::BiomeModifier,
    ) -> Result<(), crate::BiomeModifierError> {
        let result = (|| {
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            // The id becomes a file path under the mod's data folder.
            let path_ok = name
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/'))
                && !name.contains("..")
                && !name.contains("//")
                && !name.starts_with('/')
                && !name.ends_with('/');
            if namespace.is_empty() || name.is_empty() || namespace != self.mod_id || !path_ok {
                return Err(crate::BiomeModifierError::InvalidId);
            }
            let id = format!("{namespace}:{name}");
            if self.biome_modifiers.iter().any(|d| d.id == id) {
                return Err(crate::BiomeModifierError::Duplicate);
            }
            modifier.check()?;
            self.biome_modifiers
                .push(BiomeModifierDeclaration { id, modifier });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} biome modifier {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}
