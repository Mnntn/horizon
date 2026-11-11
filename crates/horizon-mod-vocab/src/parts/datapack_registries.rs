use super::*;
use crate::world::datapack_registry::{
    DatapackField, DatapackRegistrationError, DatapackRegistryDeclaration, DatapackRegistrySync,
    check_schema, valid_namespaced,
};

/// The datapack registries mods declared.
pub fn datapack_registry_declarations() -> &'static [DatapackRegistryDeclaration] {
    #[cfg(any(test, feature = "testing"))]
    if let Some(declarations) = PROCESS_DECLARATIONS.get() {
        return declarations;
    }
    installed().map_or(&[], |installed| &installed.datapack_registries)
}

/// Declarations a test process shares with every thread. A test's own installation is per
/// thread, but the connection a server answers a client on runs on threads the test did not
/// start.
#[cfg(any(test, feature = "testing"))]
static PROCESS_DECLARATIONS: OnceLock<Vec<DatapackRegistryDeclaration>> = OnceLock::new();

/// Declare `declarations` for every thread of this test process; the first call wins.
#[cfg(any(test, feature = "testing"))]
pub fn install_process_datapack_registry_declarations(
    declarations: Vec<DatapackRegistryDeclaration>,
) {
    let _ = PROCESS_DECLARATIONS.set(declarations);
}

/// Registration of mod-owned datapack registries
/// ([`Registrar::declare_datapack_registry`](DatapackRegistriesRegistrar::declare_datapack_registry)).
pub trait DatapackRegistriesRegistrar {
    /// Declare a datapack registry of the mod's own (NeoForge's
    /// `DataPackRegistryEvent.NewRegistry`): `path` in the mod's namespace
    /// (`spell` of `example` is `example:spell`), the flat record `fields` an
    /// entry has, and how clients receive the entries.
    ///
    /// Entries are JSON files in data packs: entry `addon:fire` of
    /// `example:spell` is `data/addon/example/spell/fire.json`
    /// ([`DatapackRegistryDeclaration::entry_file`]). The game decodes every
    /// file against `fields`: no mod code runs while decoding, so native and
    /// wasm mods get the same bounds. A key the schema does not
    /// declare, a missing required key or an out-of-range value fails the
    /// entry; a default never stands in for invalid data.
    ///
    /// Only both-side packages may declare datapack registries. Refused,
    /// naming the mod: a key outside the mod's namespace (a vanilla
    /// registry's included), a malformed key, a duplicate (of this mod's or
    /// another's, caught where the declarations join), an empty or repeated
    /// field name, unusable bounds, or a default that is not a value of its
    /// field inside its bounds.
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError>;
}

impl DatapackRegistriesRegistrar for PartRegistrar {
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        mut fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError> {
        let result = (|| {
            if crate::world::phase::LoadedMods::new()
                .get(self.mod_id)
                .is_some_and(|package| package.side() != crate::Side::Both)
            {
                return Err(DatapackRegistrationError::WrongSide);
            }
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            let key = format!("{namespace}:{name}");
            if namespace != self.mod_id || !valid_namespaced(&key) {
                return Err(DatapackRegistrationError::InvalidKey);
            }
            if self
                .datapack_registries
                .iter()
                .any(|declared| declared.key == key)
            {
                return Err(DatapackRegistrationError::Duplicate);
            }
            for field in &mut fields {
                if matches!(
                    field.kind,
                    crate::DatapackFieldKind::Identifier
                        | crate::DatapackFieldKind::Reference { .. }
                ) && let Some(crate::DatapackValue::Text(value)) = &mut field.default
                {
                    *value = crate::Identifier::parse(value)
                        .map_err(|_| DatapackRegistrationError::InvalidDefault)?
                        .to_string();
                }
            }
            check_schema(&fields)?;
            self.datapack_registries.push(DatapackRegistryDeclaration {
                mod_id: self.mod_id.to_owned(),
                key,
                fields,
                sync,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} datapack registry {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::DatapackFieldKind;

    fn power() -> Vec<DatapackField> {
        vec![DatapackField::required(
            "power",
            DatapackFieldKind::Int { min: 0, max: 100 },
        )]
    }

    /// A key in the mod's namespace stands; foreign, vanilla and malformed keys, a
    /// duplicate and a bad schema refuse, naming the mod.
    #[test]
    pub(crate) fn datapack_registry_declarations_validate() {
        use DatapackRegistrationError as E;
        let mut registrar = PartRegistrar::new("example");
        let sync = DatapackRegistrySync::Required;
        registrar
            .declare_datapack_registry("spell", sync, power())
            .unwrap();
        assert_eq!(
            registrar.datapack_registries[0].key, "example:spell",
            "a bare path is in the declaring mod's namespace"
        );
        assert_eq!(
            registrar.declare_datapack_registry("example:spell", sync, power()),
            Err(E::Duplicate)
        );
        assert_eq!(
            registrar.declare_datapack_registry("minecraft:spell", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("addon:spell", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("Not A Key", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("", sync, power()),
            Err(E::InvalidKey)
        );
        let mut bad = power();
        bad.push(bad[0].clone());
        assert_eq!(
            registrar.declare_datapack_registry("other", sync, bad),
            Err(E::InvalidFieldName)
        );
        // A refused schema leaves no half-declared registry behind.
        assert_eq!(registrar.datapack_registries.len(), 1);
        assert_eq!(registrar.refused.len(), 6);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod example"))
        );
    }

    #[test]
    fn datapack_declarations_round_trip_the_wire() {
        use crate::wire::{Reader, Wire, Writer};
        let declaration = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: power(),
            sync: DatapackRegistrySync::Optional,
        };
        let mut writer = Writer::new();
        declaration.put(&mut writer);
        let bytes = writer.into_bytes();
        assert_eq!(
            DatapackRegistryDeclaration::take(&mut Reader::new(&bytes)).unwrap(),
            declaration
        );
    }
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn registered_identifier_defaults_match_explicit_values() {
        for kind in [
            crate::DatapackFieldKind::Identifier,
            crate::DatapackFieldKind::Reference {
                registry: "minecraft:item".into(),
            },
        ] {
            let mut registrar = PartRegistrar::new("defaults");
            registrar
                .declare_datapack_registry(
                    "entry",
                    DatapackRegistrySync::default(),
                    vec![DatapackField::with_default(
                        "id",
                        kind,
                        crate::DatapackValue::Text("ice".into()),
                    )],
                )
                .unwrap();
            let declaration = &registrar.datapack_registries[0];
            assert_eq!(
                declaration.decode_entry("{}").unwrap(),
                declaration.decode_entry(r#"{"id":"ice"}"#).unwrap()
            );
        }
    }
}
