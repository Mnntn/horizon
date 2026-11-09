//! A mod's own files in the game's `config/` folder (`Host::config_files`).
//!
//! The game binds the service to the mod whose hook or part it is inside
//! (`crash::current_native_mod`), so a mod never names its own id and can't
//! reach another mod's files.

/// Bytes a config file may hold, read or written.
pub const CONFIG_FILE_MAX: u64 = 4 << 20;

/// Entries one `list` answers at most.
pub const CONFIG_LIST_MAX: usize = 1024;

/// The calling mod's files in `config/`: read, write, list and stat. Every
/// call takes `&self`, so reads and writes work through `&dyn Host` on both
/// sides; a write changes the file, not the host. The game's reload poll
/// (`config_reloading`) sees a written registered config as any edit.
pub trait ConfigFiles {
    /// The file's bytes, at most `CONFIG_FILE_MAX`.
    fn read(&self, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError>;
    /// Replaces the file's bytes, at most `CONFIG_FILE_MAX`, making its
    /// folders when missing. The new bytes replace the old ones at once: a
    /// reader never sees half a write.
    fn write(&self, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError>;
    /// The entries of a folder, by name, at most `CONFIG_LIST_MAX`.
    fn list(&self, folder: &ConfigPath) -> Result<Vec<ConfigEntry>, ConfigFileError>;
    /// What is at `path`; None where nothing is.
    fn stat(&self, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError>;
}

/// A path inside `config/`: relative, `/`-separated, with no empty, `.` or
/// `..` part, no `\`, `:` or NUL. Checked once here; the service checks only
/// that it is the calling mod's.
// no wire: decoding calls ConfigPath::new to reject unsafe relative paths, traversal and invalid components.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ConfigPath(String);

impl From<&ConfigPath> for String {
    fn from(path: &ConfigPath) -> Self {
        path.0.clone()
    }
}

/// A path checked as [`ConfigPath::new`] checks it.
impl TryFrom<String> for ConfigPath {
    type Error = ConfigFileError;
    fn try_from(path: String) -> Result<Self, ConfigFileError> {
        Self::new(&path)
    }
}

impl ConfigPath {
    /// Checks `path` and makes a `ConfigPath` of it, or refuses with `ConfigFileError::InvalidPath`.
    pub fn new(path: &str) -> Result<Self, ConfigFileError> {
        let valid = path
            .split('/')
            .all(|part| !matches!(part, "" | "." | "..") && !part.contains(['\\', ':', '\0']));
        valid
            .then(|| Self(path.to_owned()))
            .ok_or(ConfigFileError::InvalidPath)
    }

    /// The path as written, `/`-separated.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the path is `mod_id`'s: its folder `<mod_id>/` and anything
    /// in it, or a file beside it named `<mod_id>.<ext>` or
    /// `<mod_id>-<rest>` (NeoForge's `<mod_id>-common.toml`). `.` and `-`
    /// are not id characters, so no other mod's id starts the same way.
    pub fn owned_by(&self, mod_id: &str) -> bool {
        let (first, rest) = self.0.split_once('/').unwrap_or((&self.0, ""));
        match first.strip_prefix(mod_id) {
            Some("") => true,
            Some(tail) => rest.is_empty() && (tail.starts_with('.') || tail.starts_with('-')),
            None => false,
        }
    }
}

/// What a path holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConfigEntryKind {
    /// A file.
    File,
    /// A folder.
    Folder,
}

/// A file's or folder's size and age (`java.io.File.length`/`lastModified`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConfigStat {
    /// Whether the path is a file or a folder.
    pub kind: ConfigEntryKind,
    /// Bytes, for a file; 0 for a folder.
    pub len: u64,
    /// Milliseconds since the Unix epoch; 0 where the system doesn't say.
    pub modified_millis: u64,
}

/// One entry of a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ConfigEntry {
    /// The entry's name, without its folder.
    pub name: String,
    /// The entry's size and age.
    pub stat: ConfigStat,
}

/// Why a config file call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ConfigFileError {
    /// Called outside any mod's hook or part, so no mod owns the call.
    NoCallingMod,
    /// The path is another mod's, or `config/` itself.
    NotOwned,
    /// Not a path `ConfigPath::new` takes.
    InvalidPath,
    /// More than `CONFIG_FILE_MAX` bytes, or a folder of more than
    /// `CONFIG_LIST_MAX` entries.
    TooLarge,
    /// Nothing is at that path.
    NotFound,
    /// A folder where a file was asked for.
    NotAFile,
    /// A file where a folder was asked for.
    NotAFolder,
    /// A symbolic link on the path, which could lead outside `config/`.
    Symlink,
    /// Any other failure of the file system, as it describes itself.
    Io(String),
}

impl std::fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCallingMod => f.write_str("no mod is calling"),
            Self::NotOwned => f.write_str("the path is not the calling mod's"),
            Self::InvalidPath => f.write_str("not a path inside config/"),
            Self::TooLarge => f.write_str("too large"),
            Self::NotFound => f.write_str("not found"),
            Self::NotAFile => f.write_str("not a file"),
            Self::NotAFolder => f.write_str("not a folder"),
            Self::Symlink => f.write_str("a symbolic link is on the path"),
            Self::Io(error) => f.write_str(error),
        }
    }
}

impl std::error::Error for ConfigFileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_stays_inside_config() {
        for good in ["fd.json", "fd/a/b.toml", "fd-common.toml"] {
            assert!(ConfigPath::new(good).is_ok(), "{good}");
        }
        for bad in [
            "", "/fd.json", "fd/", "fd//a", "./fd", "fd/../x", "..", "fd\\a", "C:fd", "fd\0",
        ] {
            assert_eq!(
                ConfigPath::new(bad),
                Err(ConfigFileError::InvalidPath),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_mod_owns_its_folder_and_the_files_named_after_it() {
        let owned = |path: &str| ConfigPath::new(path).unwrap().owned_by("fd");
        assert!(owned("fd"));
        assert!(owned("fd/a/b.json"));
        assert!(owned("fd.json"));
        assert!(owned("fd-common.toml"));
        // Another mod whose id starts the same, and names under another folder.
        assert!(!owned("fdx.json"));
        assert!(!owned("fd_extra/a.json"));
        assert!(!owned("fdx/a.json"));
        assert!(!owned("fd.json/a"));
        assert!(!owned("other/fd.json"));
    }
}

#[doc(hidden)]
pub(crate) mod built {
    #[allow(unused_imports)]
    use super::*;
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(horizon_mod_api_macros::Wire)]
    pub struct ConfigStat {
        pub kind: ConfigEntryKind,
        pub len: u64,
        pub modified_millis: u64,
    }
    impl From<ConfigStat> for super::ConfigStat {
        fn from(v: ConfigStat) -> Self {
            let ConfigStat {
                kind,
                len,
                modified_millis,
            } = v;
            Self {
                kind,
                len,
                modified_millis,
            }
        }
    }
    // exhaustive: the game builds it with a literal; mods only read it
    #[derive(horizon_mod_api_macros::Wire)]
    pub struct ConfigEntry {
        pub name: String,
        pub stat: super::ConfigStat,
    }
    impl From<ConfigEntry> for super::ConfigEntry {
        fn from(v: ConfigEntry) -> Self {
            let ConfigEntry { name, stat } = v;
            Self { name, stat }
        }
    }
}
