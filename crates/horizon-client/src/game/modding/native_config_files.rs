//! `Host::config_files`: the calling mod's own files in the game's `config/`
//! folder, for every mod (native or wasm) without a file system of its own.
use super::Game;
use mod_api::{
    CONFIG_FILE_MAX, CONFIG_LIST_MAX, ConfigEntry, ConfigEntryKind, ConfigFileError, ConfigFiles,
    ConfigPath, ConfigStat,
};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

/// The game's `config/` folder, bound to whichever mod calls.
struct GameConfigFiles;

static CONFIG_FILES: GameConfigFiles = GameConfigFiles;

impl Game<'_> {
    /// The calling mod's own files in the game's `config/` folder, beside
    /// `options.txt` (beside `server.properties` on a dedicated server):
    /// its folder `config/<mod id>/` and the files beside it named
    /// `<mod id>.<ext>` or `<mod id>-<rest>`, as Fabric and NeoForge mods
    /// keep theirs. The service is bound to the mod whose hook or part is
    /// running, so a mod never names its id and can't reach another mod's
    /// files; a world's `serverconfig/` is read only through
    /// `config_loading`.
    pub(super) fn config_files() -> &'static dyn ConfigFiles {
        &CONFIG_FILES
    }
    /// Whether a mod with the `horizon-mod.toml` id `id` is installed: a
    /// native, wasm or content package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    pub(super) fn is_loaded(id: &str) -> bool {
        mod_api::__private::loaded_mods().get(id).is_some()
    }
}

fn io(error: std::io::Error) -> ConfigFileError {
    match error.kind() {
        ErrorKind::NotFound => ConfigFileError::NotFound,
        _ => ConfigFileError::Io(error.to_string()),
    }
}

/// `path` in `root` for the calling mod: refused outside a mod's call, for
/// another mod's path, and where a symbolic link is on the way (`root`
/// itself may be one, as the player set it up).
fn resolve(root: &Path, path: &ConfigPath) -> Result<PathBuf, ConfigFileError> {
    let mod_id =
        mod_api::content::crash::current_native_mod().ok_or(ConfigFileError::NoCallingMod)?;
    if !path.owned_by(mod_id) {
        return Err(ConfigFileError::NotOwned);
    }
    let mut at = root.to_path_buf();
    for part in path.as_str().split('/') {
        at.push(part);
        match fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(ConfigFileError::Symlink),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => break,
            Err(e) => return Err(io(e)),
        }
    }
    Ok(root.join(path.as_str()))
}

fn stat_of(meta: &fs::Metadata) -> ConfigStat {
    let modified_millis = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    let (kind, len) = if meta.is_dir() {
        (ConfigEntryKind::Folder, 0)
    } else {
        (ConfigEntryKind::File, meta.len())
    };
    ConfigStat::from(mod_api::__private::built::ConfigStat {
        kind,
        len,
        modified_millis,
    })
}

