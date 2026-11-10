//! Declared internals and typed raw listeners, shared by both native build routes.
use super::{api, inject, sites};
use crate::i18n::trf;
use horizon_mod_package::internals::{self, ListedInternal};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(super) const INTERNALS: &str = "internals.json";

#[derive(Default)]
pub(super) struct Catalog {
    pub(super) internals: Vec<ListedInternal>,
    pub(super) points: Vec<inject::ListedPoint>,
    pub(super) sites: Vec<sites::Site>,
    pub(super) discovery: bool,
    /// The `mod_api` types a mod point's `api` record may hold, as the game's build lists them.
    pub(super) wire: BTreeSet<String>,
    /// Every code mod's own points (#2339), listed from its source.
    pub(super) mods: Vec<super::mod_points::ModLibrary>,
}

pub(super) struct Facade {
    pub(super) source: String,
    pub(super) crates: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    Inject(horizon_mod_package::manifest::inject::InjectError),
    Missing {
        mod_id: String,
        path: String,
    },
    Changed {
        mod_id: String,
        path: String,
        signature: String,
    },
    Gated {
        mod_id: String,
        path: String,
    },
    Ambiguous {
        mod_id: String,
        path: String,
        kinds: Vec<String>,
    },
    NotRaw {
        mod_id: String,
        target: String,
    },
    Path {
        path: String,
    },
    /// An injection at another mod's point that doesn't load (#2339).
    ModPoint(horizon_mod_package::manifest::inject::mod_points::Refusal),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Inject(error) => super::inject_text::inject_error(error),
            Self::ModPoint(refusal) => super::mod_points::refusal(refusal),
            Self::Missing { mod_id, path } => trf(
                "E_INTERNAL_MISSING: {mod} uses {path}, which this game doesn't have; run cargo horizon-mod internals sync to discover this pin's used internals.",
                &[("mod", mod_id), ("path", path)],
            ),
            Self::Changed { mod_id, path, signature } => trf(
                "E_INTERNAL_CHANGED: {mod} uses {path}, whose signature is now {signature}; run cargo horizon-mod update to see what changed.",
                &[("mod", mod_id), ("path", path), ("signature", signature)],
            ),
            Self::Gated { mod_id, path } => trf(
                "E_INTERNAL_GATED: {mod} uses {path}, which exists only under a cfg; target-specific internals aren't supported.",
                &[("mod", mod_id), ("path", path)],
            ),
            Self::Ambiguous { mod_id, path, kinds } => trf(
                "E_INTERNAL_AMBIGUOUS: {mod} uses {path}, which has multiple kinds; choose a kind from: {kinds}.",
                &[("mod", mod_id), ("path", path), ("kinds", &kinds.join(", "))],
            ),
            Self::NotRaw { mod_id, target } => trf(
                "E_NOT_RAW: {mod}: {target} has no raw form: it is generic or takes impl Trait.",
                &[("mod", mod_id), ("target", target)],
            ),
            Self::Path { path } => trf(
                "E_INTERNAL_PATH: {path} cannot be re-exported as a declared internal; declare its types or free functions instead.",
                &[("path", path)],
            ),
        };
        f.write_str(&message)
    }
}
impl std::error::Error for Error {}

impl From<internals::Error> for Error {
    fn from(error: internals::Error) -> Self {
        match error {
            internals::Error::Missing { mod_id, path } => Self::Missing { mod_id, path },
            internals::Error::Changed {
                mod_id,
                path,
                signature,
            } => Self::Changed {
                mod_id,
                path,
                signature,
            },
            internals::Error::Gated { mod_id, path } => Self::Gated { mod_id, path },
            internals::Error::Ambiguous {
                mod_id,
                path,
                kinds,
            } => Self::Ambiguous {
                mod_id,
                path,
                kinds,
            },
        }
    }
}

