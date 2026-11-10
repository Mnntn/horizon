//! The trust boundary for a mod-owned registry lock. Never resolve missing pins online.
use std::collections::BTreeMap;

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Identity {
    pub name: String,
    pub version: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub(super) struct RegistryLock {
    pub packages: BTreeMap<Identity, toml::Table>,
}

impl RegistryLock {
    /// Validate once, before any fetch. Local package records are deliberately not imported:
    /// the staging boundary owns their generated manifests and source paths.
    pub fn parse(text: &str) -> Result<Self, String> {
        let lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get("package")
            .and_then(toml::Value::as_array)
            .ok_or("Cargo.lock: missing packages")?;
        let mut packages = BTreeMap::new();
        for record in records {
            let table = record.as_table().ok_or("Cargo.lock: invalid package")?;
            let field = |key| {
                table
                    .get(key)
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("Cargo.lock: package missing {key}"))
            };
            let name = field("name")?;
            let version = field("version")?;
            semver::Version::parse(version).map_err(|e| format!("Cargo.lock: {name}: {e}"))?;
            let Some(source) = table.get("source") else {
                continue;
            };
            if source.as_str() != Some(CRATES_IO) {
                return Err(format!(
                    "Cargo.lock: {name} {version}: only crates.io sources are allowed"
                ));
            }
            let checksum = field("checksum")?;
            if checksum.len() != 64
                || !checksum
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(format!(
                    "Cargo.lock: {name} {version}: invalid crates.io checksum"
                ));
            }
            let identity = Identity {
                name: name.into(),
                version: version.into(),
                source: CRATES_IO.into(),
            };
            if packages.insert(identity, table.clone()).is_some() {
                return Err(format!("Cargo.lock: duplicate package {name} {version}"));
            }
        }
        Ok(Self { packages })
    }

    /// The same package identity must never acquire a different checksum or dependency list.
    pub fn extend(&mut self, other: &Self) -> Result<(), String> {
        for (identity, record) in &other.packages {
            if let Some(previous) = self.packages.get(identity) {
                if previous != record {
                    return Err(format!(
                        "Cargo.lock: conflicting pins for {} {}",
                        identity.name, identity.version
                    ));
                }
            } else {
                self.packages.insert(identity.clone(), record.clone());
            }
        }
        Ok(())
    }
    /// A closed, inert fetch workspace: no mod manifest, source, SDK path or Cargo config.
    pub fn fetch_workspace(&self, roots: &Self, dir: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
        let mut dependencies = toml::Table::new();
        let mut refs = Vec::new();
        for (index, identity) in roots.packages.keys().enumerate() {
            let mut dep = toml::Table::new();
            dep.insert("package".into(), identity.name.clone().into());
            dep.insert("version".into(), format!("={}", identity.version).into());
            dep.insert("default-features".into(), false.into());
            dependencies.insert(format!("pin-{index}"), dep.into());
            refs.push(toml::Value::String(format!(
                "{} {} ({})",
                identity.name, identity.version, identity.source
            )));
        }
        let manifest = toml::toml! {
            [package]
            name = "horizon-registry-fetch"
            version = "0.0.0"
            edition = "2021"
            [workspace]
        };
        let mut manifest = manifest;
        manifest.insert("dependencies".into(), dependencies.into());
        let mut root = toml::toml! { name = "horizon-registry-fetch" version = "0.0.0" };
        root.insert("dependencies".into(), refs.into());
        let mut records = vec![toml::Value::Table(root)];
        records.extend(self.packages.values().cloned().map(toml::Value::Table));
        let mut lock = toml::Table::new();
        lock.insert("version".into(), 4.into());
        lock.insert("package".into(), records.into());
        for (file, text) in [
            ("Cargo.toml", manifest.to_string()),
            ("Cargo.lock", lock.to_string()),
            ("src/lib.rs", String::new()),
        ] {
            std::fs::write(dir.join(file), text).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Compare resolved identities and checksums, and normalize Cargo's optional dependency
    /// qualifiers before comparing edges. Adding a second version qualifies existing edges.
    pub fn check_resolution(&self, resolved: &Self, game: &Self) -> Result<(), String> {
        for (id, record) in &resolved.packages {
            let Some(pin) = self.packages.get(id) else {
                let versions = game
                    .packages
                    .keys()
                    .filter(|old| old.name == id.name)
                    .map(|old| old.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Cargo.lock: {} moved from [{}] to {}; resolution is not pinned by a mod lock",
                    id.name, versions, id.version
                ));
            };
            if pin.get("checksum") != record.get("checksum") {
                return Err(format!(
                    "Cargo.lock: checksum changed for {} {}",
                    id.name, id.version
                ));
            }
        }
        for id in game.packages.keys() {
            let Some(record) = resolved.packages.get(id) else {
                let versions = resolved
                    .packages
                    .keys()
                    .filter(|new| new.name == id.name)
                    .map(|new| new.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Cargo.lock: game crate {} moved from {} to [{}]",
                    id.name, id.version, versions
                ));
            };
            if game.edges(id)? != resolved.edges(id)?
                || game.packages[id].get("checksum") != record.get("checksum")
            {
                return Err(format!(
                    "Cargo.lock: game dependency graph changed for {} {}",
                    id.name, id.version
                ));
            }
        }
        Ok(())
    }

    fn edges(&self, id: &Identity) -> Result<std::collections::BTreeSet<Identity>, String> {
        let mut edges = std::collections::BTreeSet::new();
        for dep in self.packages[id]
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            let dep = dep
                .as_str()
                .ok_or("Cargo.lock: invalid dependency reference")?;
            let mut parts = dep.split_whitespace();
            let name = parts
                .next()
                .ok_or("Cargo.lock: empty dependency reference")?;
            let version = parts.next();
            let matches: Vec<_> = self
                .packages
                .keys()
                .filter(|id| id.name == name && version.is_none_or(|v| v == id.version))
                .collect();
            if matches.len() != 1 {
                return Err(format!("Cargo.lock: ambiguous or missing dependency {dep}"));
            }
            edges.insert(matches[0].clone());
        }
        Ok(edges)
    }

    pub fn seed(&self, text: &str) -> Result<String, String> {
        let mut lock: toml::Table = text.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        let records = lock
            .get_mut("package")
            .and_then(toml::Value::as_array_mut)
            .ok_or("Cargo.lock: missing packages")?;
        records.retain(|record| record.get("source").is_none());
        records.extend(self.packages.values().cloned().map(toml::Value::Table));
        Ok(lock.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn lock(source: &str, checksum: &str) -> String {
        format!("version = 4\n[[package]]\nname = 'itoa'\nversion = '1.0.15'\nsource = '{source}'\nchecksum = '{checksum}'\n")
    }
    #[test]
    fn game_version_movement_is_named_and_added_versions_are_allowed() {
        let game = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let other =
            RegistryLock::parse(&lock(CRATES_IO, &"b".repeat(64)).replace("1.0.15", "0.4.8"))
                .unwrap();
        let mut allowed = game.clone();
        allowed.extend(&other).unwrap();
        allowed.check_resolution(&allowed, &game).unwrap();
        let error = allowed.check_resolution(&other, &game).unwrap_err();
        assert!(
            error.contains("itoa") && error.contains("1.0.15") && error.contains("0.4.8"),
            "{error}"
        );
    }

    #[test]
    fn fetch_workspace_disables_default_features_and_excludes_local_packages() {
        let pins = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let dir = tempfile::tempdir().unwrap();
        pins.fetch_workspace(&pins, dir.path()).unwrap();
        let manifest: toml::Table = std::fs::read_to_string(dir.path().join("Cargo.toml"))
            .unwrap()
            .parse()
            .unwrap();
        let dep = &manifest["dependencies"]["pin-0"];
        assert_eq!(dep["version"].as_str(), Some("=1.0.15"));
        assert_eq!(dep["default-features"].as_bool(), Some(false));
        assert!(!dir.path().join(".cargo").exists());
        assert!(pins
            .seed("version = 4\n[[package]]\nname = 'local-game'\nversion = '0.1.0'\n")
            .unwrap()
            .contains("local-game"));
    }

    #[test]
    fn source_and_checksum_boundary() {
        let checksum = "a".repeat(64);
        assert_eq!(
            RegistryLock::parse(&lock(CRATES_IO, &checksum))
                .unwrap()
                .packages
                .len(),
            1
        );
        for source in [
            "git+https://example.invalid/repo",
            "registry+https://example.invalid/index",
            "sparse+https://index.crates.io/",
        ] {
            assert!(RegistryLock::parse(&lock(source, &checksum))
                .unwrap_err()
                .contains("only crates.io"));
        }
        for checksum in ["", "no checksum", &"A".repeat(64), &"a".repeat(63)] {
            assert!(RegistryLock::parse(&lock(CRATES_IO, checksum))
                .unwrap_err()
                .contains("checksum"));
        }
        assert!(RegistryLock::parse("version = 4").is_err());
    }
    #[test]
    fn union_preserves_pins_and_refuses_conflicting_checksums() {
        let mut a = RegistryLock::parse(&lock(CRATES_IO, &"a".repeat(64))).unwrap();
        let same = a.clone();
        a.extend(&same).unwrap();
        assert_eq!(a.packages.len(), 1);
        let different = RegistryLock::parse(&lock(CRATES_IO, &"b".repeat(64))).unwrap();
        assert!(a.extend(&different).unwrap_err().contains("itoa 1.0.15"));
    }
}
