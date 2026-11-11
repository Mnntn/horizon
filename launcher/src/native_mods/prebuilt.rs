//! The prebuilt game (#589): mods that need nothing but the mod API compile against a downloaded
//! build of the game (`tools/prebuilt_game.py`, released daily by `prebuilt-game.yml`) with rustc
//! alone instead of building the game from source. The route is chosen before
//! anything runs: a mod set the prebuilt game can't serve, or a revision with no prebuilt
//! release, builds from source, and the build log says why. A prebuilt build that fails is an
//! error like any other build's, never retried from source.
use super::{api, dependencies, internals, Target};
use crate::i18n::{tr, trf};
use crate::rust_toolchain::{self, Toolchain};
use horizon_mod_package::index::native;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const IDENTITY: &str = "horizon-prebuilt.json";
pub(super) const TEMPLATE: &str = "native_main.rs.template";
const SUMS: &str = "SHA256SUMS";
/// The optimization level of the `release` profile the prebuilt game is built with: the mods' code
/// matches the game's.
const OPT_LEVEL: &str = "opt-level=3";
/// The archive's largest plausible unpacked size: a corrupt or hostile archive stops there.
const UNPACKED_LIMIT: u64 = 8 << 30;

/// Why a build compiles the game from source instead of using the prebuilt game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceReason {
    /// A mod has a `[[native.mixin]]`, bridge, state field or support module, which change game code.
    Mixins { mod_id: String },
    /// A mod has a build script, which the prebuilt route doesn't run.
    BuildScript { mod_id: String },
    /// A mod declares Cargo features, or asks the mod API for features of its own.
    Features { mod_id: String },
    /// No prebuilt game for this computer.
    Host,
    /// This release has an older SDK whose registry patch ownership is incomplete.
    IncompatibleSdk { archive: String },
    /// The prebuilt game's release couldn't be downloaded.
    Download { why: String },
    /// No prebuilt game was released for this revision and target.
    NoRelease { revision: String },
}

impl std::fmt::Display for SourceReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Mixins { mod_id } => trf("{mod}: mixins compile game code", &[("mod", mod_id)]),
            Self::BuildScript { mod_id } => trf(
                "{mod} has a build script, so the game builds from source",
                &[("mod", mod_id)],
            ),
            Self::Features { mod_id } => trf(
                "{mod} uses Cargo features, so the game builds from source",
                &[("mod", mod_id)],
            ),
            Self::IncompatibleSdk { archive } => trf(
                "SDK {archive} lacks game registry patches; install a matched SDK v5 release",
                &[("archive", archive)],
            ),
            Self::Host => {
                tr("no prebuilt game for this computer, so the game builds from source").into()
            }
            Self::Download { why } => trf(
                "The prebuilt game couldn't be downloaded ({why}). Try again.",
                &[("why", why)],
            ),
            Self::NoRelease { revision } => trf(
                "no prebuilt game for {revision}, so the game builds from source",
                &[("revision", &revision[..revision.len().min(10)])],
            ),
        };
        f.write_str(&text)
    }
}

impl SourceReason {
    pub(super) fn forces_source(&self) -> bool {
        !matches!(self, Self::Download { .. } | Self::IncompatibleSdk { .. })
    }
}

