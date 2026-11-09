//! The launcher's generated executable for trait mods.
use super::{internals, Bundle};
use std::fs;
use std::path::Path;

pub(super) fn read(path: &Path) -> Result<toml::Table, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.parse().map_err(|e| format!("{}: {e}", path.display()))
}

fn write(path: &Path, value: &toml::Table) -> Result<(), String> {
    let text = toml::to_string(value).map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(super) const PACKAGE: &str = "horizon-native-game";
/// Where the generated executable's package is staged, in the workspace.
const GAME: &str = "crates/native-game";

fn path_dep(path: &str) -> toml::Table {
    toml::Table::from_iter([("path".into(), path.into())])
}

const MAIN_TEMPLATE: &str = "crates/horizon-client/src/native_main.rs.template";

pub(super) fn template(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).ok()
        .filter(|template| ["/* MOD_ENTRIES */", "/* MOD_SLOTS */", "/* BUILT_MODS */"].iter()
            .all(|marker| template.matches(marker).count() == 1))
        .ok_or_else(|| crate::i18n::tr("This game source has no native inventory entry template. Update Horizon and rebuild the mods against this game.").to_string())
}

pub(super) fn main_rs(
    template: &str,
    mods: &[&horizon_mod_package::Manifest],
    listed: &[super::inject::ListedPoint],
    sites: &[super::sites::Site],
) -> String {
    let entries = mods
        .iter()
        .enumerate()
        .map(|(i, m)| format!("        {:?} => mod_{i}::hooks,\n", m.id))
        .collect::<String>();
    let inventory = mods.iter().map(|m| format!(
        "    horizon_mod_package::runtime::BuiltMod {{ id: {:?}, version: {:?}, side: horizon_mod_package::Side::{:?} }},\n",
        m.id, m.version, m.side,
    )).collect::<String>();
    let raw = mods.iter().enumerate().flat_map(|(i, manifest)| {
        super::slots::points(listed, manifest).map(move |injection| {
            let point = listed.iter().find(|point| internals::plain_path(&point.target) == internals::plain_path(&injection.target)).expect("raw target checked");
            let place = point.place(injection.at_return);
            let entry = manifest.inject.iter().find(|entry| entry.id == injection.id)
                .expect("unified point entry");
            let order = format!(".ordered({}, {:?})", entry.priority, entry.id);
            format!("        mod_{i}_internals::raw::{place}({:?}, mod_{i}::{}){order}, // injection {:?}\n", manifest.id, injection.method, injection.id)
        })
    }).collect::<String>();
    let raw = raw
        + &mods
            .iter()
            .enumerate()
            .flat_map(|(i, manifest)| {
                let api_points = listed
                    .iter()
                    .map(super::inject::ListedPoint::emitted)
                    .collect::<Vec<_>>();
                horizon_mod_point_emit::api(manifest, &api_points, &format!("mod_{i}::"))
                    .expect("API slots checked")
                    .into_iter()
                    .map(|expression| format!("        {expression},\n"))
            })
            .collect::<String>();
    let raw = raw
        + &mods
            .iter()
            .enumerate()
            .flat_map(|(index, manifest)| {
                super::slots::sites(sites, manifest).map(move |(entry, site)| {
                    let adapter =
                        super::sites::adapter(entry.kind, &format!("mod_{index}::{}", entry.with));
                    format!(
                        "        mod_{index}_internals::raw::{}({:?}, {}, {:?}, {adapter}),\n",
                        site.constructor(),
                        manifest.id,
                        entry.priority,
                        entry.id
                    )
                })
            })
            .collect::<String>();
    let markers = [
        ("/* MOD_SLOTS */", &raw),
        ("/* MOD_ENTRIES */", &entries),
        ("/* BUILT_MODS */", &inventory),
    ];
    template
        .split_inclusive("*/")
        .map(|chunk| {
            markers
                .iter()
                .find_map(|(marker, value)| {
                    chunk
                        .strip_suffix(*marker)
                        .map(|prefix| prefix.trim_end_matches([' ', '\t']).to_owned() + value)
                })
                .unwrap_or_else(|| chunk.to_owned())
        })
        .collect::<String>()
}

