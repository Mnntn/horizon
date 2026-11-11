use super::*;
/// An entity type's initial attribute set, declared in code (NeoForge's
/// `EntityAttributeCreationEvent`): the type's id (`mymod:ghoul`), the
/// vanilla type its content file copies (`minecraft:zombie`, whose row the
/// set starts from, as `Zombie.createAttributes` builds on the template's)
/// and each attribute's base value that differs from the template's. The
/// game resolves the set onto the type's attribute row once, at load.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityAttributeDeclaration {
    /// The entity type the set belongs to.
    pub id: String,
    /// The template the type copies; its row is the set's start.
    pub template: String,
    /// Each attribute's id (`minecraft:max_health`) and its base.
    pub attributes: Vec<(String, f64)>,
}

/// The entity attribute sets mods declared.
pub fn entity_attribute_declarations() -> &'static [EntityAttributeDeclaration] {
    installed().map_or(&[], |installed| &installed.entity_attributes)
}

/// Attributes a mod adds to an existing entity type's default set (NeoForge's
/// `EntityAttributeModificationEvent`): the type (`minecraft:zombie`) and each
/// attribute's id with its base, or none for the attribute's own default (the
/// event's one-argument `add`). The game puts them on the type's attribute row
/// once, at load, after every declared set, so new entities of the type have
/// them; live entities are not edited.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct EntityAttributeModification {
    /// The entity type whose default set grows.
    pub entity: String,
    /// Each attribute's id (`minecraft:luck`) and its base, if not the default.
    pub attributes: Vec<(String, Option<f64>)>,
}

/// The attribute additions mods made to existing entity types, in
/// registration order.
pub fn entity_attribute_modifications() -> &'static [EntityAttributeModification] {
    installed().map_or(&[], |installed| &installed.entity_attribute_modifications)
}
/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn entity_attribute_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar
            .declare_entity_attributes(
                "ghoul",
                "minecraft:zombie",
                vec![("minecraft:max_health".to_string(), 55.0)],
            )
            .unwrap();
        // The same type twice is a duplicate, whatever declares it.
        assert_eq!(
            registrar.declare_entity_attributes("ghoul", "minecraft:zombie", vec![]),
            Err(EntityAttributeError::Duplicate)
        );
        // The path stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_entity_attributes("foreign:ghoul", "minecraft:zombie", vec![]),
            Err(EntityAttributeError::InvalidId)
        );
        // A set validates itself: a duplicate attribute, a non-finite base.
        assert_eq!(
            registrar.declare_entity_attributes(
                "wraith",
                "minecraft:zombie",
                vec![
                    ("minecraft:max_health".to_string(), 1.0),
                    ("minecraft:max_health".to_string(), 2.0),
                ]
            ),
            Err(EntityAttributeError::DuplicateAttribute)
        );
        assert_eq!(
            registrar.declare_entity_attributes(
                "wraith",
                "minecraft:zombie",
                vec![("minecraft:max_health".to_string(), f64::NAN)]
            ),
            Err(EntityAttributeError::NotFinite)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.entity_attributes.len(), 1);
        assert_eq!(
            registrar.entity_attributes[0].attributes,
            [("minecraft:max_health".to_string(), 55.0)]
        );
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }

    /// Additions to existing types (#846, `EntityAttributeModificationEvent`)
    /// reach any namespace, may repeat a type, and validate each call's set.
    #[test]
    pub(crate) fn entity_attribute_modifications_validate_each_set() {
        let mut registrar = super::PartRegistrar::new("ruby");
        // A vanilla type, by bare name or full id, and the same type again.
        registrar
            .modify_entity_attributes("zombie", vec![("minecraft:luck".to_string(), Some(2.0))])
            .unwrap();
        registrar
            .modify_entity_attributes(
                "minecraft:zombie",
                vec![("minecraft:follow_range".to_string(), None)],
            )
            .unwrap();
        // Another mod's type is fine too.
        registrar
            .modify_entity_attributes("other:ghoul", vec![])
            .unwrap();
        assert_eq!(
            registrar.modify_entity_attributes("Not An Id", vec![]),
            Err(EntityAttributeError::InvalidId)
        );
        assert_eq!(
            registrar.modify_entity_attributes(
                "minecraft:skeleton",
                vec![
                    ("minecraft:luck".to_string(), None),
                    ("minecraft:luck".to_string(), Some(1.0)),
                ]
            ),
            Err(EntityAttributeError::DuplicateAttribute)
        );
        assert_eq!(
            registrar.modify_entity_attributes(
                "minecraft:skeleton",
                vec![("minecraft:luck".to_string(), Some(f64::INFINITY))]
            ),
            Err(EntityAttributeError::NotFinite)
        );
        let entities: Vec<_> = registrar
            .entity_attribute_modifications
            .iter()
            .map(|m| m.entity.as_str())
            .collect();
        assert_eq!(
            entities,
            ["minecraft:zombie", "minecraft:zombie", "other:ghoul"]
        );
        assert_eq!(
            registrar.entity_attribute_modifications[1].attributes,
            [("minecraft:follow_range".to_string(), None)]
        );
        assert_eq!(registrar.refused.len(), 3);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }
}