/// The first structural reason these projects cannot use the prebuilt game.
pub(super) fn refusal(projects: &[api::Project<'_>]) -> Option<SourceReason> {
    projects.iter().find_map(project_refusal)
}

/// Refuse injections without compiled slots before selecting an artifact.
pub(super) fn source_refusal(
    game: &Path,
    projects: &[api::Project<'_>],
) -> Result<Option<SourceReason>, String> {
    let catalog =
        internals::Catalog::read(game)?.with_mods(projects.iter().map(|project| project.bundle))?;
    for (_, route) in slot_routes(&catalog, projects) {
        match route {
            super::Route::Game { reason } => return Ok(Some(reason)),
            super::Route::Invalid { reason } => return Err(reason),
            super::Route::Prebuilt => {}
        }
    }
    Ok(None)
}

/// One slot attachment decision per project, including named refusals. Cargo restrictions are
/// reported separately by `project_refusal`; both consumers use the same slot router here.
pub(super) fn slot_routes(
    catalog: &internals::Catalog,
    projects: &[api::Project<'_>],
) -> Vec<(String, super::Route)> {
    projects
        .iter()
        .map(|project| {
            let manifest = &project.bundle.manifest;
            let route = match super::slots::routes(catalog, manifest) {
                Ok(_) => super::Route::Prebuilt,
                Err(error) => super::Route::Invalid {
                    reason: error.to_string(),
                },
            };
            (manifest.id.clone(), route)
        })
        .collect()
}

/// Why `project` alone can't build against the prebuilt game, or `None` when it can.
pub(super) fn project_refusal(project: &api::Project<'_>) -> Option<SourceReason> {
    {
        let mod_id = || project.bundle.manifest.id.clone();
        if project.bundle.needs_mixins() {
            return Some(SourceReason::Mixins { mod_id: mod_id() });
        }
        let cargo = &project.cargo;
        let package = cargo.get("package");
        let has_script = package.and_then(|p| p.get("build")).is_some()
            || project.bundle.dir.join("build.rs").is_file()
            || cargo
                .get("build-dependencies")
                .and_then(toml::Value::as_table)
                .is_some_and(|t| !t.is_empty());
        if has_script {
            return Some(SourceReason::BuildScript { mod_id: mod_id() });
        }
        if cargo
            .get("features")
            .and_then(toml::Value::as_table)
            .is_some_and(|t| !t.is_empty())
        {
            return Some(SourceReason::Features { mod_id: mod_id() });
        }
        for (_, dependencies) in project.dependency_tables() {
            for (key, value) in dependencies {
                let krate = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                if !api::PACKAGES.iter().any(|(package, _)| *package == krate) {
                    continue;
                }
                let own_features = value
                    .get("features")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|f| !f.is_empty())
                    || value.get("default-features").and_then(toml::Value::as_bool) == Some(false);
                if own_features {
                    return Some(SourceReason::Features { mod_id: mod_id() });
                }
            }
        }
        None
    }
}

/// The prebuilt game for `revision` and `target`, downloaded and checked once, then kept in the
/// launcher's data (`downloading` is told before a download starts); `Err(reason)` inside when
/// there is none to use, or its download failed. An archive that arrives must match the release's
/// `SHA256SUMS` before it is unpacked, and its identity this revision, this computer and the
/// launcher's Rust: a mismatch is an error, never a source build.
pub(super) fn fetch(
    revision: &str,
    target: Target,
    downloading: impl FnOnce(),
) -> Result<Result<PathBuf, SourceReason>, String> {
    let Some(host) = rust_toolchain::host() else {
        return Ok(Err(SourceReason::Host));
    };
    let name = native::prebuilt_archive(host.target, matches!(target, Target::Server));
    let dir = cache_dir(revision, &name);
    if dir.join(IDENTITY).is_file() {
        crate::storage::mark_used(&dir).map_err(super::cache_error)?;
        return Ok(Ok(dir));
    }
    let release = release_url(revision);
    let download = |why: String| Ok(Err(SourceReason::Download { why }));
    let expected = match release_sum(revision, &name) {
        Ok(sum) => sum,
        Err(reason) => return Ok(Err(reason)),
    };
    let parent = dir.parent().expect("under prebuilt/");
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let partial = parent.join(format!(".{name}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    downloading();
    let file = partial.join(&name);
    let mut body = match crate::backend::get(&format!("{release}/{name}")) {
        Ok(response) => response.into_reader(),
        Err(why) => return download(why),
    };
    let mut out = fs::File::create(&file).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = match body.read(&mut buffer) {
            Ok(read) => read,
            Err(e) => {
                let _ = fs::remove_dir_all(&partial);
                return download(e.to_string());
            }
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut out, &buffer[..read]).map_err(|e| e.to_string())?;
    }
    drop(out);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        let _ = fs::remove_dir_all(&partial);
        return Err(trf(
            "The prebuilt game {name} doesn't match its release's checksum. Try again.",
            &[("name", &name)],
        ));
    }
    let _publication = crate::run_leases::installation().map_err(super::cache_error)?;
    let unpacked = partial.join("game");
    // `unpack_in` writes inside a folder that exists.
    fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(fs::File::open(&file).map_err(|e| e.to_string())?)
        .take(UNPACKED_LIMIT);
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        if kind.is_file() || kind.is_dir() {
            entry
                .unpack_in(&unpacked)
                .map_err(|e| format!("Could not unpack the prebuilt game: {e}"))?;
        }
    }
    check_identity(&unpacked, revision, host.target, target)?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&unpacked, &dir).map_err(|e| e.to_string())?;
    crate::run_leases::publish(&dir).map_err(super::cache_error)?;
    let _ = fs::remove_dir_all(&partial);
    crate::storage::mark_used(&dir).map_err(super::cache_error)?;
    Ok(Ok(dir))
}

/// Keep retryable failures distinct from structural reasons to compile the game locally.
pub(super) fn or_source(game: Result<PathBuf, SourceReason>) -> Result<Option<PathBuf>, String> {
    match game {
        Ok(game) => Ok(Some(game)),
        Err(reason) if reason.forces_source() => {
            super::log_warning(&reason.to_string());
            Ok(None)
        }
        Err(reason) => Err(reason.to_string()),
    }
}

fn release_url(revision: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}{revision}",
        native::PREBUILT_REPO,
        native::PREBUILT_TAG
    )
}

fn release_sum(revision: &str, name: &str) -> Result<String, SourceReason> {
    let missing = || SourceReason::NoRelease {
        revision: revision.to_owned(),
    };
    let download = |why: String| SourceReason::Download { why };
    let sums = crate::backend::get_unless_missing(&format!("{}/{SUMS}", release_url(revision)))
        .map_err(download)?
        .ok_or_else(missing)?;
    let mut text = String::new();
    sums.into_reader()
        .read_to_string(&mut text)
        .map_err(|error| download(error.to_string()))?;
    if expected_sum(&text, name).is_none() {
        let old = name.replace("prebuilt-v5-", "prebuilt-v4-");
        if expected_sum(&text, &old).is_some() {
            return Err(SourceReason::IncompatibleSdk { archive: old });
        }
    }
    expected_sum(&text, name).ok_or_else(missing)
}