impl Catalog {
    pub(super) fn read(dir: &Path) -> Result<Self, String> {
        let path = dir.join(INTERNALS);
        let listed = inject::catalog(&dir.join(inject::POINTS))?;
        let mut points = listed.points;
        for point in horizon_platform::mod_build::api_gen::points::source::read(dir)? {
            let name = horizon_platform::mod_build::point_shape::camel(&point.name);
            points.push(inject::ListedPoint {
                name: point.name,
                target: point.target,
                raw: true,
                at_return: true,
                api_refusal: Some("a private source point has no API record".into()),
                returns: "none".into(),
                host: "none".into(),
                raw_shape: None,
                return_ty: None,
                listener: Some(format!("{}::__{name}Listener", point.module)),
                return_listener: Some(format!("{}::__{name}ReturnListener", point.module)),
                private: true,
                record_name: Some(point.record_export),
            });
        }
        let mut sites = listed.sites;
        for site in horizon_platform::mod_build::api_gen::sites::source::read(dir)? {
            if sites.iter().any(|public| public.name == site.name) {
                return Err(format!("E_SOURCE_COLLISION: {}", site.name));
            }
            sites.push(site.into());
        }
        Ok(Self {
            internals: internals::read(&path)?,
            points,
            sites,
            discovery: false,
            wire: listed.wire,
            mods: Vec::new(),
        })
    }

    /// The catalog entry an `[[inject]]` at `at` targets: a function, method or (for a field)
    /// struct, never a shape-only entry.
    pub(super) fn target(&self, at: &str) -> Option<&ListedInternal> {
        self.internals.iter().find(|item| {
            !item.shape_only
                && plain_path(&item.path) == plain_path(at)
                && matches!(item.kind.as_str(), "fn" | "method" | "struct")
        })
    }

    /// `manifest` with each absent `hash` taken from this catalog, the pinned game's as the build
    /// checks it (`[native] horizon` fixes it exactly). The build records this form in
    /// `native-mods.json`; a declaration that doesn't resolve keeps no hash and is refused by
    /// [`Self::check`].
    pub(super) fn pinned(
        &self,
        manifest: &horizon_mod_package::Manifest,
    ) -> horizon_mod_package::Manifest {
        let mut pinned = manifest.clone();
        for inject in pinned
            .inject
            .iter_mut()
            .filter(|inject| inject.hash.is_none())
        {
            inject.hash = self.target(&inject.at).map(|item| item.hash.clone());
        }
        for declared in pinned
            .internals
            .iter_mut()
            .filter(|declared| declared.hash.is_none())
        {
            declared.hash = internals::resolve(
                &self.internals,
                &manifest.id,
                &declared.path,
                declared.kind.as_deref(),
            )
            .ok()
            .map(|entry| entry.hash.clone());
        }
        pinned
    }

    pub(super) fn check(&self, manifest: &horizon_mod_package::Manifest) -> Result<(), Error> {
        super::slots::check(self, manifest).map_err(Error::Inject)?;
        super::mod_points::routes(self, manifest).map_err(Error::ModPoint)?;
        if manifest.native.is_none() {
            return Ok(());
        }
        for declared in &manifest.internals {
            internals::check(&self.internals, &manifest.id, declared)?;
        }
        for injection in super::slots::points(&self.points, manifest) {
            if !self.points.iter().any(|point| {
                plain_path(&point.target) == plain_path(&injection.target)
                    && point.raw
                    && point.listener.is_some()
                    && point.return_listener.is_some()
            }) {
                return Err(Error::NotRaw {
                    mod_id: manifest.id.clone(),
                    target: injection.target,
                });
            }
        }
        Ok(())
    }

