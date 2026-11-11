//! `horizon-mod.toml`, a mod's manifest, and the `.hmod` file around it.
//!
//! Raw injection schema adapted from horizon-mixins (Copyright (c) 2026 levzzz),
//! MIT licensed; see `HORIZON_MIXINS_LICENSE` and `THIRD_PARTY_NOTICES.md`.
pub mod inject;
mod jar;
pub mod mixin;
mod wasm;
pub use jar::{
    JarConflict, JarInputsError, JarRef, JarRequirement, MissingJar, collect_jar_requirements,
    missing_jars,
};
pub use wasm::{MAX_WASM, Wasm, WasmError, sha512_of};

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.hmod` or a mod folder.
pub const MANIFEST: &str = "horizon-mod.toml";
/// Executable components are not content and are refused in a `.hmod`.
pub const WASM: &str = "mod.wasm";
/// The content manifest API this build loads.
pub const API: &str = "0.2";
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/modding/src/how-to/content-mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "horizon"];
/// A mod's saved settings, shared by native mods and modpack overrides.
pub const STORAGE_DIR: &str = ".horizon-mod-data";
/// The largest manifest this build reads.
/// Maximum manifest bytes accepted by package readers and developer tools.
pub const MAX_MANIFEST: u64 = 32 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The Mods screen's `License:` line (a name such as `MIT`); empty shows it as not given.
    #[serde(default)]
    pub license: String,
    /// Where the license text is: the screen makes its name a link.
    #[serde(default)]
    pub license_url: Option<String>,
    /// The Mods screen's `Credits:` line.
    #[serde(default)]
    pub credits: String,
    /// The mod's home page, the Mods screen's Homepage button.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Where to report problems, the Mods screen's Issues button.
    #[serde(default)]
    pub issues: Option<String>,
    /// The content API line: `0.2`.
    pub api: String,
    #[serde(default)]
    pub side: Side,
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Other native mods whose hooks must be available before this one.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, Dependency>,
    /// Original Java jars, layered in list order below this mod's own files.
    #[serde(default)]
    pub jar: Vec<JarRef>,
    /// Exact build pins and support modules for a code mod.
    pub native: Option<Native>,
    /// A wasm mod's `mod.wasm`, its toolchain and what it was linked against.
    pub wasm: Option<Wasm>,
    /// Game internals used by this mod, fingerprinted against its pinned build.
    #[serde(default)]
    pub internals: Vec<Internal>,
    /// Listeners at compiled slots; they never compile game code.
    #[serde(default, deserialize_with = "inject::rows")]
    pub inject: Vec<inject::Inject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    Version(semver::VersionReq),
    Detailed(DependencyDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyDetails {
    pub version: semver::VersionReq,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Native {
    pub game: String,
    pub horizon: String,
    pub mixins: String,
    /// Source-level edits of game functions, forwarded as horizon-mixins' `[[injection]]`.
    /// Only the tables of `[native]` compile game code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mixin: Vec<mixin::Mixin>,
    #[serde(default)]
    pub module: Vec<Module>,
    /// Accessor and invoker bridges, forwarded as horizon-mixins' `[[bridge]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bridge: Vec<mixin::Forwarded>,
    /// State fields added to game types, forwarded as horizon-mixins' `[[state]]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<mixin::Forwarded>,
}

/// One declared game internal, as listed in the pinned game's `internals.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Internal {
    pub path: String,
    /// The catalog's member kind, required when several kinds share the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional pinning: the catalog fingerprint `internals add` writes. Absent, the build takes
    /// it from the pinned game's catalog, which `[native] horizon` fixes exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// A support module merged into a game crate, forwarded as horizon-mixins'
/// `[[module]]`: self-contained Rust items, emitted into `file` as
/// `mod name`, for helpers that need more than one function.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    /// The game source file that receives the module, as the pin indexes it.
    pub file: String,
    /// The module's Rust name under the destination file's top level.
    pub name: String,
    /// The payload's file in this package, under `src/`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Local content, not compared with the server's mods when joining.
    #[default]
    Client,
    /// Both, and the server tells clients which version it expects.
    Both,
}

/// What a content mod adds; the player sees it before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2).
    Registries,
}
impl Capability {
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
        }
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Registries => f.write_str("registries"),
        }
    }
}

/// Whether a source pin is an exact lowercase Git object id.
pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !reserved_mod_id(id)
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn reserved_mod_id(id: &str) -> bool {
    matches!(id, "con" | "prn" | "aux" | "nul")
        || (id.len() == 4
            && (id.starts_with("com") || id.starts_with("lpt"))
            && matches!(id.as_bytes()[3], b'1'..=b'9'))
}

