//! The runtime for wasm mods. Packages declaring `[wasm]` are grouped by their `[dependencies]`
//! (the same model as native mods: required and optional, semver on `horizon-mod.toml`
//! versions, one library copy shared by its dependents). Every refusal names its reason for the
//! Mods page.
mod compile_child;
#[cfg(feature = "runtime-fixture")]
pub mod fixture;
pub mod link;
mod module_cache;
pub use compile_child::compile_child;
#[cfg(test)]
#[path = "../tests/support/no_hooks.rs"]
mod no_hooks;
mod points;
mod registration_capabilities;
mod runtime;
#[allow(
    dead_code,
    unused_variables,
    unused_mut,
    unsafe_op_in_unsafe_fn,
    clippy::all
)]
pub(crate) mod boundary {
    include!(concat!(env!("OUT_DIR"), "/wire_host_generated.rs"));
}
pub(crate) use runtime::WeakWasmMod;
pub use runtime::{CallClass, WasmHandler, WasmMod};

pub(crate) trait NoAnswer {
    fn no_answer() -> Self;
}
impl<T: Default> NoAnswer for T {
    fn no_answer() -> Self {
        T::default()
    }
}
pub(crate) fn call_refused<E: From<(mod_api::Identifier, String)>>() -> E {
    E::from((
        mod_api::Identifier::parse("horizon:wasm_mod").expect("valid constant identifier"),
        "the mod's call didn't run".into(),
    ))
}

use horizon_mod_package::dependencies;
use horizon_mod_package::manifest::{Manifest, inject::Access};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One wasm mod as installed: its manifest and its checked `mod.wasm`.
pub struct WasmPackage {
    pub manifest: Manifest,
    pub module: Vec<u8>,
}

/// Why a wasm mod doesn't load, for the Mods page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A required dependency isn't installed, or none installed matches its version.
    MissingDependency { dependency: String, version: String },
    /// The shared native/wasm resolver refused the dependency graph.
    DependencyOrder { reason: String },
    /// A dependency's installed `mod.wasm` isn't the one this mod was linked against.
    LinkedAgainst { dependency: String },
    /// Members of one group were built with different toolchains.
    Toolchain { found: String, group: String },
    /// A dependency of this mod was refused, so its group can't link.
    DependencyRefused { dependency: String },
    /// The mod declares engine internals or an engine `access = "internals"` injection.
    Internals,
    /// The package's `mod.wasm` failed its checks.
    Package(String),
    /// The group could not instantiate, initialize or run.
    Runtime(String),
    /// The mod was compiled against another generated boundary.
    Layout { built: u64, game: u64 },
    /// The mod's generated point records differ from this game's.
    PointLayout { built: u64, game: u64 },
    /// The game has no shared Rust standard library beside its executable.
    NoStd,
    /// A running group trapped or exhausted a resource limit.
    Stopped,
    /// Another thread owns a group whose import is running native code.
    Busy,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDependency {
                dependency,
                version,
            } => {
                write!(f, "needs {dependency} {version}, which isn't installed")
            }
            Self::DependencyOrder { reason } => write!(f, "{reason}"),
            Self::LinkedAgainst { dependency } => {
                write!(f, "built against another build of {dependency}")
            }
            Self::Toolchain { found, group } => {
                write!(f, "built with {found}, its dependencies with {group}")
            }
            Self::DependencyRefused { dependency } => {
                write!(f, "its dependency {dependency} didn't load")
            }
            Self::Internals => write!(f, "internals access is native-only"),
            Self::Package(why) => write!(f, "{why}"),
            Self::Runtime(why) => write!(f, "{why}"),
            Self::Layout { .. } => write!(f, "built for another version of the mod API"),
            Self::PointLayout { .. } => {
                write!(f, "built for another version of the mod points API")
            }
            Self::NoStd => write!(f, "this game has no wasm standard library member"),
            Self::Stopped => write!(f, "stopped after a trap or resource limit"),
            Self::Busy => write!(f, "its wasm group is busy"),
        }
    }
}

