use super::*;
/// The structure types registered at startup, for the executable to hand to
/// the engine's `StructureRegistry`.
pub(crate) fn installed_structure_type_list()
-> &'static [crate::world::structure_type::StructureTypeInfo] {
    installed().map_or(&[], |installed| installed.structure_types.as_slice())
}

impl dyn Registrar + '_ {
    /// Register structure type `name` (`tower` or `mymod:tower`), the way
    /// NeoForge's registries `STRUCTURE_TYPE` and `STRUCTURE_PIECE` register
    /// once at startup: a data pack structure with `"type": "modid:tower"`
    /// then generates by `ty`'s code — its own start and pieces, placed by the
    /// game's own spacing, biome and terrain-adaptation checks. The type's id
    /// is in the calling mod's namespace (`mymod:tower`); another mod's
    /// namespace, an empty name and an id registered twice refuse the mods at
    /// startup, naming the mod. `ty`'s `load_piece` is its saved pieces'
    /// loader: keep the type for as long as the game runs.
    pub fn register_structure_type<T: crate::StructureType>(&mut self, name: &str, ty: T) {
        self.add_structure_type(name, Arc::new(ty));
    }
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    /// A structure type twice, one in another mod's namespace and one without
    /// a name: every registration is refused at startup, naming the mod.
    #[test]
    pub(crate) fn duplicate_and_foreign_structure_types_refuse_registration() {
        struct Tower;
        impl mod_api::StructureType for Tower {
            fn find_generation_point(
                &self,
                _: &mut dyn mod_api::GenerationContext,
            ) -> Option<mod_api::FoundStructure> {
                None
            }
            fn load_piece(
                &self,
                _: &dyn mod_api::BlockMetadata,
                _: &mod_api::Compound,
            ) -> Result<std::sync::Arc<dyn mod_api::StructurePiece>, mod_api::StructureError>
            {
                Err(mod_api::StructureError("fixture has no pieces".into()))
            }
        }
        struct Valid;
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Valid))])
            .unwrap();
        let types = crate::__private::structure_type_declarations();
        assert!(types.iter().any(|(id, _)| id == "test:tower"));
        assert_eq!(types.len(), 1);

        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
                r.register_structure_type("tower", Tower);
                r.register_structure_type("other:tower", Tower);
                r.register_structure_type("", Tower);
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("test registered structure type test:tower twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("no name"));
    }
}

/// Registrar operations for structures.
pub trait StructuresRegistrar {
    /// Register structure type `name` with `ty`; mods reach it through
    /// `register_structure_type`.
    ///
    /// On WebAssembly, worker calls run under the linked group's store lock and call budget.
    fn add_structure_type(&mut self, name: &str, ty: Arc<dyn crate::StructureType>);
}

impl StructuresRegistrar for PartRegistrar {
    fn add_structure_type(&mut self, name: &str, ty: Arc<dyn crate::StructureType>) {
        let (namespace, path) = name.split_once(':').unwrap_or((self.mod_id, name));
        let id = format!("{}:{}", self.mod_id, path);
        let refusal = if namespace != self.mod_id {
            Some(format!(
                "{} registered structure type {name:?} in the namespace {namespace:?}, which is not its own",
                self.mod_id
            ))
        } else if path.is_empty() {
            Some(format!(
                "{} registered a structure type with no name",
                self.mod_id
            ))
        } else {
            self.structure_types
                .iter()
                .any(|t| *t.id == *id)
                .then(|| format!("{} registered structure type {id} twice", self.mod_id))
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.structure_types
            .push(crate::world::structure_type::StructureTypeInfo { id: id.into(), ty });
    }
}
