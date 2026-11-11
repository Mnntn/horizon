//! Data-only inputs carried from an owning load boundary to native load hooks.
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The phase that owns this data load, selected before handing work to loaders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataLoadOrigin {
    ServerDataLoad,
    ClientPreload,
}

/// Immutable metadata captured with the loaded item data. Later data loads
/// borrow the same registry snapshot instead of scanning it again.
pub struct LoadInputs {
    registries: BTreeMap<String, Arc<[String]>>,
    /// The fluid tags a recipe load reads for its ingredients' `#tags`:
    /// each tag id to its fluids, in the sources' order. Empty until a load
    /// that reads recipe resources fills them.
    fluid_tags: Arc<[(String, Arc<[String]>)]>,
    settings_root: PathBuf,
}

impl LoadInputs {
    pub fn new(registries: BTreeMap<String, Vec<String>>, settings_root: PathBuf) -> Self {
        Self {
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            fluid_tags: Arc::from([]),
            settings_root,
        }
    }
    /// The inputs with the fluid `tags` a recipe load reads
    /// (`data/<ns>/tags/fluid`), each tag's fluids in the sources' order.
    pub fn with_fluid_tags(mut self, tags: HashMap<String, Vec<String>>) -> Self {
        self.fluid_tags = tags
            .into_iter()
            .map(|(tag, fluids)| (tag, fluids.into()))
            .collect();
        self
    }
}

/// A load's registry snapshot with the settings captured for its mods.
#[derive(Clone)]
struct Snapshot {
    inputs: Arc<LoadInputs>,
    settings: BTreeMap<String, BTreeMap<String, String>>,
}

impl Snapshot {
    /// Captures settings for `mod_ids`; the registry inputs are shared, not scanned again.
    fn capture<'a>(inputs: Arc<LoadInputs>, mod_ids: impl IntoIterator<Item = &'a str>) -> Self {
        let settings = capture_settings(&inputs.settings_root, mod_ids);
        Self { inputs, settings }
    }

    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.inputs.registries.get(name).cloned()
    }

    /// The fluid tags the inputs carry, each tag's fluids in the sources'
    /// order; empty until a load that reads recipe resources fills them.
    fn fluid_tags(&self) -> &Arc<[(String, Arc<[String]>)]> {
        &self.inputs.fluid_tags
    }

    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.settings.get(mod_id)?.get(key).cloned()
    }
}

/// Origin and borrowed engine inputs for one owning load. This contains no
/// game, world handles or mutable registry access.
pub struct LoadContext {
    origin: DataLoadOrigin,
    snapshot: Snapshot,
}

impl LoadContext {
    /// Called at a load boundary, never by a reader of already-loaded data.
    /// Captures settings for the owning phase's loaded package IDs; reusing the
    /// registry inputs for another load still captures fresh settings.
    pub fn new<'a>(
        origin: DataLoadOrigin,
        inputs: Arc<LoadInputs>,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self {
            origin,
            snapshot: Snapshot::capture(inputs, mod_ids),
        }
    }

    /// A new owning load of `origin` over this load's registry inputs, with fresh settings.
    pub fn for_load<'a>(
        &self,
        origin: DataLoadOrigin,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        Self::new(origin, self.snapshot.inputs.clone(), mod_ids)
    }

    /// This load over the same registries and settings, with the fluid `tags`
    /// its recipe pass reads (`data/<ns>/tags/fluid`): what that pass's
    /// ingredient hooks resolve fluid `#tags` with. A recipe load builds it
    /// where it knows its overlay; loads that read no recipe resources share
    /// the inputs as-is, with no tags.
    pub fn with_fluid_tags(&self, tags: HashMap<String, Vec<String>>) -> Self {
        let inputs = Arc::new(LoadInputs {
            registries: self.snapshot.inputs.registries.clone(),
            fluid_tags: tags
                .into_iter()
                .map(|(tag, fluids)| (tag, fluids.into()))
                .collect(),
            settings_root: self.snapshot.inputs.settings_root.clone(),
        });
        Self {
            origin: self.origin,
            snapshot: Snapshot {
                inputs,
                settings: self.snapshot.settings.clone(),
            },
        }
    }

    /// The fluid tags this load's recipe pass reads: each tag id to its
    /// fluids, in the sources' order. Empty where no recipe pass filled them.
    pub fn fluid_tags(&self) -> &Arc<[(String, Arc<[String]>)]> {
        self.snapshot.fluid_tags()
    }

    /// The context of a menu, recipe or player-list loop of `side`, over this
    /// load's registry inputs, with fresh settings. Built where the side's loop
    /// owner starts or reloads its resources, never by a reader.
    pub fn loop_context<'a>(
        &self,
        side: LoopSide,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> LoopContext {
        LoopContext {
            side,
            snapshot: Snapshot::capture(self.snapshot.inputs.clone(), mod_ids),
            recipe_properties: Default::default(),
        }
    }

    /// The context of the server's world generation, over this load's
    /// registry inputs, with fresh settings. Built where the server starts or
    /// reloads its resources, never by a reader.
    pub fn worldgen_context<'a>(
        &self,
        mod_ids: impl IntoIterator<Item = &'a str>,
    ) -> WorldgenContext {
        WorldgenContext {
            snapshot: Snapshot::capture(self.snapshot.inputs.clone(), mod_ids),
        }
    }

    pub fn origin(&self) -> DataLoadOrigin {
        self.origin
    }

    /// Names in registry ID order, from the load's registry snapshot.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured at this load boundary, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// The side a loop context serves, chosen where the context is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoopSide {
    Server,
    Client,
}

