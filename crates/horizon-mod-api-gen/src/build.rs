//! Cargo-facing generation: the mod API's and the game's generated files.
use super::{Catalog, Target, free_function_source, host, native_hook_files, sources};
use quote::ToTokens;
use std::{fs, path::Path};

/// Generate only this crate's files, tracking every source read by the generator.
/// `root` is supplied at run time by the consuming crate's build script.
pub fn generate(root: &Path, out_dir: &Path, target: Target) -> Result<(), String> {
    track(root, !matches!(&target, Target::Vocab))?;
    let generated = sources(root, super::FillMode::Require)?;
    let outputs = match target {
        Target::Client => vec![
            (
                "game_modules.rs",
                super::game_modules::declarations(&root.join("crates/horizon-client/src/game"))?,
            ),
            ("native_hook_modules.rs", native_hook_modules(root)?),
            (
                "lan_action_modules.rs",
                super::game_modules::lan_action_modules(
                    &root.join("crates/horizon-client/src/game/network/lan_actions"),
                )?,
            ),
            ("game_host_modules.rs", game_host_modules(root)?),
            ("mod_api_fire_generated.rs", generated.fire),
            // The mob tick's goal read view (#1077), included beside the
            // engine's `GoalCx` in `mobs/part_goals.rs`.
            ("mod_api_goal_view_generated.rs", generated.goal_view),
        ],
        Target::Api | Target::Vocab => {
            if matches!(target, Target::Api) {
                let directory = root.join(super::exports::API_DIRECTORY);
                println!("cargo:rerun-if-changed={}", directory.display());
                for area in Catalog::modules(&directory).map_err(|e| e.to_string())? {
                    println!(
                        "cargo:rerun-if-changed={}",
                        directory.join(format!("{area}.rs")).display()
                    );
                }
                for (name, code) in super::exports::api_sources(root)? {
                    fs::write(out_dir.join(name), code).map_err(|e| e.to_string())?;
                }
            }
            if matches!(target, Target::Vocab) {
                fs::write(
                    out_dir.join(super::parts::OUTPUT),
                    super::parts::source(root)?,
                )
                .map_err(|e| e.to_string())?;
                for (name, code) in super::exports::sources(root)? {
                    fs::write(out_dir.join(name), code).map_err(|e| e.to_string())?;
                }
            }
            let hooks = generated.hooks;
            let included = [
                (host::AREAS_OUTPUT, generated.areas.as_str()),
                ("mod_api_generated.rs", hooks.as_str()),
                ("mod_api_generated_host.rs", generated.host.as_str()),
            ];
            let (providers, sources) = super::capability_providers::generate(root, &included)?;
            // The resolver recurses through the API's types: give it a deep stack.
            let generated_wire = std::thread::scope(|scope| {
                std::thread::Builder::new()
                    .stack_size(64 << 20)
                    .spawn_scoped(scope, || super::wire::generate(root, &included))
                    .map_err(|e| e.to_string())?
                    .join()
                    .map_err(|_| "generating the wire impls panicked".to_string())?
            })?;
            for path in sources.iter().chain(&generated_wire.sources) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
            let outputs = vec![
                (host::AREAS_OUTPUT, generated.areas),
                ("mod_api_generated.rs", hooks),
                ("mod_api_generated_host.rs", generated.host),
                ("capability_providers_generated.rs", providers),
                ("wire_generated.rs", generated_wire.wire),
                ("wire_guest_generated.rs", generated_wire.guest),
            ];
            outputs
                .into_iter()
                .filter(|(name, _)| match target {
                    Target::Api => *name == "wire_guest_generated.rs",
                    Target::Vocab => *name != "wire_guest_generated.rs",
                    Target::Client => unreachable!(),
                })
                .collect()
        }
    };
    for (name, code) in outputs {
        fs::write(out_dir.join(name), &code).map_err(|e| e.to_string())?;
        syn::parse_file(&code).map_err(|e| format!("generated Rust: {name}: {e}"))?;
    }
    Ok(())
}