/// Checks release metadata without downloading the prebuilt game. Run only on a worker.
/// Download failures are retryable, never permission to rebuild the game from source.
pub fn prebuilt_availability(revision: &str, target: Target) -> Option<SourceReason> {
    let Some(host) = rust_toolchain::host() else {
        return Some(SourceReason::Host);
    };
    let name = native::prebuilt_archive(host.target, matches!(target, Target::Server));
    let cached = cache_dir(revision, &name);
    if cached.join(IDENTITY).is_file() {
        return None;
    }
    release_sum(revision, &name).err()
}

fn cache_dir(revision: &str, name: &str) -> PathBuf {
    super::root()
        .join("prebuilt")
        .join(format!("{revision}-{}", name.trim_end_matches(".tar.gz")))
}

/// Read checked SDK metadata; the caller owns consent and the native-cache lease.
pub(super) fn planning_catalog(
    lease: &crate::native_cache::Cache,
    revision: &str,
) -> Result<internals::Catalog, SourceReason> {
    let host = rust_toolchain::host().ok_or(SourceReason::Host)?;
    let name = native::prebuilt_archive(host.target, false);
    let game = cache_dir(revision, &name);
    if game.join(IDENTITY).is_file() {
        if let Ok(catalog) = internals::Catalog::read(&game) {
            crate::storage::mark_used(&game).map_err(|error| SourceReason::Download {
                why: super::cache_error(error),
            })?;
            return Ok(catalog);
        }
    }
    let expected = release_sum(revision, &name)?;
    let download = |why| SourceReason::Download { why };
    let unit = format!("catalog-{revision}-{}", host.target);
    let cache = super::root().join("internals-sync").join(&unit);
    let directory = lease
        .directory(&["internals-sync", &unit])
        .map_err(|error| download(super::cache_error(error)))?;
    crate::storage::mark_used(&cache).map_err(|error| download(super::cache_error(error)))?;
    catalog_pair(&directory, &cache, &name, &expected, || {
        crate::backend::get(&format!("{}/{name}", release_url(revision)))
            .map_err(download)
            .map(|response| response.into_reader())
    })
}

fn catalog_pair<R: Read>(
    directory: &cap_std::fs::Dir,
    path: &Path,
    name: &str,
    expected: &str,
    fetch: impl FnOnce() -> Result<R, SourceReason>,
) -> Result<internals::Catalog, SourceReason> {
    let download = |why| SourceReason::Download { why };
    if directory.read_to_string("archive.sha256").ok().as_deref() == Some(expected) {
        if let Ok(catalog) = internals::Catalog::read(path) {
            return Ok(catalog);
        }
    }
    let reader = fetch()?;
    // A failed second rename must never leave an old marker authorizing a mixed pair.
    match directory.remove_file("archive.sha256") {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(download(error.to_string())),
    }
    horizon_mod_package::internals::cache_catalogs(directory, name, reader, expected)
        .map_err(|error| download(error.to_string()))?;
    let catalog = internals::Catalog::read(path).map_err(download)?;
    directory
        .write("archive.sha256", expected)
        .map_err(|error| download(error.to_string()))?;
    Ok(catalog)
}

/// `name`'s sum in a `sha256sum` listing.
fn expected_sum(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| sum.to_ascii_lowercase())
    })
}

#[derive(serde::Deserialize)]
pub(super) struct Identity {
    commit: String,
    rustc: String,
    target: String,
    side: String,
    pub(super) link_args: Vec<String>,
    #[serde(default)]
    pub(super) registry_lock: Option<String>,
    #[serde(default)]
    pub(super) registry_patches: Option<toml::Table>,
    #[serde(default)]
    pub(super) native_links: Option<Vec<super::registry_inventory::NativeLink>>,
    #[serde(default)]
    facade_crates: std::collections::BTreeSet<String>,
}

pub(super) fn identity(dir: &Path) -> Result<Identity, String> {
    let path = dir.join(IDENTITY);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[derive(serde::Deserialize)]
    struct Format {
        #[serde(default)]
        format_version: u32,
    }
    let format: Format =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    match format.format_version {
        5 => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        0..=4 => Err(trf("SDK format {version} is too old; install a newer SDK (format 5)", &[("version", &format.format_version.to_string())])),
        version => Err(trf("SDK format {version} is newer than this launcher's supported format 5; update the launcher", &[("version", &version.to_string())])),
    }
}

