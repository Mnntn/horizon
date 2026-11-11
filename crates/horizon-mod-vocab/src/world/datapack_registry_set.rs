//! The entries of the datapack registries mods declared, as one world loads them
//! (arismoko/horizon#811, stage 3): a candidate built from the winning pack files and checked
//! as a whole, then published as a set with a session and a generation, and read through
//! handles that check both. Nothing here touches the world; the game builds a candidate before
//! it commits any table and keeps the old set when building fails. See
//! `docs/modding/src/concepts/mod-datapack-registries.md`.
use super::datapack_registry::{
    DatapackDecodeError, DatapackEntry, DatapackFieldKind, DatapackRegistryDeclaration,
    DatapackValue, valid_namespaced,
};
use std::collections::BTreeMap;

/// Why a candidate was refused. The whole candidate fails, never one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
#[non_exhaustive]
pub enum DatapackLoadReason {
    /// The winning file is not UTF-8 text.
    NotUtf8,
    /// The entry's id (from its namespace and file name) is not a lowercase `namespace:path`.
    InvalidEntryId,
    /// The file did not decode against the registry's schema.
    Decode(DatapackDecodeError),
    /// A reference names an entry of a declared registry that no pack defines.
    UnresolvedReference {
        /// The referring field.
        field: String,
        /// The registry the field names.
        registry: String,
        /// The entry it names (`namespace:path`).
        target: String,
    },
}

/// A refused candidate: which registry, entry and file, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: loading runs in the game; mods read published entries, never load failures
// exhaustive: the game builds it with a literal and reports it
pub struct DatapackLoadError {
    /// The registry's key (`example:spell`).
    pub registry: String,
    /// The entry's id (`addon:fire`), as far as it was known.
    pub entry: String,
    /// The file the entry came from.
    pub file: String,
    /// What was wrong.
    pub reason: DatapackLoadReason,
}

/// Every entry of every declared registry, decoded and checked, not yet published.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game's candidate; a mod reads the published set through handles
// exhaustive: built only by [`DatapackCandidate::build`]
pub struct DatapackCandidate {
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
}

/// The id of the entry in `file` (`data/addon/example/spell/fire.json` is `addon:fire`), when
/// the file is in `declaration`'s folder.
fn entry_of(declaration: &DatapackRegistryDeclaration, file: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (namespace, rest) = rest.split_once('/')?;
    let (registry_namespace, registry_path) = declaration.key.split_once(':')?;
    let path = rest
        .strip_prefix(registry_namespace)?
        .strip_prefix('/')?
        .strip_prefix(registry_path)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    (!path.is_empty()).then(|| format!("{namespace}:{path}"))
}

impl DatapackCandidate {
    /// Decodes the winning file of every entry of every declared registry in `files`
    /// (`data/<entry namespace>/<registry namespace>/<registry path>/<entry>.json`, each with
    /// its versions in pack order, the last winning as `ResourceManager` hands them out), then
    /// resolves every reference to a declared registry against the whole candidate, so a
    /// forward reference works and an absent target refuses everything. A reference to a
    /// registry no declaration here covers (a vanilla one) is not checked. The caller has
    /// already dropped files whose pack conditions do not hold. The error is boxed: it is
    /// large, and a refused candidate is rare.
    pub fn build(
        declarations: &[DatapackRegistryDeclaration],
        files: &[(String, Vec<Vec<u8>>)],
    ) -> Result<Self, Box<DatapackLoadError>> {
        let mut registries: BTreeMap<String, BTreeMap<String, DatapackEntry>> = declarations
            .iter()
            .map(|declaration| (declaration.key.clone(), BTreeMap::new()))
            .collect();
        for declaration in declarations {
            let fail = |entry: &str, file: &str, reason| {
                Box::new(DatapackLoadError {
                    registry: declaration.key.clone(),
                    entry: entry.to_owned(),
                    file: file.to_owned(),
                    reason,
                })
            };
            for (file, versions) in files {
                let Some(entry) = entry_of(declaration, file) else {
                    continue;
                };
                if !valid_namespaced(&entry) {
                    return Err(fail(&entry, file, DatapackLoadReason::InvalidEntryId));
                }
                let Some(text) = versions.last() else {
                    continue;
                };
                let text = std::str::from_utf8(text)
                    .map_err(|_| fail(&entry, file, DatapackLoadReason::NotUtf8))?;
                let decoded = declaration
                    .decode_entry(text)
                    .map_err(|error| fail(&entry, file, DatapackLoadReason::Decode(error)))?;
                registries
                    .get_mut(&declaration.key)
                    .expect("every declaration has a registry")
                    .insert(entry, decoded);
            }
        }
        resolve_references(declarations, &registries)?;
        Ok(Self { registries })
    }
}

