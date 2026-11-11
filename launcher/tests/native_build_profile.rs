//! Exercises the real CLI and SDK planner with an installed release and API-only mod.
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination);
        } else {
            fs::copy(&source, &destination).unwrap();
        }
    }
}

fn resource(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("set {name}")))
        .canonicalize()
        .unwrap()
}

#[test]
#[ignore = "requires a prepared installed release, a matching API-only native mod and a downloadable prebuilt SDK"]
fn native_build_folder_uses_sdk_unless_profile_is_explicit() {
    for name in [
        "HORIZON_NATIVE_BUILD_TEST_RELEASE",
        "HORIZON_NATIVE_BUILD_TEST_MOD",
    ] {
        if std::env::var_os(name).is_none() {
            eprintln!(
                "Skipping native build profile test: set {name} to provide optional fixtures"
            );
            return;
        }
    }
    let release = resource("HORIZON_NATIVE_BUILD_TEST_RELEASE");
    let project = resource("HORIZON_NATIVE_BUILD_TEST_MOD");
    let version = release.file_name().unwrap().to_str().unwrap();
    assert!(release.join(".installed").is_file());
    assert!(!release.join("developer.json").exists());
    assert!(!release.join("native-mods.json").exists());

    for (requested, saved, expected) in [
        (
            None,
            "balanced",
            "Prebuilt game: only the mods compile (optimized)",
        ),
        (
            Some("balanced"),
            "optimized",
            "First build: the whole game (balanced)",
        ),
        (
            Some("developer"),
            "optimized",
            "First build: the whole game (developer)",
        ),
    ] {
        // A fresh home prevents a previous native build from hiding the route as up to date.
        let scratch = tempfile::tempdir().unwrap();
        let launcher = scratch.path().join("launcher");
        copy_tree(&release, &launcher.join("versions").join(version));
        let folder = scratch.path().join("mod");
        copy_tree(&project, &folder);
        fs::write(
            launcher.join("instances.json"),
            serde_json::json!({"instances": {"horizon": {"native_profile": saved}}}).to_string(),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_horizon-launcher"));
        command
            .arg("native-build")
            .arg(&folder)
            .args([
                "--version",
                version,
                "--instance",
                "horizon",
                "--trust-native-code",
            ])
            .current_dir(scratch.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for name in [
            "HORIZON_HOME",
            "HORIZON_LAUNCHER_HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "HORIZON_LAUNCHER_TRASH",
            "HORIZON_LAUNCHER_CLIPBOARD",
            "HORIZON_LAUNCHER_DESKTOP",
            "CARGO_HOME",
            "RUSTUP_HOME",
        ] {
            command.env(
                name,
                if name == "HORIZON_LAUNCHER_HOME" {
                    launcher.clone()
                } else {
                    scratch.path().join(name)
                },
            );
        }
        command
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8");
        if let Some(requested) = requested {
            command.args(["--profile", requested]);
        }
        let mut child = command.spawn().unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let mut output = String::new();
        let mut plan = None;
        for line in stderr.lines() {
            let line = line.unwrap();
            output.push_str(&line);
            output.push('\n');
            if line.starts_with("Prebuilt game: ") || line.starts_with("First build: ") {
                plan = Some(line);
                break;
            }
        }
        // The plan is emitted only after the real SDK has been fetched and checked. Stop before
        // compiling a complete game: this test verifies route selection, not the compiler.
        if plan.is_some() {
            child.kill().unwrap();
        }
        let status = child.wait().unwrap();
        assert_eq!(
            plan.as_deref(),
            Some(expected),
            "{requested:?}: {status}\n{output}"
        );
        let store: serde_json::Value =
            serde_json::from_slice(&fs::read(launcher.join("instances.json")).unwrap()).unwrap();
        assert_eq!(
            store["instances"]["horizon"]["native_profile"],
            requested.unwrap_or(saved)
        );
    }
}
