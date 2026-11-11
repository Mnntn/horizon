//! The one change function replacement carries on the pinned horizon-mixins until upstream takes
//! it: `tools/mixins/narrow-reference-rule.patch`, which narrows the tool's "calls beneath a
//! reference" rule from the whole function to the site a hook rewrites. The launcher downloads the
//! tool's source at the pin, applies the patch only to the exact text it was written against, and
//! builds the patched tool beside the plain one.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{root, run, Step, MIXINS_REV};
use crate::rust_toolchain::{self, Toolchain};

const PATCH: &str = include_str!("../../../tools/mixins/narrow-reference-rule.patch");
const PATCH_NAME: &str = "tools/mixins/narrow-reference-rule.patch";
const SOURCE: &str = "https://codeload.github.com/horizon-rust/horizon-mixins/tar.gz";

/// The patched tool at [`MIXINS_REV`], built once per pin and patch.
pub(super) fn tool(rust: &Toolchain, progress: &impl Fn(Step)) -> Result<PathBuf, String> {
    let hash = format!("{:x}", Sha256::digest(PATCH.as_bytes()));
    // Inside the pin's own tool folder, so storage cleanup counts it with the pin.
    let dir = root()
        .join("tools")
        .join(&MIXINS_REV[..12])
        .join(format!("patched-{}", &hash[..12]));
    let bin = dir
        .join("bin")
        .join(format!("horizon-mixins{}", std::env::consts::EXE_SUFFIX));
    if bin.is_file() {
        return Ok(bin);
    }
    progress(Step::Tool);
    let partial = dir.join(".source");
    let top = super::unpack_archive(&format!("{SOURCE}/{MIXINS_REV}"), &partial, 256 << 20)?;
    apply(&top, PATCH).map_err(|why| {
        format!("E_MIXINS_PATCH: {PATCH_NAME} does not apply to horizon-mixins {MIXINS_REV}: {why}")
    })?;
    let mut cmd = rust.command(&rust.cargo());
    rust_toolchain::low_priority(&mut cmd);
    rust.configure_cargo(&mut cmd);
    cmd.args(["install", "--locked", "--path"])
        .arg(top.join("crates/horizon-mixins-cli"))
        .arg("--root")
        .arg(&dir);
    let built = run(cmd, "Building horizon-mixins", |_| {});
    let _ = fs::remove_dir_all(&partial);
    built?;
    Ok(bin)
}

/// Applies the unified diff `patch` to the files under `root`. Every hunk must find its old
/// lines exactly where the diff says; nothing is written unless every file's hunks apply.
fn apply(root: &Path, patch: &str) -> Result<(), String> {
    let mut changed = Vec::new();
    let mut lines = patch.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(path) = line.strip_prefix("+++ b/") else {
            continue;
        };
        let file = root.join(path);
        let text = fs::read_to_string(&file).map_err(|e| format!("{path}: {e}"))?;
        let old: Vec<&str> = text.lines().collect();
        let mut new: Vec<String> = Vec::new();
        let mut at = 0;
        while let Some(header) = lines.next_if(|l| l.starts_with("@@ ")) {
            let start: usize = header
                .strip_prefix("@@ -")
                .and_then(|rest| rest.split([',', ' ']).next())
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| format!("{path}: bad hunk header `{header}`"))?;
            let start = start.saturating_sub(1);
            if start < at {
                return Err(format!("{path}: hunks out of order at `{header}`"));
            }
            new.extend(old[at..start.min(old.len())].iter().map(|l| l.to_string()));
            at = start;
            while let Some(body) = lines.next_if(|l| {
                l.starts_with([' ', '-', '+']) && !l.starts_with("--- ") && !l.starts_with("+++ ")
                    || l.is_empty()
            }) {
                let (mark, rest) = body.split_at(body.len().min(1));
                match mark {
                    "+" => new.push(rest.to_owned()),
                    // An empty line in a diff is an unchanged empty line.
                    " " | "" | "-" => {
                        if old.get(at) != Some(&rest) {
                            return Err(format!("{path}: line {} differs at `{header}`", at + 1));
                        }
                        at += 1;
                        if mark != "-" {
                            new.push(rest.to_owned());
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
        new.extend(old[at..].iter().map(|l| l.to_string()));
        let mut out = new.join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        changed.push((file, out));
    }
    if changed.is_empty() {
        return Err("the patch changes no file".into());
    }
    for (file, text) in changed {
        fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -2,3 +2,4 @@ fn a()
 two
-three
+three, narrowed
+three and a half
 four
";

    fn tree(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), text).unwrap();
        dir
    }

    #[test]
    fn a_hunk_applies_where_its_old_lines_are() {
        let dir = tree("one\ntwo\nthree\nfour\nfive\n");
        apply(dir.path(), DIFF).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "one\ntwo\nthree, narrowed\nthree and a half\nfour\nfive\n"
        );
    }

    #[test]
    fn changed_source_is_refused_and_left_alone() {
        let dir = tree("one\ntwo\nTHREE\nfour\n");
        let why = apply(dir.path(), DIFF).unwrap_err();
        assert!(why.contains("line 3 differs"), "{why}");
        assert_eq!(
            fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "one\ntwo\nTHREE\nfour\n"
        );
    }
}
