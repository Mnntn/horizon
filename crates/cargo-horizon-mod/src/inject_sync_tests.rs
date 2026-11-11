use super::*;
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    mod_dir: PathBuf,
    repo: PathBuf,
}
impl Fixture {
    fn function(name: &str, id: &str) -> Self {
        Self::new(name, id, &entry("function", "lower::run", "head", ""))
    }
    fn new(name: &str, id: &str, declarations: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/inject-sync-{name}-{}",
            std::process::id()
        ));
        let mod_dir = root.join("mod");
        let repo = root.join("game");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::create_dir_all(repo.join(DIRECTORY)).unwrap();
        fs::write(repo.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(mod_dir.join(MANIFEST), format!("id = '{id}'\nname = 'Test'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n{declarations}", "a".repeat(40), "b".repeat(40))).unwrap();
        Self {
            root,
            mod_dir,
            repo,
        }
    }
    fn file(&self, name: &str) -> PathBuf {
        self.repo.join(DIRECTORY).join(name)
    }
    fn run(&self) -> Result<(), String> {
        sync(&self.mod_dir, &self.repo)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn entry(id: &str, at: &str, kind: &str, call: &str) -> String {
    format!(
        "\n[[inject]]\nid = '{id}'\nat = '{at}'\nkind = '{kind}'\nwith = 'listener'\nhash = 'signature'\n{call}"
    )
}

#[test]
fn inject_sync_sorted_union_and_idempotence() {
    let declarations = entry("function", "lower::run", "head", "")
        + &entry(
            "call",
            "lower::run",
            "wrap_call",
            "call = 'lower::callee'\nnth = 2\nctx = 'context-two'\n",
        )
        + &entry("shared", "lower::shared", "return", "");
    let f = Fixture::new("union", "example", &declarations);
    fs::write(f.file("other.txt"), "lower::shared\n").unwrap();
    fs::write(f.file("example.txt"), "z::old\na::old\n").unwrap();
    f.run().unwrap();
    let expected = "# example: written by cargo horizon-mod inject sync\na::old\nlower::run\nlower::run | lower::callee | 2 | context-two\nz::old\n";
    assert_eq!(fs::read_to_string(f.file("example.txt")).unwrap(), expected);
    f.run().unwrap();
    assert_eq!(fs::read_to_string(f.file("example.txt")).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(f.file("other.txt")).unwrap(),
        "lower::shared\n"
    );
}

#[test]
fn inject_sync_conflicts_name_both_files_and_do_not_write() {
    let declarations = entry("new", "new::function", "head", "")
        + &entry(
            "call",
            "lower::run",
            "before_call",
            "call = 'lower::callee'\nnth = 2\nctx = 'new-context'\n",
        );
    let f = Fixture::new("conflict", "example", &declarations);
    fs::write(
        f.file("other.txt"),
        "lower::run | lower::callee | 2 | old-context\n",
    )
    .unwrap();
    fs::write(f.file("example.txt"), "existing::function\n").unwrap();
    let error = f.run().unwrap_err();
    assert!(
        error.contains("other.txt:1") && error.contains("example.txt:"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(f.file("example.txt")).unwrap(),
        "existing::function\n"
    );
    fs::write(
        f.file("a.txt"),
        "lower::run | lower::callee | 2 | third-context\n",
    )
    .unwrap();
    let error = f.run().unwrap_err();
    assert!(
        error.contains("a.txt:1") && error.contains("other.txt:1"),
        "{error}"
    );
}

#[test]
fn inject_sync_refuses_owner_escape_and_reserved_seed() {
    for (name, id) in [
        ("reserved", "horizon"),
        ("escape", "../escape"),
        ("charset", "bad-id"),
    ] {
        let f = Fixture::new(name, id, &entry("function", "lower::run", "head", ""));
        fs::write(f.file("horizon.txt"), "engine::seed\n").unwrap();
        assert!(f.run().is_err(), "accepted {id}");
        assert_eq!(
            fs::read_to_string(f.file("horizon.txt")).unwrap(),
            "engine::seed\n"
        );
        assert!(
            !f.repo
                .join("crates/horizon-mod-api-gen/escape.txt")
                .exists()
        );
    }
}

#[test]
fn inject_sync_refuses_invalid_fields_and_missing_repository() {
    for (name, at) in [
        ("syntax", "not a path"),
        ("newline", "lower::run\nother::run"),
        ("comment", "lower::run#hidden"),
    ] {
        let declarations = entry("function", at, "head", "");
        let f = Fixture::new(name, "example", &declarations);
        assert!(f.run().is_err());
        assert!(!f.file("example.txt").exists());
    }
    let f = Fixture::new(
        "repo",
        "example",
        &entry("function", "lower::run", "head", ""),
    );
    fs::remove_file(f.repo.join("Cargo.toml")).unwrap();
    assert!(f.run().unwrap_err().contains("not a Horizon workspace"));
    assert!(!f.file("example.txt").exists());
    for args in [
        vec![],
        vec!["--repo"],
        vec!["--repo", "-invalid"],
        vec!["--other", "path"],
    ] {
        assert!(run(&args.into_iter().map(String::from).collect::<Vec<_>>()).is_err());
    }
}

#[cfg(unix)]
#[test]
fn reviewer_inject_does_not_overwrite_symlink_target() {
    let f = Fixture::function("review-link", "example");
    let target = f.root.join("user-data.txt");
    fs::write(&target, "user::slot\n").unwrap();
    std::os::unix::fs::symlink(&target, f.file("example.txt")).unwrap();
    f.run().unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "user::slot\n");
    assert!(
        !fs::symlink_metadata(f.file("example.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn reviewer_unchanged_file_keeps_mtime() {
    let f = Fixture::function("review-mtime", "example");
    f.run().unwrap();
    let output = f.file("example.txt");
    let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    fs::File::open(&output)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old))
        .unwrap();
    f.run().unwrap();
    assert_eq!(fs::metadata(output).unwrap().modified().unwrap(), old);
}

#[test]
fn reviewer_rejects_non_native() {
    let f = Fixture::new("review-nonnative", "example", "");
    fs::write(f.mod_dir.join(MANIFEST), "id = 'example'\nname = 'Test'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n").unwrap();
    assert!(f.run().is_err());
    assert!(!f.file("example.txt").exists());
}

#[test]
fn reviewer_uses_manifest_owner() {
    let f = Fixture::function("review-owner", "second");
    f.run().unwrap();
    assert!(f.file("second.txt").exists());
    assert!(!f.file("example.txt").exists());
}

#[test]
fn reviewer_preserves_user_comment() {
    let f = Fixture::function("review-comment", "example");
    for text in [
        "# downstream patch\nuser::slot\n",
        "user::slot # keep this reason\n",
    ] {
        fs::write(f.file("example.txt"), text).unwrap();
        let error = f.run().unwrap_err();
        assert!(
            error.contains("example.txt:1:") && error.contains("move the comment out"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(f.file("example.txt")).unwrap(), text);
    }
}