    pub(super) fn prepare<'a>(
        &self,
        manifests: impl IntoIterator<Item = &'a horizon_mod_package::Manifest>,
    ) -> Result<BTreeMap<&'a str, Facade>, Error> {
        manifests
            .into_iter()
            .map(|manifest| {
                (if self.discovery {
                    self.discovery_facade(manifest)
                } else {
                    self.facade(manifest)
                })
                .map(|facade| (manifest.id.as_str(), facade))
            })
            .collect()
    }

    pub(super) fn log_raw(&self, manifest: &horizon_mod_package::Manifest) -> Result<(), String> {
        for decision in super::slots::report(self, manifest) {
            super::append_log(&decision)?;
        }
        for injection in super::slots::points(&self.points, manifest) {
            let point = self
                .points
                .iter()
                .find(|point| plain_path(&point.target) == plain_path(&injection.target))
                .expect("raw target checked");
            let place = point.place(injection.at_return);
            super::append_log(&trf(
                "{mod} listens raw at {point}",
                &[("mod", &manifest.id), ("point", &place)],
            ))?;
        }
        Ok(())
    }

    /// Discovery opens the staged view, not the production declaration boundary. Keep the
    /// exact raw constructors from the normal facade; they are not internal-use declarations.
    fn discovery_facade(&self, manifest: &horizon_mod_package::Manifest) -> Result<Facade, Error> {
        let mut manifest = manifest.clone();
        manifest.internals.clear();
        let mut facade = self.facade(&manifest)?;
        facade.source.insert_str(0, "#![allow(deprecated)]\n");
        let mut tree = Module::default();
        // A member is reached through its catalogued owner. Never open a whole engine
        // crate: unlisted public APIs have no diagnostic marker and would be missed.
        for entry in self.internals.iter().filter(|entry| {
            !entry.shape_only
                && entry.cfg.is_empty()
                && !matches!(entry.kind.as_str(), "mod" | "field" | "method")
                && !entry.signature.starts_with("impl ")
        }) {
            tree.export(&self.export_path(entry)?, &mut facade.crates);
        }
        tree.write(&mut facade.source, 0);
        Ok(facade)
    }

    /// Re-exports are the compiler's boundary: no module wildcard can widen the declaration.
    pub(super) fn facade(&self, manifest: &horizon_mod_package::Manifest) -> Result<Facade, Error> {
        self.check(manifest)?;
        let mut tree = Module::default();
        let mut crates = BTreeSet::from(["horizon_client".into()]);
        if manifest.native.is_some() {
            for declared in &manifest.internals {
                let entry = internals::resolve(
                    &self.internals,
                    &manifest.id,
                    &declared.path,
                    declared.kind.as_deref(),
                )?;
                let path = self.export_path(entry)?;
                tree.export(&path, &mut crates);
            }
        }
        let mut text = String::from(
            "//! This mod's declared game internals, generated from its pinned build.\n",
        );
        tree.write(&mut text, 0);
        text.push_str("pub mod raw {\n");
        let mut constructors = BTreeSet::new();
        let mut exports = BTreeSet::new();
        for injection in super::slots::points(&self.points, manifest) {
            let point = self
                .points
                .iter()
                .find(|point| plain_path(&point.target) == plain_path(&injection.target))
                .expect("raw target checked");
            let listener = if injection.at_return {
                &point.return_listener
            } else {
                &point.listener
            }
            .as_ref()
            .expect("raw listener checked");
            crates.insert(
                listener
                    .split("::")
                    .next()
                    .expect("listener path")
                    .to_owned(),
            );
            let name = listener.rsplit("::").next().expect("listener path");
            exports.insert(format!("    pub use {listener};\n"));
            let head = point.listener.as_ref().expect("raw listener checked");
            let raw = head
                .strip_suffix("Listener")
                .expect("generated listener suffix")
                .to_owned()
                + "Raw";
            if let Some(name) = &point.record_name {
                exports.insert(format!("    pub use {raw} as {name};\n"));
            } else {
                exports.insert(format!("    pub use {raw};\n"));
            }
            let constructor = point.place(injection.at_return);
            let variant = name
                .strip_prefix("__")
                .and_then(|name| name.strip_suffix("Listener"))
                .expect("generated listener name");
            constructors.insert(format!(
                "    /// Registers a function of this place's exact listener type.\n    pub const fn {constructor}(id: &'static str, f: {name}) -> mod_points::__private::SlotListener {{\n        // SAFETY: `f` has this place's listener type, checked by the signature.\n        unsafe {{ mod_points::__private::SlotListener::new(id, mod_points::PointId::{variant}.place(), f as *const ()) }}\n    }}\n"
            ));
        }
        for (_, site) in super::slots::sites(&self.sites, manifest) {
            site.facade(&mut exports, &mut constructors, &mut crates);
        }
        for export in exports {
            text.push_str(&export);
        }
        for constructor in constructors {
            text.push_str(&constructor);
        }
        text.push_str("}\n");
        Ok(Facade {
            source: text,
            crates,
        })
    }

    fn export_path(&self, entry: &ListedInternal) -> Result<String, Error> {
        let invalid = || Error::Path {
            path: entry.path.clone(),
        };
        if entry.kind == "mod" {
            return Err(invalid());
        }
        let member = matches!(entry.kind.as_str(), "field" | "method")
            || entry.signature.starts_with("impl ");
        let path = if member {
            entry.path.rsplit_once("::").ok_or_else(invalid)?.0
        } else {
            &entry.path
        };
        let path = plain_path(path);
        syn::parse_str::<syn::Path>(&path).map_err(|_| invalid())?;
        if !member {
            return Ok(path);
        }
        let name = path.rsplit("::").next().ok_or_else(invalid)?;
        // An inherent impl in a child module names a type imported from its parent. Resolve to
        // the nearest enclosing declaration rather than re-exporting that private import.
        let mut scope = path.rsplit_once("::").ok_or_else(invalid)?.0;
        loop {
            if entry.kind == "field"
                && self
                    .internals
                    .iter()
                    .any(|entry| entry.path == scope && entry.kind == "enum" && !entry.shape_only)
            {
                return Ok(scope.to_owned());
            }
            let candidate = format!("{scope}::{name}");
            if self.internals.iter().any(|entry| {
                entry.path == candidate
                    && matches!(entry.kind.as_str(), "struct" | "enum" | "type" | "trait")
                    && !entry.shape_only
            }) {
                return Ok(candidate);
            }
            let Some((parent, _)) = scope.rsplit_once("::") else {
                break;
            };
            scope = parent;
        }
        Err(invalid())
    }
}