fn rust_ident(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

impl Internal {
    fn check_all(internals: &[Self]) -> Result<(), String> {
        let mut paths = std::collections::HashSet::new();
        for internal in internals {
            if internal.path.is_empty()
                || internal.hash.as_ref().is_some_and(String::is_empty)
                || internal.kind.as_ref().is_some_and(|kind| kind.is_empty())
                || !paths.insert((&internal.path, &internal.kind))
            {
                return Err(format!(
                    "{MANIFEST}: empty internal path, kind or hash, or duplicate internal path and kind"
                ));
            }
        }
        Ok(())
    }
}

impl Manifest {
    /// The declared dependency whose normalized crate namespace owns a mod-point target.
    /// Targets outside these namespaces belong to the engine catalog.
    pub fn mod_point_dependency(&self, at: &str) -> Option<&str> {
        self.dependencies
            .keys()
            .find(|id| at.split("::").next() == Some(id.replace('-', "_").as_str()))
            .map(String::as_str)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let both = || {
            format!(
                "{MANIFEST}: a mod is [native] or [wasm], not both; only [native] tables \
                 compile game code, so a wasm mod has no mixin, bridge, state or module"
            )
        };
        // A wasm manifest whose only fault is its `[native]` table is told that, not which
        // pin a half-written `[native]` lacks. Any other error stays the one reported.
        let m: Manifest = toml::from_str(text).map_err(|e| {
            let native_alone = toml::from_str::<toml::Table>(text).is_ok_and(|mut table| {
                table.contains_key("wasm")
                    && table.remove("native").is_some()
                    && table.try_into::<Manifest>().is_ok()
            });
            if native_alone {
                both()
            } else {
                format!("{MANIFEST}: {e}")
            }
        })?;
        Internal::check_all(&m.internals)?;
        if m.wasm.is_some()
            && m.internals.is_empty()
            && m.inject.iter().any(|i| {
                i.access == inject::Access::Internals && m.mod_point_dependency(&i.at).is_none()
            })
        {
            return Err(format!(
                "{MANIFEST}: a wasm internals listener needs declared internals, or a declared \
                 dependency whose mod id, with `-` as `_`, starts its `at`"
            ));
        }
        if !m.internals.is_empty() && m.native.is_none() && m.wasm.is_none() {
            return Err(format!(
                "{MANIFEST}: internals need a [native] or [wasm] mod"
            ));
        }
        if reserved_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is a reserved Windows device name",
                m.id
            ));
        }
        if !valid_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        for id in m.dependencies.keys() {
            if !valid_mod_id(id) {
                return Err(format!("{MANIFEST}: invalid dependency id {id:?}"));
            }
        }
        if m.native.is_some() && m.wasm.is_some() {
            return Err(both());
        }
        inject::Inject::check_all(&m.inject, &m.id, m.native.is_some() || m.wasm.is_some())
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(native) = &m.native {
            // horizon-mixins keeps one id space per mod, which injects share.
            let mut ids: std::collections::HashSet<_> =
                m.inject.iter().map(|entry| &entry.id).collect();
            mixin::Mixin::check_all(&native.mixin, &m).map_err(|e| format!("{MANIFEST}: {e}"))?;
            for id in native
                .mixin
                .iter()
                .map(|entry| &entry.id)
                .chain(native.bridge.iter().chain(&native.state).map(|row| &row.id))
            {
                if id.trim().is_empty() || !ids.insert(id) {
                    return Err(format!(
                        "{MANIFEST}: empty or duplicate mixin, bridge or state id"
                    ));
                }
            }
            for module in &native.module {
                if module.id.trim().is_empty() || !ids.insert(&module.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate module id"));
                }
                if !plain_path(&module.file) {
                    return Err(format!("{MANIFEST}: module file must be a plain path"));
                }
                if !rust_ident(&module.name) {
                    return Err(format!("{MANIFEST}: module name must be an identifier"));
                }
                if !plain_path(&module.source) || !source_file(&module.source) {
                    return Err(format!(
                        "{MANIFEST}: module source must be a file under src/"
                    ));
                }
                if let Some(visibility) = &module.visibility
                    && !matches!(visibility.as_str(), "pub" | "pub(crate)")
                {
                    return Err(format!(
                        "{MANIFEST}: module visibility must be pub or pub(crate)"
                    ));
                }
            }
            if m.version.is_empty()
                || native.game.is_empty()
                || !revision(&native.horizon)
                || !revision(&native.mixins)
            {
                return Err(format!(
                    "{MANIFEST}: native requires game and exact revisions"
                ));
            }
        }
        if m.api != API {
            return Err(format!(
                "unsupported content API {}; this Horizon reads {API}",
                m.api
            ));
        }
        if m.capabilities != [Capability::Registries] {
            return Err(format!(
                "{MANIFEST}: a content mod has capabilities = [\"registries\"]"
            ));
        }
        for (field, url) in [
            ("license_url", &m.license_url),
            ("homepage", &m.homepage),
            ("issues", &m.issues),
        ] {
            if let Some(url) = url
                && (url.len() > 2048
                    || !(url.starts_with("https://") || url.starts_with("http://")))
            {
                return Err(format!(
                    "{MANIFEST}: {field} is an http(s) link of at most 2048 characters"
                ));
            }
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
}