fn read(root: &Path, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError> {
    let path = resolve(root, file)?;
    let meta = fs::metadata(&path).map_err(io)?;
    if meta.is_dir() {
        return Err(ConfigFileError::NotAFile);
    }
    let mut bytes = Vec::new();
    // One byte past the cap tells a file that grew since its length was read.
    fs::File::open(&path)
        .and_then(|f| f.take(CONFIG_FILE_MAX + 1).read_to_end(&mut bytes))
        .map_err(io)?;
    if bytes.len() as u64 > CONFIG_FILE_MAX {
        return Err(ConfigFileError::TooLarge);
    }
    Ok(bytes)
}

/// Whether `name` is the temporary file of a write, `<name>.<pid>-<n>.tmp`.
fn is_write_temp(name: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    name.strip_suffix(".tmp")
        .and_then(|rest| rest.rsplit_once('.'))
        .and_then(|(_, tail)| tail.split_once('-'))
        .is_some_and(|(pid, n)| digits(pid) && digits(n))
}

/// Written beside the file, then renamed over it. The temporary name starts
/// with the file's own, so it stays the mod's own; `list` leaves it out, so a
/// scan never meets a half-written file, nor one a crash left behind.
fn write(root: &Path, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if bytes.len() as u64 > CONFIG_FILE_MAX {
        return Err(ConfigFileError::TooLarge);
    }
    let path = resolve(root, file)?;
    // `config/<mod id>` is the mod's folder, so it can't be a file.
    if path.is_dir() || Some(file.as_str()) == mod_api::content::crash::current_native_mod() {
        return Err(ConfigFileError::NotAFile);
    }
    let folder = path.parent().unwrap_or(root);
    fs::create_dir_all(folder).map_err(io)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = folder.join(format!(
        "{name}.{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::File::create(&temp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&temp, &path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(io)
}

fn list(root: &Path, folder: &ConfigPath) -> Result<Vec<ConfigEntry>, ConfigFileError> {
    let path = resolve(root, folder)?;
    if !fs::metadata(&path).map_err(io)?.is_dir() {
        return Err(ConfigFileError::NotAFolder);
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path).map_err(io)? {
        let entry = entry.map_err(io)?;
        if entries.len() == CONFIG_LIST_MAX {
            return Err(ConfigFileError::TooLarge);
        }
        // A link is left out, never followed (`DirEntry::metadata` doesn't).
        let meta = entry.metadata().map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.file_type().is_symlink() || is_write_temp(&name) {
            continue;
        }
        entries.push(ConfigEntry::from(mod_api::__private::built::ConfigEntry {
            name,
            stat: stat_of(&meta),
        }));
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn stat(root: &Path, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError> {
    match fs::metadata(resolve(root, path)?) {
        Ok(meta) => Ok(Some(stat_of(&meta))),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(e)),
    }
}

fn root() -> Result<PathBuf, ConfigFileError> {
    Game::native_config_dir().map_err(io)
}

impl ConfigFiles for GameConfigFiles {
    fn read(&self, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError> {
        read(&root()?, file)
    }
    fn write(&self, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError> {
        write(&root()?, file, bytes)
    }
    fn list(&self, folder: &ConfigPath) -> Result<Vec<ConfigEntry>, ConfigFileError> {
        list(&root()?, folder)
    }
    fn stat(&self, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError> {
        stat(&root()?, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::crash::{ModCall, enter_native_mod};

    struct Root(PathBuf);
    impl Root {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "horizon-config-files-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn path(path: &str) -> ConfigPath {
        ConfigPath::new(path).unwrap()
    }

    fn as_mod<R>(id: &'static str, f: impl FnOnce() -> R) -> R {
        let _call = enter_native_mod(ModCall {
            id,
            hook: "world_load",
            part: None,
        });
        f()
    }

    #[test]
    fn a_mod_writes_reads_lists_and_stats_its_own_files() {
        let root = Root::new("own");
        let root = &root.0;
        as_mod("fd", || {
            assert_eq!(stat(root, &path("fd/state.json")), Ok(None));
            write(root, &path("fd/state.json"), b"{\"a\":1}").unwrap();
            write(root, &path("fd.json"), b"top").unwrap();
            assert_eq!(read(root, &path("fd/state.json")).unwrap(), b"{\"a\":1}");
            assert_eq!(read(root, &path("fd.json")).unwrap(), b"top");
            // A second write replaces the bytes and leaves no temporary file.
            write(root, &path("fd/state.json"), b"{}").unwrap();
            let entries = list(root, &path("fd")).unwrap();
            assert_eq!(
                entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                ["state.json"]
            );
            assert_eq!(entries[0].stat.kind, ConfigEntryKind::File);
            assert_eq!(entries[0].stat.len, 2);
            assert!(entries[0].stat.modified_millis > 0);
            assert_eq!(
                stat(root, &path("fd")).unwrap().map(|s| s.kind),
                Some(ConfigEntryKind::Folder)
            );
            assert_eq!(read(root, &path("fd")), Err(ConfigFileError::NotAFile));
            assert_eq!(
                list(root, &path("fd.json")),
                Err(ConfigFileError::NotAFolder)
            );
            assert_eq!(read(root, &path("fd/gone")), Err(ConfigFileError::NotFound));
        });
        assert_eq!(fs::read(root.join("fd/state.json")).unwrap(), b"{}");
    }

    #[test]
    fn a_file_named_for_the_mod_cant_block_its_folder_and_list_hides_write_temps() {
        let root = Root::new("folder-name");
        let root = &root.0;
        as_mod("fd", || {
            assert_eq!(
                write(root, &path("fd"), b"x"),
                Err(ConfigFileError::NotAFile)
            );
            assert!(!root.join("fd").exists());
            write(root, &path("fd/state.json"), b"{}").unwrap();
            // What a crash between the temp file and the rename leaves behind.
            fs::write(root.join("fd.json.1-0.tmp"), b"half").unwrap();
            fs::write(root.join("fd/state.json.1-0.tmp"), b"half").unwrap();
            fs::write(root.join("fd/notes.tmp"), b"mine").unwrap();
            assert_eq!(read(root, &path("fd.json.1-0.tmp")).unwrap(), b"half");
            // `list` leaves out a write's temporary file, not a mod's own `.tmp`.
            let names: Vec<_> = list(root, &path("fd"))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(names, ["notes.tmp", "state.json"]);
        });
    }

    #[test]
    fn a_mod_never_reaches_another_mods_files() {
        let root = Root::new("other");
        let root = &root.0;
        as_mod("b", || write(root, &path("b/secret.json"), b"b's").unwrap());
        as_mod("a", || {
            for other in ["b/secret.json", "b.json", "b-common.toml"] {
                let other = path(other);
                assert_eq!(read(root, &other), Err(ConfigFileError::NotOwned));
                assert_eq!(write(root, &other, b"a's"), Err(ConfigFileError::NotOwned));
                assert_eq!(stat(root, &other), Err(ConfigFileError::NotOwned));
            }
            assert_eq!(list(root, &path("b")), Err(ConfigFileError::NotOwned));
        });
        assert_eq!(fs::read(root.join("b/secret.json")).unwrap(), b"b's");
        // Outside any mod's call nothing is anyone's.
        assert_eq!(
            read(root, &path("b/secret.json")),
            Err(ConfigFileError::NoCallingMod)
        );
    }

    /// The service through a real game and the hooks' own dispatch: a mod
    /// saves its registered config from `config_loading` and keeps a file in
    /// its folder, the reload poll hands the saved text back, and the same
    /// call for another mod's file is refused. In a fresh process, run in a
    /// scratch folder, since `config/` is beside the game's working folder.
    #[test]
    #[ignore = "needs prepared resources"]
    fn a_mod_saves_its_config_through_the_game_and_the_reload_hears_it() {
        use mod_api::{
            self, ConfigLoadingEvent, ConfigReloadingEvent, Hooks, HostSide, ModEntry, Mods,
            RegisterConfigsEvent,
        };
        use std::sync::Mutex;
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn heard() -> std::sync::MutexGuard<'static, Vec<String>> {
            HEARD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_config_files::tests::a_mod_saves_its_config_through_the_game_and_the_reload_hears_it",
            "HORIZON_CONFIG_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        fn files(host: &HostSide) -> &'static dyn ConfigFiles {
            match host {
                HostSide::Server(host) => host.config_files(),
                HostSide::Client(host) => host.config_files(),
            }
        }
        struct Proof;
        #[mod_api::__subscriptions]
        impl Hooks for Proof {
            fn register_configs(&self, _: HostSide, event: &mut RegisterConfigsEvent<'_>) {
                event.configs.push(mod_api::Config::new(
                    "proof",
                    mod_api::ConfigKind::Common,
                    "count = 0\n",
                ));
            }
            fn config_loading(&self, host: HostSide, event: &mut ConfigLoadingEvent<'_>) {
                let files = files(&host);
                heard().push(format!("loading {}", event.config.text.trim()));
                files
                    .write(&path("proof-common.toml"), b"count = 1\n")
                    .unwrap();
                files.write(&path("proof/state.bin"), &[1, 2, 3]).unwrap();
                assert_eq!(files.read(&path("proof/state.bin")).unwrap(), [1, 2, 3]);
                let other = path("other/secret.json");
                assert_eq!(files.read(&other), Err(ConfigFileError::NotOwned));
                assert_eq!(files.write(&other, b"x"), Err(ConfigFileError::NotOwned));
            }
            fn config_reloading(&self, _: HostSide, event: &mut ConfigReloadingEvent<'_>) {
                heard().push(format!("reloading {}", event.config.text.trim()));
            }
        }
        let mods: Mods = vec![ModEntry::new("proof", Box::new(Proof))].into();
        mod_api::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_configs_client_start();
                game.native_configs_poll();
            });
        });
        assert_eq!(*heard(), ["loading count = 0", "reloading count = 1"]);
        let config = std::env::current_dir().unwrap().join("config");
        assert_eq!(fs::read(config.join("proof/state.bin")).unwrap(), [1, 2, 3]);
        assert!(!config.join("other").exists());
        // Outside the mod's hooks the service is no one's.
        assert_eq!(
            Game::config_files().read(&path("proof-common.toml")),
            Err(ConfigFileError::NoCallingMod)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_never_leads_out_of_config() {
        let root = Root::new("link");
        let outside = Root::new("link-outside");
        fs::write(outside.0.join("passwd"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside.0, root.0.join("fd")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("passwd"), root.0.join("fd.json")).unwrap();
        as_mod("fd", || {
            assert_eq!(
                read(&root.0, &path("fd/passwd")),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(
                write(&root.0, &path("fd/new"), b"x"),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(
                read(&root.0, &path("fd.json")),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(list(&root.0, &path("fd")), Err(ConfigFileError::Symlink));
        });
        assert!(!outside.0.join("new").exists());
    }

    #[test]
    fn a_file_holds_four_mebibytes_at_most() {
        let root = Root::new("size");
        let root = &root.0;
        let max = vec![b'x'; CONFIG_FILE_MAX as usize];
        as_mod("fd", || {
            write(root, &path("fd/max"), &max).unwrap();
            assert_eq!(read(root, &path("fd/max")).unwrap().len(), max.len());
            let over = [max.as_slice(), b"x"].concat();
            assert_eq!(
                write(root, &path("fd/over"), &over),
                Err(ConfigFileError::TooLarge)
            );
            fs::write(root.join("fd/over"), &over).unwrap();
            assert_eq!(read(root, &path("fd/over")), Err(ConfigFileError::TooLarge));
        });
    }
}