/// Generic arguments belong to a fingerprint, not to a use path or a site target's identity.
pub(super) fn plain_path(path: &str) -> String {
    let mut depth = 0;
    path.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth -= 1;
                false
            }
            _ => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

#[derive(Default)]
struct Module {
    items: BTreeSet<String>,
    children: BTreeMap<String, Module>,
}
impl Module {
    fn export(&mut self, path: &str, crates: &mut BTreeSet<String>) {
        let parts: Vec<_> = path.split("::").collect();
        crates.insert(parts[0].to_owned());
        // The game is the facade's root; lower crates retain their own namespace.
        let parts = if parts[0] == "horizon_client" {
            &parts[1..]
        } else {
            &parts[..]
        };
        let mut module = self;
        for part in &parts[..parts.len() - 1] {
            module = module.children.entry((*part).to_owned()).or_default();
        }
        module.items.insert(format!("pub use {path};"));
    }

    fn write(&self, text: &mut String, depth: usize) {
        let indent = "    ".repeat(depth);
        for item in &self.items {
            text.push_str(&format!("{indent}{item}\n"));
        }
        for (name, child) in &self.children {
            text.push_str(&format!("{indent}pub mod {name} {{\n"));
            child.write(text, depth + 1);
            text.push_str(&format!("{indent}}}\n"));
        }
    }
}

pub(super) fn needed(manifest: &horizon_mod_package::Manifest) -> bool {
    !manifest.inject.is_empty() || !manifest.internals.is_empty()
}

pub(super) fn package(id: &str) -> String {
    format!("horizon-internals-{id}")
}
pub(super) fn relative(id: &str) -> String {
    format!("native-internals/{id}")
}