impl std::error::Error for Refusal {}

/// Splits the packages into groups: each mod with everything it depends on, transitively;
/// two mods sharing a dependency share its group, so a library has one copy. A mod whose
/// required dependency is missing, or whose linked hashes or toolchain disagree, is refused
/// with every mod depending on it.
fn groups(packages: &[WasmPackage]) -> (Vec<Vec<usize>>, Vec<(String, Refusal)>) {
    let by_id: BTreeMap<&str, usize> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.manifest.id.as_str(), i))
        .collect();
    let mut refused: BTreeMap<usize, Refusal> = BTreeMap::new();
    let declared: Vec<_> = packages
        .iter()
        .map(|p| dependencies::from_manifest(&p.manifest.dependencies))
        .collect();
    // Engine internals are native-only. A mod dependency's internals stay inside the group.
    for (i, package) in packages.iter().enumerate() {
        if !package.manifest.internals.is_empty()
            || package.manifest.inject.iter().any(|inject| {
                inject.access == Access::Internals
                    && package.manifest.mod_point_dependency(&inject.at).is_none()
            })
        {
            refused.insert(i, Refusal::Internals);
        }
    }
    propagate_refusals(packages, &by_id, &mut refused);
    // Refuse exactly the resolver's owners, then their dependents, preserving
    // unrelated libraries. There is no second semver or topological rule here.
    let order = loop {
        let active: Vec<_> = (0..packages.len())
            .filter(|i| !refused.contains_key(i))
            .collect();
        let inputs: Vec<_> = active
            .iter()
            .map(|&i| dependencies::Mod {
                id: &packages[i].manifest.id,
                version: &packages[i].manifest.version,
                has_api: true,
                dependencies: &declared[i],
            })
            .collect();
        match dependencies::resolve(&inputs) {
            Ok(resolution) => {
                break resolution
                    .order
                    .into_iter()
                    .map(|i| active[i])
                    .collect::<Vec<_>>();
            }
            Err(error) => {
                let reason = error.to_string();
                let owners: Vec<&str> = match &error {
                    dependencies::Error::Cycle { mods } => {
                        mods.iter().map(String::as_str).collect()
                    }
                    dependencies::Error::Duplicate(id) => vec![id],
                    dependencies::Error::Manifest { owner, .. }
                    | dependencies::Error::Missing { owner, .. }
                    | dependencies::Error::NoApi { owner, .. }
                    | dependencies::Error::CargoConflict { owner, .. }
                    | dependencies::Error::Version { owner, .. } => vec![owner],
                };
                for &i in &active {
                    if owners.contains(&packages[i].manifest.id.as_str()) {
                        let refusal = match &error {
                            dependencies::Error::Missing { target, .. }
                            | dependencies::Error::Version { target, .. } => {
                                Refusal::MissingDependency {
                                    dependency: target.clone(),
                                    version: declared[i]
                                        .iter()
                                        .find(|d| d.id == *target)
                                        .unwrap()
                                        .requirement
                                        .to_string(),
                                }
                            }
                            _ => Refusal::DependencyOrder {
                                reason: reason.clone(),
                            },
                        };
                        refused.insert(i, refusal);
                    }
                }
                propagate_refusals(packages, &by_id, &mut refused);
            }
        }
    };

    for (i, package) in packages.iter().enumerate() {
        let wasm = package
            .manifest
            .wasm
            .as_ref()
            .expect("a wasm package declares [wasm]");
        for id in package.manifest.dependencies.keys() {
            let Some(&d) = by_id.get(id.as_str()) else {
                continue;
            };
            let dep = packages[d]
                .manifest
                .wasm
                .as_ref()
                .expect("a wasm package declares [wasm]");
            if wasm
                .linked()
                .get(id)
                .is_some_and(|hash| hash != dep.sha512())
            {
                refused.entry(i).or_insert(Refusal::LinkedAgainst {
                    dependency: id.clone(),
                });
            }
            if dep.toolchain() != wasm.toolchain() {
                refused.entry(i).or_insert(Refusal::Toolchain {
                    found: wasm.toolchain().into(),
                    group: dep.toolchain().into(),
                });
            }
        }
    }
    propagate_refusals(packages, &by_id, &mut refused);
    // Union-find over the dependency edges of the mods that stay.
    let mut parent: Vec<usize> = (0..packages.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, package) in packages
        .iter()
        .enumerate()
        .filter(|(i, _)| !refused.contains_key(i))
    {
        for id in package.manifest.dependencies.keys() {
            if let Some(&d) = by_id.get(id.as_str()).filter(|d| !refused.contains_key(d)) {
                let (a, b) = (root(&mut parent, i), root(&mut parent, d));
                parent[a] = b;
            }
        }
    }
    let mut by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in (0..packages.len()).filter(|i| !refused.contains_key(i)) {
        by_root.entry(root(&mut parent, i)).or_default().push(i);
    }
    let groups = by_root
        .into_values()
        .map(|members| {
            order
                .iter()
                .copied()
                .filter(|i| members.contains(i))
                .collect()
        })
        .collect();
    let refused = refused
        .into_iter()
        .map(|(i, r)| (packages[i].manifest.id.clone(), r))
        .collect();
    (groups, refused)
}

