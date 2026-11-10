//! The registries a server sends its clients (arismoko/horizon#811, stage 4, the engine-neutral
//! half): a snapshot of the synchronized registries of one published set, encoded with the
//! wire format, and the checks a client makes before it activates anything. Negotiating which
//! registries a peer gets and carrying the bytes are the game's. See
//! `docs/modding/src/concepts/mod-datapack-registries.md`.
use super::datapack_registry::{
    DatapackEntry, DatapackField, DatapackRegistryDeclaration, DatapackRegistrySync, DatapackValue,
    valid_namespaced,
};
use super::datapack_registry_set::{
    DatapackCandidate, DatapackLoadError, DatapackRegistrySet, resolve_references,
};
use crate::wire::{Reader, Wire, Writer};
use std::collections::{BTreeMap, BTreeSet};

/// The most bytes an encoded snapshot may take.
pub const MAX_SNAPSHOT_BYTES: usize = 64 << 20;
/// The most registries a snapshot may carry.
pub const MAX_SNAPSHOT_REGISTRIES: usize = 256;
/// The most entries one registry of a snapshot may carry.
pub const MAX_SNAPSHOT_ENTRIES: usize = 65_536;

/// One entry of a snapshot: its id and one value per field of the registry, in schema order.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain pair that the snapshot builds and reads
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackSnapshotEntry {
    /// The entry's id (`addon:fire`).
    pub id: String,
    /// The values, one per field of the registry's schema.
    pub values: Vec<DatapackValue>,
}

/// One registry of a snapshot: its key, the schema the server decoded it with, and its entries
/// sorted by id.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain record that the snapshot builds and reads
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackSnapshotRegistry {
    /// The registry's key (`example:spell`).
    pub key: String,
    /// The schema, which the client compares with its own declaration.
    pub fields: Vec<DatapackField>,
    /// The entries, sorted by id.
    pub entries: Vec<DatapackSnapshotEntry>,
}

/// The synchronized registries of a set at one generation, as a server sends them.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a plain record that the snapshot builds and reads
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackSnapshot {
    /// The server's world session.
    pub session: u64,
    /// The generation these entries belong to, from 1.
    pub generation: u64,
    /// The registries, sorted by key.
    pub registries: Vec<DatapackSnapshotRegistry>,
}

/// Why a snapshot was refused. A refused snapshot activates nothing.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game reports it and disconnects
#[non_exhaustive]
pub enum DatapackSnapshotError {
    /// The bytes are not a snapshot, or are more than [`MAX_SNAPSHOT_BYTES`].
    Malformed,
    /// More registries or entries than [`MAX_SNAPSHOT_REGISTRIES`] and [`MAX_SNAPSHOT_ENTRIES`].
    TooLarge,
    /// The snapshot has a registry this side did not declare.
    UnknownRegistry {
        /// The registry's key.
        registry: String,
    },
    /// This side declared the registry with a different schema, or the snapshot lists it twice.
    SchemaMismatch {
        /// The registry's key.
        registry: String,
    },
    /// This side declared the registry as required and the snapshot has none of it.
    MissingRegistry {
        /// The registry's key.
        registry: String,
    },
    /// An entry is not a valid id, is listed twice, or has a value its field refuses.
    InvalidEntry {
        /// The registry's key.
        registry: String,
        /// The entry's id.
        entry: String,
    },
    /// A reference names an entry the snapshot does not have.
    Unresolved(Box<DatapackLoadError>),
}

/// Why a server refuses to send a client its registries: a registry the server declared as
/// required is not one the client offered with the same schema.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: the game turns it into a disconnect reason before any payload
#[non_exhaustive]
pub enum DatapackNegotiationError {
    /// The client did not offer the required registry, or offered it with another schema.
    RequiredRegistry {
        /// The registry's key.
        registry: String,
        /// The mod that declared it.
        mod_id: String,
    },
    /// The client's offer is not readable, or is over the bounds.
    UnreadableOffer,
}