/// Why a handle or a lookup failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackReadError {
    /// The handle is from another world session, as after closing and reopening the world.
    WrongSession,
    /// The handle is from an earlier generation: the registries reloaded since.
    StaleGeneration,
    /// No registry has this key.
    UnknownRegistry,
    /// The registry has no such entry (a reload removed it, or no pack defines it).
    MissingEntry,
    /// The game has no world open to read from, or no host answered.
    Unavailable,
}
impl crate::NoBackend for DatapackReadError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// A reference to one entry of a published set. It checks its session and generation on every
/// read, so one kept across a reload fails instead of reading the new value by name or slot.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game issues it; a mod holds it and reads through the set
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackHandle {
    session: u64,
    generation: u64,
    registry: String,
    entry: String,
}

impl DatapackHandle {
    /// The registry's key.
    pub fn registry(&self) -> &str {
        &self.registry
    }

    /// The entry's id.
    pub fn entry(&self) -> &str {
        &self.entry
    }
}

/// Checks every reference to a declared registry against the whole `registries`, so a forward
/// reference works and an absent target refuses everything. A reference to a registry no
/// declaration covers (a vanilla one) is not checked.
pub(super) fn resolve_references(
    declarations: &[DatapackRegistryDeclaration],
    registries: &BTreeMap<String, BTreeMap<String, DatapackEntry>>,
) -> Result<(), Box<DatapackLoadError>> {
    for declaration in declarations {
        for (entry, decoded) in &registries[&declaration.key] {
            for field in &declaration.fields {
                let DatapackFieldKind::Reference { registry } = &field.kind else {
                    continue;
                };
                let Some(targets) = registries.get(registry) else {
                    continue;
                };
                let Some(DatapackValue::Text(target)) = decoded.get(&field.name) else {
                    continue;
                };
                if !targets.contains_key(target) {
                    return Err(Box::new(DatapackLoadError {
                        registry: declaration.key.clone(),
                        entry: entry.clone(),
                        file: declaration.entry_file(entry).unwrap_or_default(),
                        reason: DatapackLoadReason::UnresolvedReference {
                            field: field.name.clone(),
                            registry: registry.clone(),
                            target: target.clone(),
                        },
                    }));
                }
            }
        }
    }
    Ok(())
}

/// Why a set could not move to a new generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// no wire: the game's reload reports it
#[non_exhaustive]
pub enum DatapackGenerationExhausted {
    /// The generation counter is at its end; it never wraps.
    Exhausted,
}

/// The published entries of a world session at one generation.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game holds it; a mod reads through handles
// exhaustive: built only by [`DatapackRegistrySet::first`] and [`DatapackRegistrySet::reload`]
pub struct DatapackRegistrySet {
    pub(super) session: u64,
    pub(super) generation: u64,
    pub(super) registries: BTreeMap<String, BTreeMap<String, DatapackEntry>>,
}

impl DatapackRegistrySet {
    /// Publishes `candidate` as a world session's first set: generation 1.
    pub fn first(candidate: DatapackCandidate, session: u64) -> Self {
        Self {
            session,
            generation: 1,
            registries: candidate.registries,
        }
    }

    /// Publishes `candidate` after a successful reload of the same session: the next
    /// generation, even when the content is identical. A failed reload never gets here, so it
    /// takes no generation; the old set stays.
    pub fn reload(
        &self,
        candidate: DatapackCandidate,
    ) -> Result<Self, DatapackGenerationExhausted> {
        Ok(Self {
            session: self.session,
            generation: self
                .generation
                .checked_add(1)
                .ok_or(DatapackGenerationExhausted::Exhausted)?,
            registries: candidate.registries,
        })
    }

    /// The generation, from 1.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// A handle to `entry` of `registry` in this set.
    pub fn handle(&self, registry: &str, entry: &str) -> Result<DatapackHandle, DatapackReadError> {
        self.lookup(registry, entry)?;
        Ok(DatapackHandle {
            session: self.session,
            generation: self.generation,
            registry: registry.to_owned(),
            entry: entry.to_owned(),
        })
    }

    /// The entry a handle names, when the handle is of this session and generation and the
    /// entry is still there.
    pub fn get(&self, handle: &DatapackHandle) -> Result<&DatapackEntry, DatapackReadError> {
        if handle.session != self.session {
            return Err(DatapackReadError::WrongSession);
        }
        if handle.generation != self.generation {
            return Err(DatapackReadError::StaleGeneration);
        }
        self.lookup(&handle.registry, &handle.entry)
    }

    /// The entry ids of `registry` in this set, sorted.
    pub fn entries(&self, registry: &str) -> Result<Vec<&str>, DatapackReadError> {
        let entries = self
            .registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?;
        Ok(entries.keys().map(String::as_str).collect())
    }