/// A refused mod takes its dependents with it, until nothing changes.
fn propagate_refusals(
    packages: &[WasmPackage],
    by_id: &BTreeMap<&str, usize>,
    refused: &mut BTreeMap<usize, Refusal>,
) {
    loop {
        let before = refused.len();
        for (i, package) in packages.iter().enumerate() {
            if refused.contains_key(&i) {
                continue;
            }
            if let Some(id) = package.manifest.dependencies.keys().find(|id| {
                by_id
                    .get(id.as_str())
                    .is_some_and(|d| refused.contains_key(d))
            }) {
                refused.insert(
                    i,
                    Refusal::DependencyRefused {
                        dependency: id.clone(),
                    },
                );
            }
        }
        if refused.len() == before {
            break;
        }
    }
}

static REFUSED: OnceLock<Vec<(String, Refusal)>> = OnceLock::new();
static INITIALIZED_REFUSALS: OnceLock<Vec<(String, Refusal)>> = OnceLock::new();

/// The wasm mods refused at startup, each with its reason, for the Mods page.
pub fn refused() -> &'static [(String, Refusal)] {
    INITIALIZED_REFUSALS
        .get()
        .or_else(|| REFUSED.get())
        .map_or(&[], Vec::as_slice)
}

/// The current startup refusal or live trap/limit status, for a Mods-page entry.
pub fn refusal(id: &str) -> Option<Refusal> {
    refused()
        .iter()
        .find(|(owner, _)| owner == id)
        .map(|(_, why)| why.clone())
        .or_else(|| {
            MODULES
                .get()?
                .iter()
                .find(|(manifest, _)| manifest.id == id)
                .and_then(|(_, module)| module.is_stopped().then_some(Refusal::Stopped))
        })
}