pub(crate) fn native_hook_modules(root: &Path) -> Result<String, String> {
    let mut modules = String::new();
    for path in native_hook_files(root)? {
        let module = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("{}: invalid native hook module name", path.display()))?;
        if matches!(module, "mod" | "shared") {
            continue;
        }
        syn::parse_str::<syn::Ident>(module).map_err(|e| e.to_string())?;
        let path = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        modules.push_str(&format!("#[path = {path:?}] mod {module};\n"));
    }
    Ok(modules)
}

/// The game's side of the host traits: one module for each area file of `game/host/`.
pub(crate) fn game_host_modules(root: &Path) -> Result<String, String> {
    let dir = root.join("crates/horizon-client/src/game/host");
    let mut modules = String::new();
    for stem in ["draw", "services"] {
        let path = fs::canonicalize(dir.join(format!("{stem}.rs"))).map_err(|e| e.to_string())?;
        modules.push_str(&format!("#[path = {path:?}] pub(crate) mod {stem};\n"));
    }
    modules.push_str(&format!(
        "mod generated {{\n{}\n}}\n",
        host::game_forwarders(root)?
    ));
    Ok(modules)
}

/// Generate the native-only raw render records, without trait dispatch or subscriptions.
pub fn generate_render(root: &Path, out_dir: &Path) -> Result<(), String> {
    let declarations = root.join(super::catalog::DECLARATIONS);
    println!("cargo:rerun-if-changed={}", declarations.display());
    for module in Catalog::modules(&declarations)? {
        println!(
            "cargo:rerun-if-changed={}",
            declarations.join(format!("{module}.rs")).display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        root.join("crates/horizon-mod-render/src/render.rs")
            .display()
    );
    let source = super::dispatch::render(root, &Catalog::load(root)?)?.render_dispatch;
    use syn::spanned::Spanned;
    let file = syn::parse_file(&source).map_err(|error| error.to_string())?;
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |point: proc_macro2::LineColumn| lines[point.line - 1] + point.column;
    let mut records = String::from("use super::*;\n");
    for item in file.items {
        let syn::Item::Struct(record) = item else {
            continue;
        };
        let start = record
            .attrs
            .iter()
            .find(|attr| !attr.path().is_ident("doc"))
            .map_or_else(|| record.vis.span().start(), |attr| attr.span().start());
        records.push_str(&source[offset(start)..offset(record.span().end())]);
        records.push('\n');
    }
    syn::parse_file(&records).map_err(|error| error.to_string())?;
    fs::write(out_dir.join("native_render_generated.rs"), records)
        .map_err(|error| error.to_string())
}

#[test]
fn render_records_need_no_game_sources() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = workspace.join(format!("target/render-records-only-{}", std::process::id()));
    let declarations = root.join(super::catalog::DECLARATIONS);
    fs::create_dir_all(&declarations).unwrap();
    for entry in fs::read_dir(workspace.join(super::catalog::DECLARATIONS)).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            fs::copy(&path, declarations.join(path.file_name().unwrap())).unwrap();
        }
    }
    let raw = "crates/horizon-mod-render/src/render.rs";
    fs::create_dir_all(root.join(raw).parent().unwrap()).unwrap();
    fs::copy(workspace.join(raw), root.join(raw)).unwrap();
    assert!(!root.join("crates/horizon-client").exists());
    generate_render(&root, &root).unwrap();
    let generated = fs::read_to_string(root.join("native_render_generated.rs")).unwrap();
    let file = syn::parse_file(&generated).unwrap();
    assert_eq!(
        file.items
            .iter()
            .filter(|item| matches!(item, syn::Item::Struct(_)))
            .count(),
        3
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn listed_render_slots_have_eligible_points() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    super::points::game_points(&root).unwrap();
}