/// Whether the unpacked game in `dir` is the one asked for: built from `revision` for `host` and
/// `target`'s side by the launcher's Rust, whose compiler is the only one that reads its metadata.
fn check_identity(dir: &Path, revision: &str, host: &str, target: Target) -> Result<(), String> {
    let found = identity(dir)?;
    let release = format!("release: {}", rust_toolchain::VERSION);
    let matches = found.commit == revision
        && found.target == host
        && found.side == target.name()
        && found.rustc.lines().any(|line| line == release);
    if matches {
        Ok(())
    } else {
        Err(trf(
            "The prebuilt game for {revision} was made for another build ({commit}, {target}, {side}, {rustc}).",
            &[
                ("revision", revision),
                ("commit", &found.commit),
                ("target", &found.target),
                ("side", &found.side),
                ("rustc", found.rustc.lines().next().unwrap_or_default()),
            ],
        ))
    }
}

/// The library file a crate named `krate` left in `deps`: its metadata, or its own dylib.
pub(super) fn library(deps: &Path, krate: &str) -> Result<PathBuf, String> {
    // A cached target can still contain metadata from an earlier static build. Prefer the dylib:
    // callers installing runtime libraries need its code, not a stale metadata-only artifact.
    let dylib = ["so", "dll", "dylib"]
        .iter()
        .flat_map(|ext| [format!("lib{krate}.{ext}"), format!("{krate}.{ext}")])
        .map(|name| deps.join(name))
        .find(|path| path.is_file());
    if let Some(path) = dylib {
        return Ok(path);
    }
    let prefix = format!("lib{krate}-");
    let candidates: Vec<_> = fs::read_dir(deps)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".rmeta")
        })
        .collect();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(trf(
            "The prebuilt game has no {crate}.",
            &[("crate", krate)],
        )),
        _ => Err(facade_refused(krate)),
    }
}

fn facade_refused(krate: &str) -> String {
    trf(
        "The prebuilt game cannot expose {crate}: it is build-only or has ambiguous metadata.",
        &[("crate", krate)],
    )
}

pub(super) fn facade_library(deps: &Path, krate: &str, sdk: &Identity) -> Result<PathBuf, String> {
    if !sdk.facade_crates.contains(krate) {
        return Err(facade_refused(krate));
    }
    library(deps, krate)
}

/// Rust's standard library as a dynamic library: what the prebuilt game and its executable load.
pub(super) fn standard_library(rust: &Toolchain) -> Result<PathBuf, String> {
    let dir = rust
        .target_libdir()
        .ok_or("no Rust standard library for this computer")?;
    fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            (name.starts_with("libstd-") || name.starts_with("std-"))
                && [".so", ".dll", ".dylib"]
                    .iter()
                    .any(|ext| name.ends_with(ext))
        })
        .ok_or_else(|| format!("{}: no dynamic standard library", dir.display()))
}

/// rustc with the prebuilt game's settings, its libraries and `out`'s on the search path, and the
/// native libraries the game's build scripts link (`native/`: the `windows` crates' import
/// libraries).
fn rustc(rust: &Toolchain, game: &Path, out: &Path, edition: &str) -> Command {
    let mut cmd = rust.command(&rust.rustc());
    rust_toolchain::low_priority(&mut cmd);
    cmd.args([
        "--edition",
        edition,
        "-C",
        "prefer-dynamic",
        "-C",
        OPT_LEVEL,
    ])
    .arg("-L")
    .arg(format!("dependency={}", game.join("deps").display()))
    .arg("-L")
    .arg(format!("dependency={}", game.join("host").display()))
    .arg("-L")
    .arg(format!("native={}", game.join("native").display()))
    .arg("-L")
    .arg(format!("dependency={}", out.display()));
    cmd
}