/// Registries, settings and the side for hooks fired inside a menu, recipe or
/// player-list loop, where no game is at hand. It holds no game, world handle
/// or mutable registry access, and is built once per resource generation.
#[derive(Clone)]
pub struct LoopContext {
    side: LoopSide,
    snapshot: Snapshot,
    recipe_properties: Arc<HashMap<String, std::collections::HashSet<String>>>,
}

#[path = "native_load_context/recipe_properties.rs"]
mod recipe_properties;

impl LoopContext {
    pub fn side(&self) -> LoopSide {
        self.side
    }

    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// Capture regular UTF-8 files only, never following symlinks (even within the root).
/// Refused keys are absent. Limits are measured before whitespace trimming.
pub const MAX_SETTING_BYTES: usize = 64 * 1024;
const MAX_VISITED_NAMES: usize = 4096;
pub const MAX_SETTING_KEYS: usize = 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

pub fn capture_settings<'a>(
    settings_root: &Path,
    mod_ids: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut settings = BTreeMap::new();
    let mut total = 0;
    let mut refused = [false; 4];
    for mod_id in mod_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    {
        // Package ids are normally validated at load time; do not let this public
        // filesystem boundary interpret a path supplied by a caller.
        if Path::new(mod_id).components().count() != 1
            || !matches!(
                Path::new(mod_id).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            refused[0] = true;
            continue;
        }
        let Ok(files) = snapshot_files(settings_root, mod_id) else {
            refused[0] = true;
            continue;
        };
        let mut values = BTreeMap::new();
        for entry in files {
            let Ok((key, mut file)) = entry else {
                refused[0] = true;
                continue;
            };
            if values.len() >= MAX_SETTING_KEYS {
                refused[1] = true;
                break;
            }
            use std::io::Read;
            let mut bytes = Vec::new();
            if file
                .by_ref()
                .take((MAX_SETTING_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .is_err()
            {
                refused[0] = true;
                continue;
            }
            if bytes.len() > MAX_SETTING_BYTES || bytes.len() > MAX_SNAPSHOT_BYTES - total {
                refused[2] = true;
                continue;
            }
            let Ok(value) = String::from_utf8(bytes) else {
                refused[3] = true;
                continue;
            };
            total += value.len();
            values.insert(key, value.trim().to_owned());
        }
        settings.insert(mod_id.to_owned(), values);
    }
    for (refused, class) in refused.into_iter().zip([
        "unsafe or unreadable entry, or enumeration limit",
        "key limit",
        "byte limit",
        "invalid UTF-8",
    ]) {
        if refused {
            log::warn!("Mod settings snapshot omitted keys: {class}");
        }
    }
    settings
}

// Reject the whole package beyond the enumeration budget: retaining an arbitrary
// prefix would make the result depend on filesystem enumeration order again.
fn sorted_names(
    names: impl Iterator<Item = std::io::Result<String>>,
) -> std::io::Result<Vec<String>> {
    let mut result = Vec::new();
    for name in names {
        if result.len() == MAX_VISITED_NAMES {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        result.push(name?);
    }
    result.sort_unstable();
    Ok(result)
}

#[cfg(unix)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> std::io::Result<impl Iterator<Item = std::io::Result<(String, fs::File)>>> {
    snapshot_files_with_hook(root, mod_id, |_| {})
}

#[cfg(unix)]
fn snapshot_files_with_hook(
    root: &Path,
    mod_id: &str,
    after_open: impl Fn(&str),
) -> std::io::Result<impl Iterator<Item = std::io::Result<(String, fs::File)>>> {
    use rustix::fs::{Dir, Mode, OFlags, open, openat};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let root = open(root, flags | OFlags::DIRECTORY, Mode::empty())?;
    let directory = openat(&root, mod_id, flags | OFlags::DIRECTORY, Mode::empty())?;
    let entries = Dir::read_from(&directory)?;
    let names = sorted_names(entries.filter_map(|entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error.into())),
        };
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            return None;
        }
        Some(
            name.to_str()
                .map(str::to_owned)
                .map_err(|_| std::io::ErrorKind::InvalidData.into()),
        )
    }))?;
    Ok(names.into_iter().map(move |key| {
        let file = fs::File::from(openat(&directory, key.as_str(), flags, Mode::empty())?);
        after_open(&key);
        if !file.metadata()?.is_file() {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        Ok((key, file))
    }))
}