pub(super) fn stage(
    stage: &Path,
    project: &api::Project<'_>,
    facade: &Facade,
) -> Result<(), String> {
    let manifest = &project.bundle.manifest;
    let dest = stage.join(relative(&manifest.id));
    fs::create_dir_all(dest.join("src")).map_err(|e| e.to_string())?;
    let mut cargo = format!("[package]\nname = {:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\nname = \"internals\"\n[dependencies]\nhorizon-client = {{ path = \"../../crates/horizon-client\", default-features = false }}\nmod_points = {{ package = \"horizon-mod-points\", path = \"../../crates/horizon-mod-points\" }}\n", package(&manifest.id));
    for krate in &facade.crates {
        if krate != "horizon_client" {
            let package = krate.replace('_', "-");
            cargo.push_str(&format!(
                "{krate} = {{ package = {package:?}, path = \"../../crates/{package}\" }}\n"
            ));
        }
    }
    fs::write(dest.join("Cargo.toml"), cargo).map_err(|e| e.to_string())?;
    fs::write(dest.join("src/lib.rs"), &facade.source).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (std::path::PathBuf, super::super::Bundle, Catalog) {
        let dir = super::super::tests::dir(&format!("internals-{name}"));
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(dir.join("Cargo.toml"), "[package]\nname='example'\nversion='1.0.0'\nedition='2024'\n[dependencies]\nhorizon-mod-api={version='0.1'}\n").unwrap();
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap().replace("[native]", "internals = [{ path = 'horizon_client::game::Game', hash = '1234' }, { path = 'horizon_client::game::maps::dimension_id', hash = '5678' }]\n[native]\n")
            + "[[inject]]\nid = 'count'\nat = \"horizon_client::game::Game<'_>::tick\"\nkind = 'head'\nwith = 'count'\naccess = 'internals'\nhash = 'tick-hash'\n";
        fs::write(path, text).unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        let catalog = Catalog {
            discovery: false,
            sites: Vec::new(),
            wire: Default::default(),
            mods: Vec::new(),
            internals: serde_json::from_value(serde_json::json!([
                { "path": "horizon_client::game::Game", "kind": "struct", "signature": "struct Game<'a> { world: World }", "hash": "1234" },
                { "path": "horizon_client::game::maps::dimension_id", "kind": "fn", "signature": "fn dimension_id() -> u8", "hash": "5678" },
                { "path": "horizon_client::game::maps::Game<'_>::dimension_id", "kind": "method", "signature": "impl Game<'_> { fn dimension_id(&self) -> u8 }", "hash": "abcd" },
                { "path": "horizon_client::game::Game<'_>::tick", "kind": "method", "signature": "fn tick(&mut self)", "hash": "tick-hash" }
            ])).unwrap(),
            points: serde_json::from_value(serde_json::json!([
                { "name": "tick", "target": "horizon_client::game::Game<'_>::tick", "raw": true, "at_return": true,
                    "listener": "horizon_client::game::__TickListener", "return_listener": "horizon_client::game::__TickReturnListener" }
            ])).unwrap(),
        };
        (dir, bundle, catalog)
    }

    #[test]
    fn explicit_hash_survives_pinned_filling() {
        let (dir, mut bundle, catalog) = fixture("explicit-pin");
        bundle.manifest.inject[0].hash = Some("explicit".into());
        bundle.manifest.internals[0].hash = Some("explicit-internal".into());
        let pinned = catalog.pinned(&bundle.manifest);
        assert_eq!(pinned.inject[0].hash.as_deref(), Some("explicit"));
        assert_eq!(
            pinned.internals[0].hash.as_deref(),
            Some("explicit-internal")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn absent_hashes_are_checked_against_the_pinned_catalog_and_recorded_from_it() {
        let (dir, mut bundle, mut catalog) = fixture("pinned");
        // An item a mod's own `[[native.module]]` adds is in the staged catalog the source build
        // checks against, and nowhere else: declared without a hash, it checks and is filled.
        catalog.internals.extend(serde_json::from_value::<Vec<ListedInternal>>(serde_json::json!([
            { "path": "horizon_client::game::ported_level::PortedGame", "kind": "struct", "signature": "pub(super) struct PortedGame", "hash": "ported" }
        ])).unwrap());
        let manifest = &mut bundle.manifest;
        manifest.inject[0].hash = None;
        manifest.internals[0].hash = None;
        manifest
            .internals
            .push(horizon_mod_package::manifest::Internal {
                path: "horizon_client::game::ported_level::PortedGame".into(),
                kind: None,
                hash: None,
            });
        catalog.check(manifest).unwrap();
        let pinned = catalog.pinned(manifest);
        assert_eq!(pinned.inject[0].hash.as_deref(), Some("tick-hash"));
        let hashes: Vec<_> = pinned.internals.iter().map(|i| i.hash.as_deref()).collect();
        assert_eq!(hashes, [Some("1234"), Some("5678"), Some("ported")]);
        // The record is what a stale pin is later checked against: the same catalog passes it, a
        // changed function refuses it, as a hash written by the tool would be.
        catalog.check(&pinned).unwrap();
        catalog.internals[3].hash = "tick-changed".into();
        assert!(matches!(
            catalog.check(&pinned),
            Err(Error::Inject(
                horizon_mod_package::manifest::inject::InjectError::Changed { .. }
            ))
        ));
        // A present hash that is stale is refused as before.
        manifest.internals[0].hash = Some("stale".into());
        assert!(matches!(
            catalog.check(manifest),
            Err(Error::Changed { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn discovery_opens_only_its_facade_and_retains_typed_raw_listeners() {
        let (dir, bundle, mut catalog) = fixture("discovery-raw");
        let normal = catalog.facade(&bundle.manifest).unwrap().source;
        catalog.internals.extend(serde_json::from_value::<Vec<ListedInternal>>(serde_json::json!([
            { "path": "lower::selected", "kind": "fn", "signature": "fn selected()", "hash": "selected" },
            { "path": "lower::Shape", "kind": "struct", "signature": "struct Shape", "hash": "shape", "shape_only": true }
        ])).unwrap());
        catalog.discovery = true;
        let discovery = catalog.prepare([&bundle.manifest]).unwrap();
        let source = &discovery[bundle.manifest.id.as_str()].source;
        assert!(source.contains("pub use horizon_client::game::Game;"));
        assert!(!source.contains("pub use horizon_client::*;"));
        assert!(source.contains("pub use lower::selected;"));
        assert!(!source.contains("pub use lower;"));
        assert!(!source.contains("pub use lower::Shape;"));
        assert!(source.contains("pub use horizon_client::game::__TickListener;"));
        assert!(source.contains("mod_points::__private::SlotListener::new"));
        assert!(!normal.contains("pub use horizon_client::*;"));
        assert_eq!(catalog.facade(&bundle.manifest).unwrap().source, normal);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn facade_has_only_two_declared_internals_and_one_typed_raw_constructor() {
        let (dir, bundle, catalog) = fixture("facade");
        let text = catalog.facade(&bundle.manifest).unwrap().source;
        assert_eq!(
            text,
            r#"//! This mod's declared game internals, generated from its pinned build.
pub mod game {
    pub use horizon_client::game::Game;
    pub mod maps {
        pub use horizon_client::game::maps::dimension_id;
    }
}
pub mod raw {
    pub use horizon_client::game::__TickListener;
    pub use horizon_client::game::__TickRaw;
    /// Registers a function of this place's exact listener type.
    pub const fn tick(id: &'static str, f: __TickListener) -> mod_points::__private::SlotListener {
        // SAFETY: `f` has this place's listener type, checked by the signature.
        unsafe { mod_points::__private::SlotListener::new(id, mod_points::PointId::Tick.place(), f as *const ()) }
    }
}
"#
        );
        syn::parse_file(&text).unwrap();
        assert!(!bundle.needs_mixins());
        let project = api::Project::read(&bundle).unwrap();
        assert_eq!(super::super::prebuilt::project_refusal(&project), None);
        let main = api::main_rs(
            include_str!("../../../crates/horizon-client/src/native_main.rs.template"),
            &[&bundle.manifest],
            &catalog,
        );
        assert!(main.contains("#![forbid(unsafe_code)]"));
        assert!(main.contains(
            "mod_0_internals::raw::tick(\"example\", mod_0::count).ordered(0, \"count\"), // injection \"count\""
        ));
        assert!(!main.contains("/* MOD_SLOTS */"));
        assert!(!main.contains("mod_0::__horizon_points"));
        assert!(!main.contains("unsafe {"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn freshness_refuses_missing_changed_gated_and_shape_only_entries() {
        let (dir, bundle, mut catalog) = fixture("freshness");
        let manifest = &bundle.manifest;
        catalog.check(manifest).unwrap();
        catalog.internals[0].hash = "new".into();
        let error = catalog.check(manifest).unwrap_err();
        assert!(matches!(error, Error::Changed { .. }));
        assert!(error.to_string().contains("E_INTERNAL_CHANGED"));
        assert!(error
            .to_string()
            .contains("struct Game<'a> { world: World }"));
        catalog.internals[0].cfg = "feature = \"controllers\"".into();
        assert!(matches!(catalog.check(manifest), Err(Error::Gated { .. })));
        assert!(catalog
            .check(manifest)
            .unwrap_err()
            .to_string()
            .contains("E_INTERNAL_GATED"));
        catalog.internals[0].shape_only = true;
        assert!(matches!(
            catalog.check(manifest),
            Err(Error::Missing { .. })
        ));
        catalog.internals.remove(0);
        assert!(matches!(
            catalog.check(manifest),
            Err(Error::Missing { .. })
        ));
        assert!(catalog
            .check(manifest)
            .unwrap_err()
            .to_string()
            .contains("E_INTERNAL_MISSING"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_raw_target_must_have_a_raw_form_on_both_routes() {
        let (dir, bundle, mut catalog) = fixture("not-raw");
        for missing in [false, true] {
            if missing {
                catalog.points.clear();
                catalog.internals.pop();
            } else {
                catalog.points[0].raw = false;
            }
            let error = catalog.check(&bundle.manifest).unwrap_err();
            if missing {
                assert!(matches!(
                    error,
                    Error::Inject(
                        horizon_mod_package::manifest::inject::InjectError::Missing { .. }
                    )
                ));
            } else {
                assert!(matches!(error, Error::NotRaw { .. }));
                assert!(error.to_string().contains("E_NOT_RAW"));
            }
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unified_source_fallback_does_not_bypass_raw_abi_refusal() {
        let (dir, bundle, catalog) = fixture("raw-abi");
        for fault in 0..3 {
            let mut points = catalog.points.clone();
            match fault {
                0 => points[0].raw = false,
                1 => points[0].listener = None,
                _ => points[0].return_listener = None,
            }
            let broken = Catalog {
                points,
                internals: catalog.internals.clone(),
                ..Catalog::default()
            };
            // Unified routing can select Source when a compiled slot lacks a raw form.
            // The facade must still refuse its untyped/incomplete ABI, never erase it.
            super::super::slots::check(&broken, &bundle.manifest).unwrap();
            let expected = Error::NotRaw {
                mod_id: "example".into(),
                target: "horizon_client::game::Game<'_>::tick".into(),
            };
            assert_eq!(broken.check(&bundle.manifest), Err(expected));
            assert!(matches!(
                broken.facade(&bundle.manifest),
                Err(Error::NotRaw { .. })
            ));
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn facade_is_needed_only_for_unified_injections_or_declared_internals() {
        let (dir, mut bundle, _) = fixture("needed");
        assert!(needed(&bundle.manifest));
        bundle.manifest.internals.clear();
        assert!(needed(&bundle.manifest));
        bundle.manifest.inject.clear();
        assert!(!needed(&bundle.manifest));
        bundle
            .manifest
            .internals
            .push(horizon_mod_package::manifest::Internal {
                path: "horizon_client::game::Game".into(),
                hash: Some("1234".into()),
                kind: None,
            });
        assert!(needed(&bundle.manifest));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_lower_crate_raw_listener_adds_its_crate_to_the_facade() {
        let (dir, bundle, mut catalog) = fixture("lower-raw");
        catalog.points[0].listener = Some("horizon_world::ticking::__TickListener".into());
        catalog.points[0].return_listener =
            Some("horizon_world::ticking::__TickReturnListener".into());
        let facade = catalog.facade(&bundle.manifest).unwrap();
        assert!(facade.crates.contains("horizon_world"));
        assert!(facade
            .source
            .contains("pub use horizon_world::ticking::__TickRaw;"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn method_entries_export_the_owner_and_return_listeners_use_the_return_place() {
        let (dir, mut bundle, catalog) = fixture("owner-return");
        bundle.manifest.internals[1] = horizon_mod_package::manifest::Internal {
            path: "horizon_client::game::maps::Game<'_>::dimension_id".into(),
            kind: None,
            hash: Some("abcd".into()),
        };
        bundle.manifest.inject[0].kind = horizon_mod_package::manifest::inject::Kind::Return;
        let text = catalog.facade(&bundle.manifest).unwrap().source;
        assert_eq!(
            text.matches("pub use horizon_client::game::Game;").count(),
            1
        );
        assert!(!text.contains("maps::Game"));
        assert!(text.contains("pub use horizon_client::game::__TickReturnListener;"));
        assert!(
            text.contains("pub const fn tick_return(id: &'static str, f: __TickReturnListener)")
        );
        assert!(text.contains("PointId::TickReturn"));
        let main = api::main_rs("/* MOD_SLOTS */", &[&bundle.manifest], &catalog);
        assert!(main.contains(
            "mod_0_internals::raw::tick_return(\"example\", mod_0::count).ordered(0, \"count\")"
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn member_kind_reaches_the_facade_and_ambiguity_is_translated() {
        let (dir, mut bundle, mut catalog) = fixture("member-kind");
        let method = catalog.internals[2].clone();
        let mut field = method.clone();
        field.kind = "field".into();
        field.hash = "field".into();
        field.signature = "u8".into();
        catalog.internals.insert(2, field);
        bundle.manifest.internals[1] = horizon_mod_package::manifest::Internal {
            path: method.path.clone(),
            kind: Some("method".into()),
            hash: Some(method.hash),
        };
        let facade = catalog.facade(&bundle.manifest).unwrap();
        assert!(facade
            .source
            .contains("pub use horizon_client::game::Game;"));
        bundle.manifest.internals[1].hash = Some("field".into());
        assert!(
            matches!(catalog.check(&bundle.manifest), Err(Error::Changed { signature, .. }) if signature == method.signature)
        );
        bundle.manifest.internals[1].kind = None;
        let error = catalog.check(&bundle.manifest).unwrap_err();
        assert!(matches!(error, Error::Ambiguous { .. }));
        let text = error.to_string();
        assert!(text.contains("E_INTERNAL_AMBIGUOUS"), "{text}");
        assert!(text.contains(&method.path), "{text}");
        assert!(text.contains("field, method"), "{text}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn source_staging_uses_a_per_mod_facade_and_preserves_the_library_name() {
        let (dir, bundle, mut catalog) = fixture("stage");
        let path = dir.join("Cargo.toml");
        let text = fs::read_to_string(&path).unwrap()
            + &format!("horizon-internals = {{ git = 'https://github.com/arismoko/horizon', rev = '{}' }}\n", bundle.native().horizon);
        fs::write(&path, text).unwrap();
        let project = api::Project::read(&bundle).unwrap();
        assert_eq!(super::super::prebuilt::project_refusal(&project), None);
        let stage = dir.with_extension("stage");
        fs::create_dir_all(stage.join("crates/horizon-client/src")).unwrap();
        fs::write(
            stage.join("crates/horizon-client/Cargo.toml"),
            "[package]\nname='horizon-client'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            stage.join("crates/horizon-client/src/native_main.rs.template"),
            include_str!("../../../crates/horizon-client/src/native_main.rs.template"),
        )
        .unwrap();
        fs::write(stage.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        api::stage(
            &stage,
            &[project],
            &[vec![]],
            &super::super::dependencies::Resolution { order: vec![0] },
            &catalog,
        )
        .unwrap();
        let manifest = api::read(&stage.join("native-api/example/Cargo.toml")).unwrap();
        assert_eq!(
            manifest["dependencies"]["internals"]["package"].as_str(),
            Some("horizon-internals-example")
        );
        assert_eq!(
            manifest["dependencies"]["internals"]["path"].as_str(),
            Some("../../native-internals/example")
        );
        let facade = fs::read_to_string(stage.join("native-internals/example/src/lib.rs")).unwrap();
        assert_eq!(facade, catalog.facade(&bundle.manifest).unwrap().source);
        let game = api::read(&stage.join("crates/native-game/Cargo.toml")).unwrap();
        assert_eq!(
            game["dependencies"]["mod_0_internals"]["package"].as_str(),
            Some("horizon-internals-example")
        );
        assert!(super::super::crates::wanted(&stage, &BTreeMap::new())
            .unwrap()
            .is_empty());
        super::super::tests::write_bundle(&dir, "example", "pub fn count<T>(_: &mut T) {}\n");
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        fs::write(&path, fs::read_to_string(&path).unwrap() + "[[inject]]\nid='/* MOD_ENTRIES *//* MOD_SLOTS */'\nat='horizon_client::net::receive_brand'\nkind='head'\naccess='internals'\nwith='count'\nhash='5678'\n").unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname='example'\nversion='1.0.0'\n[dependencies]\nhorizon-mod-api='0.1'\n",
        )
        .unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        catalog.internals[1].path = "horizon_client::net::receive_brand".into();
        catalog.internals[1].signature = "fn receive_brand()".into();
        catalog.points[0].target = catalog.internals[1].path.clone();
        catalog.points[0].name = "receive_brand".into();
        catalog.points[0].listener = Some("horizon_client::net::__ReceiveBrandListener".into());
        let project = api::Project::read(&bundle).unwrap();
        assert_eq!(super::super::prebuilt::project_refusal(&project), None);
        api::stage(
            &stage,
            &[project],
            &[vec![]],
            &super::super::dependencies::Resolution { order: vec![0] },
            &catalog,
        )
        .unwrap();
        let game = api::read(&stage.join("crates/native-game/Cargo.toml")).unwrap();
        assert!(game["dependencies"].get("mod_0_internals").is_some());
        let facade = fs::read_to_string(stage.join("native-internals/example/src/lib.rs")).unwrap();
        assert!(facade.contains("fn receive_brand("));
        let main = fs::read_to_string(stage.join("crates/native-game/src/main.rs")).unwrap();
        assert!(main.contains(".ordered(0, \"/* MOD_ENTRIES *//* MOD_SLOTS */\")"));
        fs::remove_dir_all(stage).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn direct_game_dependencies_are_refused_in_every_cargo_dependency_table() {
        let (dir, bundle, _) = fixture("game-dependency");
        let path = dir.join("Cargo.toml");
        let original = fs::read_to_string(&path).unwrap();
        for table in [
            "dependencies",
            "dev-dependencies",
            "build-dependencies",
            "target.'cfg(unix)'.dependencies",
            "target.'cfg(unix)'.build-dependencies",
        ] {
            let extra = "game = { package = 'horizon-client', version = '0.1' }\n";
            let text = if table == "dependencies" {
                format!("{original}{extra}")
            } else {
                format!("{original}[{table}]\n{extra}")
            };
            fs::write(&path, text).unwrap();
            let Err(error) = api::Project::read(&bundle) else {
                panic!("accepted {table}");
            };
            assert!(matches!(error, api::ProjectError::GameDependency { .. }));
            assert!(error.to_string().contains("E_GAME_DEPENDENCY"));
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