/// The `mod_api` types with a `Wire` layout ([`super::wire`]), for point records.
/// A source tree without the mod API (a generator fixture's) has none.
pub(crate) fn wire_types(root: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    if !root.join("crates/horizon-mod-vocab/src/hooks").is_dir() {
        return Ok(Default::default());
    }
    let generated = sources(root, super::FillMode::Require)?;
    let included = [
        (host::AREAS_OUTPUT, generated.areas.as_str()),
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
    ];
    // The resolver recurses through the API's types: give it a deep stack.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn_scoped(scope, || super::wire::owned_types(root, &included))
            .map_err(|e| e.to_string())?
            .join()
            .map_err(|_| "reading the wire types panicked".to_string())?
    })
}

/// `horizon-mod-points`' generated file: each point's record, `PointId` and typed function constructors,
/// from the game's source, tracking the game module's files.
pub fn generate_points(root: &Path, out_dir: &Path) -> Result<(), String> {
    let src = root.join("crates/horizon-client/src");
    println!("cargo:rerun-if-changed={}", src.join("game.rs").display());
    println!("cargo:rerun-if-changed={}", src.join("game").display());
    for path in super::points::tracked(root)?
        .into_iter()
        .chain(super::sites::tracked(root).map_err(|e| e.to_string())?)
    {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let private = super::points::source::build_input(root)?;
    println!("cargo:rerun-if-env-changed=HORIZON_POINT_NAMES");
    let selected = std::env::var("HORIZON_POINT_NAMES")
        .ok()
        .map(|json| {
            serde_json::from_str::<std::collections::BTreeSet<String>>(&json)
                .map_err(|error| format!("HORIZON_POINT_NAMES: {error}"))
        })
        .transpose()?;
    let code = super::points::records_selected(
        &super::points::game_points(root)?,
        &private,
        selected.as_ref(),
    ) + &super::sites::ids(root).map_err(|e| e.to_string())?;
    let generated = syn::parse_file(&code).map_err(|e| format!("generated Rust: points: {e}"))?;
    // A single multi-megabyte line makes Clippy's source lookups prohibitively slow.
    let mut code = String::new();
    for item in generated.items {
        code.push_str(&item.to_token_stream().to_string());
        code.push('\n');
    }
    fs::write(out_dir.join("points_generated.rs"), code).map_err(|e| e.to_string())
}

pub(super) fn track(root: &Path, all_game: bool) -> Result<(), String> {
    fn directory(path: &Path, extension: &str, recursive: bool) -> Result<(), String> {
        // Track the directory as well: adding a helper source or fragment changes the inputs.
        println!("cargo:rerun-if-changed={}", path.display());
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() && recursive {
                directory(&path, extension, recursive)?;
            } else if path.extension().is_some_and(|ext| ext == extension) {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
        Ok(())
    }
    println!(
        "cargo:rerun-if-changed={}",
        root.join("crates/horizon-mod-vocab/src/lib.rs").display()
    );
    for retired in super::catalog::RETIRED {
        // Only while present: Cargo reruns a build script on every build for a path that doesn't
        // exist. One that appears fails the next generation, and every fresh build, such as CI's.
        let path = root.join(retired);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    directory(&root.join(super::catalog::DECLARATIONS), "rs", false)?;
    directory(&root.join(super::exports::DIRECTORY), "rs", false)?;
    directory(
        &root.join("crates/horizon-mod-vocab/src/parts"),
        "rs",
        false,
    )?;
    if all_game {
        directory(&root.join("crates/horizon-client/src/game"), "rs", true)?;
        println!(
            "cargo:rerun-if-changed={}",
            root.join("crates/horizon-client/src/game.rs").display()
        );
    } else {
        directory(
            &root.join("crates/horizon-client/src/game/native_hooks"),
            "rs",
            false,
        )?;
    }
    for hook in Catalog::load(root)?.hooks {
        if let Some((krate, path)) = free_function_source(&hook.target) {
            let (module, _) = path.rsplit_once("::").ok_or("target missing module")?;
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    Ok(())
}