#[cfg(windows)]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> std::io::Result<impl Iterator<Item = std::io::Result<(String, fs::File)>>> {
    use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions};
    // Open the parent as a capability, then refuse a reparse point at the root
    // and package boundary as well as at each individual file boundary.
    let parent = root.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let name = root.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let parent = Dir::open_ambient_dir(parent, cap_std::ambient_authority())?;
    let root = parent.open_dir_nofollow(name)?;
    let directory = root.open_dir_nofollow(mod_id)?;
    let names = sorted_names(directory.entries()?.map(|entry| {
        entry?
            .file_name()
            .into_string()
            .map_err(|_| std::io::ErrorKind::InvalidData.into())
    }))?;
    Ok(names.into_iter().map(move |key| {
        let metadata = directory.symlink_metadata(&key)?;
        use cap_std::fs::MetadataExt;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory.open_with(&key, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        Ok((key, file.into_std()))
    }))
}

#[cfg(not(any(unix, windows)))]
fn snapshot_files(
    root: &Path,
    mod_id: &str,
) -> std::io::Result<impl Iterator<Item = std::io::Result<(String, fs::File)>>> {
    // Fail closed on targets without descriptor-relative no-follow traversal.
    let _ = (root, mod_id);
    Err::<std::iter::Empty<std::io::Result<(String, fs::File)>>, _>(
        std::io::ErrorKind::Unsupported.into(),
    )
}

/// Registries and settings for hooks fired while a feature generates, on a
/// worldgen worker or the server thread, where no game is at hand. It holds no
/// game, world handle or mutable registry access, and is built once per
/// resource generation of the server.
pub struct WorldgenContext {
    snapshot: Snapshot,
}

impl WorldgenContext {
    /// Names in registry ID order, from the registry snapshot it was built over.
    pub fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.snapshot.registry(name)
    }

    /// A loaded mod's setting captured when this context was built, with no file I/O.
    pub fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.snapshot.setting(mod_id, key)
    }
}

