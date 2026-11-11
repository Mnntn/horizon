//! Function replacement (fn-replace): `[[native.mixin]]` rows built against the prebuilt game, with no
//! game rebuild. Each function the rows target is copied out of the pinned source into one patch
//! crate (`horizon_mod_api_gen::patch`), horizon-mixins applies every mod's rows to those copies
//! in one pass (one combined copy per function, in the tool's order), rustc compiles the crate
//! against the prebuilt game and the mods, and the generated `main` installs each copy through
//! its point's replacement slot. What can't take this route is refused by name; nothing here
//! falls back to compiling the game.
use super::{api, prebuilt::Identity, Bundle};
use crate::i18n::trf;
use crate::rust_toolchain::Toolchain;
use horizon_platform::mod_build::api_gen;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The patch crate's name: the copies' ids and the executable's `--extern` use it.
pub(super) const CRATE: &str = "horizon_patch";

/// The compiled patch crates, `(crate name, rlib)`, and the `Redirect` expressions the
/// generated `main` installs.
pub(super) struct Patch {
    pub rlibs: Vec<(String, PathBuf)>,
    pub installs: Vec<String>,
}

/// Why a mod's tables can't take the copy route, before anything runs.
fn table_refusals(bundle: &Bundle) -> Vec<String> {
    let id = &bundle.manifest.id;
    let native = bundle.manifest.native.as_ref();
    native
        .map(|n| n.state.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|state| {
            format!(
                "E_MIXIN_STATE_LAYOUT: {id}:{}: a state field changes its type's layout, which \
                 the prebuilt game fixes; keep the value in an attachment (`mod_api` attachments \
                 on the entity, block entity, level or player)",
                state.id
            )
        })
        .collect()
}

/// The support modules and bridges `bundles` add to the patch crate.
fn extras(bundles: &[&Bundle]) -> Result<api_gen::patch::Extras, String> {
    let mut extras = api_gen::patch::Extras::default();
    let normalize = |s: &str| s.replace('-', "_").to_ascii_lowercase();
    for bundle in bundles {
        let id = &bundle.manifest.id;
        let Some(native) = bundle.manifest.native.as_ref() else {
            continue;
        };
        for module in &native.module {
            let path = bundle.dir.join(&module.source);
            extras.modules.push(api_gen::patch::SupportModule {
                id: format!("{id}:{}", module.id),
                file: module.file.clone(),
                name: module.name.clone(),
                text: fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
                public: module.visibility.as_deref() == Some("pub"),
            });
        }
        for bridge in &native.bridge {
            let field = |key: &str| bridge.fields.get(key).and_then(toml::Value::as_str);
            extras.bridges.push(api_gen::patch::Bridge {
                id: format!("{id}:{}", bridge.id),
                method: format!(
                    "__horizon_mixins_{}_{}",
                    normalize(id),
                    normalize(&bridge.id)
                ),
                kind: field("kind").unwrap_or_default().to_owned(),
                target: field("target").unwrap_or_default().to_owned(),
                member: field("member").map(str::to_owned),
            });
        }
    }
    Ok(extras)
}