/// Compiles the mods (`resolution`'s order, each with its installed mod dependencies) and the
/// launcher's executable against the prebuilt game in `game`, into `out`. Returns the executable
/// and the dynamic libraries it loads, to be installed beside it.
// Unlike the source workspace, this route invokes rustc with only selected --extern
// libraries; it does not resolve or compile arbitrary transitive Cargo dependencies.
pub(super) fn compile(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    consent: Option<&dyn Fn(&super::RegistryInventory) -> bool>,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    if projects.iter().any(|project| {
        project.dependency_tables().any(|(_, dependencies)| {
            dependencies.iter().any(|(key, value)| {
                let name = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                !api::PACKAGES.iter().any(|(package, _)| *package == name)
            })
        })
    }) {
        return super::registry_compile::compile(
            rust,
            game,
            out,
            projects,
            declared,
            resolution,
            super::registry_compile::Options {
                consent,
                adapter: &std::env::current_exe().map_err(|e| e.to_string())?,
            },
        );
    }
    let catalog =
        internals::Catalog::read(game)?.with_mods(projects.iter().map(|project| project.bundle))?;
    let generated = catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    for project in projects {
        catalog.log_raw(&project.bundle.manifest)?;
    }
    let _ = fs::remove_dir_all(out);
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let deps = game.join("deps");
    let sdk = identity(game)?;
    let api_library = library(&deps, "mod_api")?;
    let mut libraries = std::collections::HashMap::new();
    for (package, name) in api::PACKAGES {
        if package != api::INTERNALS_PACKAGE {
            libraries.insert(name, library(&deps, name)?);
        }
    }
    let game_library = library(&deps, "horizon_client")?;
    let mut facades = std::collections::HashMap::new();
    let mut rlibs: std::collections::HashMap<&str, PathBuf> = std::collections::HashMap::new();
    for &index in &resolution.order {
        let project = &projects[index];
        let id = project.bundle.manifest.id.as_str();
        let edition = project
            .cargo
            .get("package")
            .and_then(|p| p.get("edition"))
            .and_then(toml::Value::as_str)
            .unwrap_or("2015");
        let lib = project.cargo.get("lib");
        let krate = lib
            .and_then(|l| l.get("name"))
            .and_then(toml::Value::as_str)
            .map_or_else(|| project.package.replace('-', "_"), str::to_owned);
        // Compiled from a staged copy: the crate with its own points (#2339).
        let staged = out.join("staged").join(id);
        super::copy_bundle(project.bundle, &staged)?;
        super::mod_points::stage(&staged, &catalog.wire)?;
        let source = staged.join(
            lib.and_then(|l| l.get("path"))
                .and_then(toml::Value::as_str)
                .unwrap_or("src/lib.rs"),
        );
        let api_dependencies = project.api_dependencies();
        if internals::needed(&project.bundle.manifest)
            || api_dependencies
                .iter()
                .any(|(_, package)| *package == "internals")
        {
            let source = out.join(format!("internals_{id}.rs"));
            let facade = &generated[id];
            fs::write(&source, &facade.source).map_err(|e| e.to_string())?;
            let rlib = out.join(format!("libinternals_{id}.rlib"));
            let mut cmd = rustc(rust, game, out, "2024");
            cmd.args(["--crate-type", "rlib", "--crate-name", "internals"])
                .arg("-C")
                .arg(format!("metadata={id}"))
                .arg("--extern")
                .arg(format!("horizon_client={}", game_library.display()))
                .arg("--extern")
                .arg(format!("mod_points={}", libraries["mod_points"].display()))
                .arg("-o")
                .arg(&rlib)
                .arg(source);
            for krate in &facade.crates {
                if krate != "horizon_client" {
                    cmd.arg("--extern").arg(format!(
                        "{krate}={}",
                        facade_library(&deps, krate, &sdk)?.display()
                    ));
                }
            }
            super::run(
                cmd,
                &trf("Compiling {mod}'s declared internals", &[("mod", id)]),
                |_| {},
            )?;
            facades.insert(id, rlib);
        }
        let mut cmd = rustc(rust, game, out, edition);
        cmd.args(["--crate-type", "rlib", "--crate-name", &krate])
            .arg("-C")
            .arg(format!("metadata={id}"));
        for (name, package) in api_dependencies {
            let library = if package == "internals" {
                &facades[id]
            } else {
                &libraries[package]
            };
            cmd.arg("--extern")
                .arg(format!("{name}={}", library.display()));
        }
        for dependency in &declared[index] {
            let Some(rlib) = rlibs.get(dependency.id.as_str()) else {
                continue;
            };
            cmd.arg("--extern").arg(format!(
                "{}={}",
                dependency.id.replace('-', "_"),
                rlib.display()
            ));
            if dependency.optional {
                cmd.arg("--cfg")
                    .arg(format!("feature=\"{}\"", dependency.id));
            }
        }
        cmd.arg("--out-dir").arg(out).arg(&source);
        super::run(cmd, &format!("Compiling {id}"), |_| {})?;
        rlibs.insert(id, out.join(format!("lib{krate}.rlib")));
    }
    let template = api::template(&game.join(TEMPLATE))?;
    let manifests: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| &projects[i].bundle.manifest)
        .collect();
    let main = out.join("main.rs");
    fs::write(&main, api::main_rs(&template, &manifests, &catalog)).map_err(|e| e.to_string())?;
    let binary = out.join(crate::backend::CLIENT);
    let mut cmd = rustc(rust, game, out, "2024");
    cmd.args(["--crate-type", "bin", "--crate-name", "horizon_native_game"])
        .arg("--extern")
        .arg(format!("horizon_client={}", game_library.display()))
        .arg("--extern")
        .arg(format!("mod_api={}", api_library.display()))
        .arg("--extern")
        .arg(format!("mod_points={}", libraries["mod_points"].display()))
        .arg("--extern")
        .arg(format!(
            "horizon_mod_package={}",
            library(&deps, "horizon_mod_package")?.display()
        ));
    if template.contains("horizon_mod_host::") {
        cmd.arg("--extern").arg(format!(
            "horizon_mod_host={}",
            library(&deps, "horizon_mod_host")?.display()
        ));
    }
    for (position, &index) in resolution.order.iter().enumerate() {
        let id = projects[index].bundle.manifest.id.as_str();
        cmd.arg("--extern")
            .arg(format!("mod_{position}={}", rlibs[id].display()));
        if let Some(facade) = facades.get(id) {
            cmd.arg("--extern")
                .arg(format!("mod_{position}_internals={}", facade.display()));
        }
    }
    // The game's libraries load from beside the executable.
    let origin = match std::env::consts::OS {
        "linux" => Some("-Wl,-rpath,$ORIGIN"),
        "macos" => Some("-Wl,-rpath,@executable_path"),
        _ => None,
    };
    for arg in origin.into_iter().map(str::to_owned).chain(sdk.link_args) {
        cmd.arg("-C").arg(format!("link-arg={arg}"));
    }
    cmd.arg("-o").arg(&binary).arg(&main);
    super::run(cmd, "Linking the game", |_| {})?;
    let mut runtime = game_dylibs(&deps)?;
    runtime.push(standard_library(rust)?);
    Ok((binary, runtime))
}