/// Read the same per-key storage as the runtime settings service. No folder is
/// created, and unrelated mod data files are not scanned or loaded eagerly.
pub fn read_setting(directory: &Path, key: &str) -> Option<String> {
    fs::read_to_string(directory.join(key))
        .ok()
        .map(|value| value.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "snapshot-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(root.join("reader")).unwrap();
        root
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_capture_follows_external_symlink() {
        let root = root("symlinks");
        let data = root.join("data");
        fs::create_dir_all(data.join("reader")).unwrap();
        fs::write(root.join("secret"), "outside-token").unwrap();
        std::os::unix::fs::symlink(root.join("secret"), data.join("reader/token")).unwrap();
        fs::write(data.join("reader/normal"), "inside").unwrap();
        std::os::unix::fs::symlink("normal", data.join("reader/internal")).unwrap();
        std::os::unix::fs::symlink("missing", data.join("reader/dangling")).unwrap();
        std::os::unix::fs::symlink(root.join("reader"), data.join("alias")).unwrap();
        fs::write(root.join("reader/token"), "escaped").unwrap();
        let captured = capture_settings(&data, ["reader", "alias", "../reader"]);
        assert_eq!(captured["reader"].get("token"), None);
        assert_eq!(captured["reader"].get("internal"), None);
        assert_eq!(captured["reader"].get("dangling"), None);
        assert_eq!(captured["reader"]["normal"], "inside");
        assert!(!captured.contains_key("alias"));
        assert!(!captured.contains_key("../reader"));
        std::os::unix::fs::symlink(&data, root.join("linked-root")).unwrap();
        assert!(capture_settings(&root.join("linked-root"), ["reader"]).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn fifo_and_directory_do_not_block_or_become_settings() {
        let root = root("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join("reader/pipe"))
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join("reader/directory")).unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let input = root.clone();
        let worker =
            std::thread::spawn(move || send.send(capture_settings(&input, ["reader"])).unwrap());
        let captured = receive
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("snapshot blocked on FIFO");
        assert!(captured["reader"].is_empty());
        worker.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn admission_is_sorted_and_enumeration_is_bounded() {
        let root = root("ordered");
        fs::create_dir(root.join("second")).unwrap();
        for i in 0..1025 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
            fs::write(root.join(format!("second/key{:04}", 1024 - i)), "x").unwrap();
        }
        let captured = capture_settings(&root, ["second", "reader"]);
        assert_eq!(captured["reader"], captured["second"]);
        assert!(!captured["reader"].contains_key("key1024"));
        assert_eq!(captured["reader"]["key0000"], "x");
        for i in 1025..4097 {
            fs::write(root.join(format!("reader/key{i:04}")), "x").unwrap();
        }
        assert!(!capture_settings(&root, ["reader"]).contains_key("reader"));
        assert!(sorted_names((0..4097).map(|i| Ok(i.to_string()))).is_err());
        assert_eq!(
            sorted_names((0..4096).map(|i| Ok(i.to_string())))
                .unwrap()
                .len(),
            4096
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn validates_open_handle_after_entry_replacement() {
        use std::io::Read;
        let root = root("replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened value").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        })
        .unwrap();
        let (_, mut file) = entries.into_iter().next().unwrap().unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened value");

        // Now open the FIFO, then replace its path with a regular file. Path
        // metadata would wrongly accept it, but descriptor metadata must refuse.
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            fs::write(&path, "replacement").unwrap();
        })
        .unwrap();
        assert!(entries.into_iter().next().unwrap().is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn byte_limits_and_invalid_utf8() {
        let root = root("bytes");
        fs::write(root.join("reader/exact"), vec![b'a'; 65536]).unwrap();
        fs::write(root.join("reader/large"), vec![b'b'; 65537]).unwrap();
        fs::write(root.join("reader/invalid"), [0xff]).unwrap();
        let captured = capture_settings(&root, ["reader"]);
        assert_eq!(captured["reader"]["exact"].len(), 65536);
        assert_eq!(captured["reader"].get("large"), None);
        assert_eq!(captured["reader"].get("invalid"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn key_limit() {
        let root = root("count");
        for i in 0..1025 {
            fs::write(root.join(format!("reader/key{i}")), "x").unwrap();
        }
        assert_eq!(capture_settings(&root, ["reader"])["reader"].len(), 1024);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reviewer_whitespace_spends_raw_aggregate_budget() {
        let root = root("raw-budget");
        for i in 0..256 {
            fs::write(root.join(format!("reader/key{i:04}")), vec![b' '; 65536]).unwrap();
        }
        fs::write(root.join("reader/z-extra"), "x").unwrap();
        let captured = capture_settings(&root, ["reader"]);
        assert_eq!(captured["reader"].len(), 256);
        assert!(!captured["reader"].contains_key("z-extra"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn reviewer_symlink_replacement_keeps_opened_value() {
        use std::io::Read;
        let root = root("symlink-replaced");
        let path = root.join("reader/value");
        fs::write(&path, "opened").unwrap();
        fs::write(root.join("target"), "replacement").unwrap();
        let entries = snapshot_files_with_hook(&root, "reader", |_| {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(root.join("target"), &path).unwrap();
        })
        .unwrap();
        let (_, mut file) = entries.into_iter().next().unwrap().unwrap();
        let mut value = String::new();
        file.read_to_string(&mut value).unwrap();
        assert_eq!(value, "opened");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aggregate_limit() {
        let root = root("aggregate");
        fs::create_dir(root.join("second")).unwrap();
        for i in 0..129 {
            for package in ["reader", "second"] {
                fs::write(root.join(format!("{package}/key{i}")), vec![b'x'; 65536]).unwrap();
            }
        }
        let captured = capture_settings(&root, ["second", "reader"]);
        assert_eq!(captured["reader"].len(), 129);
        assert_eq!(captured["second"].len(), 127);
        assert_eq!(
            captured
                .values()
                .flat_map(|values| values.values())
                .map(String::len)
                .sum::<usize>(),
            16 * 1024 * 1024
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_are_scoped_and_frozen_until_a_new_load() {
        let root = std::env::temp_dir().join(format!(
            "horizon-load-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let active = root.join("active");
        fs::create_dir_all(&active).unwrap();
        fs::create_dir_all(root.join("inactive")).unwrap();
        fs::write(active.join("mode"), "  before\n").unwrap();
        fs::write(active.join("empty"), " \n").unwrap();
        fs::write(active.join("invalid"), [0xff]).unwrap();
        fs::write(root.join("inactive/mode"), "not loaded").unwrap();
        let inputs = Arc::new(LoadInputs::new(BTreeMap::new(), root.clone()));
        let before = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs.clone(), ["active"]);

        // Change the files before the first lookup, not just between reads.
        fs::write(active.join("mode"), "  after\n").unwrap();
        fs::write(active.join("late"), "new key").unwrap();
        assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
        assert_eq!(before.setting("active", "empty").as_deref(), Some(""));
        assert_eq!(before.setting("active", "invalid"), None);
        assert_eq!(before.setting("active", "late"), None);
        assert_eq!(before.setting("inactive", "mode"), None);
        assert_eq!(read_setting(&active, "mode").as_deref(), Some("after"));

        let after = LoadContext::new(DataLoadOrigin::ClientPreload, inputs.clone(), ["active"]);
        assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
        assert_eq!(after.setting("active", "late").as_deref(), Some("new key"));
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(before.setting("active", "mode").as_deref(), Some("before"));
        assert_eq!(after.setting("active", "mode").as_deref(), Some("after"));
        let missing = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        assert_eq!(missing.setting("active", "mode"), None);
        assert!(!root.exists());
    }

    #[test]
    fn a_loop_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "horizon-loop-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("mode"), "one").unwrap();
        let registries = BTreeMap::from([("item".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, ["active"]);

        let client = load.loop_context(LoopSide::Client, ["active"]);
        fs::write(mod_dir.join("mode"), "two").unwrap();
        let server = load.loop_context(LoopSide::Server, ["active"]);

        assert_eq!(client.side(), LoopSide::Client);
        assert_eq!(server.side(), LoopSide::Server);
        assert_eq!(
            client.registry("item").as_deref(),
            Some(&["a:b".to_owned()][..])
        );
        assert_eq!(server.registry("item"), client.registry("item"));
        assert_eq!(client.setting("active", "mode").as_deref(), Some("one"));
        assert_eq!(server.setting("active", "mode").as_deref(), Some("two"));
        assert_eq!(load.setting("active", "mode").as_deref(), Some("one"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_worldgen_context_shares_the_registries_and_captures_its_own_settings() {
        let root = std::env::temp_dir().join(format!(
            "horizon-worldgen-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let mod_dir = root.join("active");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("ground"), "podzol").unwrap();
        let registries = BTreeMap::from([("block".to_owned(), vec!["a:b".to_owned()])]);
        let inputs = Arc::new(LoadInputs::new(registries, root.clone()));
        let load = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, ["active"]);
        fs::write(mod_dir.join("ground"), "mud").unwrap();

        let worldgen = load.worldgen_context(["active"]);
        fs::write(mod_dir.join("ground"), "moss").unwrap();

        assert_eq!(
            worldgen.registry("block").as_deref(),
            Some(&["a:b".to_owned()][..])
        );
        assert_eq!(worldgen.setting("active", "ground").as_deref(), Some("mud"));
        assert_eq!(worldgen.setting("inactive", "ground"), None);
        assert_eq!(load.setting("active", "ground").as_deref(), Some("podzol"));
        fs::remove_dir_all(&root).unwrap();
    }
}
