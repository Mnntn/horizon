//! Data maps for native server mods (docs/modding/src/how-to/native-mods.md): values a data pack
//! attaches to the entries of a registry, under the types a mod registered
//! (`native_data_map_types`). Vanilla has no data maps; the contract is
//! NeoForge's `DataMapType`: files at
//! `data/<map namespace>/data_maps/<registry path>/<map path>.json` hold
//! `replace`, `values` (entry ids and `#tags` to values) and `remove`, and a
//! newer value overrides an older one for the same entry. The files are the
//! world's data packs' and what mods contribute through
//! `native_data_pack_files` (key `data_maps/<ns>:<registry path>/<map path>`),
//! merged each time the server's tables are built: a world's first open and
//! every `/reload`.
use super::Game;
use super::data_map_json::to_tag;
use horizon_core::nbt::Tag;
pub use mod_api::game::DataMapType;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The registries data maps can be attached to so far, with their folder
/// (the registry's path: `minecraft` registries have no namespace folder).
const REGISTRIES: [(&str, &str); 2] = [("minecraft:item", "item"), ("minecraft:block", "block")];

/// The registries `types` are for that have data maps, in `REGISTRIES` order.
pub(crate) fn registries_of(types: &[DataMapType]) -> Vec<&'static str> {
    REGISTRIES
        .iter()
        .map(|(registry, _)| *registry)
        .filter(|registry| types.iter().any(|ty| ty.registry == *registry))
        .collect()
}

/// The merged values of every registered type: registry, then type id, to the
/// value of each entry that has one.
#[derive(Default)]
pub(crate) struct DataMaps {
    maps: HashMap<&'static str, HashMap<String, BTreeMap<String, Value>>>,
}

/// `minecraft:` for an id written without a namespace.
fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// A file's value for an entry: the data itself, or the wrapper
/// `{"value": ..., "replace": false}` (`DataMapEntry`'s codec). `replace` only
/// matters to mergers, which types don't have yet: a newer value overrides.
fn entry_value(raw: &Value) -> Option<Value> {
    if let Value::Object(map) = raw
        && let Some(value) = map.get("value")
        && map
            .iter()
            .all(|(k, v)| k == "value" || (k == "replace" && v.is_boolean()))
    {
        return Some(value.clone());
    }
    (!raw.is_null()).then(|| raw.clone())
}

/// One file over what earlier files made of the type: `replace` clears it,
/// `values` attach (a tag to every entry in it), then `remove` takes entries
/// away, as `DataMapFile` is applied. False when the file isn't a data map.
fn apply_file(
    map: &mut BTreeMap<String, Value>,
    file: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> bool {
    let (Some(root), Some(values)) = (file.as_object(), file.get("values")) else {
        return false;
    };
    let Some(values) = values.as_object() else {
        return false;
    };
    if root.get("replace").and_then(Value::as_bool) == Some(true) {
        map.clear();
    }
    // The members of a tag or the entry itself.
    let members = |key: &str| -> Vec<String> {
        match key.strip_prefix('#') {
            Some(tag) => tags
                .get(&full_id(tag))
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default(),
            None => vec![full_id(key)],
        }
    };
    for (key, raw) in values {
        let Some(value) = entry_value(raw) else {
            continue;
        };
        for entry in members(key) {
            map.insert(entry, value.clone());
        }
    }
    for removal in root
        .get("remove")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // An entry or `{"key": entry}`, as `DataMapEntry.Removal` reads it.
        let key = removal.as_str().or_else(|| removal["key"].as_str());
        for entry in key.map(members).unwrap_or_default() {
            map.remove(&entry);
        }
    }
    true
}

