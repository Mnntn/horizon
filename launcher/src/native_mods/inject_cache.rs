//! Reusable, immutable mixin output. API packages are staged after this snapshot.
use super::{Bundle, Profile};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Hash all mixin inputs but not an API crate that mixins do not read.
pub(super) fn key(
    revision: &str,
    mixins: &str,
    profile: Profile,
    mods: &[Bundle],
) -> Result<String, String> {
    let mut hash = Sha256::new();
    for part in [
        // Bump when launcher generation or validation changes the snapshot's contents.
        "inject-mixin-split-v2",
        super::STAGING,
        revision,
        mixins,
        // The Cargo profile: `Balanced` and `Developer` stage the same source.
        profile.cargo().0,
        super::rust_toolchain::VERSION,
    ] {
        field(&mut hash, part.as_bytes());
    }
    for bundle in mods {
        field(&mut hash, b"bundle");
        field(&mut hash, bundle.manifest.id.as_bytes());
        // API-only edits do not change patched source. Supporting modules may
        // include other crate files, so hash their source conservatively.
        let reads_source = bundle.needs_mixins();
        for (name, path) in super::bundle_files(bundle)? {
            if !reads_source
                && (name == "Cargo.toml" || name == "build.rs" || name.starts_with("src/"))
            {
                continue;
            }
            field(&mut hash, b"file");
            field(&mut hash, name.as_bytes());
            field(
                &mut hash,
                &fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn snapshot(root: &Path, key: &str) -> PathBuf {
    root.join("patched").join(key)
}