/// The game's own dynamic libraries, which the executable loads from beside itself: the game and
/// the wasm mod runtime it links (`tools/prebuilt_game.py` checks the archive holds both).
pub(super) fn game_dylibs(deps: &Path) -> Result<Vec<PathBuf>, String> {
    let top = if library(deps, "horizon_mod_host").is_ok() {
        "horizon_mod_host"
    } else {
        "horizon_client"
    };
    let crates = [top, "horizon_mod_wasm"];
    crates
        .into_iter()
        .map(|krate| {
            let path = library(deps, krate)?;
            if path.extension().is_some_and(|ext| ext != "rmeta") {
                Ok(path)
            } else {
                Err(trf(
                    "The prebuilt game has no {crate}.",
                    &[("crate", krate)],
                ))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_identity_refuses_old_and_unknown_newer_formats_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for version in [0, 4, 6] {
            let text = serde_json::json!({"format_version":version});
            std::fs::write(dir.path().join(IDENTITY), text.to_string()).unwrap();
            let error = identity(dir.path()).err().unwrap();
            assert!(error.contains(&format!("SDK format {version}")), "{error}");
            assert!(
                error.contains(if version < 5 {
                    "install a newer SDK"
                } else {
                    "update the launcher"
                }),
                "{error}"
            );
        }
    }

    #[test]
    fn older_sdk_layout_is_a_named_refusal_not_source_permission() {
        let reason = SourceReason::IncompatibleSdk {
            archive: "horizon-prebuilt-v4-fixture.tar.gz".into(),
        };
        assert!(!reason.forces_source());
        let error = or_source(Err(reason)).unwrap_err();
        assert!(
            error.contains("horizon-prebuilt-v4-fixture.tar.gz"),
            "{error}"
        );
        assert!(error.contains("SDK v5"), "{error}");
    }

    #[test]
    fn facades_refuse_build_only_crates_and_ambiguous_runtime_metadata() {
        let deps = super::super::tests::dir("prebuilt-facade-eligibility");
        for name in [
            "libpoint_shape-host.rmeta",
            "libpoint_shape-target.rmeta",
            "libbuild_tool-one.rmeta",
            "libhorizon_core-one.rmeta",
        ] {
            fs::write(deps.join(name), b"").unwrap();
        }
        let mut sdk: Identity = serde_json::from_value(serde_json::json!({
            "format_version": 5, "commit": "test", "rustc": "test", "target": "test", "side": "client",
            "link_args": [], "facade_crates": ["horizon_core"],
        }))
        .unwrap();
        assert_eq!(
            facade_library(&deps, "horizon_core", &sdk).unwrap(),
            deps.join("libhorizon_core-one.rmeta")
        );
        for krate in ["point_shape", "build_tool"] {
            let refusal = facade_library(&deps, krate, &sdk).unwrap_err();
            assert!(refusal.contains(krate), "{refusal}");
            assert!(
                refusal.contains("build-only or has ambiguous metadata"),
                "{refusal}"
            );
        }
        // Even an incorrect allowlist cannot make lookup select one of two variants.
        sdk.facade_crates.insert("point_shape".into());
        assert!(facade_library(&deps, "point_shape", &sdk).is_err());
        fs::remove_dir_all(deps).unwrap();
    }

    #[test]
    fn the_executable_carries_the_game_and_its_wasm_runtime() {
        let deps = super::super::tests::dir("prebuilt-dylibs");
        let ext = std::env::consts::DLL_EXTENSION;
        fs::write(deps.join(format!("libhorizon_client.{ext}")), b"").unwrap();
        fs::write(deps.join("libhorizon_mod_wasm-0123.rmeta"), b"").unwrap();
        // Metadata alone has no code to load: the archive is incomplete.
        assert!(game_dylibs(&deps).is_err());
        fs::write(deps.join(format!("libhorizon_mod_wasm.{ext}")), b"").unwrap();
        assert_eq!(game_dylibs(&deps).unwrap().len(), 2);
        fs::write(deps.join(format!("libhorizon_mod_host.{ext}")), b"").unwrap();
        let names: Vec<_> = game_dylibs(&deps)
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                format!("libhorizon_mod_host.{ext}"),
                format!("libhorizon_mod_wasm.{ext}")
            ]
        );
        fs::remove_dir_all(deps).unwrap();
    }

    #[test]
    fn injects_without_slots_are_invalid_and_never_plan_source() {
        use serde_json::json;
        let root = super::super::tests::dir("slot-planning");
        let mut bundles = Vec::new();
        for (id, mut entry) in [
            (
                "slotted",
                json!({"at":"Example::slotted","kind":"head","access":"internals"}),
            ),
            (
                "private_head",
                json!({"at":"Example::private","kind":"head","access":"internals"}),
            ),
            (
                "unlisted_call",
                json!({"at":"Example::calls","kind":"before_call","access":"internals","call":"unlisted","nth":0,"ctx":"context"}),
            ),
            (
                "api_refusal",
                json!({"at":"Example::private","kind":"head","access":"api"}),
            ),
        ] {
            let dir = root.join(id);
            super::super::tests::write_bundle(&dir, id, "");
            fs::write(
                dir.join("Cargo.toml"),
                "[package]\nname='mod_project'\n[dependencies]\nhorizon-mod-api='0.1'\n",
            )
            .unwrap();
            let mut bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            entry["id"] = json!("probe");
            entry["hash"] = json!("signature");
            entry["with"] = json!("handler");
            bundle.manifest.inject = vec![serde_json::from_value(entry).unwrap()];
            bundles.push(bundle);
        }
        let catalog = internals::Catalog {
            internals: ["Example::slotted", "Example::private", "Example::calls"].into_iter().map(|path| serde_json::from_value(json!({"path":path,"kind":"method","signature":"fn tick()","hash":"signature"})).unwrap()).collect(),
            points: serde_json::from_value(json!([{"name":"tick","target":"Example::slotted","raw":true,"at_return":true,"listener":"Head","return_listener":"Return"}])).unwrap(),
            ..Default::default()
        };
        let projects: Vec<_> = bundles
            .iter()
            .map(|bundle| api::Project::read(bundle).unwrap())
            .collect();
        let routes = slot_routes(&catalog, &projects);
        let classes: Vec<_> = routes
            .iter()
            .map(|(_, route)| match route {
                super::super::Route::Prebuilt => "prebuilt",
                super::super::Route::Game { .. } => "source",
                super::super::Route::Invalid { .. } => "invalid",
            })
            .collect();
        assert_eq!(classes, ["prebuilt", "invalid", "invalid", "invalid"]);
        let causes: Vec<_> = routes
            .iter()
            .filter(|(_, route)| matches!(route, super::super::Route::Game { .. }))
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(causes.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_partial_catalog_pair_cannot_reuse_its_previous_marker() {
        use super::super::{inject::POINTS, internals::INTERNALS};
        fn packed(internals: &str, points: &str) -> Vec<u8> {
            let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::fast(),
            ));
            for (name, text) in [("internals.json", internals), ("points.json", points)] {
                let mut header = tar::Header::new_gnu();
                header.set_size(text.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar.append_data(&mut header, name, text.as_bytes()).unwrap();
            }
            tar.into_inner().unwrap().finish().unwrap()
        }
        let root = super::super::tests::dir("catalog-pair");
        let lease = crate::native_cache::Cache::acquire(&root).unwrap();
        let directory = lease.directory(&["internals-sync", "catalog"]).unwrap();
        let path = root.join("native/internals-sync/catalog");
        let old = packed("[]", r#"{"points":[],"sites":[]}"#);
        let old_hash = format!("{:x}", Sha256::digest(&old));
        catalog_pair(&directory, &path, "game", &old_hash, || Ok(old.as_slice())).unwrap();
        directory.remove_file(POINTS).unwrap();
        let requests = std::cell::Cell::new(0);
        catalog_pair(&directory, &path, "game", &old_hash, || {
            requests.set(requests.get() + 1);
            Ok(old.as_slice())
        })
        .unwrap();
        assert_eq!(requests.get(), 1);
        directory.remove_file(POINTS).unwrap();
        directory.create_dir(POINTS).unwrap();
        let new = packed(
            r#"[{"path":"Changed","kind":"struct","signature":"struct Changed","hash":"new"}]"#,
            r#"{"points":[],"sites":[]}"#,
        );
        let new_hash = format!("{:x}", Sha256::digest(&new));
        assert!(catalog_pair(&directory, &path, "game", &new_hash, || Ok(new.as_slice())).is_err());
        assert!(!directory.try_exists("archive.sha256").unwrap());
        assert!(directory
            .read_to_string(INTERNALS)
            .unwrap()
            .contains("Changed"));
        directory.remove_dir(POINTS).unwrap();
        directory
            .write(POINTS, r#"{"points":[],"sites":[]}"#)
            .unwrap();
        assert!(
            catalog_pair(&directory, &path, "game", &old_hash, || Ok(old.as_slice()))
                .unwrap()
                .internals
                .is_empty()
        );
        let invalid = packed("[]", r#"{"points":"not a list","sites":[]}"#);
        let invalid_hash = format!("{:x}", Sha256::digest(&invalid));
        assert!(catalog_pair(&directory, &path, "game", &invalid_hash, || {
            Ok(invalid.as_slice())
        })
        .is_err());
        assert!(!directory.try_exists("archive.sha256").unwrap());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("internals-sync/catalog", root.join("native/redirect"))
                .unwrap();
            assert!(lease.directory(&["redirect", "child"]).is_err());
            assert!(!path.join("child").exists());
        }
        drop(directory);
        drop(lease);
        fs::remove_dir_all(root).unwrap();
    }

    /// What [`refusal`] says of one mod whose Cargo.toml is `cargo`.
    fn refusal_of(cargo: &str) -> Option<SourceReason> {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname='example-mod'\nversion='1.0.0'\nedition='2024'\n{cargo}\n"),
        )
        .unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        refusal(&[api::Project::read(&bundle).unwrap()])
    }

    #[test]
    fn only_the_exact_build_of_the_prebuilt_game_is_accepted() {
        let dir = super::super::tests::dir("prebuilt-exact-build");
        let check = |commit: &str, rustc: &str, target: &str, side: &str| {
            let identity = serde_json::json!({
                "format_version": 5, "commit": commit, "rustc": rustc, "target": target, "side": side, "link_args": [],
            });
            fs::write(dir.join(IDENTITY), identity.to_string()).unwrap();
            check_identity(&dir, "abc123", "x86_64-pc-windows-gnu", Target::Client)
        };
        let rustc = format!("rustc x\nrelease: {}\nhost: y", rust_toolchain::VERSION);
        let host = "x86_64-pc-windows-gnu";
        assert_eq!(check("abc123", &rustc, host, "client"), Ok(()));
        assert!(check("abc124", &rustc, host, "client").is_err());
        assert!(check("abc123", &rustc, "x86_64-unknown-linux-gnu", "client").is_err());
        assert!(check("abc123", &rustc, host, "server").is_err());
        assert!(check("abc123", "rustc x\nrelease: 0.0.1", host, "client").is_err());
        let longer = format!("release: {}.1", rust_toolchain::VERSION);
        assert!(check("abc123", &longer, host, "client").is_err());
        fs::remove_file(dir.join(IDENTITY)).unwrap();
        assert!(check_identity(&dir, "abc123", host, Target::Client).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_download_never_selects_a_source_build() {
        assert!(or_source(Err(SourceReason::Download {
            why: "offline".into()
        }))
        .is_err());
        assert_eq!(
            or_source(Ok(PathBuf::from("cached-game"))).unwrap(),
            Some(PathBuf::from("cached-game"))
        );
    }

    #[test]
    fn missing_host_or_release_still_allows_an_explicit_source_build() {
        let _guard = super::super::TEST_STATE.lock().unwrap();
        for reason in [
            SourceReason::Host,
            SourceReason::NoRelease {
                revision: "deadbeef".into(),
            },
        ] {
            assert!(or_source(Err(reason)).unwrap().is_none());
        }
    }

    #[test]
    fn mods_on_the_mod_api_alone_take_the_prebuilt_game() {
        assert_eq!(refusal_of("[dependencies]\nhorizon-mod-api='0.1'"), None);
        assert_eq!(
            refusal_of("[dependencies.api]\npackage='horizon-mod-api'\nversion='0.1'"),
            None
        );
    }

    #[test]
    fn registry_dependencies_do_not_rebuild_the_game() {
        for cargo in [
            "[dependencies]\nhorizon-mod-api='0.1'\nserde='1'",
            "[dependencies]\nhorizon-mod-api='0.1'\n[target.'cfg(unix)'.dependencies]\nlibc='0.2'",
        ] {
            assert_eq!(refusal_of(cargo), None);
        }
    }

    #[test]
    fn anything_the_prebuilt_game_cant_build_names_why() {
        let id = || "example".to_owned();
        for (cargo, reason) in [
            (
                "[dependencies]\nhorizon-mod-api={version='0.1',features=['testing']}",
                SourceReason::Features { mod_id: id() },
            ),
            (
                "[dependencies]\nhorizon-mod-api='0.1'\n[features]\nextra=[]",
                SourceReason::Features { mod_id: id() },
            ),
            (
                "build='make.rs'\n[dependencies]\nhorizon-mod-api='0.1'",
                SourceReason::BuildScript { mod_id: id() },
            ),
        ] {
            assert_eq!(refusal_of(cargo), Some(reason), "{cargo}");
        }
    }

    #[test]
    fn the_archive_is_found_by_name_in_the_release_sums() {
        let name = native::prebuilt_archive("x86_64-unknown-linux-gnu", true);
        assert_eq!(
            name,
            "horizon-prebuilt-v5-x86_64-unknown-linux-gnu-server.tar.gz"
        );
        let listing =
            format!("AB12  horizon-prebuilt-v5-x86_64-unknown-linux-gnu.tar.gz\ncd34 *{name}\n");
        assert_eq!(expected_sum(&listing, &name).as_deref(), Some("cd34"));
        assert_eq!(expected_sum(&listing, "horizon-prebuilt-x.tar.gz"), None);
    }
}