impl DatapackRegistrySet {
    /// The snapshot of this set a peer receives: the registries `declarations` list as
    /// synchronized and `accepts` lets through (the game passes what the peer negotiated).
    /// Server-only registries are never in it.
    pub fn snapshot(
        &self,
        declarations: &[DatapackRegistryDeclaration],
        accepts: impl Fn(&DatapackRegistryDeclaration) -> bool,
    ) -> DatapackSnapshot {
        let mut registries: Vec<_> = declarations
            .iter()
            .filter(|declaration| declaration.sync != DatapackRegistrySync::ServerOnly)
            .filter(|declaration| accepts(declaration))
            .filter_map(|declaration| {
                let entries = self.registries.get(&declaration.key)?;
                Some(DatapackSnapshotRegistry {
                    key: declaration.key.clone(),
                    fields: declaration.fields.clone(),
                    entries: entries
                        .iter()
                        .map(|(id, entry)| DatapackSnapshotEntry {
                            id: id.clone(),
                            values: entry.values.iter().map(|(_, v)| v.clone()).collect(),
                        })
                        .collect(),
                })
            })
            .collect();
        registries.sort_by(|a, b| a.key.cmp(&b.key));
        DatapackSnapshot {
            session: self.session,
            generation: self.generation,
            registries,
        }
    }

    /// The snapshot for a client that `offered` these registries (`offer`): a registry goes
    /// out when the client offered the same key with the same schema. A required registry the
    /// client did not offer so refuses it before any payload; an optional one it did not offer
    /// is left out, with nothing sent in its place.
    pub fn snapshot_for(
        &self,
        declarations: &[DatapackRegistryDeclaration],
        offered: &[DatapackRegistryDeclaration],
    ) -> Result<DatapackSnapshot, DatapackNegotiationError> {
        let matches = |declaration: &DatapackRegistryDeclaration| {
            offered
                .iter()
                .any(|o| o.key == declaration.key && o.fields == declaration.fields)
        };
        if let Some(missing) = declarations.iter().find(|declaration| {
            declaration.sync == DatapackRegistrySync::Required && !matches(declaration)
        }) {
            return Err(DatapackNegotiationError::RequiredRegistry {
                registry: missing.key.clone(),
                mod_id: missing.mod_id.clone(),
            });
        }
        Ok(self.snapshot(declarations, matches))
    }

    /// Publishes a snapshot's candidate on a client at the generation the server sent. The
    /// session is this client's own, so a handle from an earlier world fails as on a server.
    pub fn received(
        snapshot: &DatapackSnapshot,
        candidate: DatapackCandidate,
        session: u64,
    ) -> Self {
        Self {
            session,
            generation: snapshot.generation,
            registries: candidate.registries,
        }
    }
}

impl DatapackRegistrySet {
    /// Publishes a later snapshot's candidate over this set, on a client: the same session, the
    /// server's generation. `None` when the snapshot's generation does not come after this
    /// one's, so a snapshot out of order or sent twice changes nothing.
    pub fn received_next(
        &self,
        snapshot: &DatapackSnapshot,
        candidate: DatapackCandidate,
    ) -> Option<Self> {
        (snapshot.generation > self.generation).then_some(Self {
            session: self.session,
            generation: snapshot.generation,
            registries: candidate.registries,
        })
    }
}

impl DatapackSnapshot {
    /// What a client offers its server: the registries `declarations` list as synchronized,
    /// with their schemas, as wire bytes.
    pub fn encode_offer(declarations: &[DatapackRegistryDeclaration]) -> Option<Vec<u8>> {
        let offer: Vec<DatapackRegistryDeclaration> = declarations
            .iter()
            .filter(|declaration| declaration.sync != DatapackRegistrySync::ServerOnly)
            .cloned()
            .collect();
        let mut writer = Writer::new();
        writer.limit(MAX_SNAPSHOT_BYTES);
        offer.put(&mut writer);
        writer.error().is_none().then(|| writer.into_bytes())
    }