    fn lookup(&self, registry: &str, entry: &str) -> Result<&DatapackEntry, DatapackReadError> {
        self.registries
            .get(registry)
            .ok_or(DatapackReadError::UnknownRegistry)?
            .get(entry)
            .ok_or(DatapackReadError::MissingEntry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::{DatapackField, DatapackRegistrySync};

    /// `example:spell`: `{power: integer 0..100, next: reference to example:spell = example:fire}`.
    fn spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("example:fire".into()),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    fn file(path: &str, versions: &[&str]) -> (String, Vec<Vec<u8>>) {
        (
            path.to_owned(),
            versions.iter().map(|v| v.as_bytes().to_vec()).collect(),
        )
    }

    fn power(set: &DatapackRegistrySet, entry: &str) -> DatapackValue {
        let handle = set.handle("example:spell", entry).unwrap();
        set.get(&handle).unwrap().get("power").unwrap().clone()
    }

    fn candidate(files: &[(String, Vec<Vec<u8>>)]) -> DatapackCandidate {
        DatapackCandidate::build(&[spell()], files).unwrap()
    }

    /// Base fire power 3, a higher pack's 7: one entry, the winner's value, generation 1.
    #[test]
    fn the_winning_pack_file_is_the_one_entry() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 3}"#, r#"{"power": 7}"#],
        )];
        let set = DatapackRegistrySet::first(candidate(&files), 1);
        assert_eq!(set.generation(), 1);
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(7));
        assert_eq!(set.entries("example:spell").unwrap(), ["example:fire"]);
    }

    /// An entry in another mod's namespace sits in that namespace's folder; nested paths work;
    /// files of other registries and other folders are not entries.
    #[test]
    fn entries_come_from_their_namespaced_folder_only() {
        let files = [
            file("data/example/example/spell/fire.json", &[r#"{"power": 3}"#]),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 4}"#]),
            file(
                "data/addon/example/spell/nested/hail.json",
                &[r#"{"power": 5}"#],
            ),
            file("data/example/example/other/fire.json", &[r#"{"power": 1}"#]),
            file("data/example/spell/fire.json", &[r#"{"power": 1}"#]),
            file("data/addon/example/spell/ice.txt", &[r#"{"power": 1}"#]),
        ];
        let set = DatapackRegistrySet::first(candidate(&files), 1);
        assert_eq!(
            set.entries("example:spell").unwrap(),
            ["addon:ice", "addon:nested/hail", "example:fire"]
        );
        assert_eq!(
            set.entries("example:nothing"),
            Err(DatapackReadError::UnknownRegistry)
        );
    }

    /// Reload to 9: new read, generation 2, the generation-1 handle is stale. Remove fire: a
    /// fresh lookup is MissingEntry, the old handle StaleGeneration. An identical reload
    /// still advances once.
    #[test]
    fn a_reload_stales_old_handles_and_removal_is_missing() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let one = DatapackRegistrySet::first(candidate(&files), 1);
        let held = one.handle("example:spell", "example:fire").unwrap();
        let nine = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 9}"#],
        )];
        let two = one.reload(candidate(&nine)).unwrap();
        assert_eq!((one.generation(), two.generation()), (1, 2));
        assert_eq!(power(&two, "example:fire"), DatapackValue::Int(9));
        // The old handle fails on the new set, whatever the name and slot.
        assert_eq!(two.get(&held), Err(DatapackReadError::StaleGeneration));
        // On the old set it still reads the old value.
        assert_eq!(
            one.get(&held).unwrap().get("power"),
            Some(&DatapackValue::Int(7))
        );
        // Identical content: still the next generation, once.
        let three = two.reload(candidate(&nine)).unwrap();
        assert_eq!(three.generation(), 3);
        // Remove fire.
        let four = three.reload(candidate(&[])).unwrap();
        assert_eq!(
            four.handle("example:spell", "example:fire"),
            Err(DatapackReadError::MissingEntry)
        );
        let held_three = three.handle("example:spell", "example:fire").unwrap();
        assert_eq!(
            four.get(&held_three),
            Err(DatapackReadError::StaleGeneration)
        );
    }

    /// Close and reopen: a distinct session at generation 1; the old handle is WrongSession even
    /// with the same name and generation.
    #[test]
    fn a_reopened_world_is_another_session() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let first = DatapackRegistrySet::first(candidate(&files), 1);
        let held = first.handle("example:spell", "example:fire").unwrap();
        let reopened = DatapackRegistrySet::first(candidate(&files), 2);
        assert_eq!(reopened.generation(), 1);
        assert_eq!(reopened.get(&held), Err(DatapackReadError::WrongSession));
    }

    /// Invalid winning data (power 101, malformed JSON) fails the candidate with the file and
    /// field, and does not fall back to the lower-priority version; the old set is untouched.
    #[test]
    fn invalid_winning_data_refuses_the_candidate_and_keeps_the_old_set() {
        let good = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#],
        )];
        let old = DatapackRegistrySet::first(candidate(&good), 1);
        let bad = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, r#"{"power": 101}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &bad),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                reason: DatapackLoadReason::Decode(DatapackDecodeError::OutOfRange {
                    field: "power".into()
                }),
            }))
        );
        let malformed = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 7}"#, "{"],
        )];
        let refused = DatapackCandidate::build(&[spell()], &malformed).unwrap_err();
        assert!(matches!(
            refused.reason,
            DatapackLoadReason::Decode(DatapackDecodeError::Malformed(_))
        ));
        // A later valid reload still publishes.
        assert_eq!(old.generation(), 1);
        assert_eq!(power(&old, "example:fire"), DatapackValue::Int(7));
        assert_eq!(old.reload(candidate(&good)).unwrap().generation(), 2);
    }

    /// A forward reference resolves after the candidate completes; an absent target refuses it
    /// all.
    #[test]
    fn references_resolve_against_the_whole_candidate() {
        let forward = [
            file(
                "data/example/example/spell/fire.json",
                &[r#"{"power": 4, "next": "addon:ice"}"#],
            ),
            file("data/addon/example/spell/ice.json", &[r#"{"power": 2}"#]),
        ];
        // `ice` leaves `next` at its default, `example:fire`, which exists: a cycle, which
        // references by name allow.
        let set = DatapackRegistrySet::first(candidate(&forward), 1);
        assert_eq!(power(&set, "example:fire"), DatapackValue::Int(4));
        let missing = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 4, "next": "addon:ice"}"#],
        )];
        assert_eq!(
            DatapackCandidate::build(&[spell()], &missing),
            Err(Box::new(DatapackLoadError {
                registry: "example:spell".into(),
                entry: "example:fire".into(),
                file: "data/example/example/spell/fire.json".into(),
                reason: DatapackLoadReason::UnresolvedReference {
                    field: "next".into(),
                    registry: "example:spell".into(),
                    target: "addon:ice".into(),
                },
            }))
        );
    }

    /// A reference to a registry no declaration covers (a vanilla one) is not checked here.
    #[test]
    fn a_reference_to_an_undeclared_registry_is_left_to_its_owner() {
        let mut declaration = spell();
        declaration.fields[1].kind = DatapackFieldKind::Reference {
            registry: "minecraft:block".into(),
        };
        declaration.fields[1].default = Some(DatapackValue::Text("minecraft:stone".into()));
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 1, "next": "minecraft:nothing"}"#],
        )];
        assert!(DatapackCandidate::build(&[declaration], &files).is_ok());
    }

    /// An entry whose id is not a lowercase `namespace:path`, or whose file is not UTF-8, refuses
    /// the candidate.
    #[test]
    fn an_unusable_entry_file_refuses_the_candidate() {
        let reason = |files: &[(String, Vec<Vec<u8>>)]| {
            DatapackCandidate::build(&[spell()], files)
                .unwrap_err()
                .reason
        };
        assert_eq!(
            reason(&[file(
                "data/example/example/spell/Fire.json",
                &[r#"{"power": 1}"#]
            )]),
            DatapackLoadReason::InvalidEntryId
        );
        assert_eq!(
            reason(&[(
                "data/example/example/spell/fire.json".to_owned(),
                vec![vec![0xff, 0xfe]]
            )]),
            DatapackLoadReason::NotUtf8
        );
    }

    /// With nothing declared, no file is read and the set is empty.
    #[test]
    fn nothing_declared_decodes_nothing() {
        let files = [file(
            "data/example/example/spell/fire.json",
            &[r#"{"power": 101}"#],
        )];
        let candidate = DatapackCandidate::build(&[], &files).unwrap();
        let set = DatapackRegistrySet::first(candidate, 1);
        assert_eq!(
            set.entries("example:spell"),
            Err(DatapackReadError::UnknownRegistry)
        );
    }

    /// The generation never wraps.
    #[test]
    fn the_generation_counter_refuses_to_wrap() {
        let mut set = DatapackRegistrySet::first(candidate(&[]), 1);
        set.generation = u64::MAX;
        assert_eq!(
            set.reload(candidate(&[])),
            Err(DatapackGenerationExhausted::Exhausted)
        );
    }
}

#[cfg(feature = "testing")]
#[path = "datapack_registry_fixture_support.rs"]
mod fixture_support;
