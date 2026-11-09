//! Raw function declarations and the staged point catalog.
use std::{fs, path::Path};

/// One raw function declaration and its head or return target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PointInjection {
    pub(super) mod_id: String,
    pub(super) id: String,
    pub(super) target: String,
    pub(super) method: String,
    /// At the function's return, not its head.
    pub(super) at_return: bool,
}

/// A game build's point, as its `points.json` lists it.
#[derive(Clone, Debug, serde::Deserialize)]
pub(super) struct ListedPoint {
    pub(super) name: String,
    #[serde(default)]
    pub(super) private: bool,
    #[serde(default)]
    pub(super) record_name: Option<String>,
    pub(super) target: String,
    pub(super) raw: bool,
    pub(super) at_return: bool,
    /// Why its API listeners are refused, naming the parameter with no API type.
    #[serde(default)]
    pub(super) api_refusal: Option<String>,
    /// What its listeners may do with the return: `cancel`, `result` or `none`.
    #[serde(default)]
    pub(super) returns: String,
    /// Its API listeners' host: `none`, or the host they take.
    #[serde(default)]
    pub(super) host: String,
    /// Its raw form's return type, named when a `replace` is refused.
    #[serde(default)]
    pub(super) raw_shape: Option<horizon_mod_point_emit::RawReturn>,
    /// The function's return type as written, which a `replace` at a `cancel` point needs `()`.
    #[serde(default)]
    pub(super) return_ty: Option<String>,
    pub(super) listener: Option<String>,
    pub(super) return_listener: Option<String>,
}

impl ListedPoint {
    /// The point as the shared API constructor emitter reads it.
    pub(super) fn emitted(&self) -> horizon_mod_point_emit::Point {
        horizon_mod_point_emit::Point {
            name: self.name.clone(),
            target: self.target.clone(),
            private: self.private,
            at_return: self.at_return,
            api_refusal: self.api_refusal.clone(),
            returns: self.returns.clone(),
            host: self.host.clone(),
            raw_shape: self.raw_shape.clone(),
            return_ty: self.return_ty.clone(),
        }
    }

