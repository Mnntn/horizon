//! The code mods a world was saved with: `level.dat`'s `fml.LoadingModList`, the list
//! NeoForge keeps, and how it compares with the mods loaded now (`ModMismatchEvent`).
use super::{Level, Tag, compound_or_empty, put, string, text};
use std::sync::RwLock;

/// A code mod as the list names it: one `{ModId, ModVersion}` compound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModRecord {
    pub id: String,
    /// The mod's `horizon-mod.toml` version; `""` when it wasn't known.
    pub version: String,
}

/// The code mods this process loaded, once the game's start said which.
static LOADED: RwLock<Option<Vec<ModRecord>>> = RwLock::new(None);

/// The code mods loaded now, in load order, each with its version (`""` when unknown): what
/// every `level.dat` written from here on records. The game's start says it once. Returns the
/// list it replaced: the list is the whole process's, so a test that sets one puts that back.
pub fn set_loaded(mods: Option<Vec<ModRecord>>) -> Option<Vec<ModRecord>> {
    std::mem::replace(
        &mut *LOADED.write().unwrap_or_else(|e| e.into_inner()),
        mods,
    )
}

/// The loaded mods `set_loaded` gave; `None` before it (a tool or a test that didn't say).
pub fn loaded() -> Option<Vec<ModRecord>> {
    LOADED.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The list a level.dat's root holds; `None` without one (vanilla, or saved before Horizon
/// wrote it).
pub(super) fn saved(data: &Tag) -> Option<Vec<ModRecord>> {
    let Some(Tag::List(mods)) = data.get("fml").and_then(|fml| fml.get("LoadingModList")) else {
        return None;
    };
    Some(
        mods.iter()
            .map(|entry| ModRecord {
                id: text(entry.get("ModId")).unwrap_or_default(),
                version: text(entry.get("ModVersion")).unwrap_or_default(),
            })
            .collect(),
    )
}

/// Puts the loaded mods' list into `data`'s `fml`, in place of the one there, keeping the rest
/// of `fml`; `data` stays as it is while the loaded mods aren't known.
pub(super) fn write(data: &mut Tag) {
    if let Some(mods) = loaded() {
        write_list(data, &mods);
    }
}

/// `write` with the list given.
fn write_list(data: &mut Tag, mods: &[ModRecord]) {
    let mut fml = compound_or_empty(data.get("fml").cloned());
    let list = mods
        .iter()
        .map(|m| Tag::compound([("ModId", string(&m.id)), ("ModVersion", string(&m.version))]))
        .collect();
    put(&mut fml, "LoadingModList", Tag::List(list));
    put(data, "fml", fml);
}

impl Level {
    /// The saved mods that differ from `loaded`, NeoForge's `ModMismatchEvent`: one saved with
    /// another version (including an unknown saved version) or no longer loaded. A mod loaded now
    /// that the world wasn't saved with is no mismatch, nor is a world without the list.
    pub fn mod_mismatches(&self, loaded: &[ModRecord]) -> Vec<mod_api::ModMismatch> {
        self.saved_mods
            .iter()
            .flatten()
            .filter_map(|saved| {
                if saved.id == "minecraft" {
                    return None;
                }
                let current = loaded.iter().find(|m| m.id == saved.id);
                let differs = current.is_none_or(|current| {
                    match (
                        horizon_mod_package::dependencies::parse_version(&saved.version),
                        horizon_mod_package::dependencies::parse_version(&current.version),
                    ) {
                        (Ok(saved), Ok(current)) => saved != current,
                        _ => saved.version != current.version,
                    }
                });
                differs.then(|| mod_api::ModMismatch {
                    mod_id: saved.id.clone(),
                    saved_version: saved.version.clone(),
                    current_version: current.map(|m| m.version.clone()),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::{read_dat, write_dat};
    use std::fs;

    fn record(id: &str, version: &str) -> ModRecord {
        ModRecord {
            id: id.into(),
            version: version.into(),
        }
    }

    #[test]
    fn malformed_id_is_reported_as_missing() {
        let root = Tag::compound([(
            "fml",
            Tag::compound([(
                "LoadingModList",
                Tag::List(vec![
                    Tag::compound([("ModId", Tag::Int(7))]),
                    Tag::Compound(Default::default()),
                ]),
            )]),
        )]);
        let level = Level {
            saved_mods: saved(&root),
            ..Level::default()
        };
        let differences = level.mod_mismatches(&[]);
        assert_eq!(differences.len(), 2);
        assert!(
            differences
                .iter()
                .all(|m| m.mod_id.is_empty() && m.current_version.is_none())
        );
    }

    /// The loaded mods replace the list the file had and the other `fml` keys stay; a write
    /// before the loaded mods are known leaves the list alone, and a world without one has none.
    /// The loaded list is given to `write_list`, not set: the process-wide one would reach
    /// the level.dat other tests write meanwhile.
    #[test]
    fn the_level_data_records_the_loaded_mods() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let entry = |id: &str, version: &str| {
            Tag::compound([
                ("ModId", Tag::String(id.into())),
                ("ModVersion", Tag::String(version.into())),
            ])
        };
        write_dat(
            &dir.join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([("LevelName", Tag::String("Test".into()))]),
                ),
                (
                    "fml",
                    Tag::compound([
                        ("Other", Tag::Int(7)),
                        ("LoadingModList", Tag::List(vec![entry("old", "0.1.0")])),
                    ]),
                ),
            ]),
        )
        .unwrap();
        let mut level = Level::read(&dir).unwrap();
        assert_eq!(level.saved_mods, Some(vec![record("old", "0.1.0")]));
        level.write(&dir).unwrap();
        assert_eq!(
            Level::read(&dir).unwrap().saved_mods,
            Some(vec![record("old", "0.1.0")])
        );
        let mut data = read_dat(&dir.join("level.dat")).unwrap();
        write_list(&mut data, &[record("first", "1.2.0"), record("second", "")]);
        assert_eq!(
            saved(&data),
            Some(vec![record("first", "1.2.0"), record("second", "")])
        );
        let fml = data.get("fml").unwrap();
        assert_eq!(fml.get("Other"), Some(&Tag::Int(7)));
        assert_eq!(
            fml.get("LoadingModList"),
            Some(&Tag::List(vec![
                entry("first", "1.2.0"),
                entry("second", "")
            ]))
        );
        assert_eq!(
            saved(&Tag::compound([(
                "LevelName",
                Tag::String("Vanilla".into())
            )])),
            None
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// `ModMismatchEvent`: a saved mod with another version, or no longer loaded, and nothing
    /// else: not a mod loaded since, including versions unknown on either side, not a world without
    /// the list.
    #[test]
    fn mod_mismatches_name_changed_and_missing_mods_only() {
        let mismatch = |mod_id: &str, saved: &str, current: Option<&str>| mod_api::ModMismatch {
            mod_id: mod_id.into(),
            saved_version: saved.into(),
            current_version: current.map(str::to_owned),
        };
        let loaded = [
            record("same", "1.0.0"),
            record("changed", "2.0.0"),
            record("unknown_now", ""),
            record("known_now", "3.0.0"),
            record("added", "1.0.0"),
            record("overflow", "99999999999999999999999999999999999999999999"),
            record("minecraft", "99"),
            record("downgrade", "3.0.0"),
        ];
        assert!(Level::default().mod_mismatches(&loaded).is_empty());
        let level = Level {
            saved_mods: Some(vec![
                record("same", "1"),
                record("minecraft", "0"),
                record("downgrade", "4.0.0"),
                record("changed", "1.0.0"),
                record("unknown_now", "1.0.0"),
                record("known_now", ""),
                record("gone", "0.5.0"),
                record("overflow", "1"),
            ]),
            ..Level::default()
        };
        assert_eq!(
            level.mod_mismatches(&loaded),
            vec![
                mismatch("downgrade", "4.0.0", Some("3.0.0")),
                mismatch("changed", "1.0.0", Some("2.0.0")),
                mismatch("unknown_now", "1.0.0", Some("")),
                mismatch("known_now", "", Some("3.0.0")),
                mismatch("gone", "0.5.0", None),
                mismatch(
                    "overflow",
                    "1",
                    Some("99999999999999999999999999999999999999999999")
                ),
            ]
        );
        let unchanged = Level {
            saved_mods: Some(loaded.to_vec()),
            ..Level::default()
        };
        assert!(unchanged.mod_mismatches(&loaded).is_empty());
    }
}