/// A source file that belongs to the flat Cargo project, not its resource packs.
pub fn source_file(name: &str) -> bool {
    if name
        .split('/')
        .any(|part| matches!(part, ".git" | "target"))
    {
        return false;
    }
    name == "Cargo.toml"
        || name == "Cargo.lock"
        || name == "build.rs"
        || (name.starts_with("src/") && (name.ends_with(".rs") || name.ends_with(".toml")))
        || (!name.contains('/')
            && ["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

fn plain_path(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// A mod as found on disk.
pub struct Package {
    pub manifest: Manifest,
    pub icon: Option<crate::icon::Icon>,
    pub layouts: crate::layouts::Layouts,
    /// A wasm mod's own points (#2339), as its build listed them; a native mod's are listed from
    /// its source when it is built.
    pub points: Option<point_shape::catalog::Catalog>,
}
impl Package {
    /// The kind of this validated package, independent of its install folder.
    pub fn is_native(&self) -> bool {
        self.manifest.native.is_some()
    }

    /// Whether the package carries Rust source: a native mod, or a wasm mod built from it.
    fn has_source(&self) -> bool {
        self.manifest.native.is_some()
            || self
                .manifest
                .wasm
                .as_ref()
                .is_some_and(|w| w.built_with().is_none())
    }

    /// The package's `mod.wasm`, checked against its `[wasm]` section.
    pub fn read_wasm(path: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(path)?;
        let wasm = package
            .manifest
            .wasm
            .ok_or(WasmError::Undeclared.to_string())?;
        // Read at most one byte past the cap, so an oversized file is refused unread.
        let mut bytes = Vec::new();
        let read = if path.is_dir() {
            std::fs::File::open(path.join(WASM))
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
        } else {
            #[cfg(target_arch = "wasm32")]
            return Err("archive I/O is unavailable on wasm32".into());
            #[cfg(not(target_arch = "wasm32"))]
            {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                zip.by_name(WASM)
                    .map_err(|_| WasmError::Missing.to_string())?
                    .take(MAX_WASM + 1)
                    .read_to_end(&mut bytes)
            }
        };
        read.map_err(|e| format!("{WASM}: {e}"))?;
        wasm.check(&bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Read a flat `.hmod` or project folder, without executing native code.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let has_wasm;
        let toml = if metadata.is_dir() {
            for old in [
                "horizon-native.json",
                "horizon-mixin.toml",
                "native",
                "rust",
            ] {
                if std::fs::symlink_metadata(path.join(old)).is_ok() {
                    return Err(format!("{old}: unsupported package path"));
                }
            }
            if std::fs::symlink_metadata(path.join("Cargo.toml")).is_ok() {
                check_folder_file(path, "Cargo.toml")?;
                has_cargo = true;
            }
            if let Ok(source) = std::fs::symlink_metadata(path.join("src")) {
                if !source.is_dir() {
                    return Err("src: unsupported package path".into());
                }
                has_source = true;
            }
            let icon_path = path.join("icon.png");
            match std::fs::symlink_metadata(&icon_path) {
                Ok(metadata) => {
                    check_folder_file(path, "icon.png")?;
                    if metadata.len() > crate::icon::MAX_ICON as u64 {
                        return Err("icon.png exceeds 256 KiB".into());
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&icon_path)
                        .map_err(|e| format!("icon.png: {e}"))?
                        .take(crate::icon::MAX_ICON as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("icon.png: {e}"))?;
                    icon = Some(crate::icon::Icon::parse(bytes)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("icon.png: {e}")),
            }
            check_folder_file(path, MANIFEST)?;
            has_wasm = std::fs::symlink_metadata(path.join(WASM)).is_ok();
            for (name, _) in crate::content::folder_files(path)? {
                check_folder_file(path, &name)?;
            }
            let mut text = String::new();
            std::fs::File::open(path.join(MANIFEST))
                .map_err(|e| format!("{MANIFEST}: {e}"))?
                .take(MAX_MANIFEST + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{MANIFEST}: {e}"))?;
            if text.len() as u64 > MAX_MANIFEST {
                return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
            }
            text
        } else {
            if !metadata.is_file() {
                return Err(format!(
                    "{}: unsupported file type in a content mod",
                    path.display()
                ));
            }
            #[cfg(target_arch = "wasm32")]
            return Err("archive I/O is unavailable on wasm32".into());
            #[cfg(not(target_arch = "wasm32"))]
            {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                return Self::read_zip(&mut zip);
            }
        };
        let mut package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        match std::fs::File::open(path.join(crate::layouts::FILE)) {
            Ok(file) => {
                check_folder_file(path, crate::layouts::FILE)?;
                package.layouts = crate::layouts::Layouts::read(file)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("layouts.json: {e}")),
        }
        if let Some(wasm) = &package.manifest.wasm {
            let bytes = match std::fs::File::open(path.join(inject::mod_points::FILE)) {
                Ok(file) => {
                    check_folder_file(path, inject::mod_points::FILE)?;
                    Some(inject::mod_points::read_bytes(file)?)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(format!("{}: {e}", inject::mod_points::FILE)),
            };
            package.points = inject::mod_points::bound(bytes.as_deref(), wasm.points())?;
        } else if std::fs::symlink_metadata(path.join(inject::mod_points::FILE)).is_ok() {
            return Err(format!(
                "{}: only a wasm package carries its points",
                inject::mod_points::FILE
            ));
        }
        if let Some(wasm) = &package.manifest.wasm {
            let bytes = std::fs::read(path.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(package)
    }

    /// Read and validate an archive, including all selected source and resource entries.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_zip<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Self, String> {
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let mut has_wasm = false;
        let mut entries = Vec::new();
        let mut layouts = None;
        let mut points = None;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let mode = entry.unix_mode().unwrap_or_default();
            entries.push((name.clone(), entry.size(), entry.is_dir()));
            let first = name.split('/').next().unwrap_or("");
            if matches!(
                first,
                "horizon-native.json" | "horizon-mixin.toml" | "native" | "rust"
            ) {
                return Err(format!("{name}: unsupported package path"));
            }
            has_source |= first == "src";
            if entry.is_dir() && matches!(mode & 0o170000, 0 | 0o040000) {
                continue;
            }
            has_cargo |= name == "Cargo.toml";
            // The one executable a package may carry, checked against `[wasm]` below.
            if name == WASM && matches!(mode & 0o170000, 0 | 0o100000) {
                has_wasm = true;
                continue;
            }
            if name == crate::layouts::FILE && matches!(mode & 0o170000, 0 | 0o100000) {
                if layouts.is_some() {
                    return Err("duplicate layouts.json".into());
                }
                layouts = Some(crate::layouts::Layouts::read(&mut entry)?);
                continue;
            }
            if name == inject::mod_points::FILE && matches!(mode & 0o170000, 0 | 0o100000) {
                if points.is_some() {
                    return Err(format!("duplicate {name}"));
                }
                points = Some(inject::mod_points::read_bytes(&mut entry)?);
                continue;
            }
            check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
            if name == "icon.png" {
                if icon.is_some() {
                    return Err("duplicate icon.png".into());
                }
                if entry.size() > crate::icon::MAX_ICON as u64 {
                    return Err("icon.png exceeds 256 KiB".into());
                }
                let mut bytes = Vec::new();
                drop(entry);
                zip.by_index(i)
                    .map_err(|e| e.to_string())?
                    .take(crate::icon::MAX_ICON as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("icon.png: {e}"))?;
                icon = Some(crate::icon::Icon::parse(bytes)?);
            }
        }
        let mut toml = String::new();
        zip.by_name(MANIFEST)
            .map_err(|e| format!("{MANIFEST}: {e}"))?
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut toml)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if toml.len() as u64 > MAX_MANIFEST {
            return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
        }
        let mut package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        if let Some(wasm) = &package.manifest.wasm {
            let mut bytes = Vec::new();
            zip.by_name(WASM)
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        package.layouts = layouts.unwrap_or_default();
        match &package.manifest.wasm {
            Some(wasm) => {
                package.points = inject::mod_points::bound(points.as_deref(), wasm.points())?;
            }
            None if points.is_some() => {
                return Err(format!(
                    "{}: only a wasm package carries its points",
                    inject::mod_points::FILE
                ));
            }
            None => {}
        }
        if package.has_source() || package.manifest.wasm.is_some() {
            let mut names = std::collections::HashSet::new();
            let mut source_count = 0usize;
            let mut source_bytes = 0u64;
            let mut resource_count = 0usize;
            let mut resource_bytes = 0u64;
            for (name, size, directory) in entries {
                let path = if directory {
                    name.trim_end_matches('/')
                } else {
                    &name
                };
                if !plain_path(path) || !names.insert(name.clone()) {
                    return Err(format!("invalid or duplicate package path: {name}"));
                }
                if !directory {
                    if name != MANIFEST
                        && name != crate::layouts::FILE
                        && !(name == inject::mod_points::FILE && package.manifest.wasm.is_some())
                        && name != "icon.png"
                        && !(name == WASM && package.manifest.wasm.is_some())
                        && !name.starts_with("assets/")
                        && !name.starts_with("data/")
                        && !source_file(&name)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                    if source_file(&name) {
                        source_count += 1;
                        source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                    } else if name.starts_with("assets/") || name.starts_with("data/") {
                        crate::content::check_path(&name)?;
                        crate::content::check_namespace(&name, &package.manifest)?;
                        if size > crate::content::MAX_FILE {
                            return Err(format!("{name} exceeds the content file size limit"));
                        }
                        resource_count += 1;
                        resource_bytes = resource_bytes
                            .checked_add(size)
                            .ok_or("content too large")?;
                    }
                }
            }
            if source_count > 511 || source_bytes > 64 << 20 {
                return Err("native package exceeds its entry or size limit".into());
            }
            if resource_count > crate::content::MAX_FILES
                || resource_bytes > crate::content::MAX_TOTAL
            {
                return Err("content exceeds its entry or size limit".into());
            }
        }
        Ok(package)
    }

    fn from_parts(
        text: &str,
        icon: Option<crate::icon::Icon>,
        has_cargo: bool,
        has_source: bool,
        has_wasm: bool,
    ) -> Result<Self, String> {
        let manifest = Manifest::parse(text)?;
        let package = Package {
            manifest,
            icon,
            layouts: Default::default(),
            points: None,
        };
        if has_cargo != package.has_source() || has_source != has_cargo {
            return Err("inconsistent code declaration ([native] or [wasm]) and source".into());
        }
        match (has_wasm, package.manifest.wasm.is_some()) {
            (true, false) => return Err(WasmError::Undeclared.to_string()),
            (false, true) => return Err(WasmError::Missing.to_string()),
            _ => {}
        }
        Ok(package)
    }
    /// Whether the mod at `path` (a `.hmod` or a folder) has a [`MANIFEST`].
    pub fn has_manifest(path: &Path) -> bool {
        if path.is_dir() {
            return path.join(MANIFEST).is_file();
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::fs::File::open(path)
                .ok()
                .and_then(|f| zip::ZipArchive::new(f).ok())
                .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
        }
        #[cfg(target_arch = "wasm32")]
        false
    }
    /// Select and validate a flat project's source and resource files deterministically.
    pub fn directory_files(
        &self,
        root: &Path,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
        let content = crate::Content::read(root, &self.manifest)?;
        let mut files = std::collections::BTreeMap::new();
        let mut reader = content.reader()?;
        for name in &content.files {
            files.insert(name.clone(), reader.read(name)?);
        }
        if let Ok(file) = std::fs::File::open(root.join(crate::layouts::FILE)) {
            let mut bytes = Vec::new();
            file.take((1 << 20) + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            crate::layouts::Layouts::read(bytes.as_slice())?;
            files.insert(crate::layouts::FILE.into(), bytes);
        }
        if self.manifest.wasm.is_some()
            && let Ok(file) = std::fs::File::open(root.join(inject::mod_points::FILE))
        {
            files.insert(
                inject::mod_points::FILE.into(),
                inject::mod_points::read_bytes(file)?,
            );
        }
        if let Some(icon) = &self.icon {
            files.insert("icon.png".to_string(), icon.bytes().to_vec());
        }
        if let Some(wasm) = &self.manifest.wasm {
            let bytes = std::fs::read(root.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
            files.insert(WASM.to_owned(), bytes);
        }
        if self.has_source() {
            fn walk(
                root: &Path,
                dir: &Path,
                files: &mut std::collections::BTreeMap<String, Vec<u8>>,
                source_bytes: &mut u64,
                source_count: &mut usize,
            ) -> Result<(), String> {
                for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
                    let name = relative
                        .to_str()
                        .ok_or("non-UTF-8 package path")?
                        .replace('\\', "/");
                    let kind = entry.file_type().map_err(|e| e.to_string())?;
                    if kind.is_dir() {
                        if matches!(entry.file_name().to_str(), Some(".git" | "target"))
                            || matches!(name.as_str(), "assets" | "data")
                        {
                            continue;
                        }
                        walk(root, &path, files, source_bytes, source_count)?;
                    } else if source_file(&name) {
                        check_folder_file(root, &name)?;
                        let size = entry.metadata().map_err(|e| e.to_string())?.len();
                        *source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                        *source_count += 1;
                        if *source_bytes > 64 << 20 || *source_count > 511 {
                            return Err("native package exceeds its entry or size limit".into());
                        }
                        files.insert(name, std::fs::read(path).map_err(|e| e.to_string())?);
                    } else if name.ends_with(".rs") || (name.ends_with(".toml") && name != MANIFEST)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                }
                Ok(())
            }
            let (mut source_bytes, mut source_count) = (0, 0);
            walk(root, root, &mut files, &mut source_bytes, &mut source_count)?
        }
        files.insert(
            MANIFEST.to_owned(),
            std::fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
        );
        Ok(files)
    }

    /// Pack a flat project, selecting its source and resource files deterministically.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn pack_directory(root: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let mut files = package.directory_files(root)?;
        let text = String::from_utf8(files.remove(MANIFEST).expect("selected manifest"))
            .map_err(|e| e.to_string())?;
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Self::write_files(&text, &files)
    }

    /// The `.hmod` bytes for `manifest_text` and selected project files.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_files(manifest_text: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let manifest = Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        for (name, bytes) in files {
            let code = manifest.native.is_some() || manifest.wasm.is_some();
            let source = code && source_file(name) && plain_path(name);
            if *name == crate::layouts::FILE {
                crate::layouts::Layouts::read(*bytes)?;
                put(name, bytes)?;
                continue;
            }
            if *name == inject::mod_points::FILE && manifest.wasm.is_some() {
                put(name, bytes)?;
                continue;
            }
            if *name == WASM && manifest.wasm.is_some() {
                put(name, bytes)?;
                continue;
            }
            if *name != "icon.png" && !source {
                crate::content::check_path(name)?;
            }
            check_file(name, 0, || Ok(*bytes)).map_err(|e| e.to_string())?;
            put(name, bytes)?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(err)?;
        Self::read_zip(&mut archive)?;
        Ok(bytes)
    }
}

/// A content entry that cannot be safely loaded as data.
#[derive(Debug)]
pub enum ContentFileError {
    UnsupportedType(String),
    Executable(String),
    Read {
        name: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ContentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => {
                write!(f, "{name}: unsupported file type in a content mod")
            }
            Self::Executable(name) => {
                write!(f, "{name}: unsupported executable file in a content mod")
            }
            Self::Read { name, source } => write!(f, "{name}: {source}"),
        }
    }
}

impl std::error::Error for ContentFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The shared content-file boundary. Refuse type/mode before opening a file,
/// then inspect its header even when its name has no executable extension.
pub(crate) fn check_file<R: Read>(
    name: &str,
    mode: u32,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<(), ContentFileError> {
    if !matches!(mode & 0o170000, 0 | 0o100000) {
        return Err(ContentFileError::UnsupportedType(name.into()));
    }
    let extension = name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'));
    let executable = extension.is_some_and(|(_, ext)| {
        [
            "wasm", "exe", "dll", "so", "dylib", "class", "jar", "com", "bat", "cmd", "ps1",
        ]
        .iter()
        .any(|kind| ext.eq_ignore_ascii_case(kind))
    });
    if executable || mode & 0o111 != 0 {
        return Err(ContentFileError::Executable(name.into()));
    }
    let error = |source| ContentFileError::Read {
        name: name.into(),
        source,
    };
    let mut file = open().map_err(error)?;
    let mut header = [0; 4];
    let mut read = 0;
    while read < header.len() {
        let size = file.read(&mut header[read..]).map_err(error)?;
        if size == 0 {
            break;
        }
        read += size;
    }
    let executable = matches!(
        &header,
        b"\0asm"
            | b"\x7fELF"
            | b"\xfe\xed\xfa\xce"
            | b"\xce\xfa\xed\xfe"
            | b"\xfe\xed\xfa\xcf"
            | b"\xcf\xfa\xed\xfe"
            | b"\xca\xfe\xba\xbe"
            | b"\xbe\xba\xfe\xca"
    ) || header.starts_with(b"MZ")
        || (header.starts_with(b"#!") && header[2] != b'[');
    if executable {
        return Err(ContentFileError::Executable(name.into()));
    }
    Ok(())
}

fn check_folder_file(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    let mode = if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0
        }
    } else {
        0o120000
    };
    check_file(name, mode, || std::fs::File::open(&path)).map_err(|e| e.to_string())
}

/// The `.hmod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hmod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_device_mod_ids_are_refused() {
        for id in ["con", "prn", "aux", "nul", "com1", "com9", "lpt1", "lpt9"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id))
                    .unwrap_err()
                    .contains("reserved Windows device name"),
                "{id}"
            );
        }
        for id in ["console", "com0", "com10", "lpt0", "con_tool"] {
            assert!(
                Manifest::parse(&EXAMPLE.replace("ruby", id)).is_ok(),
                "{id}"
            );
        }
    }

    const EXAMPLE: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
"#;

    #[test]
    fn unified_injections_and_native_modules_share_the_id_namespace() {
        let pin = "a".repeat(40);
        let text = |module_id| {
            format!(
                "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n[[inject]]\nid='shared'\nat='game::tick'\nkind='head'\nwith='tick'\nhash='signature'\n[[native.module]]\nid='{module_id}'\nfile='game.rs'\nname='runtime'\nsource='src/runtime.rs'\n"
            )
        };
        assert!(Manifest::parse(&text("distinct")).is_ok());
        let error = Manifest::parse(&text("shared")).unwrap_err();
        assert!(error.contains("duplicate module id"), "{error}");
    }

    #[test]
    fn wasm_internals_declarations_parse_with_and_without_kind_and_need_a_wasm_or_native_mod() {
        let wasm = format!(
            "[wasm]\nsha512='{}'\ntoolchain='t'\nbuilt_with='t'\n",
            "a".repeat(128)
        );
        let injection = "[[inject]]\nid='fell'\nat='Game::break_block'\nkind='head'\nwith='fell'\naccess='internals'\nhash='h'\n";
        let text = |internals: &str, tail: &str| format!("{EXAMPLE}{internals}{tail}");
        let ok = Manifest::parse(&text(
            "internals=[{path='Game::world',kind='field',hash='1'},{path='Game::tick',hash='2'}]\n",
            &format!("{wasm}{injection}"),
        ))
        .unwrap();
        assert_eq!(ok.internals[0].kind.as_deref(), Some("field"));
        assert_eq!(ok.internals[1].kind, None);
        // A wasm internals listener with nothing declared, and declarations with no code mod.
        assert!(Manifest::parse(&text("", &format!("{wasm}{injection}"))).is_err());
        assert!(Manifest::parse(&text("internals=[{path='Game::world',hash='1'}]\n", "")).is_err());
    }

    #[test]
    fn refuses_bad_ids_and_retired_manifests() {
        let bad_id = EXAMPLE.replace("ruby", "Ruby!");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        for api in ["0.1", "0.3", "0.4"] {
            let text = EXAMPLE.replace("api = \"0.2\"", &format!("api = \"{api}\""));
            assert!(
                Manifest::parse(&text)
                    .unwrap_err()
                    .contains("unsupported content API")
            );
        }
        for caps in ["[]", "[\"hud\"]", "[\"registries\", \"registries\"]"] {
            let text = EXAMPLE.replace("[\"registries\"]", caps);
            assert!(Manifest::parse(&text).is_err());
        }
        for field in ["network = true", "experimental = []", "side = \"server\""] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{field}\n")).is_err());
        }
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().side, Side::Client);
        assert_eq!(
            Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n"))
                .unwrap()
                .side,
            Side::Both
        );
    }

    #[test]
    fn native_manifest_requires_exact_pins_and_known_fields() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        Manifest::parse(&format!("{EXAMPLE}{native}")).unwrap();
        assert!(
            Manifest::parse(&format!(
                "{EXAMPLE}{}",
                native.replace(&"a".repeat(40), "main")
            ))
            .is_err()
        );
        for unsupported in ["unexpected = true", "[[native.post_build]]\nbin = 'bake'"] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{native}{unsupported}\n")).is_err());
        }
    }

    #[test]
    fn native_unknown_fields_are_refused_at_the_owning_boundary() {
        let pin = "a".repeat(40);
        let base = format!("{EXAMPLE}[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n");
        let mut accepted = Vec::new();
        for field in [
            "injection=[]",
            "[[native.injection]]\nid='tick'\ntarget='game::tick'\nkind='HEAD'\nraw='tick'",
            "internals=[]",
            "[[native.internals]]\npath='Game'\nhash='h'",
            "unexpected=true",
        ] {
            let text = format!("{base}{field}\n");
            match Manifest::parse(&text) {
                Ok(_) => accepted.push(field),
                Err(error) => assert!(error.contains("unknown field"), "{error}"),
            }
            // The same owning serde boundary applies outside Manifest::parse.
            if toml::from_str::<Native>(text.split("[native]\n").nth(1).unwrap()).is_ok() {
                accepted.push(field);
            }
        }
        assert!(
            accepted.is_empty(),
            "accepted unknown native fields: {accepted:?}"
        );
    }

    #[test]
    fn internal_kinds_distinguish_members_and_round_trip() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let declaration = "internals=[{path='Round::listening',kind='field',hash='1234'}, {path='Round::listening',kind='method',hash='5678'}]\n";
        let manifest =
            Manifest::parse(&base.replace("[native]", &format!("{declaration}[native]"))).unwrap();
        assert_eq!(manifest.internals[1].kind.as_deref(), Some("method"));
        let row = toml::to_string(&manifest.internals[1]).unwrap();
        assert_eq!(
            toml::from_str::<Internal>(&row).unwrap(),
            manifest.internals[1]
        );
        let native = manifest.native.unwrap();
        let text = toml::to_string(&native).unwrap();
        assert_eq!(toml::from_str::<Native>(&text).unwrap(), native);
        for kind in ["field", ""] {
            let invalid = declaration.replace("kind='method'", &format!("kind='{kind}'"));
            assert!(
                Manifest::parse(&base.replace("[native]", &format!("{invalid}[native]"))).is_err()
            );
        }
        let untyped = Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("1234".into()),
        };
        assert!(!toml::to_string(&untyped).unwrap().contains("kind"));
    }

    #[test]
    fn declared_internals_and_unified_injections_have_one_explicit_form() {
        let pin = "a".repeat(40);
        let base = format!(
            "{EXAMPLE}internals=[{{path='Game',hash='1234'}}]\n[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n"
        );
        let text = format!(
            "{base}[[inject]]\nid='count'\nat='Game::tick'\nkind='head'\nwith='count'\naccess='internals'\nhash='h'\n"
        );
        let manifest = Manifest::parse(&text).unwrap();
        assert_eq!(manifest.internals[0].path, "Game");
        assert_eq!(manifest.inject[0].with, "count");
        for name in ["", "nested::count", "0count"] {
            assert!(
                Manifest::parse(&text.replace("with='count'", &format!("with='{name}'"))).is_err()
            );
        }
        for obsolete in [
            "helper='src/a.rs'",
            "cancellable=true",
            "bindings=[{name='a',ty='u8',value='a'}]",
            "hook='tick'",
        ] {
            assert!(Manifest::parse(&format!("{text}{obsolete}\n")).is_err());
        }
        for bad in [
            text.replace("hash='1234'", "hash=''"),
            text.replace("path='Game'", "path=''"),
            text.replace("hash='1234'}", "hash='1234'},{path='Game',hash='5678'}"),
            text.replace("with='count'", "with=''"),
            text.replace("with='count'", "with='count'\nraw='count'"),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "accepted {bad}");
        }
        Manifest::parse(&text.replace("kind='head'", "kind='return'")).unwrap();
        let content = format!("{EXAMPLE}internals=[{{path='Game',hash='h'}}]\n");
        assert!(
            Manifest::parse(&content)
                .unwrap_err()
                .contains("internals need")
        );
    }

    #[test]
    fn native_modules_are_validated_like_injections() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let module = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        let manifest = Manifest::parse(&format!("{EXAMPLE}{native}{module}")).unwrap();
        let module = &manifest.native.as_ref().unwrap().module[0];
        assert_eq!(module.name, "runtime");
        assert_eq!(module.visibility, None);
        // Support modules still forward whole to horizon-mixins.
        let forwarded = toml::to_string(manifest.native.as_ref().unwrap()).unwrap();
        assert!(forwarded.contains("[[module]]"));
        assert!(forwarded.contains("id = \"runtime\""));
        let valid = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        for bad in [
            // escapes the package's src/
            valid.replace("source = 'src/runtime.rs'", "source = '../escape.rs'"),
            // unknown field
            format!("{valid}order = 1\n"),
            // duplicate id, also against an injection's
            format!("{valid}{valid}"),
            format!(
                "{valid}[[inject]]\nid = 'runtime'\nat = 'a'\nkind = 'head'\nwith = 'b'\nhash = 'h'\n"
            ),
            // not an identifier
            valid.replace("name = 'runtime'", "name = 'run time'"),
            // file must stay a plain path
            valid.replace(
                "file = 'crates/horizon-client/src/game.rs'",
                "file = '/abs.rs'",
            ),
            // horizon-mixins accepts exactly pub and pub(crate)
            valid.to_owned() + "visibility = 'public'\n",
        ] {
            assert!(
                Manifest::parse(&format!("{EXAMPLE}{native}{bad}")).is_err(),
                "accepted: {bad}"
            );
        }
        let public = valid.to_owned() + "visibility = 'pub(crate)'\n";
        assert!(Manifest::parse(&format!("{EXAMPLE}{native}{public}")).is_ok());
    }

    #[test]
    fn dependencies_are_typed_for_content_and_code_mods() {
        let deps =
            "[dependencies]\ncreate = '^6.0'\naddon = { version = '^1.2', optional = true }\n";
        let content = Manifest::parse(&format!("{EXAMPLE}{deps}")).unwrap();
        assert_eq!(content.dependencies.len(), 2);
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let text = format!("{EXAMPLE}{deps}{native}");
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            matches!(&manifest.dependencies["addon"], Dependency::Detailed(details) if details.optional)
        );
        assert!(matches!(
            &manifest.dependencies["create"],
            Dependency::Version(_)
        ));
        for bad in [
            text.replace("^6.0", "not semver"),
            text.replace("create =", "\"not-valid\" ="),
            text.replace("optional = true", "unexpected = true"),
        ] {
            assert!(Manifest::parse(&bad).is_err());
        }
    }

    #[test]
    fn layout_metadata_round_trips_and_malformed_archives_are_refused() {
        let layouts = br#"{"ruby":{"ruby::T":"0123456789abcdef"}}"#;
        let bytes = Package::write_files(EXAMPLE, &[("layouts.json", layouts)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        let changed =
            crate::layouts::Layouts::read(br#"{"ruby":{"ruby::T":"1123456789abcdef"}}"#.as_slice())
                .unwrap();
        assert_eq!(
            package.layouts.first_difference("ruby", &changed),
            Some("ruby::T")
        );
        assert!(
            Package::write_files(EXAMPLE, &[("layouts.json", b"{")])
                .unwrap_err()
                .contains("layouts.json")
        );
    }

    #[test]
    fn flat_native_archive_round_trips_through_shared_writer() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname = 'other'\nversion = '2.0.0'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = Package::write_files(&manifest, files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        assert_eq!(package.manifest.id, "ruby");
        assert_eq!(package.manifest.version, "0.1.0");
        assert_eq!(package.manifest.native.unwrap().game, "26.3");
        assert!(
            Package::write_files(&manifest, &[("Cargo.toml", b""), ("src/../bad.rs", b"")])
                .is_err()
        );
        assert!(
            Package::write_files(
                &manifest,
                &[
                    ("Cargo.toml", b""),
                    ("src/lib.rs", b""),
                    ("src/lib.rs", b"")
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn flat_project_packs_only_selected_safe_files() {
        let root = std::env::temp_dir().join(format!("horizon-flat-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(root.join("target/generated")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'other'\nversion = '2.0.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        let registry_lock = "version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.18'\n";
        std::fs::write(root.join("Cargo.lock"), registry_lock).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("target/generated/bad.rs"), "#!/bin/sh\n").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        assert!(zip.by_name("Cargo.toml").is_ok());
        assert!(zip.by_name("src/lib.rs").is_ok());
        let mut packed_lock = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("Cargo.lock").unwrap(), &mut packed_lock)
            .unwrap();
        assert_eq!(packed_lock, registry_lock);
        assert!(zip.by_name(".git/hooks/pre-commit").is_err());
        assert!(zip.by_name("target/generated/bad.rs").is_err());
        std::fs::write(root.join("src/lib.rs"), b"MZ executable").unwrap();
        assert!(
            Package::pack_directory(&root)
                .unwrap_err()
                .contains("unsupported executable")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_kind_requires_matching_source_and_native_declaration() {
        let root =
            std::env::temp_dir().join(format!("horizon-package-kind-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&root).is_ok());
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'unrelated'\nversion = '7.0.0'\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        assert!(Package::read(&root).is_err());
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), format!("{EXAMPLE}{native}")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::create_dir(root.join("native")).unwrap();
        assert!(Package::read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checks_extra_namespaces() {
        let with = |list: &str| Manifest::parse(&format!("{EXAMPLE}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["ruby", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "horizon"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"ruby\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
    }

    #[test]
    fn the_mods_screen_fields_are_optional_and_links_must_be_http() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert!(m.license.is_empty() && m.credits.is_empty());
        assert!(m.license_url.is_none() && m.homepage.is_none() && m.issues.is_none());
        let full = format!(
            "{EXAMPLE}license = \"MIT\"
credits = \"Alex\"
homepage = \"https://example.org\"
issues = \"http://example.org/issues\"
"
        );
        let m = Manifest::parse(&full).unwrap();
        assert_eq!((m.license.as_str(), m.credits.as_str()), ("MIT", "Alex"));
        assert_eq!(m.homepage.as_deref(), Some("https://example.org"));
        for field in ["license_url", "homepage", "issues"] {
            let e = Manifest::parse(&format!(
                "{EXAMPLE}{field} = \"file:///etc/passwd\"
"
            ))
            .unwrap_err();
            assert!(e.contains(field), "{e}");
        }
    }

    #[test]
    fn a_wasm_mod_carries_exactly_its_declared_module() {
        // A minimal valid shared member: a `dylink.0` section and the three guest exports.
        let module = wat::parse_str(
            r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
                (func (export "horizon_layout")) (func (export "horizon_in")) (func (export "horizon_hook")))"#,
        )
        .unwrap();
        let text = format!(
            "{}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"1.99.1\"\nbuilt_with = \"test\"\n",
            EXAMPLE.trim_end(),
            sha512_of(&module)
        );
        let bytes = Package::write_files(&text, &[(WASM, &module)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(Package::read_zip(&mut zip).unwrap().manifest.wasm.is_some());
        let error = Package::write_files(&text, &[(WASM, b"\0asm\x02\0\0\0")]).unwrap_err();
        assert_eq!(error, WasmError::Hash.to_string());
        assert_eq!(
            Package::write_files(&text, &[]).unwrap_err(),
            WasmError::Missing.to_string()
        );
        let both = format!(
            "{text}\n[native]\ngame = \"g\"\nhorizon = \"{0}\"\nmixins = \"{0}\"\n",
            "a".repeat(40)
        );
        assert!(
            Manifest::parse(&both)
                .unwrap_err()
                .contains("a mod is [native] or [wasm], not both")
        );
    }

    #[test]
    fn wasm_packages_refuse_native_tables_even_with_injects() {
        use sha2::{Digest, Sha512};
        use std::io::Write;
        let module = b"\0asm\x01\0\0\0";
        let base = format!(
            "{EXAMPLE}[wasm]\nsha512='{:x}'\ntoolchain='1.99.1'\n",
            Sha512::digest(module)
        );
        let foreign = "[native]\ngame='26.3'\nhorizon='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'\nmixins='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'\n[[native.injection]]\nid='non_raw'\ntarget='game::tick'\nkind='HEAD'\nhelper='src/count.rs'\n";
        let missing_raw = foreign.replace("helper='src/count.rs'\n", "");
        assert!(
            Manifest::parse(&format!("{base}{missing_raw}"))
                .unwrap_err()
                .contains("a mod is [native] or [wasm], not both")
        );
        for unified in [
            "",
            "[[inject]]\nid='new'\nat='game::tick'\nkind='head'\nwith='tick'\nhash='signature'\n",
        ] {
            assert!(Manifest::parse(&format!("{base}{unified}")).is_ok());
            // Build an input archive directly: the public writer rightly refuses its manifest.
            let text = format!("{base}{foreign}{unified}");
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, bytes) in [(MANIFEST, text.as_bytes()), (WASM, module.as_slice())] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
            let error = Package::read_zip(&mut archive)
                .err()
                .expect("native tables refused before package classification");
            assert!(
                error.contains("a mod is [native] or [wasm], not both"),
                "{error}"
            );
        }
    }

    #[test]
    fn only_a_wasm_package_carries_its_points_and_only_the_ones_its_build_pinned() {
        use std::io::Write;
        let module = wat::parse_str(
            r#"(module (@custom "dylink.0" "\01\04\00\00\00\00")
                (func (export "horizon_layout")) (func (export "horizon_in")) (func (export "horizon_hook")))"#,
        )
        .unwrap();
        let points = br#"{"crate":"ruby","table":"__horizon_points","points":[]}"#;
        let manifest = |pin: Option<&[u8]>| {
            format!(
                "{}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"1.99.1\"\nbuilt_with = \"test\"\n{}",
                EXAMPLE.trim_end(),
                sha512_of(&module),
                pin.map(|bytes| format!("points = \"{}\"\n", sha512_of(bytes)))
                    .unwrap_or_default()
            )
        };
        // Build an archive directly: the writer refuses what the reader must refuse too.
        let archive = |text: &str, catalog: Option<&[u8]>| {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let mut files = vec![(MANIFEST, text.as_bytes()), (WASM, module.as_slice())];
            files.extend(catalog.map(|bytes| (inject::mod_points::FILE, bytes)));
            for (name, bytes) in files {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
            Package::read_zip(&mut archive).map(|package| package.points)
        };
        let pinned = manifest(Some(points));
        assert_eq!(
            archive(&pinned, Some(points)).unwrap().unwrap().krate,
            "ruby"
        );
        assert!(
            Package::write_files(
                &pinned,
                &[(WASM, &module), (inject::mod_points::FILE, points)]
            )
            .is_ok()
        );
        assert_eq!(archive(&manifest(None), None).unwrap(), None);
        let tampered = br#"{"crate":"attacker","table":"other","points":[]}"#;
        for (case, text, catalog, error) in [
            (
                "missing but pinned",
                &pinned,
                None,
                "mod-points.json: missing; [wasm] pins one",
            ),
            (
                "changed after the build",
                &pinned,
                Some(tampered.as_slice()),
                "mod-points.json: does not match the hash [wasm] pins: changed after the build",
            ),
            (
                "present but unpinned",
                &manifest(None),
                Some(points.as_slice()),
                "mod-points.json: [wasm] pins no points; only the build writes mod-points.json",
            ),
        ] {
            assert_eq!(archive(text, catalog).err().unwrap(), error, "read: {case}");
            let mut files = vec![(WASM, module.as_slice())];
            files.extend(catalog.map(|bytes| (inject::mod_points::FILE, bytes)));
            assert_eq!(
                Package::write_files(text, &files).unwrap_err(),
                error,
                "write: {case}"
            );
        }
        // A pinned but malformed catalog is refused by its parse.
        let malformed = b"{";
        assert!(
            archive(&manifest(Some(malformed)), Some(malformed))
                .err()
                .unwrap()
                .starts_with("mod-points.json:")
        );
        // A content package carrying one is refused: no build of it wrote points.
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in [
            (MANIFEST, EXAMPLE.as_bytes()),
            (inject::mod_points::FILE, points.as_slice()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        assert_eq!(
            Package::read_zip(&mut archive).err().unwrap(),
            "mod-points.json: only a wasm package carries its points"
        );
        // The folder reader holds the same rule.
        let folder = std::env::temp_dir().join(format!("mod-points-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&folder).is_ok());
        std::fs::write(folder.join(inject::mod_points::FILE), points).unwrap();
        let refused = Package::read(&folder).err();
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(
            refused.unwrap(),
            "mod-points.json: only a wasm package carries its points"
        );
    }

    #[test]
    fn a_wasm_listener_uses_root_internal_declarations_without_native_code() {
        let wasm = format!(
            "{}\n[wasm]\nsha512 = '{}'\ntoolchain = '1.99.1'\n",
            EXAMPLE.trim_end(),
            "0".repeat(128)
        );
        let native = "internals=[{path='Game::world',kind='field',hash='world-hash'}]\n";
        let injection = "[[inject]]\nid='fell'\nat='Game::break_block'\nkind='head'\nwith='fell'\naccess='internals'\nhash='h'\n";
        let parsed = Manifest::parse(&format!(
            "{}{injection}",
            wasm.replace("[wasm]", &format!("{native}[wasm]"))
        ))
        .unwrap();
        assert!(parsed.native.is_none());
        assert_eq!(parsed.internals[0].hash.as_deref(), Some("world-hash"));

        for wrong in [
            "internals=[{path='',hash='h'}]",
            "internals=[{path='Game',hash=''}]",
            "internals=[{path='Game',hash='h'}, {path='Game',hash='h'}]",
            "module=[]",
            "game='g'",
            "horizon='pin'",
            "mixins='pin'",
        ] {
            assert!(
                Manifest::parse(&format!("{wasm}[native]\n{wrong}\n{injection}")).is_err(),
                "{wrong}"
            );
        }
        let mod_injection = injection.replace("Game::break_block", "chem_lib::transfer");
        Manifest::parse(&format!(
            "{wasm}{mod_injection}[dependencies]\nchem_lib='^1.0'\n"
        ))
        .unwrap();
        assert!(
            Manifest::parse(&format!("{wasm}{mod_injection}"))
                .unwrap_err()
                .contains("needs declared internals")
        );
        assert!(
            Manifest::parse(&format!("{wasm}{injection}"))
                .unwrap_err()
                .contains("needs declared internals")
        );
    }

    #[test]
    fn content_round_trips_and_executable_files_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        let archive = root.join("ruby.hmod");
        std::fs::write(&archive, Package::write_files(EXAMPLE, &[]).unwrap()).unwrap();
        assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        assert_eq!(Package::read(&dir).unwrap().manifest.id, "ruby");
        let cases: &[(&str, &[u8])] = &[
            ("mod.wasm", b""),
            ("./mod.wasm", b"\0asm"),
            ("MOD.WASM", b""),
            ("sub/mod.wasm", b""),
            ("assets/ruby/CODE.WaSm", b""),
            ("assets/ruby/model.bin", b"\0asm"),
            ("assets/ruby/native.bin", b"\x7fELF"),
            ("assets/ruby/windows.bin", b"MZ"),
            ("assets/ruby/mac.bin", b"\xcf\xfa\xed\xfe"),
            ("assets/ruby/script", b"#!/bin/sh"),
            ("assets/ruby/program.EXE", b""),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            let error = Package::read(&archive).err().unwrap();
            // The one place a module may sit is refused for want of `[wasm]`.
            let expected = if *name == WASM {
                WasmError::Undeclared.to_string()
            } else {
                "unsupported executable file".to_string()
            };
            assert!(error.contains(&expected), "{name}: {error}");
            if name.starts_with("assets/") {
                let error = Package::read(&dir).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
            } else if std::fs::symlink_metadata(dir.join(WASM)).is_ok() {
                // On disk `./mod.wasm` is the top-level module too, and so is `MOD.WASM` where the
                // filesystem ignores case (Windows, macOS).
                let undeclared = WasmError::Undeclared.to_string();
                assert_eq!(
                    Package::read(&dir).err(),
                    Some(undeclared),
                    "unpackaged {name}"
                );
            } else {
                assert!(Package::read(&dir).is_ok(), "unpackaged {name}");
            }
            std::fs::remove_file(file).unwrap();
        }
        assert!(Package::write_files(EXAMPLE, &[("assets/ruby/model.bin", b"\0asm")]).is_err());
        // Native bundle sources can start with Rust inner attributes, not a script shebang.
        assert!(
            Package::write_files(
                EXAMPLE,
                &[("assets/ruby/source.txt", b"#![allow(dead_code)]")]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_folder_loads_without_packaging_git_hooks() {
        let root = std::env::temp_dir().join(format!("horizon-mod-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join(".git/hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("assets/ruby")).unwrap();
        std::fs::write(root.join("assets/ruby/icon.png"), b"image").unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::write(root.join("assets/ruby/icon.png"), b"\0asm").unwrap();
        assert!(
            Package::read(&root)
                .err()
                .unwrap()
                .contains("unsupported executable")
        );
        std::fs::remove_file(root.join("assets/ruby/icon.png")).unwrap();
        std::fs::create_dir_all(root.join("assets/minecraft/lang")).unwrap();
        std::fs::write(root.join("assets/minecraft/lang/en_us.json"), "{}").unwrap();
        let package = Package::read(&root).unwrap();
        assert!(crate::Content::read(&root, &package.manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_links_and_executable_modes_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ruby.hmod");
        for link in [false, true] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            // Made by a Unix tool on every OS: the writer's default on Windows is DOS attributes,
            // which hold no mode to refuse.
            let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            if link {
                zip.add_symlink("assets/ruby/link", "missing", options)
                    .unwrap();
            } else {
                zip.start_file("assets/ruby/program", options.unix_permissions(0o755))
                    .unwrap();
                zip.write_all(b"ordinary bytes").unwrap();
            }
            zip.finish().unwrap();
            let error = Package::read(&path).err().unwrap();
            assert!(
                error.contains(if link {
                    "unsupported file type"
                } else {
                    "unsupported executable"
                }),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_executable_modes_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("horizon-mods-folder-kinds-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        let file = dir.join("assets/ruby/program");
        std::fs::write(&file, b"ordinary bytes").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Package::read(&dir).err().unwrap();
        assert!(error.contains("unsupported executable"), "{error}");
        std::fs::remove_file(&file).unwrap();
        for name in ["assets/ruby/mod.wasm", "assets/ruby/nested/link"] {
            let link = dir.join(name);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink("missing", &link).unwrap();
            let error = Package::read(&dir).err().unwrap();
            assert!(error.contains("unsupported file type"), "{error}");
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        symlink("missing", dir.join("assets")).unwrap();
        assert!(
            Package::read(&dir)
                .err()
                .unwrap()
                .contains("unsupported file type")
        );
        let link = root.join("linked-folder");
        symlink(&dir, &link).unwrap();
        let error = Package::read(&link).err().unwrap();
        assert!(error.contains("unsupported file type"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