impl DataMaps {
    /// The types' values from `files` (the server's tables' pack files, in
    /// pack order, mods' below the packs'), tags resolved against the
    /// loaded `item_tags` and `block_tags`. A type with an unsupported
    /// registry or a bad id is left out with a warning; a file that isn't a
    /// data map is skipped alone.
    pub(crate) fn load(
        types: &[DataMapType],
        overlay: &horizon_world::loot::PackOverlay<'_>,
        item_tags: &HashMap<String, HashSet<String>>,
        block_tags: &HashMap<String, HashSet<String>>,
    ) -> Self {
        let mut maps: HashMap<&'static str, HashMap<String, BTreeMap<String, Value>>> =
            HashMap::new();
        for ty in types {
            let Some((registry, dir)) = REGISTRIES.iter().find(|(r, _)| *r == ty.registry) else {
                log::warn!(
                    "Horizon: data map {} is for {}, which has no data maps yet: left out",
                    ty.id,
                    ty.registry
                );
                continue;
            };
            let valid = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
            };
            let ok = ty
                .id
                .split_once(':')
                .is_some_and(|(ns, path)| ns != "minecraft" && valid(ns) && valid(path));
            if !ok {
                log::warn!(
                    "Horizon: data map type {} is not a mod's id: left out",
                    ty.id
                );
                continue;
            }
            let registered = maps.entry(registry).or_default();
            if registered.contains_key(&ty.id) {
                log::warn!(
                    "Horizon: data map {} registered twice: the first is kept",
                    ty.id
                );
                continue;
            }
            let tags = if *dir == "item" {
                item_tags
            } else {
                block_tags
            };
            let (ns, path) = ty.id.split_once(':').unwrap_or_default();
            let wanted = format!("data_maps/{ns}:{dir}/{path}");
            let mut map = BTreeMap::new();
            if let Some((_, versions)) = overlay.files.iter().find(|(k, _)| *k == wanted) {
                for bytes in versions {
                    let file = serde_json::from_slice::<Value>(bytes).ok();
                    if file
                        .as_ref()
                        .is_some_and(|file| !overlay.conditions.allows(file))
                    {
                        continue;
                    }
                    let applied = file.is_some_and(|file| apply_file(&mut map, &file, tags));
                    if !applied {
                        log::warn!(
                            "Horizon: a data map file of {} is not a data map, skipped",
                            ty.id
                        );
                    }
                }
            }
            registered.insert(ty.id.clone(), map);
        }
        Self { maps }
    }

    /// `entry`'s value in map `type_id` of `registry`, as NBT: none where the
    /// map gives it none, refused where no mod registered the map there.
    fn value(
        &self,
        registry: &str,
        type_id: &str,
        entry: &str,
    ) -> Result<Option<Tag>, mod_api::DataMapError> {
        let map = self
            .maps
            .get(registry)
            .and_then(|maps| maps.get(type_id))
            .ok_or(mod_api::DataMapError::UnknownType)?;
        map.get(entry).map(to_tag).transpose()
    }
}

impl Game<'_> {
    /// Tell the mods the data maps of every registry with a type were merged
    /// (`DataMapsUpdatedEvent`), once the server's tables hold them.
    pub(in crate::game) fn announce_data_maps_updated(
        &mut self,
        cause: mod_api::DataMapsUpdateCause,
    ) {
        for registry in self.data_map_registries.clone() {
            self.native_data_maps_updated(registry, cause);
        }
    }
}