/// Registrar operations for entity attributes.
pub trait EntityAttributesRegistrar {
    /// Declare the initial attribute set of the mod's entity type `path`
    /// (NeoForge's `EntityAttributeCreationEvent`), starting from `template`'s
    /// (`minecraft:zombie`, the one the type's content file copies, as
    /// `Zombie.createAttributes` builds on the template's): each attribute's
    /// id (`minecraft:max_health`) and the base value that differs from the
    /// template's. The values reach the type's own fields, its live attribute
    /// map and the attribute sync, so the reads of #1469 and #1493 answer the
    /// same defaults with no second copy.
    ///
    /// Resolved once, at load: an id outside the mod's manifest-owned namespaces, a duplicate
    /// attribute, a non-finite base or a second declaration of the same type
    /// refuses the mods, naming the mod; an attribute the game does not know,
    /// or a template without attribute defaults, refuses the resource load,
    /// naming the type.
    fn declare_entity_attributes(
        &mut self,
        path: &str,
        template: &str,
        attributes: Vec<(String, f64)>,
    ) -> Result<(), crate::EntityAttributeError>;

    /// Add attributes to an existing entity type's default set (NeoForge's
    /// `EntityAttributeModificationEvent`): `entity` is any type with an
    /// attribute row (`minecraft:zombie`; a bare name is in `minecraft`): a
    /// vanilla type, or a mod's type with a declared set. A living type that
    /// only a content file defines has no row of its own yet; declare its set
    /// with [`declare_entity_attributes`](Self::declare_entity_attributes)
    /// first, and the additions apply on top of it. Each attribute's id
    /// (`minecraft:luck`) takes its base, or with none the attribute's own
    /// default, as the event's one-argument `add`. An attribute the type
    /// already has takes the new base. The change is the type's default:
    /// entities created after load have it, through their spawn fields, their
    /// live attribute map and the attribute sync; nothing edits a live entity.
    ///
    /// Applied once, at load, after every declared set
    /// ([`declare_entity_attributes`](Self::declare_entity_attributes)), in
    /// registration order, so a later addition of the same attribute wins.
    /// An invalid type id, the same attribute twice in one call or a
    /// non-finite base refuses the mods, naming the mod; an attribute the
    /// game does not know, or a type without an attribute row, refuses the
    /// resource load, naming the type.
    fn modify_entity_attributes(
        &mut self,
        entity: &str,
        attributes: Vec<(String, Option<f64>)>,
    ) -> Result<(), crate::EntityAttributeError>;
}

impl EntityAttributesRegistrar for PartRegistrar {
    fn declare_entity_attributes(
        &mut self,
        path: &str,
        template: &str,
        attributes: Vec<(String, f64)>,
    ) -> Result<(), crate::EntityAttributeError> {
        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return Err(crate::EntityAttributeError::InvalidId);
            }
        };
        let result = (|| {
            let (template_ns, template_name) =
                template.split_once(':').unwrap_or(("minecraft", template));
            if template_ns.is_empty() || template_name.is_empty() {
                return Err(crate::EntityAttributeError::InvalidId);
            }
            if self.entity_attributes.iter().any(|d| d.id == id) {
                return Err(crate::EntityAttributeError::Duplicate);
            }
            check_attribute_set(&attributes, |base| Some(*base))?;
            self.entity_attributes.push(EntityAttributeDeclaration {
                id,
                template: format!("{template_ns}:{template_name}"),
                attributes,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} entity attributes {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }

    fn modify_entity_attributes(
        &mut self,
        entity: &str,
        attributes: Vec<(String, Option<f64>)>,
    ) -> Result<(), crate::EntityAttributeError> {
        let result = (|| {
            let id = if entity.contains(':') {
                entity.to_owned()
            } else {
                format!("minecraft:{entity}")
            };
            if crate::Identifier::parse_namespaced(&id).is_none() {
                return Err(crate::EntityAttributeError::InvalidId);
            }
            check_attribute_set(&attributes, |base| *base)?;
            self.entity_attribute_modifications
                .push(EntityAttributeModification {
                    entity: id,
                    attributes,
                });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} entity attribute additions {entity:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

/// Refuses a set that names an attribute twice or has a non-finite base.
fn check_attribute_set<B>(
    attributes: &[(String, B)],
    base: impl Fn(&B) -> Option<f64>,
) -> Result<(), crate::EntityAttributeError> {
    for (index, (attribute, value)) in attributes.iter().enumerate() {
        if base(value).is_some_and(|base| !base.is_finite()) {
            return Err(crate::EntityAttributeError::NotFinite);
        }
        if attributes[..index]
            .iter()
            .any(|(other, _)| other == attribute)
        {
            return Err(crate::EntityAttributeError::DuplicateAttribute);
        }
    }
    Ok(())
}