    pub(super) fn place(&self, at_return: bool) -> String {
        if at_return {
            format!("{}_return", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// The `points.json` the game's build script writes, beside the prebuilt game or in the
/// staged game's build output.
pub(super) const POINTS: &str = "points.json";

#[derive(serde::Deserialize)]
pub(super) struct Catalog {
    pub points: Vec<ListedPoint>,
    pub sites: Vec<super::sites::Site>,
}

pub(super) fn catalog(path: &Path) -> Result<Catalog, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::super::{Bundle, MANIFEST};
    use super::*;

    fn fixture(name: &str, injection: &str) -> (std::path::PathBuf, Bundle) {
        let root = super::super::tests::dir(&format!("inject-{name}"));
        super::super::tests::write_bundle(&root, "example", "");
        let path = root.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap() + injection;
        fs::write(path, text).unwrap();
        let bundle = super::super::bundle_in(&root).unwrap().unwrap();
        (root, bundle)
    }

    #[test]
    fn a_module_stages_beside_a_unified_callback() {
        let (root, _bundle) = fixture(
            "module",
            "[[inject]]\nid = 'hook'\nat = 'horizon_client::game::tick'\nkind = 'head'\nwith = 'tick_hook'\nhash = 'signature'\n",
        );
        // The callback remains in the mod crate; only the support module reaches the tool.
        fs::write(
            root.join("src/lib.rs"),
            "mod runtime; pub fn tick_hook() { runtime::bump(&mut 0); }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/runtime.rs"),
            "pub(crate) fn bump(value: &mut i32) { *value += 1; }\n",
        )
        .unwrap();
        let path = root.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        fs::write(&path, text).unwrap();
        let bundle = super::super::bundle_in(&root).unwrap().unwrap();
        let mixin_input = root.with_extension("mixins");
        let tool_mods =
            super::super::stage_mixins(std::slice::from_ref(&bundle), &mixin_input).unwrap();
        assert_eq!(tool_mods.len(), 1);
        let staged = &tool_mods[0].dir;
        let manifest = fs::read_to_string(staged.join(MANIFEST)).unwrap();
        assert!(manifest.contains("[[module]]"));
        assert!(!manifest.contains("[[injection]]"));
        assert!(manifest.contains("name = \"runtime\""));
        assert!(manifest.contains("source = \"src/runtime.rs\""));
        // `visibility` stays out of the tool manifest until a package sets it.
        assert!(!manifest.contains("visibility"));
        assert_eq!(
            fs::read_to_string(staged.join("src/runtime.rs")).unwrap(),
            "pub(crate) fn bump(value: &mut i32) { *value += 1; }\n"
        );
        assert!(staged.join("src/lib.rs").is_file());
    }

    #[test]
    fn a_module_only_bundle_still_needs_the_tool() {
        let root = super::super::tests::dir("inject-module-only");
        super::super::tests::write_bundle(&root, "example", "");
        let path = root.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/lib.rs'\n";
        fs::write(&path, text).unwrap();
        let bundle = super::super::bundle_in(&root).unwrap().unwrap();
        let mixin_input = root.with_extension("mixins");
        let tool_mods =
            super::super::stage_mixins(std::slice::from_ref(&bundle), &mixin_input).unwrap();
        assert_eq!(tool_mods.len(), 1);
        assert!(tool_mods[0].dir.join(MANIFEST).is_file());
    }

    #[test]
    fn callback_edits_reuse_the_patch_but_invalidate_the_installed_build() {
        let (root, bundle) = fixture(
            "api-helper",
            "[[inject]]\nid = 'hook'\nat = 'horizon_client::game::tick'\nkind = 'head'\nwith = 'tick_hook'\nhash = 'signature'\n",
        );
        let key = || {
            super::super::inject_cache::key(
                "revision",
                "mixins",
                super::super::Profile::Balanced,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let installed = || {
            super::super::version_for(
                "revision",
                "mixins",
                super::super::Profile::Balanced,
                super::super::Target::Client,
                None,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let before = key();
        let built = installed();
        fs::write(root.join("src/lib.rs"), "fn changed() {}\n").unwrap();
        assert_eq!(before, key());
        assert_ne!(built, installed());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn module_source_edits_invalidate_the_patch_key() {
        let (root, _) = fixture("module-cache", "");
        let path = root.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap()
            + "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        fs::write(path, text).unwrap();
        fs::write(root.join("src/runtime.rs"), "pub fn run() {}\n").unwrap();
        let bundle = super::super::bundle_in(&root).unwrap().unwrap();
        let key = || {
            super::super::inject_cache::key(
                "revision",
                "mixins",
                super::super::Profile::Balanced,
                std::slice::from_ref(&bundle),
            )
            .unwrap()
        };
        let before = key();
        fs::write(
            root.join("src/runtime.rs"),
            "pub fn run() { let _value = 7; }\n",
        )
        .unwrap();
        assert_ne!(before, key());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_manual_tool_crate_declarations() {
        let (root, _) = fixture("tool-route", "");
        let path = root.join(horizon_mod_package::manifest::MANIFEST);
        let text = fs::read_to_string(&path).unwrap();
        fs::write(path, text + "[[crate]]\nid='code'\npath='src'\n").unwrap();
        assert!(super::super::bundle_in(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installed_build_key_tracks_dispatch_order() {
        let (root, first) = fixture("order", "");
        let mut second = first.clone();
        second.manifest.id = "second".into();
        let a = super::super::version_for(
            "0123456789012345678901234567890123456789",
            "mixins",
            super::super::Profile::Balanced,
            super::super::Target::Client,
            None,
            &[first.clone(), second.clone()],
        )
        .unwrap();
        let b = super::super::version_for(
            "0123456789012345678901234567890123456789",
            "mixins",
            super::super::Profile::Balanced,
            super::super::Target::Client,
            None,
            &[second, first],
        )
        .unwrap();
        assert_ne!(a, b);
        fs::remove_dir_all(root).unwrap();
    }
}
