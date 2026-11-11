//! Cargo owns registry compilation and the final link; the SDK remains binary-only.
use super::{api, dependencies, internals, prebuilt, registry_plan, registry_rustc};
use crate::{
    i18n::{tr, trf},
    rust_toolchain::Toolchain,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(super) struct Options<'a> {
    pub consent: Option<&'a dyn Fn(&super::RegistryInventory) -> bool>,
    pub adapter: &'a Path,
}

pub(super) fn compile(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    options: Options<'_>,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let sdk = prebuilt::identity(game)?;
    let lock = sdk.registry_lock.as_ref().ok_or_else(|| tr("The prebuilt SDK does not declare a locked registry graph; install an SDK that supports registry-bearing native mods").to_string())?;
    let mut patches = sdk.registry_patches.clone().ok_or_else(|| {
        tr("The prebuilt SDK does not declare game registry patches; install a newer SDK")
            .to_string()
    })?;
    let sdk_root = game.canonicalize().map_err(|e| e.to_string())?;
    for (_, registry) in patches.iter_mut() {
        for (_, spec) in registry
            .as_table_mut()
            .ok_or_else(|| tr("Invalid SDK patch registry").to_string())?
            .iter_mut()
        {
            let spec = spec
                .as_table_mut()
                .ok_or_else(|| tr("Invalid SDK patch").to_string())?;
            let relative = spec
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| tr("SDK patch has no path").to_string())?;
            let path = sdk_root.join(relative).canonicalize().map_err(|e| {
                trf(
                    "SDK patch {path}: {error}",
                    &[("path", relative), ("error", &e.to_string())],
                )
            })?;
            if !path.starts_with(&sdk_root) {
                return Err(tr("SDK patch escapes the verified SDK").into());
            }
            spec.insert("path".into(), path.to_string_lossy().into_owned().into());
        }
    }
    let links = sdk.native_links.as_ref().ok_or_else(|| tr("The prebuilt SDK does not declare native library providers; registry-bearing mods cannot be linked safely").to_string())?;
    let catalog =
        internals::Catalog::read(game)?.with_mods(projects.iter().map(|project| project.bundle))?;
    let generated = catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut bindings = BTreeMap::new();
    let mut members = Vec::new();
    let mut wrapper_deps = toml::Table::new();
    let deps = game.join("deps");
    let packages: BTreeMap<_, _> = projects
        .iter()
        .map(|project| {
            (
                project.bundle.manifest.id.as_str(),
                project.package.as_str(),
            )
        })
        .collect();
    for (position, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        let bundle = project.bundle;
        let id = &bundle.manifest.id;
        // Validate the whole local manifest closure without copying or executing its code.
        let inert = out.join("validation").join(id);
        super::registry_generate::stage(bundle, &inert)?;
        let relative = format!("native-api/{id}");
        let dest = out.join(&relative);
        super::copy_bundle(bundle, &dest)?;
        super::mod_points::stage(&dest, &catalog.wire)?;
        let mut todo = vec![PathBuf::new()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(local) = todo.pop() {
            if !seen.insert(local.clone()) {
                continue;
            }
            let path = dest.join(&local).join("Cargo.toml");
            let mut manifest: toml::Table = fs::read_to_string(&path)
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            super::registry_generate::rewrite_dependencies(
                bundle,
                &local,
                &mut manifest,
                &mut todo,
            )?;
            if let Some(targets) = manifest
                .get_mut("target")
                .and_then(toml::Value::as_table_mut)
            {
                for (_, target) in targets.iter_mut() {
                    if let Some(table) = target.as_table_mut() {
                        super::registry_generate::rewrite_dependencies(
                            bundle, &local, table, &mut todo,
                        )?;
                    }
                }
            }
            if local.as_os_str().is_empty() {
                let targets: Vec<_> = declared[index]
                    .iter()
                    .map(|dependency| dependencies::Package {
                        dependency,
                        package: packages.get(dependency.id.as_str()).copied(),
                    })
                    .collect();
                let enabled = dependencies::rewrite(id, &mut manifest, &targets)
                    .map_err(|e| e.to_string())?;
                let mut dependency =
                    toml::toml! { path = (relative.clone()) package = (project.package.clone()) };
                if !enabled.is_empty() {
                    dependency.insert(
                        "features".into(),
                        enabled
                            .into_iter()
                            .map(toml::Value::String)
                            .collect::<Vec<_>>()
                            .into(),
                    );
                }
                wrapper_deps.insert(format!("mod_{position}"), dependency.into());
            }
            fs::write(path, manifest.to_string()).map_err(|e| e.to_string())?;
        }
        let mut externs = BTreeMap::new();
        for (name, package) in project.api_dependencies() {
            if package != "internals" {
                externs.insert(name, prebuilt::library(&deps, package)?);
            }
        }
        if internals::needed(&bundle.manifest)
            || project
                .api_dependencies()
                .iter()
                .any(|(_, package)| *package == "internals")
        {
            let facade = &generated[id.as_str()];
            let facade_relative = internals::relative(id);
            let facade_dir = out.join(&facade_relative);
            fs::create_dir_all(facade_dir.join("src")).map_err(|e| e.to_string())?;
            fs::write(facade_dir.join("src/lib.rs"), &facade.source).map_err(|e| e.to_string())?;
            let facade_package = internals::package(id);
            let manifest = toml::toml! { [package] name = (facade_package.clone()) version = "0.0.0" edition = "2024" };
            fs::write(facade_dir.join("Cargo.toml"), manifest.to_string())
                .map_err(|e| e.to_string())?;
            let mut facade_externs = BTreeMap::new();
            for name in facade
                .crates
                .iter()
                .map(String::as_str)
                .chain(["horizon_client", "mod_points"])
            {
                facade_externs.insert(
                    name.to_owned(),
                    prebuilt::facade_library(&deps, name, &sdk)?,
                );
            }
            bindings.insert(
                facade_dir,
                registry_rustc::Bindings {
                    externs: facade_externs,
                    link_args: Vec::new(),
                },
            );
            members.push(facade_relative.clone());
            let mut manifest: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            let dependencies = manifest
                .entry("dependencies")
                .or_insert_with(|| toml::Table::new().into())
                .as_table_mut()
                .ok_or("invalid dependencies")?;
            let aliases: Vec<_> = project
                .api_dependencies()
                .into_iter()
                .filter(|(_, package)| *package == "internals")
                .map(|(name, _)| name)
                .collect();
            for alias in aliases {
                dependencies.insert(alias, toml::toml! { path = (format!("../../{facade_relative}")) package = (facade_package.clone()) }.into());
            }
            fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
            wrapper_deps.insert(
                format!("mod_{position}_internals"),
                toml::toml! { path = facade_relative package = facade_package }.into(),
            );
        }
        bindings.insert(
            dest,
            registry_rustc::Bindings {
                externs,
                link_args: Vec::new(),
            },
        );
        members.push(relative);
    }
    let manifests: Vec<_> = resolution
        .order
        .iter()
        .map(|&index| &projects[index].bundle.manifest)
        .collect();
    fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    fs::write(
        out.join("src/main.rs"),
        api::main_rs(
            &api::template(&game.join(prebuilt::TEMPLATE))?,
            &manifests,
            &catalog,
        ),
    )
    .map_err(|e| e.to_string())?;
    let mut manifest = toml::toml! { [package] name = "horizon-native-registry-game" version = "0.0.0" edition = "2024" [workspace] resolver = "2" };
    manifest
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .expect("generated workspace")
        .insert(
            "members".into(),
            members
                .into_iter()
                .map(toml::Value::String)
                .collect::<Vec<_>>()
                .into(),
        );
    manifest.insert("dependencies".into(), wrapper_deps.into());
    fs::write(out.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
    let declared_paths = projects
        .iter()
        .map(|project| {
            (
                project.bundle.manifest.id.as_str(),
                project
                    .bundle
                    .manifest
                    .dependencies
                    .keys()
                    .map(String::as_str)
                    .collect(),
            )
        })
        .collect();
    let wanted = super::crates::wanted(out, &declared_paths)?;
    let mut plan = registry_plan::Plan::read(lock, projects, &wanted)?;
    plan.authorize_game_patches(&patches);
    plan.fetch_with_patches(rust, &out.join("fetch"), &patches)?;
    registry_plan::apply_patches(out, &patches)?;
    fs::write(
        out.join("Cargo.lock"),
        plan.mods.seed("version=4\npackage=[]\n")?,
    )
    .map_err(|e| e.to_string())?;
    let metadata = super::registry_inventory::read(rust, out)
        .map_err(|error| plan.explain_resolution(error))?;
    plan.check_mods(&super::registry_lock::RegistryLock::parse(
        &super::read_lock(out)?,
    )?)?;
    plan.allowed.check_resolution(
        &super::registry_lock::RegistryLock::parse(&super::read_lock(out)?)?,
        &super::registry_lock::RegistryLock::default(),
    )?;
    let mut game_names = plan.game.game_package_names();
    for registry in patches.values() {
        for name in registry.as_table().expect("validated SDK patches").keys() {
            game_names.remove(name);
        }
    }
    super::crates::check_game_graph_names(out, &super::read_lock(out)?, &game_names)?;
    let inventory = metadata.inventory(&super::registry_lock::RegistryLock::default(), links)?;
    if inventory.added_crates > 0 && !options.consent.is_some_and(|ask| ask(&inventory)) {
        return Err(crate::i18n::tr("Registry dependency compilation was not approved").into());
    }
    let mut externs = BTreeMap::new();
    for name in [
        "horizon_client",
        "mod_api",
        "mod_points",
        "horizon_mod_package",
    ] {
        externs.insert(name.to_owned(), prebuilt::library(&deps, name)?);
    }
    let mut link_args = sdk.link_args;
    if cfg!(target_os = "linux") {
        link_args.push("-Wl,--disable-new-dtags".into());
        link_args.push("-Wl,-rpath,$ORIGIN".into());
    }
    if cfg!(target_os = "macos") {
        link_args.push("-Wl,-rpath,@executable_path".into());
    }
    bindings.insert(
        out.to_path_buf(),
        registry_rustc::Bindings { externs, link_args },
    );
    let target_dir = out.join("target");
    let mut command = super::cargo(rust, out, "build");
    command
        .args([
            "--offline",
            "--locked",
            "--release",
            "--target",
            rust.host.target,
        ])
        .arg("--target-dir")
        .arg(&target_dir)
        .arg("--message-format=json-render-diagnostics");
    let wrapper = registry_rustc::Plan {
        rustc: rust.rustc(),
        target: rust.host.target.into(),
        cache_wrapper: std::env::var_os("RUSTC_WRAPPER").map(PathBuf::from),
        workspace_wrapper: std::env::var_os("RUSTC_WORKSPACE_WRAPPER").map(PathBuf::from),
        search: vec![deps],
        native: game.join("native"),
        crates: bindings,
    };
    wrapper.configure(&mut command, &out.join("rustc-plan.json"), options.adapter)?;
    let messages = out.join("cargo-messages.jsonl");
    let result = super::run_checked(
        command,
        "Compiling registry mods and linking the prebuilt game",
        |_| {},
        None,
        Some(fs::File::create(&messages).map_err(|e| e.to_string())?),
    );
    let outputs = super::registry_runtime::Outputs::read(&messages)?;
    super::append_log(&fs::read_to_string(&messages).map_err(|e| e.to_string())?)?;
    if let Err(failure) = result {
        let mut error = format!("{}\n{}", failure.error, outputs.errors.join("\n"));
        if error.contains("E0277") || error.contains("E0308") {
            error.push_str(&format!("\n\n{}", crate::i18n::tr("SDK values keep the SDK's Rust crate identities. A registry-built copy of a trait or type is not interchangeable, even at the same version. Type mismatches are refused; SDK libraries are never substituted.")));
        }
        return Err(error);
    }
    let binary = target_dir
        .join(rust.host.target)
        .join("release")
        .join(if cfg!(windows) {
            "horizon-native-registry-game.exe"
        } else {
            "horizon-native-registry-game"
        });
    let mut runtime = prebuilt::game_dylibs(&game.join("deps"))?;
    runtime.push(prebuilt::standard_library(rust)?);
    for library in outputs.runtime(&metadata.runtime_packages()?, &target_dir)? {
        if let Some(existing) = runtime
            .iter()
            .find(|existing| existing.file_name() == library.file_name())
        {
            if fs::read(existing).map_err(|e| e.to_string())?
                != fs::read(&library).map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "Native runtime output {} conflicts with an immutable SDK library",
                    library.display()
                ));
            }
        } else {
            runtime.push(library);
        }
    }
    Ok((binary, runtime))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires an explicitly prepared private SDK, mod and launcher executable"]
    fn real_sdk_registry_compile() {
        let required =
            |name| PathBuf::from(std::env::var_os(name).expect("explicit fixture path required"));
        let game = required("HORIZON_TEST_REGISTRY_SDK");
        let folder = required("HORIZON_TEST_REGISTRY_MOD");
        let out = required("HORIZON_TEST_REGISTRY_OUT");
        let adapter = required("HORIZON_TEST_REGISTRY_ADAPTER");
        let bundle = super::super::bundle_in(&folder).unwrap().unwrap();
        let projects = [api::Project::read(&bundle).unwrap()];
        let declared = [Vec::new()];
        let inputs = [dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            dependencies: &declared[0],
            has_api: true,
        }];
        let resolution = dependencies::resolve(&inputs).unwrap();
        let rust = crate::rust_toolchain::install(|_| {}).unwrap();
        let approve = std::env::var_os("HORIZON_TEST_REGISTRY_DECLINE").is_none();
        let result = compile(
            &rust,
            &game,
            &out,
            &projects,
            &declared,
            &resolution,
            Options {
                consent: Some(&|inventory| {
                    eprintln!("{}\n{}", inventory.label(), inventory.detail());
                    approve
                }),
                adapter: &adapter,
            },
        );
        if approve {
            let (binary, _) = result.unwrap();
            assert!(binary.is_file());
            eprintln!("compiled wrapper {}", binary.display());
        } else {
            assert!(result.unwrap_err().contains("not approved"));
            assert!(!out.join("cargo-messages.jsonl").exists());
            assert!(!out.join("target").exists());
            eprintln!(
                "detailed consent declined before target compilation or Cargo build messages"
            );
        }
    }
}