/// Loads the wasm mods the player turned on in `game_dir` (and every `dev` package), once at
/// startup: the same selection as content mods (`horizon_mod_package::selection`), so a wasm mod's
/// `assets/` and `data/` load as content beside its code. Refusals are kept for [`refused`].
pub fn load_installed(game_dir: &Path, dev: &[PathBuf]) -> Loaded {
    let cache = module_cache::ModuleCache::new(game_dir.join(module_cache::CACHE_DIR));
    let enabled = horizon_mod_package::mod_list::ModList::load(game_dir).enabled;
    let mut refused: Vec<_> = horizon_mod_package::selection::package_errors(
        horizon_mod_package::find(&game_dir.join(horizon_mod_package::mod_list::DIR))
            .into_iter()
            .chain(horizon_mod_package::find(
                &game_dir.join(horizon_mod_package::mod_list::NATIVE_DIR),
            ))
            .chain(dev.iter().cloned()),
    )
    .into_iter()
    .filter(|(id, _)| enabled.contains(id) || !dev.is_empty())
    .map(|(id, why)| (id, Refusal::Package(why)))
    .collect();
    let native: Vec<_> =
        horizon_mod_package::find(&game_dir.join(horizon_mod_package::mod_list::NATIVE_DIR))
            .into_iter()
            .chain(horizon_mod_package::find(
                &game_dir.join(horizon_mod_package::mod_list::DIR),
            ))
            .chain(std::env::current_exe().ok().into_iter().flat_map(|exe| {
                horizon_mod_package::find(
                    &exe.with_file_name(horizon_mod_package::mod_list::NATIVE_DIR),
                )
            }))
            .chain(dev.iter().cloned())
            .filter_map(|path| horizon_mod_package::Package::read(&path).ok())
            .filter(|package| package.is_native())
            .collect();
    let packages: Vec<WasmPackage> =
        horizon_mod_package::selection::content_packages(game_dir, &enabled, dev)
            .filter(|selected| selected.package.manifest.wasm.is_some())
            .filter_map(|selected| {
                if let Some(reason) = native.iter().find_map(|library| {
                    horizon_mod_package::layouts::mixed_refusal(&selected.package, library)
                }) {
                    refused.push((selected.package.manifest.id, Refusal::Package(reason)));
                    return None;
                }
                match horizon_mod_package::Package::read_wasm(&selected.path) {
                    Ok(module) => Some(WasmPackage {
                        manifest: selected.package.manifest,
                        module,
                    }),
                    Err(why) => {
                        refused.push((selected.package.manifest.id, Refusal::Package(why)));
                        None
                    }
                }
            })
            .collect();
    let mut loaded = match std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::read(exe.with_file_name("horizon-wasm-std.wasm")).ok())
    {
        Some(std) => load_with_cache(&std, packages, Some(&cache)),
        None => Loaded {
            modules: Vec::new(),
            mods: Vec::new(),
            manifests: Vec::new(),
            refused: packages
                .into_iter()
                .map(|p| (p.manifest.id, Refusal::NoStd))
                .collect(),
        },
    };
    refused.append(&mut loaded.refused);
    for (id, why) in &refused {
        log::warn!("wasm mod {id} refused: {why}");
    }
    let _ = REFUSED.set(refused.clone());
    loaded.refused = refused;
    let _ = MODULES.set(
        loaded
            .manifests
            .iter()
            .cloned()
            .zip(loaded.modules.iter().cloned())
            .collect(),
    );
    loaded
}

/// Loaded wasm entries are installed in the same dispatch list as native entries.
#[derive(Default)]
pub struct Loaded {
    pub mods: Vec<mod_api::ModEntry>,
    pub manifests: Vec<Manifest>,
    pub refused: Vec<(String, Refusal)>,
    modules: Vec<WasmMod>,
}

impl Loaded {
    /// Initializes programmatically loaded mods with the final inventory, then installs
    /// their point listeners. Call once, before sealing the point registry.
    pub fn initialize(
        &self,
        inventory: &[mod_api::PackageInfo],
    ) -> Result<(), Vec<(String, Refusal)>> {
        let refused = initialize_selected(self.manifests.iter().zip(&self.modules), inventory);
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refused)
        }
    }
}

static MODULES: OnceLock<Vec<(Manifest, WasmMod)>> = OnceLock::new();