/// Parsed once from the pipeline's immutable incoming copy, then reused for staging.
pub(super) struct Project<'a> {
    pub(super) bundle: &'a Bundle,
    pub(super) cargo: toml::Table,
    pub(super) package: String,
    api_deps: Vec<ApiDependency>,
}

/// The mod API's package, the one crate every mod depends on.
pub(super) const API_PACKAGE: &str = "horizon-mod-api";
/// The injection points' package, which a mod that listens at points also depends on (#589).
pub(super) const POINTS_PACKAGE: &str = "horizon-mod-points";

/// The per-mod facade's package, rewritten to its generated crate.
pub(super) const INTERNALS_PACKAGE: &str = "horizon-internals";
/// Each game package a mod may depend on, with its library's name.
pub(super) const PACKAGES: [(&str, &str); 3] = [
    (API_PACKAGE, "mod_api"),
    (POINTS_PACKAGE, "mod_points"),
    (INTERNALS_PACKAGE, "internals"),
];

/// A dependency on one of [`PACKAGES`], under `[dependencies]` or a `[target.*]` table.
struct ApiDependency {
    target: Option<String>,
    key: String,
    package: &'static str,
    library: &'static str,
}

impl ApiDependency {
    fn table_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Table {
        match &self.target {
            Some(target) => cargo["target"][target.as_str()]["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
            None => cargo["dependencies"]
                .as_table_mut()
                .expect("parsed dependencies"),
        }
    }

    fn value_mut<'a>(&self, cargo: &'a mut toml::Table) -> &'a mut toml::Value {
        self.table_mut(cargo)
            .get_mut(&self.key)
            .expect("parsed dependency")
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub(super) enum ProjectError {
    Cargo { mod_id: String, reason: String },
    PackageName { mod_id: String },
    ApiDependency { mod_id: String },
    GameDependency { mod_id: String, key: String },
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::i18n::trf;
        let message = match self {
            Self::Cargo { mod_id, reason } => {
                trf("{mod}: {error}", &[("mod", mod_id), ("error", reason)])
            }
            Self::PackageName { mod_id } => {
                trf("{mod}: Cargo.toml has no package name.", &[("mod", mod_id)])
            }
            Self::ApiDependency { mod_id } => trf(
                "{mod}: Cargo.toml has no horizon-mod-api dependency.",
                &[("mod", mod_id)],
            ),
            Self::GameDependency { mod_id, key } => trf(
                "E_GAME_DEPENDENCY: {mod}: Cargo.toml depends on the game ({key}). Declare game internals through horizon-internals instead.",
                &[("mod", mod_id), ("key", key)],
            ),
        };
        f.write_str(&message)
    }
}
impl std::error::Error for ProjectError {}

impl<'a> Project<'a> {
    pub(super) fn read(bundle: &'a Bundle) -> Result<Self, ProjectError> {
        let mod_id = || bundle.manifest.id.clone();
        let cargo = read(&bundle.dir.join("Cargo.toml")).map_err(|reason| ProjectError::Cargo {
            mod_id: mod_id(),
            reason,
        })?;
        let package = cargo
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| ProjectError::PackageName { mod_id: mod_id() })?
            .to_owned();
        for table in std::iter::once(&cargo).chain(
            cargo
                .get("target")
                .and_then(toml::Value::as_table)
                .into_iter()
                .flat_map(|targets| targets.values())
                .filter_map(toml::Value::as_table),
        ) {
            for kind in [
                "dependencies",
                "dev-dependencies",
                "build-dependencies",
                "dev_dependencies",
                "build_dependencies",
            ] {
                for (key, value) in table
                    .get(kind)
                    .and_then(toml::Value::as_table)
                    .into_iter()
                    .flatten()
                {
                    if value
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .unwrap_or(key)
                        == "horizon-client"
                    {
                        return Err(ProjectError::GameDependency {
                            mod_id: mod_id(),
                            key: key.clone(),
                        });
                    }
                }
            }
        }
        let mut api_deps = Vec::new();
        for (target, dependencies) in dependency_tables(&cargo) {
            for (key, value) in dependencies {
                let package = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                if let Some(&(package, library)) =
                    PACKAGES.iter().find(|(name, _)| *name == package)
                {
                    api_deps.push(ApiDependency {
                        target: target.map(str::to_owned),
                        key: key.clone(),
                        package,
                        library,
                    });
                    continue;
                }
            }
        }
        if !api_deps
            .iter()
            .any(|api| api.target.is_none() && api.package == API_PACKAGE)
        {
            return Err(ProjectError::ApiDependency { mod_id: mod_id() });
        }
        Ok(Self {
            bundle,
            cargo,
            package,
            api_deps,
        })
    }

    /// The manifest's dependency tables: `[dependencies]` (no target), then each
    /// `[target.<cfg>.dependencies]`.
    pub(super) fn dependency_tables(
        &self,
    ) -> impl Iterator<Item = (Option<&str>, &toml::Table)> + '_ {
        dependency_tables(&self.cargo)
    }

    /// The names the mod's code calls the game's packages by (`mod_api`, `mod_points`, or a
    /// renamed dependency's key), each with its package's library name.
    pub(super) fn api_dependencies(&self) -> std::collections::BTreeSet<(String, &'static str)> {
        self.api_deps
            .iter()
            .map(|api| {
                let name = if api.key == api.package {
                    api.library.to_owned()
                } else {
                    api.key.replace('-', "_")
                };
                (name, api.library)
            })
            .collect()
    }
}

