//! `cargo horizon-mod`: make Horizon content mods (`docs/modpacks.md`): blocks,
//! items, data and assets, no code. Code mods are native (docs/modding/src/how-to/native-mods.md).
//!
//! - `new <path> [--name "Name"]`: a content mod from the template; with
//!   `--template native [--horizon <commit>]`, a native trait mod's bundle, pinned to the
//!   newest prebuilt game unless `--horizon` names a commit.
//! - `update [--horizon <commit>]`: repin this native mod and diff/refresh declared internals.
//! - `inject update [--accept <id>]…`: refresh declarations without repinning.
//! - `inject sync --repo <game checkout>`: publish missing slots to a per-mod list.
//! - `inject add <at> --kind <kind> [--call <callee>] [--nth N] [--with <fn>]`: pin an inject.
//! - `internals sync [--repo <path>]`: sync declarations for native or wasm mods.
//! - `internals add <path>… [--kind <kind>]`: declare internals from the prebuilt catalog.
//! - `build`: the `.hmod` in `target/horizon-mod/`, with the mod's `assets/`
//!   and `data/`; with `--wasm`, a Rust mod built to `mod.wasm` first (`wasm.rs`).
//! - `wasm-std <out.wasm>`: the standard library member the game ships for wasm mods.
//! - `run [--game <horizon-client>] [-- <game args>]`: build, then play with
//!   the mod.
//! - `check <file> [--json]`: what the registry checks of a submission
//!   (`.hmod`, `.hpack`, resource pack `.zip`, `.hshader`).
//! - `registry build|review|check|list`: the registry repository's CI.
//! - `use-jar <path>`: list an exact original Java jar as a passive asset/data source.
use horizon_mod_package::{Content, Manifest, Package, index::native, manifest::MANIFEST};
mod bounded;
mod check;
mod inject;
mod inject_sync;
mod internals;
mod jar;
mod layouts;
mod registry;
mod wasm;

use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn is_compiler_wrapper(name: &str) -> bool {
    name.strip_suffix(".exe").unwrap_or(name).ends_with("rustc")
}

