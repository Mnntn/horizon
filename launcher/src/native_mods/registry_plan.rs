//! One dependency plan shared by source staging and immutable prebuilt SDK builds.
use super::{api::Project, crates::Wanted, registry_lock::RegistryLock};
use std::path::Path;

pub(super) struct Plan {
    pub game: RegistryLock,
    pub mods: RegistryLock,
    pub allowed: RegistryLock,
    owned: Vec<(String, String, RegistryLock)>,
    roots: std::collections::BTreeSet<String>,
}

impl Plan {
    pub fn read(game: &str, projects: &[Project<'_>], wanted: &[Wanted]) -> Result<Self, String> {
        let game = RegistryLock::parse(game)?;
        let mut mods = RegistryLock::default();
        let mut owned = Vec::new();
        let roots = projects
            .iter()
            .map(|project| project.package.clone())
            .collect();
        for project in projects {
            let bundle = project.bundle;
            let requested: Vec<_> = wanted
                .iter()
                .filter(|w| w.mod_id == bundle.manifest.id)
                .collect();
            if requested.is_empty() {
                continue;
            }
            let pins = RegistryLock::parse(&super::read_lock(&bundle.dir)?)?;
            for request in requested {
                if !pins
                    .packages
                    .keys()
                    .any(|id| id.name == request.name && request.requirement.matches(&id.version))
                {
                    return Err(format!(
                        "{}: Cargo.lock does not pin {} {}",
                        request.mod_id, request.name, request.version
                    ));
                }
            }
            mods.extend(&pins)?;
            owned.push((bundle.manifest.id.clone(), project.package.clone(), pins));
        }
        let mut allowed = game.clone();
        allowed.extend(&mods)?;
        Ok(Self {
            game,
            mods,
            allowed,
            owned,
            roots,
        })
    }

    pub fn check_mods(&self, resolved: &RegistryLock) -> Result<(), String> {
        for (id, root, pins) in &self.owned {
            let mut skip_local: std::collections::BTreeSet<String> = self
                .roots
                .iter()
                .filter(|other| *other != root)
                .cloned()
                .collect();
            skip_local.extend(
                super::api::PACKAGES
                    .iter()
                    .map(|(package, _)| (*package).to_owned()),
            );
            skip_local.insert(super::internals::package(id));
            resolved.check_mod(id, root, &skip_local, pins)?;
        }
        Ok(())
    }

    pub fn explain_resolution(&self, error: String) -> String {
        let mut details = Vec::new();
        for id in self
            .mods
            .packages
            .keys()
            .filter(|id| error.contains(&id.name))
        {
            let versions = self
                .game
                .packages
                .keys()
                .filter(|game| game.name == id.name && game.version != id.version)
                .map(|game| game.version.to_string())
                .collect::<Vec<_>>();
            if !versions.is_empty() {
                details.push(format!(
                    "{}: game locks [{}], mod locks {}; the game graph must not move",
                    id.name,
                    versions.join(", "),
                    id.version
                ));
            }
        }
        if details.is_empty() {
            error
        } else {
            format!("{error}\n\n{}", details.join("\n"))
        }
    }

    pub fn fetch(
        &self,
        rust: &crate::rust_toolchain::Toolchain,
        directory: &Path,
    ) -> Result<(), String> {
        self.mods.fetch_workspace(&self.mods, directory)?;
        let mut index = super::cargo(rust, directory, "update");
        index.arg("--workspace");
        super::run(index, "Checking the pinned registry index", |_| {})
            .map_err(|error| self.explain_resolution(error))?;
        let pins = RegistryLock::parse(&super::read_lock(directory)?)?;
        self.allowed
            .check_resolution(&pins, &RegistryLock::default())?;
        let mut fetch = super::cargo(rust, directory, "fetch");
        fetch.arg("--locked");
        super::run(fetch, "Downloading the mods' pinned crates", |_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_malformed_and_bad_checksum_locks_refuse_before_fetch() {
        let dir = tempfile::tempdir().unwrap();
        super::super::tests::write_bundle(dir.path(), "example", "");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='example'\nversion='1.0.0'\n[dependencies]\nhorizon-mod-api='0.1'\nitoa='1'\n").unwrap();
        let bundle = super::super::bundle_in(dir.path()).unwrap().unwrap();
        let projects = [Project::read(&bundle).unwrap()];
        let wanted = [Wanted {
            mod_id: "example".into(),
            name: "itoa".into(),
            version: "1".into(),
            requirement: semver::VersionReq::parse("1").unwrap(),
        }];
        let game = "version=4\npackage=[]\n";
        let missing = Plan::read(game, &projects, &wanted).err().unwrap();
        assert!(missing.contains("Cargo.lock"), "{missing}");
        for (text, expected) in [("version=4", "missing packages"), ("version=4\n[[package]]\nname='itoa'\nversion='1.0.18'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='bad'\n", "checksum")] {
            std::fs::write(dir.path().join("Cargo.lock"), text).unwrap();
            let error = Plan::read(game, &projects, &wanted).err().unwrap();
            assert!(error.contains(expected), "{error}");
        }
        assert!(!dir.path().join("target").exists());
    }
}
