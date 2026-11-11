//! One source staging pass for the launcher and `horizon-stage`.
use crate::{internals, replace};
use std::path::Path;

/// Filename of the versioned SDK tool catalog.
pub const SDK_CATALOG_FILE: &str = "mod-catalog.json";

/// Discovery instrumentation must never be used to build or publish a game.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Widen visibility and give functions their replacement slots, without diagnostic attributes.
    Build,
    /// Add internal-use diagnostics for an isolated metadata-only mod check.
    Discovery,
}

/// Counts and catalog produced by source staging.
pub struct Staged {
    /// Replacement slot counts.
    pub replace: replace::Staged,
    /// Exposed internal signatures and changed visibility counts.
    pub internals: internals::Widened,
}

/// Expose internals, give functions their replacement slots and write the internals manifest in
/// a disposable source copy. Internals precede the slots so their catalog contains only the
/// original game items. A pass stamps the copy before edits; even a failed pass requires fresh
/// source for a retry.
pub fn stage(
    root: &Path,
    internals_json: &Path,
    mode: Mode,
    api_version: &str,
) -> Result<Staged, String> {
    stage_with_catalog(root, internals_json, mode, || {
        crate::sdk_catalog::sdk_catalog(root, api_version)
    })
}

fn stage_with_catalog(
    root: &Path,
    internals_json: &Path,
    mode: Mode,
    catalog: impl FnOnce() -> Result<String, String>,
) -> Result<Staged, String> {
    let stamp = root.join(".horizon-source-staged");
    if stamp.exists() {
        return Err("source tree is already staged; use a fresh copy of the source".into());
    }
    std::fs::write(
        &stamp,
        "Source staging started. Use a fresh copy to stage again.\n",
    )
    .map_err(|error| format!("{}: {error}", stamp.display()))?;
    if matches!(mode, Mode::Build) {
        let destination = internals_json.with_file_name(SDK_CATALOG_FILE);
        std::fs::write(&destination, catalog()?)
            .map_err(|e| format!("{}: {e}", destination.display()))?;
    }
    let mut internals = match mode {
        Mode::Build => internals::widen(root)?,
        Mode::Discovery => internals::discover(root)?,
    };
    internals
        .entries
        .extend(crate::crossing::foreign_internals());
    let text = serde_json::to_string_pretty(&internals.entries).map_err(|e| e.to_string())?;
    std::fs::write(internals_json, text + "\n")
        .map_err(|error| format!("{}: {error}", internals_json.display()))?;
    let replace = replace::stage(root)?;
    Ok(Staged { replace, internals })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_writes_the_catalog_and_slots_and_refuses_reuse() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("crates/horizon-client/src/game.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "impl Game<'_> { fn unit(&self) {} }").unwrap();
        std::fs::write(source.with_file_name("entry.rs"), "").unwrap();
        std::fs::write(source.with_file_name("lib.rs"), "mod game; mod entry;\n").unwrap();
        std::fs::write(
            root.join("crates/horizon-client/Cargo.toml"),
            "[package]\nname = 'horizon-client'\n",
        )
        .unwrap();
        let lists = root.join("crates/horizon-mod-api-gen");
        std::fs::create_dir_all(&lists).unwrap();
        std::fs::write(lists.join("internals.txt"), "").unwrap();
        let internals = root.join("internals.json");
        let stage = |mode| stage_with_catalog(root, &internals, mode, || Ok("{}\n".into()));
        let staged = stage(Mode::Build).unwrap();
        assert_eq!(staged.replace.functions, 1);
        assert_eq!(
            std::fs::read_to_string(root.join(SDK_CATALOG_FILE)).unwrap(),
            "{}\n"
        );
        assert_eq!(std::fs::read(&internals).unwrap().last(), Some(&b'\n'));
        let before = std::fs::read(&source).unwrap();
        assert!(String::from_utf8_lossy(&before).contains("__fnr_unit"));
        for mode in [Mode::Build, Mode::Discovery] {
            let error = stage(mode).err().expect("staging twice must fail");
            assert_eq!(
                error,
                "source tree is already staged; use a fresh copy of the source"
            );
            assert_eq!(std::fs::read(&source).unwrap(), before);
        }
    }
}