fn main() -> ExitCode {
    // Cargo runs `cargo-horizon-mod horizon-mod <args>`.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os("HORIZON_GROUP_WRAPPER").is_some()
        // `rustc`, or `rustc.exe` on Windows.
        && args
            .first()
            .is_some_and(|a| is_compiler_wrapper(a))
    {
        return match wasm::rustc(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    // Cargo runs this tool as the linker of a `build --wasm`.
    if let Ok(role) = std::env::var(wasm::LINK_ENV) {
        return match wasm::link(&role, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().is_some_and(|a| a == "horizon-mod") {
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..]),
        Some("build") if args.get(1).is_some_and(|a| a == "--wasm") => wasm::build(Path::new("."), flag(&args[2..], "--catalog").map(Path::new), &args[2..].windows(2).filter(|a| a[0] == "--with").map(|a| PathBuf::from(&a[1])).collect::<Vec<_>>())
            .and_then(|()| build())
            .map(|p| println!("Built {}", p.display())),
        Some("wasm-std") => wasm::wasm_std(&args[1..]),
        Some("build") => build().map(|p| {
            println!("Built {}", p.display());
        }),
        Some("run") => run(&args[1..]).map_err(|e| e.to_string()),
        Some("check") => check(&args[1..]),
        Some("registry") => registry::main(&args[1..]),
        Some("use-jar") => jar::run(&args[1..]),
        Some("internals") => internals::run(&args[1..]),
        Some("inject") => inject::run(&args[1..]),
        Some("update") => update(
            Path::new("."),
            flag(&args[1..], "--horizon"),
            &args[1..]
                .windows(2)
                .filter(|pair| pair[0] == "--accept")
                .map(|pair| pair[1].as_str())
                .collect::<Vec<_>>(),
        ),
        _ => Err("usage: cargo horizon-mod new <path> [--name \"Name\"] [--template content|native [--horizon <commit>]] | update [--horizon <commit>] | inject update [--accept <id>]… | inject sync --repo <game checkout> | inject add <at> --kind <kind> [--call <canonical callee>] [--nth N] [--with <fn>] | internals sync [--repo <path>] | internals add <path>… [--kind <kind>] | build [--wasm] | wasm-std <out.wasm> | run [--game <horizon-client>] [-- <game args>] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json] | registry build|review|check|list | use-jar <path>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The value after `flag` in `args`.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

fn new(args: &[String]) -> Result<(), String> {
    let path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod new <path>")?;
    let dir = Path::new(path);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let base = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the path needs a folder name")?;
    let id: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let name = flag(args, "--name").map_or_else(|| base.replace(['-', '_'], " "), str::to_string);
    let fill = |t: &str| t.replace("{id}", &id).replace("{name}", &name);
    match flag(args, "--template") {
        None | Some("content") => {}
        Some("native") => return new_native(dir, &id, flag(args, "--horizon"), fill),
        Some(_) => return Err("--template is content or native".into()),
    }
    // A block copying a vanilla one, with its model, name, loot, recipe and tag.
    let files = [
        (
            MANIFEST,
            include_str!("../template/content/horizon-mod.toml"),
        ),
        (
            "data/{id}/horizon/block/glowing_amethyst.json",
            include_str!("../template/content/block.json"),
        ),
        (
            "assets/{id}/blockstates/glowing_amethyst.json",
            include_str!("../template/content/blockstate.json"),
        ),
        (
            "assets/{id}/models/block/glowing_amethyst.json",
            include_str!("../template/content/model.json"),
        ),
        (
            "assets/{id}/items/glowing_amethyst.json",
            include_str!("../template/content/item.json"),
        ),
        (
            "assets/{id}/lang/en_us.json",
            include_str!("../template/content/lang.json"),
        ),
        (
            "data/{id}/loot_table/blocks/glowing_amethyst.json",
            include_str!("../template/content/loot.json"),
        ),
        (
            "data/{id}/recipe/glowing_amethyst.json",
            include_str!("../template/content/recipe.json"),
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            include_str!("../template/content/pickaxe.json"),
        ),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    for (file, text) in files {
        let file = fill(file);
        let file = file.as_str();
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?)?;
    Content::read(dir, &manifest)?;
    println!(
        "Created {} (mod id {id}). Next: cd {path} && cargo horizon-mod run",
        dir.display()
    );
    Ok(())
}

/// The Horizon commit a native mod pins: `--horizon`'s, or the newest prebuilt game's, so the
/// launcher downloads the game instead of building it.
fn pin(horizon: Option<&str>) -> Result<String, String> {
    let Some(horizon) = horizon else {
        let api = format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            native::PREBUILT_REPO
        );
        let releases = ureq::get(&api)
            .call()
            .map_err(|e| format!("{api}: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())?;
        return newest_prebuilt(&serde_json::from_str(&releases).map_err(|e| e.to_string())?)
            .ok_or_else(|| {
                format!(
                    "no prebuilt game on {}'s releases: pass --horizon <commit>",
                    native::PREBUILT_REPO
                )
            });
    };
    if !horizon_mod_package::manifest::revision(horizon) {
        return Err("--horizon: a Horizon commit is 40 lowercase hex digits".into());
    }
    Ok(horizon.into())
}

/// The commit of the newest release (GitHub lists them newest first) carrying prebuilt archives
/// the launcher reads.
fn newest_prebuilt(releases: &serde_json::Value) -> Option<String> {
    releases.as_array()?.iter().find_map(|release| {
        let commit = release["tag_name"]
            .as_str()?
            .strip_prefix(native::PREBUILT_TAG)?;
        let readable = release["assets"].as_array()?.iter().any(|asset| {
            asset["name"]
                .as_str()
                .is_some_and(|name| name.starts_with(native::PREBUILT_ARCHIVE))
        });
        (readable && horizon_mod_package::manifest::revision(commit)).then(|| commit.to_string())
    })
}

/// Repins the native mod and its API dependencies, reporting signature changes before writing.
fn update(dir: &Path, horizon: Option<&str>, accept: &[&str]) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest = Manifest::parse(&text)?;
    let native = manifest
        .native
        .as_ref()
        .ok_or("update needs a [native] mod")?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let cargo_path = dir.join("Cargo.toml");
    let cargo_text = std::fs::read_to_string(&cargo_path)
        .map_err(|e| format!("{}: {e}", cargo_path.display()))?;
    let mut cargo: toml_edit::DocumentMut = cargo_text
        .parse()
        .map_err(|e| format!("{}: {e}", cargo_path.display()))?;
    let horizon = pin(horizon)?;
    for package in ["horizon-mod-api", "horizon-internals"] {
        let dependency = cargo
            .get_mut("dependencies")
            .and_then(|deps| deps.get_mut(package))
            .filter(|dep| !dep.is_none());
        if package == "horizon-internals" && dependency.is_none() {
            continue;
        }
        let rev = dependency
            .and_then(|dep| dep.get_mut("rev"))
            .and_then(toml_edit::Item::as_value_mut)
            .ok_or_else(|| format!("Cargo.toml: no dependencies.{package}.rev"))?;
        internals::set_string(rev, &horizon);
    }
    if !manifest.internals.is_empty() || manifest.inject.iter().any(|i| i.hash.is_none()) {
        // #2593: these become the pinned catalogs with the mod's own modules applied.
        let old = internals::load(dir, &native.horizon)?;
        let new = internals::load(dir, &horizon)?;
        let (reports, refused) = inject::update::pin_moves(&manifest, &old, &new, accept);
        for report in reports {
            println!("{report}");
        }
        if refused {
            return Err(
                "update refused; no files changed (pass --accept <inject id> to take a change)"
                    .into(),
            );
        }
        print!("{}", internals::refresh(&manifest, &mut doc, &old, &new)?);
    }
    internals::set_string(
        doc["native"]["horizon"].as_value_mut().expect("native pin"),
        &horizon,
    );
    let candidate = doc.to_string();
    std::fs::write(&path, candidate).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(&cargo_path, cargo.to_string())
        .map_err(|e| format!("{}: {e}", cargo_path.display()))?;
    println!("Pinned to Horizon {horizon}");
    Ok(())
}

/// A flat native trait mod project, pinned to the Horizon commit it is made for.
fn new_native(
    dir: &Path,
    id: &str,
    horizon: Option<&str>,
    fill: impl Fn(&str) -> String,
) -> Result<(), String> {
    if id.len() > 64 {
        return Err(format!("mod id {id}: 64 characters at most"));
    }
    let horizon = pin(horizon)?;
    let horizon = horizon.as_str();
    let rev = format!(", rev = \"{horizon}\"");
    let manifest = format!(
        "{}\n[native]\ngame = \"26.3\"\nhorizon = \"{horizon}\"\nmixins = \"{}\"\n",
        include_str!("../template/content/horizon-mod.toml"),
        native::MIXINS_REV,
    );
    let files = [
        ("Cargo.toml", include_str!("../template/native/api.toml")),
        ("src/lib.rs", include_str!("../template/native/lib.rs")),
        (MANIFEST, manifest.as_str()),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    for (file, text) in files {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        let text = fill(text).replace("{rev}", &rev);
        std::fs::write(&to, text).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    for folder in ["assets", "data"] {
        std::fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
    }
    println!(
        "Created {} (native mod id {id}). Next: horizon-launcher native-build {} --instance <instance> --trust-native-code",
        dir.display(),
        dir.display()
    );
    Ok(())
}

/// Packs the content mod in the current folder into `target/horizon-mod/<id>.hmod`: the
/// manifest and the files in `assets/` and `data/`.
fn build() -> Result<PathBuf, String> {
    build_in(Path::new("."))
}

fn build_in(root: &Path) -> Result<PathBuf, String> {
    pack_into(root, &root.join("target/horizon-mod"))
}

fn pack_into(root: &Path, dir: &Path) -> Result<PathBuf, String> {
    let package = Package::read(root)?;
    if package.manifest.native.is_some() || package.manifest.wasm.is_some() {
        layouts::build(root)?;
    }
    let bytes = Package::pack_directory(root)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.hmod", package.manifest.id));
    // Write then rename, so a running game never reads half a file.
    let partial = path.with_extension("hmod.partial");
    std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `check <file>`: the registry's checks of a submission, printed; fails
/// when one does.
fn check(args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod check <file>")?;
    let path = Path::new(file);
    let kind = match flag(args, "--kind") {
        Some(k) => horizon_mod_package::index::Kind::from(k.to_string()),
        None => check::kind_of(path)
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?,
    };
    let signature = flag(args, "--signature")
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
        .transpose()?;
    let report = check::check(path, kind, flag(args, "--key"), signature.as_deref());
    if args.iter().any(|a| a == "--json") {
        let json = match &report {
            Ok(r) => serde_json::json!({"ok": true, "report": r.json()}),
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        println!("{json}");
    }
    let report = report?;
    if !args.iter().any(|a| a == "--json") {
        print!("{}", report.text());
    }
    Ok(())
}

#[derive(Debug)]
enum RunError {
    CodeMod,
    Build(String),
    Launch { game: String, error: std::io::Error },
    Exit(std::process::ExitStatus),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CodeMod => f.write_str(
                "code mods run through the launcher's native build: horizon-launcher native-build . --trust-native-code (docs/modding/src/how-to/native-mods.md)",
            ),
            Self::Build(error) => f.write_str(error),
            Self::Launch { game, error } => write!(
                f,
                "{game}: {error} (pass --game <path to horizon-client> or set HORIZON_CLIENT)"
            ),
            Self::Exit(status) => write!(f, "the game exited with {status}"),
        }
    }
}

impl std::error::Error for RunError {}

fn run(args: &[String]) -> Result<(), RunError> {
    if Path::new("Cargo.toml").is_file() {
        return Err(RunError::CodeMod);
    }
    let path = build().map_err(RunError::Build)?;
    let game = flag(args, "--game")
        .map(str::to_string)
        .or_else(|| std::env::var("HORIZON_CLIENT").ok())
        .unwrap_or_else(|| "horizon-client".into());
    let extra = args.iter().skip_while(|a| *a != "--").skip(1);
    println!("Starting {game} with {}", path.display());
    let status = Command::new(&game)
        .arg("--dev-mod")
        .arg(&path)
        .args(extra)
        .status()
        .map_err(|error| RunError::Launch { game, error })?;
    status.success().then_some(()).ok_or(RunError::Exit(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_content_template_builds_and_checks_with_no_code() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
        let path = dir.join("gems");
        let new_args = [
            path.display().to_string(),
            "--template".into(),
            "content".into(),
        ];
        new(&new_args).unwrap();
        assert!(!path.join("Cargo.toml").exists());
        let hmod = build_in(&path).unwrap();
        assert_eq!(hmod, path.join("target/horizon-mod/gems.hmod"));
        let package = Package::read(&hmod).unwrap();
        let content = Content::read(&hmod, &package.manifest).unwrap();
        assert!(
            content
                .files
                .contains(&"data/gems/horizon/block/glowing_amethyst.json".into())
        );
        assert_eq!(content.files.len(), 8, "{:?}", content.files);
        let report = check::check(&hmod, check::kind_of(&hmod).unwrap(), None, None).unwrap();
        assert!(
            report.text().contains("no code: content only"),
            "{}",
            report.text()
        );
        let executable = path.join("assets/gems/mod.wasm");
        std::fs::write(&executable, b"\0asm").unwrap();
        assert!(
            build_in(&path)
                .unwrap_err()
                .contains("unsupported executable file")
        );
        std::fs::remove_file(executable).unwrap();
        // Code is native now.
        std::fs::write(path.join("Cargo.toml"), "").unwrap();
        assert!(build_in(&path).unwrap_err().contains("native"));
        assert!(
            new(&[
                dir.join("x").display().to_string(),
                "--template".into(),
                "lua".into()
            ])
            .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_build_and_check_share_the_content_executable_boundary() {
        use horizon_mod_package::manifest::ContentFileError;
        use std::io::Write;

        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("native-content-contract-{}", std::process::id()));
        let path = dir.join("mixed");
        let _ = std::fs::remove_dir_all(&dir);
        new(&[
            path.display().to_string(),
            "--template".into(),
            "native".into(),
            "--horizon".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
        ])
        .unwrap();
        let asset = "assets/mixed/program.bin";
        std::fs::create_dir_all(path.join("assets/mixed")).unwrap();
        std::fs::write(path.join(asset), b"ordinary asset").unwrap();
        // Only selected content ships, not the Git project's executable hooks.
        std::fs::create_dir_all(path.join(".git/hooks")).unwrap();
        std::fs::write(path.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 0\n").unwrap();
        let hmod = build_in(&path).unwrap();
        let kind = check::kind_of(&hmod).unwrap();
        assert!(check::check(&hmod, kind.clone(), None, None).is_ok());

        // Model an externally supplied archive with executable bytes under an asset name.
        let original = std::fs::read(&hmod).unwrap();
        let mut input = zip::ZipArchive::new(std::io::Cursor::new(original)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        for i in 0..input.len() {
            let mut entry = input.by_index(i).unwrap();
            let name = entry.name().unwrap().into_owned();
            output.start_file(&name, options).unwrap();
            if name == asset {
                output.write_all(b"MZ-not-an-asset").unwrap();
            } else {
                std::io::copy(&mut entry, &mut output).unwrap();
            }
        }
        std::fs::write(&hmod, output.finish().unwrap().into_inner()).unwrap();
        let expected = ContentFileError::Executable(asset.into()).to_string();
        assert_eq!(
            check::check(&hmod, kind, None, None).err().unwrap(),
            expected
        );
        std::fs::remove_file(&hmod).unwrap();
        std::fs::write(path.join(asset), b"MZ-not-an-asset").unwrap();
        assert_eq!(build_in(&path).unwrap_err(), expected);
        assert!(!hmod.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_native_template_packs_into_an_hmod_the_registry_accepts() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-native-{}", std::process::id()));
        let path = dir.join("machine-mod");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let args = [
            path.display().to_string(),
            "--template".into(),
            "native".into(),
            "--horizon".into(),
            commit.into(),
        ];
        new(&args).unwrap();
        let api = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        assert!(api.contains(&format!("rev = \"{commit}\"")), "{api}");
        let hmod = build_in(&path).unwrap();
        let bundle = horizon_mod_package::index::native::Bundle::read(&hmod).unwrap();
        assert_eq!(bundle.manifest.id, "machine_mod");
        let names: Vec<_> = bundle.files.keys().map(String::as_str).collect();
        assert_eq!(names, ["Cargo.toml", "horizon-mod.toml", "src/lib.rs"]);
        assert!(path.join("horizon-mod.toml").is_file());
        for old in [
            "api",
            "mixins",
            "content",
            "native",
            "rust",
            "horizon-native.json",
            "horizon-mixin.toml",
        ] {
            assert!(!path.join(old).exists());
        }
        assert_eq!(
            check::kind_of(&hmod),
            Some(horizon_mod_package::index::Kind::NativeMod)
        );
        assert_eq!(check::kind_of(&dir.join("machine_mod.hnmod")), None);
        let report = check::check(&hmod, check::kind_of(&hmod).unwrap(), None, None).unwrap();
        assert!(report.text().contains("machine_mod"), "{}", report.text());
        assert!(
            new(&[
                dir.join("bad").display().to_string(),
                "--template".into(),
                "native".into(),
                "--horizon".into(),
                "main".into(),
            ])
            .unwrap_err()
            .contains("40 lowercase hex")
        );
        let newer = "89abcdef0123456789abcdef0123456789abcdef";
        update(&path, Some(newer), &[]).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest.native.unwrap().horizon, newer);
        let api = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        assert!(api.contains(&format!("rev = \"{newer}\" }}")), "{api}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_pin_is_the_newest_release_the_launcher_can_download() {
        let (old, new) = (
            "0123456789abcdef0123456789abcdef01234567",
            "89abcdef0123456789abcdef0123456789abcdef",
        );
        let releases = serde_json::json!([
            {"tag_name": "v1.2.0", "assets": [{"name": "horizon-launcher.zip"}]},
            // An archive layout this launcher doesn't read.
            {"tag_name": format!("prebuilt-{new}"),
             "assets": [{"name": "horizon-prebuilt-x86_64-unknown-linux-gnu.tar.gz"}]},
            {"tag_name": format!("prebuilt-{old}"),
             "assets": [{"name": "SHA256SUMS"},
                        {"name": native::prebuilt_archive("aarch64-apple-darwin", false)}]},
        ]);
        assert_eq!(newest_prebuilt(&releases).as_deref(), Some(old));
        assert_eq!(newest_prebuilt(&serde_json::json!([])), None);
    }
}

#[cfg(test)]
mod wrapper_suffix_tests {
    #[test]
    fn compiler_names_keep_suffix_contract() {
        for (name, expected) in [
            ("diagnostic-rustc", true),
            ("xrustc", true),
            ("rustc", true),
            ("/path/to/rustc", true),
            ("rustc.exe", true),
            ("diagnostic-rustc.exe", true),
            ("rustc.custom", false),
            ("rustc-wrapper", false),
            ("cargo-horizon-mod", false),
            ("/rustc/tool", false),
        ] {
            assert_eq!(super::is_compiler_wrapper(name), expected, "{name}");
        }
    }
}