    /// Reads a client's offer, refusing more than [`MAX_SNAPSHOT_REGISTRIES`] registries.
    pub fn decode_offer(
        bytes: &[u8],
    ) -> Result<Vec<DatapackRegistryDeclaration>, DatapackNegotiationError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(DatapackNegotiationError::UnreadableOffer);
        }
        Reader::decode::<Vec<DatapackRegistryDeclaration>>(bytes)
            .ok()
            .filter(|offer| offer.len() <= MAX_SNAPSHOT_REGISTRIES)
            .ok_or(DatapackNegotiationError::UnreadableOffer)
    }

    /// The wire bytes, or `None` when the snapshot is more than [`MAX_SNAPSHOT_BYTES`].
    pub fn encode(&self) -> Option<Vec<u8>> {
        let mut writer = Writer::new();
        writer.limit(MAX_SNAPSHOT_BYTES);
        self.put(&mut writer);
        writer.error().is_none().then(|| writer.into_bytes())
    }

    /// Reads wire bytes, refusing anything over [`MAX_SNAPSHOT_BYTES`], malformed or with
    /// bytes left over.
    pub fn decode(bytes: &[u8]) -> Result<Self, DatapackSnapshotError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(DatapackSnapshotError::Malformed);
        }
        Reader::decode::<Self>(bytes).map_err(|_| DatapackSnapshotError::Malformed)
    }

    /// Checks the snapshot against this side's `declarations` and builds the candidate to
    /// activate: every registry must be one this side declared with the same schema, every
    /// value must fit its field, ids must be valid and unique, references must resolve, and a
    /// registry declared `Required` must be present. A declared registry the snapshot lacks
    /// (server-only, or optional and not negotiated) stays empty. Nothing is built unless
    /// everything passes.
    pub fn candidate(
        &self,
        declarations: &[DatapackRegistryDeclaration],
    ) -> Result<DatapackCandidate, DatapackSnapshotError> {
        if self.registries.len() > MAX_SNAPSHOT_REGISTRIES {
            return Err(DatapackSnapshotError::TooLarge);
        }
        let mut registries: BTreeMap<String, BTreeMap<String, DatapackEntry>> = declarations
            .iter()
            .map(|declaration| (declaration.key.clone(), BTreeMap::new()))
            .collect();
        let mut seen = BTreeSet::new();
        for sent in &self.registries {
            let registry = sent.key.clone();
            let declaration = declarations
                .iter()
                .find(|declaration| declaration.key == sent.key)
                .ok_or_else(|| DatapackSnapshotError::UnknownRegistry {
                    registry: registry.clone(),
                })?;
            if !seen.insert(sent.key.as_str()) || declaration.fields != sent.fields {
                return Err(DatapackSnapshotError::SchemaMismatch { registry });
            }
            if sent.entries.len() > MAX_SNAPSHOT_ENTRIES {
                return Err(DatapackSnapshotError::TooLarge);
            }
            let entries = registries.get_mut(&sent.key).expect("declared");
            for entry in &sent.entries {
                let invalid = || DatapackSnapshotError::InvalidEntry {
                    registry: registry.clone(),
                    entry: entry.id.clone(),
                };
                if !valid_namespaced(&entry.id)
                    || entry.values.len() != declaration.fields.len()
                    || !declaration
                        .fields
                        .iter()
                        .zip(&entry.values)
                        .all(|(field, value)| field.kind.accepts(value))
                {
                    return Err(invalid());
                }
                let values = declaration
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .zip(entry.values.iter().cloned())
                    .collect();
                if entries
                    .insert(entry.id.clone(), DatapackEntry { values })
                    .is_some()
                {
                    return Err(invalid());
                }
            }
        }
        if let Some(missing) = declarations.iter().find(|declaration| {
            declaration.sync == DatapackRegistrySync::Required
                && !seen.contains(declaration.key.as_str())
        }) {
            return Err(DatapackSnapshotError::MissingRegistry {
                registry: missing.key.clone(),
            });
        }
        resolve_references(declarations, &registries).map_err(DatapackSnapshotError::Unresolved)?;
        Ok(DatapackCandidate { registries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::datapack_registry::DatapackFieldKind;

    fn spell(sync: DatapackRegistrySync) -> DatapackRegistryDeclaration {
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
                    DatapackValue::Text("addon:fire".into()),
                ),
            ],
            sync,
        }
    }

    fn set(declaration: &DatapackRegistryDeclaration) -> DatapackRegistrySet {
        let files = vec![
            (
                "data/addon/example/spell/fire.json".to_owned(),
                vec![br#"{"power": 7}"#.to_vec()],
            ),
            (
                "data/addon/example/spell/ice.json".to_owned(),
                vec![br#"{"power": 4, "next": "addon:fire"}"#.to_vec()],
            ),
        ];
        let candidate =
            DatapackCandidate::build(std::slice::from_ref(declaration), &files).unwrap();
        DatapackRegistrySet::first(candidate, 1)
    }

    #[test]
    fn a_client_reads_what_the_server_sent_at_the_same_generation() {
        let declaration = spell(DatapackRegistrySync::Required);
        let server = set(&declaration);
        let server = server
            .reload(
                DatapackCandidate::build(
                    std::slice::from_ref(&declaration),
                    &[(
                        "data/addon/example/spell/fire.json".to_owned(),
                        vec![br#"{"power": 9}"#.to_vec()],
                    )],
                )
                .unwrap(),
            )
            .unwrap();
        let snapshot = server.snapshot(std::slice::from_ref(&declaration), |_| true);
        let bytes = snapshot.encode().unwrap();
        let received = DatapackSnapshot::decode(&bytes).unwrap();
        assert_eq!(received, snapshot);
        let candidate = received
            .candidate(std::slice::from_ref(&declaration))
            .unwrap();
        let client = DatapackRegistrySet::received(&received, candidate, 99);
        assert_eq!(client.generation(), 2);
        let handle = client.handle("example:spell", "addon:fire").unwrap();
        assert_eq!(
            client.get(&handle).unwrap().get("power"),
            Some(&DatapackValue::Int(9))
        );
    }

    #[test]
    fn server_only_and_unaccepted_registries_are_not_sent() {
        let only = spell(DatapackRegistrySync::ServerOnly);
        assert!(
            set(&only)
                .snapshot(std::slice::from_ref(&only), |_| true)
                .registries
                .is_empty()
        );
        let optional = spell(DatapackRegistrySync::Optional);
        assert!(
            set(&optional)
                .snapshot(std::slice::from_ref(&optional), |_| false)
                .registries
                .is_empty()
        );
    }

    #[test]
    fn a_client_refuses_a_snapshot_that_does_not_match_its_declarations() {
        let declaration = spell(DatapackRegistrySync::Required);
        let snapshot = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        // A registry this side never declared.
        assert_eq!(
            snapshot.candidate(&[]).unwrap_err(),
            DatapackSnapshotError::UnknownRegistry {
                registry: "example:spell".into()
            }
        );
        // A different schema.
        let mut other = declaration.clone();
        other.fields.pop();
        assert_eq!(
            snapshot.candidate(&[other]).unwrap_err(),
            DatapackSnapshotError::SchemaMismatch {
                registry: "example:spell".into()
            }
        );
        // A required registry the snapshot lacks.
        let empty = DatapackSnapshot {
            session: 1,
            generation: 1,
            registries: Vec::new(),
        };
        assert_eq!(
            empty
                .candidate(std::slice::from_ref(&declaration))
                .unwrap_err(),
            DatapackSnapshotError::MissingRegistry {
                registry: "example:spell".into()
            }
        );
        // An optional one it lacks stays empty.
        let optional = spell(DatapackRegistrySync::Optional);
        let candidate = empty.candidate(std::slice::from_ref(&optional)).unwrap();
        let client = DatapackRegistrySet::received(&empty, candidate, 2);
        assert_eq!(client.entries("example:spell").unwrap(), Vec::<&str>::new());
    }

    #[test]
    fn a_client_refuses_bad_entries_and_dangling_references() {
        let declaration = spell(DatapackRegistrySync::Required);
        let good = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        let refuses = |change: &dyn Fn(&mut DatapackSnapshot)| {
            let mut snapshot = good.clone();
            change(&mut snapshot);
            snapshot
                .candidate(std::slice::from_ref(&declaration))
                .unwrap_err()
        };
        // A value past its field's bound.
        assert!(matches!(
            refuses(&|s| s.registries[0].entries[0].values[0] = DatapackValue::Int(101)),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // The wrong number of values.
        assert!(matches!(
            refuses(&|s| {
                s.registries[0].entries[0].values.pop();
            }),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // A bad id, and an id listed twice.
        assert!(matches!(
            refuses(&|s| s.registries[0].entries[0].id = "NoColon".into()),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        assert!(matches!(
            refuses(&|s| {
                let first = s.registries[0].entries[0].clone();
                s.registries[0].entries.push(first);
            }),
            DatapackSnapshotError::InvalidEntry { .. }
        ));
        // A reference to an entry the snapshot does not have.
        assert!(matches!(
            refuses(&|s| {
                s.registries[0].entries[1].values[1] = DatapackValue::Text("addon:gone".into())
            }),
            DatapackSnapshotError::Unresolved(_)
        ));
        // The same registry twice.
        assert!(matches!(
            refuses(&|s| {
                let first = s.registries[0].clone();
                s.registries.push(first);
            }),
            DatapackSnapshotError::SchemaMismatch { .. }
        ));
    }

    #[test]
    fn a_server_sends_a_client_only_what_it_offered() {
        let required = spell(DatapackRegistrySync::Required);
        let server = set(&required);
        let declarations = std::slice::from_ref(&required);
        let offer = DatapackSnapshot::encode_offer(declarations).unwrap();
        let offered = DatapackSnapshot::decode_offer(&offer).unwrap();
        assert_eq!(offered, declarations);
        let snapshot = server.snapshot_for(declarations, &offered).unwrap();
        assert_eq!(snapshot.registries.len(), 1);
        // A client without the mod, or with another schema, is refused before any payload.
        let refused = DatapackNegotiationError::RequiredRegistry {
            registry: "example:spell".into(),
            mod_id: "example".into(),
        };
        assert_eq!(server.snapshot_for(declarations, &[]), Err(refused.clone()));
        let mut other = required.clone();
        other.fields.pop();
        assert_eq!(server.snapshot_for(declarations, &[other]), Err(refused));
        // An optional registry the client lacks is left out; the join goes on.
        let optional = spell(DatapackRegistrySync::Optional);
        let snapshot = set(&optional)
            .snapshot_for(std::slice::from_ref(&optional), &[])
            .unwrap();
        assert!(snapshot.registries.is_empty());
        // A server-only registry is not in a client's offer.
        let only = spell(DatapackRegistrySync::ServerOnly);
        let offer = DatapackSnapshot::encode_offer(std::slice::from_ref(&only)).unwrap();
        assert!(DatapackSnapshot::decode_offer(&offer).unwrap().is_empty());
    }

    #[test]
    fn an_offer_that_is_not_one_is_refused() {
        let required = spell(DatapackRegistrySync::Required);
        let mut bytes = DatapackSnapshot::encode_offer(std::slice::from_ref(&required)).unwrap();
        bytes.push(0);
        assert_eq!(
            DatapackSnapshot::decode_offer(&bytes),
            Err(DatapackNegotiationError::UnreadableOffer)
        );
        // More registries than the bound is refused.
        let many: Vec<DatapackRegistryDeclaration> = (0..=MAX_SNAPSHOT_REGISTRIES)
            .map(|n| DatapackRegistryDeclaration {
                key: format!("example:spell_{n}"),
                ..required.clone()
            })
            .collect();
        let mut writer = Writer::new();
        many.put(&mut writer);
        assert_eq!(
            DatapackSnapshot::decode_offer(&writer.into_bytes()),
            Err(DatapackNegotiationError::UnreadableOffer)
        );
    }

    #[test]
    fn bytes_that_are_not_a_snapshot_are_refused() {
        let declaration = spell(DatapackRegistrySync::Required);
        let snapshot = set(&declaration).snapshot(std::slice::from_ref(&declaration), |_| true);
        let mut bytes = snapshot.encode().unwrap();
        bytes.push(0);
        assert_eq!(
            DatapackSnapshot::decode(&bytes),
            Err(DatapackSnapshotError::Malformed)
        );
        bytes.truncate(bytes.len() - 3);
        assert_eq!(
            DatapackSnapshot::decode(&bytes),
            Err(DatapackSnapshotError::Malformed)
        );
    }
}