fn dependency_tables(cargo: &toml::Table) -> impl Iterator<Item = (Option<&str>, &toml::Table)> {
    let targets = cargo
        .get("target")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flatten()
        .map(|(target, config)| (Some(target.as_str()), config.get("dependencies")));
    std::iter::once((None, cargo.get("dependencies")))
        .chain(targets)
        .filter_map(|(target, table)| Some((target, table?.as_table()?)))
}

// Add API packages and the generated executable to the fresh staged workspace.
pub(super) fn stage(
    stage: &Path,
    projects: &[Project<'_>],
    declared: &[Vec<super::dependencies::Dependency>],
    resolution: &super::dependencies::Resolution,
    catalog: &internals::Catalog,
) -> Result<bool, String> {
    let mods: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| (i, &projects[i]))
        .collect();
    if mods.is_empty() {
        return Ok(false);
    }
    let facades = catalog
        .prepare(mods.iter().map(|(_, project)| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    let template = template(&stage.join(MAIN_TEMPLATE))?;
    let packages: std::collections::HashMap<_, _> = mods
        .iter()
        .map(|(_, project)| (project.bundle.manifest.id.as_str(), project.package.clone()))
        .collect();
    let mut deps = toml::Table::new();
    // The generated game's features own sound/controllers, including the server's
    // `--no-default-features`; its client dependency never enables them independently.
    // `horizon-threads` stays for games pinned before the allocator moved into
    // `horizon-client`'s library: their template still names `horizon_threads::ALLOCATOR`.
    for name in [
        "horizon-client",
        "horizon-mod-api",
        "horizon-mod-points",
        "horizon-threads",
        "horizon-mod-package",
    ] {
        let mut dependency = path_dep(&format!("../{name}"));
        if name == "horizon-client" {
            dependency.insert("default-features".into(), false.into());
        }
        deps.insert(name.into(), toml::Value::Table(dependency));
    }
    let mut members = Vec::new();
    for (i, (index, project)) in mods.iter().enumerate() {
        let bundle = project.bundle;
        let relative = format!("native-api/{}", bundle.manifest.id);
        let dest = stage.join(&relative);
        super::copy_bundle(bundle, &dest)?;
        let path = dest.join("Cargo.toml");
        let mut manifest = project.cargo.clone();
        let package = project.package.clone();
        for api in &project.api_deps {
            let dependency = api.value_mut(&mut manifest);
            let mut rewritten = path_dep(&if api.package == INTERNALS_PACKAGE {
                format!("../../{}", internals::relative(&bundle.manifest.id))
            } else {
                format!("../../crates/{}", api.package)
            });
            if let Some(old) = dependency.as_table() {
                // A renamed dependency also needs its package name.
                for key in ["package", "features", "default-features"] {
                    if let Some(value) = old.get(key) {
                        rewritten.insert(key.into(), value.clone());
                    }
                }
            }
            if api.package == INTERNALS_PACKAGE {
                rewritten.insert(
                    "package".into(),
                    internals::package(&bundle.manifest.id).into(),
                );
            }
            *dependency = toml::Value::Table(rewritten);
            // Cargo treats a key differing from `package` as the crate's alias. Preserve the
            // facade's library name when giving its per-mod package a unique name.
            if api.package == INTERNALS_PACKAGE && api.key == INTERNALS_PACKAGE {
                let table = api.table_mut(&mut manifest);
                let value = table.remove(&api.key).expect("rewritten dependency");
                table.insert("internals".into(), value);
            }
        }
        let targets: Vec<_> = declared[*index]
            .iter()
            .map(|dependency| super::dependencies::Package {
                dependency,
                package: packages.get(dependency.id.as_str()).map(String::as_str),
            })
            .collect();
        let enabled = super::dependencies::rewrite(&bundle.manifest.id, &mut manifest, &targets)
            .map_err(|e| e.to_string())?;
        write(&path, &manifest)?;
        let mut dep = toml::Table::new();
        dep.insert("path".into(), format!("../../{relative}").into());
        dep.insert("package".into(), package.into());
        if !enabled.is_empty() {
            dep.insert(
                "features".into(),
                toml::Value::Array(enabled.into_iter().map(toml::Value::String).collect()),
            );
        }
        deps.insert(format!("mod_{i}"), toml::Value::Table(dep));
        if internals::needed(&bundle.manifest)
            || project
                .api_deps
                .iter()
                .any(|dep| dep.package == INTERNALS_PACKAGE)
        {
            internals::stage(stage, project, &facades[bundle.manifest.id.as_str()])?;
            let facade = internals::relative(&bundle.manifest.id);
            let mut dependency = path_dep(&format!("../../{facade}"));
            dependency.insert(
                "package".into(),
                internals::package(&bundle.manifest.id).into(),
            );
            deps.insert(format!("mod_{i}_internals"), toml::Value::Table(dependency));
            members.push(facade);
        }
        members.push(relative);
    }
    // Beside horizon-client, so its build script (the Windows stack size and icon, for the
    // `horizon-client` binary this package owns now) finds `packaging/` where it looks.
    let game = stage.join(GAME);
    fs::create_dir_all(game.join("src")).map_err(|e| e.to_string())?;
    let mut manifest = toml::Table::new();
    manifest.insert(
        "package".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("name".into(), PACKAGE.into()),
            ("version".into(), "0.1.0".into()),
            ("edition".into(), "2024".into()),
            ("publish".into(), false.into()),
            ("build".into(), "../horizon-client/build.rs".into()),
        ])),
    );
    // This sibling package uses the client's build.rs, so its build dependencies apply too;
    // relative paths still point to the same crates from crates/native-game.
    let client = read(&stage.join("crates/horizon-client/Cargo.toml"))?;
    if let Some(dependencies) = client.get("build-dependencies") {
        manifest.insert("build-dependencies".into(), dependencies.clone());
    }
    let windows = toml::Table::from_iter([(
        "build-dependencies".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "winresource".into(),
            "0.1".into(),
        )])),
    )]);
    manifest.insert(
        "target".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "cfg(windows)".into(),
            toml::Value::Table(windows),
        )])),
    );
    manifest.insert(
        "bin".into(),
        toml::Value::Array(vec![toml::Value::Table(toml::Table::from_iter([
            ("name".into(), "horizon-client".into()),
            ("path".into(), "src/main.rs".into()),
        ]))]),
    );
    manifest.insert("dependencies".into(), toml::Value::Table(deps));
    manifest.insert(
        "features".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "default".into(),
            toml::Value::Array(vec!["horizon-client/default".into()]),
        )])),
    );
    write(&game.join("Cargo.toml"), &manifest)?;
    let manifests: Vec<_> = mods
        .iter()
        .map(|(_, project)| &project.bundle.manifest)
        .collect();
    fs::write(
        game.join("src/main.rs"),
        main_rs(&template, &manifests, &catalog.points, &catalog.sites),
    )
    .map_err(|e| e.to_string())?;
    let root = stage.join("Cargo.toml");
    let mut workspace = read(&root)?;
    let list = workspace
        .get_mut("workspace")
        .and_then(|w| w.get_mut("members"))
        .and_then(toml::Value::as_array_mut)
        .ok_or("workspace has no members")?;
    members.push(GAME.into());
    list.extend(members.into_iter().map(toml::Value::String));
    write(&root, &workspace)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example-mod'\nversion='9.0.0'\nedition='2024'\n[dependencies]\nhorizon-mod-api='0.1'\n").unwrap();
        dir
    }

    #[test]
    fn projects_refuse_a_game_dependency_and_need_the_mod_api() {
        let dir = fixture();
        for (dependencies, game) in [
            (
                "[dependencies]\nhorizon-mod-api='0.1'\nhorizon-client='0.1'",
                Some("horizon-client"),
            ),
            (
                "[dependencies.api]\npackage='horizon-mod-api'\nversion='0.1'\n[target.'cfg(unix)'.dependencies.game]\npackage='horizon-client'\nversion='0.1'",
                Some("game"),
            ),
            ("[dependencies]\nserde='1'", None),
            (
                "[target.'cfg(unix)'.dependencies]\nhorizon-mod-api='0.1'",
                None,
            ),
        ] {
            let text = format!("[package]\nname='example-mod'\nversion='1.0.0'\n{dependencies}\n");
            fs::write(dir.join("Cargo.toml"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let error = Project::read(&bundle).err().unwrap();
            match game {
                Some(game) => assert!(
                    matches!(&error, ProjectError::GameDependency { mod_id, key } if mod_id == "example" && key == game),
                    "{error}"
                ),
                None => assert!(
                    matches!(&error, ProjectError::ApiDependency { .. }),
                    "{error}"
                ),
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generates_only_tool_contract_tables_without_mutating_package() {
        let dir = fixture();
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid='test'\nfile='crates/horizon-client/src/game.rs'\nname='support'\nsource='src/lib.rs'\n";
        fs::write(&path, &text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(bundle.needs_mixins());
        super::super::package::tool_manifest(&bundle).unwrap();
        let value = read(&dir.join(super::super::MANIFEST)).unwrap();
        assert!(!value.contains_key("api"));
        assert!(!value.contains_key("native"));
        assert!(value.contains_key("module"));
        assert!(!value.contains_key("injection"));
        assert_eq!(value["mod"]["id"].as_str(), Some("example"));
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_main_keeps_mod_order() {
        let template = include_str!("../../../crates/horizon-client/src/native_main.rs.template");
        let dir = fixture();
        let mut base = horizon_mod_package::Package::read(&dir).unwrap().manifest;
        base.id = "base".into();
        base.version = "1.2.3".into();
        base.side = horizon_mod_package::Side::Both;
        let mut extra = base.clone();
        extra.id = "extra".into();
        extra.version = "2.0.0".into();
        extra.side = horizon_mod_package::Side::Client;
        let mods = [&base, &extra];
        for count in 0..=2 {
            let text = main_rs(template, &mods[..count], &[], &[]);
            let padded = ["/* MOD_ENTRIES */", "/* MOD_SLOTS */", "/* BUILT_MODS */"]
                .into_iter()
                .fold(template.to_owned(), |text, marker| {
                    text.replace(marker, &format!("    {marker}"))
                });
            assert_eq!(main_rs(&padded, &mods[..count], &[], &[]), text);
            let facts: Vec<_> = text
                .lines()
                .filter(|line| line.contains(" => mod_"))
                .collect();
            assert_eq!(
                facts,
                &[
                    "        \"base\" => mod_0::hooks,",
                    "        \"extra\" => mod_1::hooks,",
                ][..count]
            );
            let inventory: Vec<_> = text
                .lines()
                .filter(|line| line.contains("BuiltMod {"))
                .collect();
            assert_eq!(
                inventory,
                &[
                    "    horizon_mod_package::runtime::BuiltMod { id: \"base\", version: \"1.2.3\", side: horizon_mod_package::Side::Both },",
                    "    horizon_mod_package::runtime::BuiltMod { id: \"extra\", version: \"2.0.0\", side: horizon_mod_package::Side::Client },",
                ][..count]
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shared_api_emitter_preserves_native_slot_bytes() {
        use horizon_mod_package::manifest::inject::{Access, Inject, Kind};
        let dir = fixture();
        let mut manifest = horizon_mod_package::Package::read(&dir).unwrap().manifest;
        manifest.id = "fixture".into();
        manifest.inject = [Kind::Head, Kind::Return, Kind::Replace]
            .into_iter()
            .enumerate()
            .map(|(index, kind)| Inject {
                id: format!("listen_{index}"),
                at: "game::Game<'_>::tick".into(),
                kind,
                with: format!("listen_{index}"),
                access: Access::Api,
                priority: 12,
                hash: "pin".into(),
                call: None,
                nth: None,
                ctx: None,
            })
            .collect();
        let listed = [super::super::inject::ListedPoint {
            name: "tick".into(),
            private: false,
            record_name: None,
            target: "game::Game::tick".into(),
            raw: true,
            at_return: true,
            api_refusal: None,
            returns: "cancel".into(),
            host: "both".into(),
            raw_shape: None,
            return_ty: Some("()".into()),
            listener: None,
            return_listener: None,
        }];
        let output = main_rs(
            "/* MOD_ENTRIES */START\n/* MOD_SLOTS */END/* BUILT_MODS */",
            &[&manifest],
            &listed,
            &[],
        );
        let slots = output
            .split_once("START\n")
            .unwrap()
            .1
            .split_once("END")
            .unwrap()
            .0;
        assert_eq!(slots, concat!(
            "        mod_points::listeners::tick(\"fixture\", 12, \"listen_0\", mod_0::listen_0),\n",
            "        mod_points::listeners::tick_return(\"fixture\", 12, \"listen_1\", mod_0::listen_1),\n",
            "        mod_points::replace!(tick, cancel, host, \"fixture\", 12, \"listen_2\", mod_0::listen_2),\n",
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stages_api_without_a_dependency_cycle() {
        let dir = fixture();
        let cargo = fs::read_to_string(dir.join("Cargo.toml")).unwrap()
            + "[target.'cfg(unix)'.dependencies.api]\npackage = 'horizon-mod-api'\nversion = '0.1'\nfeatures = ['testing']\n";
        fs::write(dir.join("Cargo.toml"), cargo).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let stage_dir = dir.with_extension("workspace");
        let _ = fs::remove_dir_all(&stage_dir);
        fs::create_dir_all(stage_dir.join("crates/horizon-client/src")).unwrap();
        fs::write(stage_dir.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(
            stage_dir.join("crates/horizon-client/Cargo.toml"),
            "[package]\nname = 'horizon-client'\nversion = '0.1.0'\n",
        )
        .unwrap();
        let resolution = super::super::dependencies::Resolution { order: vec![0] };
        let project = Project::read(&bundle).unwrap();
        let err = stage(
            &stage_dir,
            std::slice::from_ref(&project),
            &[vec![]],
            &resolution,
            &internals::Catalog::default(),
        )
        .unwrap_err();
        assert!(err.contains("Update Horizon"), "{err}");
        fs::write(
            stage_dir.join(MAIN_TEMPLATE),
            "fn main() { /* MOD_ENTRIES */ }",
        )
        .unwrap();
        assert!(stage(
            &stage_dir,
            std::slice::from_ref(&project),
            &[vec![]],
            &resolution,
            &internals::Catalog::default(),
        )
        .is_err());
        assert!(!stage_dir.join("native-api").exists());
        fs::write(
            stage_dir.join(MAIN_TEMPLATE),
            include_str!("../../../crates/horizon-client/src/native_main.rs.template"),
        )
        .unwrap();
        assert!(stage(
            &stage_dir,
            std::slice::from_ref(&project),
            &[vec![]],
            &resolution,
            &internals::Catalog::default(),
        )
        .unwrap());
        let root = read(&stage_dir.join("Cargo.toml")).unwrap();
        let members = root["workspace"]["members"].as_array().unwrap();
        assert!(members
            .iter()
            .any(|v| v.as_str() == Some("native-api/example")));
        assert!(members.iter().any(|v| v.as_str() == Some(GAME)));
        let engine = read(&stage_dir.join("native-api/example/Cargo.toml")).unwrap();
        assert_eq!(
            engine["dependencies"]["horizon-mod-api"]["path"].as_str(),
            Some("../../crates/horizon-mod-api")
        );
        let platform = &engine["target"]["cfg(unix)"]["dependencies"]["api"];
        assert_eq!(
            platform["path"].as_str(),
            Some("../../crates/horizon-mod-api")
        );
        assert_eq!(platform["package"].as_str(), Some("horizon-mod-api"));
        assert_eq!(platform["features"][0].as_str(), Some("testing"));
        let game = read(&stage_dir.join(GAME).join("Cargo.toml")).unwrap();
        assert_eq!(
            game["dependencies"]["horizon-mod-package"]["path"].as_str(),
            Some("../horizon-mod-package")
        );
        assert!(!game["dependencies"]
            .as_table()
            .unwrap()
            .contains_key("horizon-mods"));
        assert_eq!(
            game["dependencies"]["mod_0"]["package"].as_str(),
            Some("example-mod")
        );
        assert_eq!(
            game["dependencies"]["mod_0"]["path"].as_str(),
            Some("../../native-api/example")
        );
        assert_eq!(
            game["package"]["build"].as_str(),
            Some("../horizon-client/build.rs")
        );
        assert!(stage_dir
            .join(GAME)
            .join(
                game["dependencies"]["horizon-client"]["path"]
                    .as_str()
                    .unwrap()
            )
            .ends_with("crates/native-game/../horizon-client"));
        // The game's default features are the generated package's alone, so a server build's
        // `--no-default-features` reaches horizon-client.
        assert_eq!(
            game["dependencies"]["horizon-client"]["default-features"].as_bool(),
            Some(false)
        );
        assert_eq!(
            game["features"]["default"][0].as_str(),
            Some("horizon-client/default")
        );
        let main = || fs::read_to_string(stage_dir.join(GAME).join("src/main.rs")).unwrap();
        assert!(main().contains(r#""example" => mod_0::hooks,"#));
        fs::write(stage_dir.join("crates/horizon-client/Cargo.toml"), "[package]\nname = 'horizon-client'\nversion = '0.1.0'\n[build-dependencies]\nhorizon-mod-api-gen = { path = '../horizon-mod-api-gen' }\n").unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let project = Project::read(&bundle).unwrap();
        stage(
            &stage_dir,
            &[project],
            &[vec![]],
            &resolution,
            &internals::Catalog::default(),
        )
        .unwrap();
        assert!(main().contains(r#""example" => mod_0::hooks,"#));
        let game = read(&stage_dir.join(GAME).join("Cargo.toml")).unwrap();
        assert_eq!(
            game["build-dependencies"]["horizon-mod-api-gen"]["path"].as_str(),
            Some("../horizon-mod-api-gen")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn named_source_without_an_entry_template_is_refused_before_staging() {
        let dir = fixture();
        let stage_dir = dir.with_extension("workspace");
        let _ = fs::remove_dir_all(&stage_dir);
        let source = stage_dir.join("crates/horizon-client/src");
        fs::create_dir_all(&source).unwrap();
        let resolution = super::super::dependencies::Resolution { order: vec![0] };
        for text in ["", "pub struct ModEntry { id: &'static str }"] {
            fs::write(source.join("mod_api.rs"), text).unwrap();
            let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
            let project = Project::read(&bundle).unwrap();
            let error = stage(
                &stage_dir,
                &[project],
                &[vec![]],
                &resolution,
                &internals::Catalog::default(),
            )
            .unwrap_err();
            assert!(error.contains("Update Horizon and rebuild"), "{error}");
            assert!(!stage_dir.join(GAME).exists());
            assert!(!stage_dir.join("native-api").exists());
        }
        assert!(!stage(
            &stage_dir,
            &[],
            &[],
            &super::super::dependencies::Resolution { order: vec![] },
            &internals::Catalog::default(),
        )
        .unwrap());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn api_only_skips_mixins() {
        let dir = fixture();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        assert!(!bundle.needs_mixins());
        fs::remove_dir_all(dir).unwrap();
    }
}