/// Builds the patch crate for `projects`' `[[native.mixin]]` rows against `game`, into `out`; `None`
/// when no mod has one. `rlibs` and `crates` are the compiled mods and their crate names, which
/// helpers may call.
#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    rlibs: &HashMap<&str, PathBuf>,
    crates: &HashMap<&str, String>,
    sdk: &Identity,
    rustc: impl Fn(&str) -> std::process::Command,
) -> Result<Option<Patch>, String> {
    let bundles: Vec<&Bundle> = projects
        .iter()
        .map(|project| project.bundle)
        .filter(|bundle| {
            bundle.needs_mixins()
                || bundle.manifest.native.as_ref().is_some_and(|n| {
                    !n.state.is_empty() || !n.bridge.is_empty() || !n.module.is_empty()
                })
        })
        .collect();
    if bundles.is_empty() {
        return Ok(None);
    }
    let refusals: Vec<String> = bundles.iter().flat_map(|b| table_refusals(b)).collect();
    if !refusals.is_empty() {
        return Err(refusals.join("\n"));
    }
    // Built once per pin and patch, before the clock: the timing is the mixin build's own.
    let tool = super::mixins_patch::tool(rust, &|_| {})?;
    let source = super::source(sdk.commit())?;
    let started = std::time::Instant::now();
    let targets: Vec<String> = bundles
        .iter()
        .flat_map(|bundle| bundle.native().mixin.iter().map(|row| row.target.clone()))
        .collect();
    // The generator's refusals first: they name why a function can't be copied at all (missing,
    // below the game crate, a body that can't move). Then only a point with a raw form has a
    // replacement slot in the prebuilt game.
    let extras = extras(&bundles)?;
    let generated = api_gen::patch::generate(&source, CRATE, &targets, &extras);
    let refusals: Vec<String> = match &generated {
        Ok(_) => Vec::new(),
        Err(refused) => refused.iter().map(ToString::to_string).collect(),
    };
    if !refusals.is_empty() {
        return Err(refusals.join("\n"));
    }
    let generated = generated.expect("no refusals");
    let ids: HashMap<&str, &str> = generated
        .patched
        .iter()
        .map(|p| (p.target.as_str(), p.id.as_str()))
        .collect();
    // The workspace horizon-mixins plans against: one patch crate per game crate with copies.
    let workspace = out.join("patch-workspace");
    let _ = fs::remove_dir_all(&workspace);
    let write = |path: PathBuf, text: &str| {
        fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    let names: Vec<&str> = generated
        .libs
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    write(
        workspace.join("Cargo.toml"),
        &format!(
            "[workspace]\nmembers = [{}]\nresolver = \"2\"\n",
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )?;
    write(
        workspace.join("Cargo.lock"),
        &format!(
            "# Generated by the launcher.\nversion = 4\n{}",
            names
                .iter()
                .map(|n| format!("\n[[package]]\nname = \"{n}\"\nversion = \"0.0.0\"\n"))
                .collect::<String>()
        ),
    )?;
    write(workspace.join(".horizon-revision"), sdk.commit())?;
    for (name, lib) in &generated.libs {
        let package = workspace.join(name);
        fs::create_dir_all(package.join("src")).map_err(|e| e.to_string())?;
        write(
            package.join("Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n"),
        )?;
        write(package.join("src/lib.rs"), lib)?;
    }
    // Each mod's rows, retargeted at the copies, as the tool's bundles.
    let mods = out.join("patch-mods");
    let _ = fs::remove_dir_all(&mods);
    for bundle in bundles
        .iter()
        .filter(|bundle| !bundle.native().mixin.is_empty())
    {
        let dir = mods.join(&bundle.manifest.id);
        super::package::copy_tool_input(bundle, &dir)?;
        let mut rows = bundle.native().mixin.clone();
        for row in &mut rows {
            row.target = ids[row.target.as_str()].to_owned();
        }
        let mut manifest = toml::Table::new();
        manifest.insert(
            "mod".into(),
            toml::Value::Table(toml::Table::from_iter([
                ("id".into(), bundle.manifest.id.clone().into()),
                ("version".into(), bundle.manifest.version.clone().into()),
            ])),
        );
        manifest.insert(
            "compatibility".into(),
            toml::Value::Table(toml::Table::from_iter([(
                "horizon_revision".into(),
                sdk.commit().to_owned().into(),
            )])),
        );
        manifest.insert(
            "injection".into(),
            toml::Value::try_from(&rows).map_err(|e| e.to_string())?,
        );
        write(
            dir.join("horizon-mixin.toml"),
            &toml::to_string(&manifest).map_err(|e| e.to_string())?,
        )?;
    }
    let copied = started.elapsed();
    // With no rows (only modules or bridges), the crate compiles as generated.
    let patched = if targets.is_empty() {
        workspace.clone()
    } else {
        let patched = out.join("patch-applied");
        super::run(
            super::apply(rust, &tool, &workspace, &mods, &patched),
            "Applying the mixins to their functions",
            |_| {},
        )?;
        patched
    };
    let applied = started.elapsed();
    // The patched crates, against the prebuilt game and the mods.
    let deps = game.join("deps");
    let mut compiled = Vec::new();
    for name in &names {
        let rlib = out.join(format!("lib{name}.rlib"));
        let mut cmd = rustc("2024");
        cmd.args([
            "--crate-type",
            "rlib",
            "--crate-name",
            name,
            "--cap-lints",
            "allow",
        ])
        .arg("-o")
        .arg(&rlib)
        .arg(patched.join(name).join("src/lib.rs"));
        for feature in sdk.game_features() {
            cmd.arg("--cfg").arg(format!("feature=\"{feature}\""));
        }
        cmd.arg("--extern").arg(format!(
            "horizon_client={}",
            super::prebuilt::library(&deps, "horizon_client")?.display()
        ));
        for krate in sdk.facade_crates() {
            if krate != "horizon_client" {
                cmd.arg("--extern").arg(format!(
                    "{krate}={}",
                    super::prebuilt::facade_library(&deps, krate, sdk)?.display()
                ));
            }
        }
        for bundle in &bundles {
            let id = bundle.manifest.id.as_str();
            cmd.arg("--extern")
                .arg(format!("{}={}", crates[id], rlibs[id].display()));
        }
        super::run(cmd, "Compiling the patched functions", |_| {})?;
        compiled.push((name.to_string(), rlib));
    }
    super::append_log(&trf(
        "Function replacement: {count} functions patched by {rows} mixins, without rebuilding the game (copy {copy} ms, horizon-mixins {tool} ms, compile {compile} ms, total {total} ms)",
        &[
            ("count", &generated.patched.len().to_string()),
            ("rows", &targets.len().to_string()),
            ("copy", &copied.as_millis().to_string()),
            ("tool", &(applied - copied).as_millis().to_string()),
            ("compile", &(started.elapsed() - applied).as_millis().to_string()),
            ("total", &started.elapsed().as_millis().to_string()),
        ],
    ))?;
    Ok(Some(Patch {
        rlibs: compiled,
        installs: generated
            .patched
            .into_iter()
            .map(|p| format!("({:?}, {})", p.target, p.install))
            .collect(),
    }))
}