impl Game<'_> {
    /// The server's merged data maps, none unless this is a server whose
    /// tables were built from packs and mods' files.
    fn loaded_data_maps(&self) -> Result<&DataMaps, mod_api::DataMapError> {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return Err(mod_api::DataMapError::Unavailable);
        }
        self.world_tables
            .as_ref()
            .map(|tables| &tables.data_maps)
            .ok_or(mod_api::DataMapError::Unavailable)
    }
    /// Item `item`'s value in data map `map`, before the map decodes it: the merged JSON the
    /// server's data packs and mods' files gave it, as NBT. `Ok(None)` where it has none.
    pub(in crate::game) fn item_data_map_value(
        &self,
        item: mod_api::ItemKey,
        map: &str,
    ) -> Result<Option<Tag>, mod_api::DataMapError> {
        let maps = self.loaded_data_maps()?;
        let id = mod_api::__private::item_key_id(item).ok_or(mod_api::DataMapError::Unavailable)?;
        maps.value("minecraft:item", map, id)
    }
    /// Block `block`'s value in data map `map`, as `item_data_map_value` answers for an item.
    pub(in crate::game) fn block_data_map_value(
        &self,
        block: mod_api::BlockKey,
        map: &str,
    ) -> Result<Option<Tag>, mod_api::DataMapError> {
        let maps = self.loaded_data_maps()?;
        let id =
            mod_api::__private::block_key_id(block).ok_or(mod_api::DataMapError::Unavailable)?;
        maps.value("minecraft:block", map, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_map_read_refuses_lossy_payloads_instead_of_successful_substitutes() {
        for value in [
            json!({"nested": [null]}),
            json!(9223372036854775809_u64),
            json!(u64::MAX),
        ] {
            let maps = DataMaps {
                maps: HashMap::from([(
                    "minecraft:item",
                    HashMap::from([(
                        "test:map".to_owned(),
                        BTreeMap::from([("minecraft:stone".to_owned(), value)]),
                    )]),
                )]),
            };
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:stone"),
                Err(mod_api::DataMapError::Undecodable)
            );
            assert_eq!(
                maps.value("minecraft:item", "test:map", "minecraft:air"),
                Ok(None)
            );
        }
    }

    fn tags(list: &[(&str, &[&str])]) -> HashMap<String, HashSet<String>> {
        list.iter()
            .map(|(tag, members)| {
                (
                    (*tag).to_owned(),
                    members.iter().map(|m| (*m).to_owned()).collect(),
                )
            })
            .collect()
    }

    fn ty(id: &str, registry: &str) -> DataMapType {
        DataMapType {
            id: id.to_owned(),
            registry: registry.to_owned(),
        }
    }

    fn files(list: &[(&str, &[Value])]) -> Vec<(String, Vec<Vec<u8>>)> {
        list.iter()
            .map(|(key, versions)| {
                (
                    (*key).to_owned(),
                    versions
                        .iter()
                        .map(|v| v.to_string().into_bytes())
                        .collect(),
                )
            })
            .collect()
    }

    fn load(files: &[(String, Vec<Vec<u8>>)]) -> DataMaps {
        let items = tags(&[(
            "minecraft:logs",
            &["minecraft:oak_log", "minecraft:birch_log"],
        )]);
        DataMaps::load(
            &[ty("mymod:fuel", "minecraft:item")],
            &horizon_world::loot::PackOverlay {
                files,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &items,
            &HashMap::new(),
        )
    }

    fn get<'a>(maps: &'a DataMaps, entry: &str) -> Option<&'a Value> {
        maps.maps["minecraft:item"]["mymod:fuel"].get(entry)
    }

    #[test]
    fn a_value_reads_as_nbt_and_a_map_no_mod_registered_is_refused() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": 1600,
                "minecraft:stick": {"burn": 3_000_000_000_i64}
            }})],
        )]);
        let maps = load(&f);
        let value =
            |registry: &str, type_id: &str, entry: &str| maps.value(registry, type_id, entry);
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:coal"),
            Ok(Some(Tag::Int(1600)))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stick"),
            Ok(Some(Tag::Compound(
                [("burn".into(), Tag::Long(3_000_000_000))]
                    .into_iter()
                    .collect()
            )))
        );
        assert_eq!(
            value("minecraft:item", "mymod:fuel", "minecraft:stone"),
            Ok(None)
        );
        assert_eq!(
            value("minecraft:block", "mymod:fuel", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
        assert_eq!(
            value("minecraft:item", "mymod:other", "minecraft:coal"),
            Err(mod_api::DataMapError::UnknownType)
        );
    }

    #[test]
    fn registries_with_a_type_are_listed_once_in_registry_order() {
        let types = [
            ty("mymod:b", "minecraft:block"),
            ty("mymod:a", "minecraft:item"),
            ty("mymod:c", "minecraft:block"),
            ty("mymod:d", "minecraft:biome"),
        ];
        assert_eq!(registries_of(&types), ["minecraft:item", "minecraft:block"]);
        assert!(registries_of(&[ty("mymod:d", "minecraft:biome")]).is_empty());
        assert!(registries_of(&[]).is_empty());
    }

    #[test]
    fn values_attach_to_entries_and_to_every_member_of_a_tag() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"burn": 1600},
                "stick": 100,
                "#minecraft:logs": {"burn": 300}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 1600})));
        // An id without a namespace is a `minecraft` one.
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(100)));
        assert_eq!(get(&maps, "minecraft:oak_log"), Some(&json!({"burn": 300})));
        assert_eq!(
            get(&maps, "minecraft:birch_log"),
            Some(&json!({"burn": 300}))
        );
        assert_eq!(get(&maps, "minecraft:stone"), None);
    }

    #[test]
    fn a_newer_file_overrides_and_replace_clears_what_came_before() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"replace": true, "values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
    }

    #[test]
    fn remove_takes_entries_and_tags_away_after_the_files_own_values() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"#minecraft:logs": 5, "minecraft:coal": 1}}),
                // The file's own value for coal is attached, then removed.
                json!({"values": {"minecraft:coal": 9}, "remove": ["minecraft:coal", "#minecraft:logs"]}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), None);
        assert_eq!(get(&maps, "minecraft:oak_log"), None);
    }

    #[test]
    fn the_wrapper_form_gives_its_value_and_a_plain_object_is_a_value() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"value": {"burn": 3}, "replace": true},
                "minecraft:stick": {"value": 1, "other": 2}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 3})));
        // More than `value` and `replace`: the object is the data.
        assert_eq!(
            get(&maps, "minecraft:stick"),
            Some(&json!({"value": 1, "other": 2}))
        );
    }

    #[test]
    fn a_bad_file_is_skipped_alone_and_bad_types_are_left_out() {
        let mut f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {"minecraft:coal": 1}})],
        )]);
        f[0].1.insert(0, b"{nope".to_vec());
        f[0].1
            .push(json!({"nothing": true}).to_string().into_bytes());
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        let maps = DataMaps::load(
            &[
                ty("minecraft:fuel", "minecraft:item"),
                ty("mymod:x", "minecraft:biome"),
                ty("nonamespace", "minecraft:item"),
                ty("mymod:a", "minecraft:block"),
                ty("mymod:a", "minecraft:block"),
            ],
            &horizon_world::loot::PackOverlay::NONE,
            &HashMap::new(),
            &HashMap::new(),
        );
        // Only the one good type, registered once, empty.
        assert_eq!(maps.maps.len(), 1);
        let blocks = &maps.maps["minecraft:block"];
        assert_eq!(blocks.len(), 1);
        assert!(blocks["mymod:a"].is_empty());
    }

    #[test]
    fn conditional_layers_are_skipped_before_replace_and_remove() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values":{"minecraft:coal":1}}),
                json!({"neoforge:conditions":[{"type":"unsupported"}],"replace":true}),
                json!({"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"provider"}],"remove":["minecraft:coal"],"values":{"minecraft:stick":2}}),
            ],
        )]);
        let off = load(&f);
        assert_eq!(get(&off, "minecraft:coal"), Some(&json!(1)));
        assert!(get(&off, "minecraft:stick").is_none());
        let enabled = horizon_world::conditions::LoadConditions::new(["provider"]);
        let on = DataMaps::load(
            &[ty("mymod:fuel", "minecraft:item")],
            &horizon_world::loot::PackOverlay {
                files: &f,
                conditions: &enabled,
            },
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(get(&on, "minecraft:coal").is_none());
        assert_eq!(get(&on, "minecraft:stick"), Some(&json!(2)));
    }

    #[test]
    fn block_maps_read_the_block_tags() {
        let blocks = tags(&[("minecraft:logs", &["minecraft:oak_log"])]);
        let f = files(&[(
            "data_maps/mymod:block/hardness",
            &[json!({"values": {"#minecraft:logs": 2}})],
        )]);
        let maps = DataMaps::load(
            &[ty("mymod:hardness", "minecraft:block")],
            &horizon_world::loot::PackOverlay {
                files: &f,
                ..horizon_world::loot::PackOverlay::NONE
            },
            &HashMap::new(),
            &blocks,
        );
        let map = &maps.maps["minecraft:block"]["mymod:hardness"];
        assert_eq!(map.get("minecraft:oak_log"), Some(&json!(2)));
    }
}