/// Supplies the complete native/content inventory after the game's dispatch order is installed,
/// before registration. Constructors and subscription queries run before this, as on native.
pub fn initialize(inventory: &[mod_api::PackageInfo]) {
    let mut refused = REFUSED.get().cloned().unwrap_or_default();
    let modules = MODULES.get().map_or(&[][..], Vec::as_slice);
    refused.extend(initialize_selected(
        modules.iter().map(|(manifest, module)| (manifest, module)),
        inventory,
    ));
    let _ = INITIALIZED_REFUSALS.set(refused);
}
fn initialize_selected<'a>(
    modules: impl Iterator<Item = (&'a Manifest, &'a WasmMod)>,
    inventory: &[mod_api::PackageInfo],
) -> Vec<(String, Refusal)> {
    let mut refused = Vec::new();
    for (manifest, module) in modules {
        let result = if initialize_modules(std::slice::from_ref(module), inventory) {
            points::listen(manifest, module)
        } else {
            Err(Refusal::Runtime("initialization failed".into()))
        };
        if let Err(why) = result {
            module.disable();
            log::error!("wasm mod {} refused: {why}", manifest.id);
            refused.push((manifest.id.clone(), why));
        }
    }
    refused
}
fn initialize_modules(modules: &[WasmMod], inventory: &[mod_api::PackageInfo]) -> bool {
    let mut succeeded = true;
    for module in modules {
        succeeded &= module
            .call(
                CallClass::Load,
                boundary::Frame::default(),
                mod_api::wire::INIT,
                None,
                |w| w.put(inventory),
                |_| Ok(()),
            )
            .is_some();
    }
    succeeded
}

pub fn load(std_member: &[u8], packages: Vec<WasmPackage>) -> Loaded {
    load_with_cache(std_member, packages, None)
}

fn load_with_cache(
    std_member: &[u8],
    packages: Vec<WasmPackage>,
    cache: Option<&module_cache::ModuleCache>,
) -> Loaded {
    let (groups, refused) = groups(&packages);
    let mut loaded = Loaded {
        refused,
        ..Loaded::default()
    };
    let engine = match runtime::engine() {
        Ok(engine) => engine,
        Err(error) => {
            loaded.refused.extend(
                packages
                    .iter()
                    .map(|p| (p.manifest.id.clone(), Refusal::Runtime(error.to_string()))),
            );
            return loaded;
        }
    };
    for indices in groups {
        let mut members = vec![link::Member {
            name: "libstd.so",
            module: std_member,
        }];
        let names: Vec<_> = indices
            .iter()
            .map(|&i| format!("lib{}.so", packages[i].manifest.id))
            .collect();
        members.extend(indices.iter().zip(&names).map(|(&i, name)| link::Member {
            name,
            module: &packages[i].module,
        }));
        let manifests: Vec<_> = indices
            .iter()
            .zip(&names)
            .map(|(&i, name)| (name.as_str(), &packages[i].manifest))
            .collect();
        let result = WasmMod::instantiate(&engine, &manifests, &members, cache).and_then(|mods| {
            if mods.len() != indices.len() {
                return Err(Refusal::Runtime("missing mod boundary exports".into()));
            }
            let mut checked = Vec::new();
            for mut module in mods {
                let subscriptions: Vec<(String, i32, bool)> = module
                    .call(
                        CallClass::Load,
                        boundary::Frame::default(),
                        boundary::SUBSCRIBED,
                        None,
                        |_| {},
                        |r| r.take(),
                    )
                    .ok_or_else(|| Refusal::Runtime("subscriptions failed".into()))?;
                registration_capabilities::check(&subscriptions)?;
                let names: Vec<String> = subscriptions
                    .iter()
                    .map(|(name, _, _)| name.clone())
                    .collect();
                module.listeners = subscriptions
                    .into_iter()
                    .filter_map(|(name, priority, receive_cancelled)| {
                        boundary::hook_id(&name).map(|hook| mod_api::Listener {
                            hook: Some(hook),
                            priority,
                            receive_cancelled,
                        })
                    })
                    .collect::<Vec<_>>()
                    .leak();
                // The ground provider runs on the worldgen workers, a contract wasm lacks yet.
                if names.iter().any(|name| name == "alter_ground") {
                    return Err(Refusal::Runtime(
                        "alter_ground: worldgen ground providers are not yet available to wasm mods"
                            .into(),
                    ));
                }
                module.subscribed = boundary::subscriptions_of("wasm mod", names);
                checked.push(module);
            }
            Ok(checked)
        });
        match result {
            Ok(mods) => {
                for (module, i) in mods.into_iter().zip(indices) {
                    let manifest = packages[i].manifest.clone();
                    let id = Box::leak(manifest.id.clone().into_boxed_str());
                    loaded.modules.push(module.clone());
                    loaded
                        .mods
                        .push(mod_api::ModEntry::new(id, Box::new(module)));
                    loaded.manifests.push(manifest);
                }
            }
            Err(why) => loaded.refused.extend(
                indices
                    .into_iter()
                    .map(|i| (packages[i].manifest.id.clone(), why.clone())),
            ),
        }
    }
    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_wasm_mixin_refuses_by_name() {
        let root = std::env::temp_dir().join(format!("wasm-mixin-{}", std::process::id()));
        let package = root.join("bad_mixin");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("horizon-mod.toml"), format!(
            "id = 'bad_mixin'\nname = 'Bad mixin'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[wasm]\nsha512 = '{}'\ntoolchain = 'test'\nbuilt_with = 'test'\n[[native.mixin]]\nid = 'patch'\ntarget = 'game::tick'\nkind = 'HEAD'\nhelper = 'src/patch.rs'\n", "a".repeat(128)
        )).unwrap();
        let loaded = load_installed(&root, &[package]);
        assert!(
            loaded.refused.iter().any(|(id, reason)| id == "bad_mixin"
                && reason
                    .to_string()
                    .contains("only [native] tables compile game code")),
            "{:?}",
            loaded.refused
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn package(
        id: &str,
        version: &str,
        deps: &str,
        sha: char,
        toolchain: &str,
        linked: &str,
    ) -> WasmPackage {
        let text = format!(
            "id = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n\n[dependencies]\n{deps}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"{toolchain}\"\nbuilt_with = \"test\"\n{linked}",
            sha.to_string().repeat(128)
        );
        WasmPackage {
            manifest: Manifest::parse(&text).unwrap(),
            module: Vec::new(),
        }
    }

    fn ids(packages: &[WasmPackage], groups: &[Vec<usize>]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|g| g.iter().map(|&i| packages[i].manifest.id.clone()).collect())
            .collect()
    }

    #[test]
    fn mods_sharing_a_library_share_one_group_in_dependency_order() {
        let packages = vec![
            package("b", "1.0.0", "lib = \"^1\"", 'b', "t", ""),
            package("lib", "1.2.0", "", 'a', "t", ""),
            package("c", "1.0.0", "lib = \"^1\"", 'c', "t", ""),
            package("alone", "1.0.0", "", 'd', "t", ""),
        ];
        let (groups, refused) = groups(&packages);
        assert!(refused.is_empty());
        assert_eq!(
            ids(&packages, &groups),
            [vec!["lib", "b", "c"], vec!["alone"]]
        );
    }

    #[test]
    fn bundle_ties_and_cycle_refusals_match_the_native_resolver() {
        let packages = vec![
            package("z", "1", "lib = '^1'", 'b', "t", ""),
            package("lib", "1.2", "", 'a', "t", ""),
            package("a", "1", "lib = '^1'", 'c', "t", ""),
        ];
        let (group, refused) = groups(&packages);
        assert!(refused.is_empty());
        assert_eq!(ids(&packages, &group), [vec!["lib", "z", "a"]]);
        let declared: Vec<_> = packages
            .iter()
            .map(|p| dependencies::from_manifest(&p.manifest.dependencies))
            .collect();
        let native: Vec<_> = packages
            .iter()
            .zip(&declared)
            .map(|(p, dependencies)| dependencies::Mod {
                id: &p.manifest.id,
                version: &p.manifest.version,
                has_api: true,
                dependencies,
            })
            .collect();
        assert_eq!(group[0], dependencies::resolve(&native).unwrap().order);
        let cyclic = vec![
            package("a", "1", "b = '^1'", 'a', "t", ""),
            package("b", "1", "a = '^1'", 'b', "t", ""),
            package("user", "1", "a = '^1'", 'c', "t", ""),
            package("free", "1", "", 'd', "t", ""),
        ];
        let (group, refused) = groups(&cyclic);
        assert_eq!(ids(&cyclic, &group), [vec!["free"]]);
        assert!(
            matches!(&refused[0].1, Refusal::DependencyOrder { reason } if reason.contains("a -> b -> a"))
        );
        assert!(matches!(&refused[1].1, Refusal::DependencyOrder { .. }));
        assert!(matches!(&refused[2].1, Refusal::DependencyRefused { .. }));
    }

    #[test]
    fn a_missing_required_dependency_refuses_the_mod_and_its_dependents() {
        let packages = vec![
            package("lib", "2.0.0", "", 'a', "t", ""),
            package("b", "1.0.0", "lib = \"^1\"", 'b', "t", ""),
            package(
                "c",
                "1.0.0",
                "b = \"^1\"\nextra = { version = \"^1\", optional = true }",
                'c',
                "t",
                "",
            ),
        ];
        let (groups, refused) = groups(&packages);
        assert_eq!(ids(&packages, &groups), [vec!["lib"]]);
        assert_eq!(
            refused,
            [
                (
                    "b".to_string(),
                    Refusal::MissingDependency {
                        dependency: "lib".into(),
                        version: "^1".into()
                    }
                ),
                (
                    "c".to_string(),
                    Refusal::DependencyRefused {
                        dependency: "b".into()
                    }
                ),
            ]
        );
    }

    #[test]
    fn internals_access_refuses_the_mod_and_its_dependents_but_api_access_loads() {
        let inject = |access: &str| {
            format!(
                "[[inject]]\nid = \"stop\"\nat = \"horizon_client::game::server_lifecycle::remember_shutdown\"\nkind = \"head\"\nwith = \"on_stop\"\naccess = \"{access}\"\nhash = \"h\"\n"
            )
        };
        let packages = vec![
            package(
                "deep",
                "1.0.0",
                "",
                'a',
                "t",
                &format!(
                    "[[internals]]\npath = \"Game::world\"\nkind = \"field\"\nhash = \"h\"\n{}",
                    inject("internals")
                ),
            ),
            package("user", "1.0.0", "deep = \"^1\"", 'b', "t", ""),
            package("typed", "1.0.0", "", 'c', "t", &inject("api")),
        ];
        let (groups, refused) = groups(&packages);
        assert_eq!(ids(&packages, &groups), [vec!["typed"]]);
        assert_eq!(
            refused,
            [
                ("deep".to_string(), Refusal::Internals),
                (
                    "user".to_string(),
                    Refusal::DependencyRefused {
                        dependency: "deep".into()
                    }
                ),
            ]
        );
        assert_eq!(
            Refusal::Internals.to_string(),
            "internals access is native-only"
        );
    }

    #[test]
    fn another_build_of_a_dependency_or_toolchain_is_refused() {
        let linked = format!("[wasm.linked]\nlib = \"{}\"\n", "f".repeat(128));
        let packages = vec![
            package("lib", "1.0.0", "", 'a', "t", ""),
            package("b", "1.0.0", "lib = \"^1\"", 'b', "t", &linked),
            package("c", "1.0.0", "lib = \"^1\"", 'c', "other", ""),
        ];
        let (_, refused) = groups(&packages);
        assert_eq!(
            refused,
            [
                (
                    "b".to_string(),
                    Refusal::LinkedAgainst {
                        dependency: "lib".into()
                    }
                ),
                (
                    "c".to_string(),
                    Refusal::Toolchain {
                        found: "other".into(),
                        group: "t".into()
                    }
                ),
            ]
        );
    }
}

#[cfg(test)]
mod mixed_install_tests {
    use super::*;
    #[test]
    fn malformed_layout_retains_identity() {
        let why = refusal("malformed");
        assert!(why.contains("layouts.json is malformed"), "{why}");
    }
    fn refusal(hash: &str) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("wasm-mixed-{}-{nonce}", std::process::id()));
        let addon = root.join("addon");
        let library = root.join("lib");
        std::fs::create_dir_all(&addon).unwrap();
        std::fs::create_dir_all(library.join("src")).unwrap();
        let header = |id| {
            format!("id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n")
        };
        let module = b"\x00\x61\x73\x6d\x01\x00\x00\x00\x00\x0f\x08\x64\x79\x6c\x69\x6e\x6b\x2e\x30\x01\x04\x00\x00\x00\x00\x01\x04\x01\x60\x00\x00\x03\x02\x01\x00\x07\x2e\x03\x0e\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x6c\x61\x79\x6f\x75\x74\x00\x00\x0a\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x69\x6e\x00\x00\x0c\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x68\x6f\x6f\x6b\x00\x00\x0a\x04\x01\x02\x00\x0b";
        std::fs::write(addon.join("mod.wasm"), module).unwrap();
        std::fs::write(
            addon.join("horizon-mod.toml"),
            format!(
                "{}[dependencies]\nlib='*'\n[wasm]\nsha512='{}'\ntoolchain='t'\nbuilt_with='t'\n",
                header("addon"),
                horizon_mod_package::manifest::sha512_of(module)
            ),
        )
        .unwrap();
        std::fs::write(
            library.join("horizon-mod.toml"),
            format!(
                "{}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                header("lib"),
                "a".repeat(40),
                "b".repeat(40)
            ),
        )
        .unwrap();
        std::fs::write(
            library.join("Cargo.toml"),
            "[package]\nname='lib'\nversion='1.0.0'\n",
        )
        .unwrap();
        std::fs::write(library.join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        std::fs::write(
            addon.join("layouts.json"),
            br#"{"lib":{"T":"0123456789abcdef"}}"#,
        )
        .unwrap();
        std::fs::write(
            library.join("layouts.json"),
            format!(r#"{{"lib":{{"T":"{hash}"}}}}"#),
        )
        .unwrap();
        if hash == "malformed" {
            std::fs::write(addon.join("layouts.json"), b"{").unwrap();
            std::fs::write(library.join("layouts.json"), b"{}").unwrap();
        }
        let loaded = load_installed(&root, &[addon, library]);
        std::fs::remove_dir_all(root).unwrap();
        assert!(loaded.mods.is_empty());
        assert_eq!(loaded.refused.len(), 1);
        assert_eq!(loaded.refused[0].0, "addon");
        loaded.refused[0].1.to_string()
    }
    #[test]
    fn wasm_to_native_layout_mismatch_names_the_type() {
        assert_eq!(
            refusal("1123456789abcdef"),
            "addon: layout mismatch in library lib for type T"
        );
    }
    #[test]
    fn wasm_to_native_matching_layout_is_still_unsupported() {
        assert_eq!(
            refusal("0123456789abcdef"),
            "lib is a native mod; wasm mod addon depends on it: mixed native and wasm dependencies aren't supported yet"
        );
    }
}

/// Resolve generated boundary operations for hand-written guest integration fixtures.
#[cfg(feature = "listener-fixture")]
pub fn fixture_import_id(owner: &str, method: &str) -> Option<u32> {
    boundary::import_id(owner, method)
}
/// Resolve generated hook operations for hand-written guest integration fixtures.
#[cfg(feature = "listener-fixture")]
pub fn fixture_export_id(owner: &str, method: &str) -> Option<u32> {
    boundary::export_id(owner, method)
}
