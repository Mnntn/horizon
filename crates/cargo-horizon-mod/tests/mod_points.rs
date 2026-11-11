//! Mod points (#2339) end to end on native: the chemicals fixture library is staged by the build
//! tool's pass, an addon's api and internals listeners are routed and registered with the same
//! emitters the launcher's generated `main` uses, and a real binary runs the patched library.
use horizon_mod_package::manifest::inject::mod_points::{self, Library};
use horizon_mod_package::manifest::inject::{Access, Inject, Kind};
use point_shape::catalog::Catalog;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// `fixtures/chem/<name>` copied to `to`, its mod API dependency pointed at this checkout.
fn copy(name: &str, to: &Path) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/chem")
        .join(name);
    let _ = std::fs::remove_dir_all(to);
    for entry in walk(&from) {
        let relative = entry.strip_prefix(&from).unwrap();
        let target = to.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let mut text = std::fs::read_to_string(&entry).unwrap();
        if relative == Path::new("Cargo.toml") {
            let api = repo().join("crates/horizon-mod-api");
            text = text.replace(
                "\"../../../../../horizon-mod-api\"",
                &format!("{:?}", api.display().to_string()),
            );
        }
        std::fs::write(target, text).unwrap();
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn inject(id: &str, at: &str, kind: Kind, access: Access, with: &str, hash: &str) -> Inject {
    Inject {
        id: id.into(),
        at: at.into(),
        kind,
        with: with.into(),
        access,
        priority: 0,
        hash: Some(hash.into()),
        call: None,
        nth: None,
        ctx: None,
    }
}

const PROBE: &str = r#"
fn main() {
    use std::sync::atomic::Ordering;
    let tanks = || {
        let water = chemlib::Chemical { id: 1, viscosity: 1 };
        [
            chemlib::Tank { chemical: water, amount: 100, capacity: 100 },
            chemlib::Tank { chemical: water, amount: 0, capacity: 100 },
        ]
    };
    let mut before = tanks();
    let unpatched = chemlib::tick(&mut before, 10);
    for slot in [/* SLOTS */] {
        mod_api::__private::points::register(&slot).unwrap();
    }
    mod_api::__private::points::seal();
    let mut after = tanks();
    let patched = chemlib::tick(&mut after, 10);
    let mut ledger = chemlib::tanks::Ledger { entries: Vec::new() };
    chemlib::tanks::record(&mut ledger, 3);
    println!(
        "{unpatched} {patched} {} {} {} {} {}",
        after[1].amount,
        chem_addon::MOVED.load(Ordering::Relaxed),
        chemlib::audit(&after),
        ledger.entries.len(),
        chem_addon::FLOW_RATE.load(Ordering::Relaxed)
    );
}
"#;

#[test]
fn an_addon_patches_a_library_through_its_mod_points() {
    let scratch = repo().join("target/mod-points-e2e");
    let (library, addon, probe) = (
        scratch.join("chemlib"),
        scratch.join("chem_addon"),
        scratch.join("probe"),
    );
    copy("chemlib", &library);
    copy("chem_addon", &addon);
    std::fs::write(
        scratch.join("Cargo.toml"),
        "[workspace]\nmembers = [\"chemlib\", \"chem_addon\", \"probe\"]\nresolver = \"3\"\n",
    )
    .unwrap();
    std::fs::copy(repo().join("Cargo.lock"), scratch.join("Cargo.lock")).unwrap();
    // `inject add` pins a dependency's mod point, and `inject update` refuses once it changed.
    let prebuilt =
        addon.join("target/horizon-mod/prebuilt/0000000000000000000000000000000000000000");
    std::fs::create_dir_all(&prebuilt).unwrap();
    std::fs::write(prebuilt.join("internals.json"), "[]").unwrap();
    std::fs::write(prebuilt.join("points.json"), r#"{"sites":[]}"#).unwrap();
    std::fs::create_dir_all(probe.join("src")).unwrap();
    std::fs::write(
        probe.join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(probe.join("src/main.rs"), "fn main() {}\n").unwrap();
    let tool = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .arg("horizon-mod")
            .args(args)
            .current_dir(&addon)
            .output()
            .unwrap()
    };
    let added = tool(&[
        "inject",
        "add",
        "chemlib::transfer",
        "--kind",
        "head",
        "--with",
        "double_rate",
    ]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&added.stdout),
        "api: available\ninternals: from the library's own target\n"
    );
    let pinned = horizon_mod_package::Manifest::parse(
        &std::fs::read_to_string(addon.join("horizon-mod.toml")).unwrap(),
    )
    .unwrap();
    let listed =
        horizon_mod_api_gen::points::mod_crate::mod_points(&library, &Default::default()).unwrap();
    assert_eq!(
        pinned.inject[0].hash.as_deref(),
        Some(
            listed
                .points
                .iter()
                .find(|p| p.target == "chemlib::transfer")
                .unwrap()
                .hash
                .as_str()
        )
    );
    let opted = tool(&[
        "inject",
        "add",
        "chemlib::audit",
        "--kind",
        "head",
        "--with",
        "double_rate",
    ]);
    assert!(String::from_utf8_lossy(&opted.stderr).contains("opted out"));
    let source = library.join("src/lib.rs");
    let text = std::fs::read_to_string(&source).unwrap();
    std::fs::write(
        &source,
        text.replace("rate: u32) -> u32", "rate: u64) -> u32"),
    )
    .unwrap();
    let update = tool(&["inject", "update"]);
    assert!(!update.status.success());
    assert!(
        String::from_utf8_lossy(&update.stdout)
            .contains("E_INJECT_CHANGED: chem_addon:double_rate: chemlib::transfer"),
        "{}",
        String::from_utf8_lossy(&update.stdout)
    );
    copy("chemlib", &library);
    let catalog: Catalog =
        horizon_mod_api_gen::points::mod_crate::stage_mod(&library, &Default::default()).unwrap();
    assert_eq!(catalog.opted_out, ["chemlib::audit"]);
    let hash = |target: &str| {
        catalog
            .points
            .iter()
            .find(|point| point.target == target)
            .unwrap()
            .hash
            .clone()
    };
    let injects = [
        inject(
            "double",
            "chemlib::transfer",
            Kind::Head,
            Access::Api,
            "double_rate",
            &hash("chemlib::transfer"),
        ),
        inject(
            "seen",
            "chemlib::transfer",
            Kind::Return,
            Access::Internals,
            "saw_moved",
            &hash("chemlib::transfer"),
        ),
        inject(
            "flow",
            "chemlib::flow",
            Kind::Head,
            Access::Api,
            "saw_flow",
            &hash("chemlib::flow"),
        ),
        inject(
            "ledger",
            "chemlib::tanks::record",
            Kind::Head,
            Access::Internals,
            "mark_ledger",
            &hash("chemlib::tanks::record"),
        ),
    ];
    let shared = Library {
        id: "chemlib",
        version: "1.0.0",
        catalog: &catalog,
    };
    // The ledger has no Wire form: its api listeners are refused naming it, its internals kept.
    let mut api_ledger = injects[3].clone();
    api_ledger.access = Access::Api;
    assert!(
        mod_points::route("chem_addon", &api_ledger, &shared)
            .unwrap_err()
            .to_string()
            .contains("`ledger` (`&mutLedger`) has no API type yet")
    );
    let audit = inject(
        "audit",
        "chemlib::audit",
        Kind::Head,
        Access::Api,
        "double_rate",
        "0",
    );
    assert!(
        mod_points::route("chem_addon", &audit, &shared)
            .unwrap_err()
            .to_string()
            .contains("opted out")
    );
    let table = "chemlib::__horizon_points";
    let mut slots = Vec::new();
    for inject in &injects {
        let routed = mod_points::route("chem_addon", inject, &shared).unwrap();
        let listener = format!("chem_addon::{}", inject.with);
        slots.push(match routed.access {
            Access::Internals => horizon_mod_point_emit::mod_internals(
                table,
                &routed.place,
                "chem_addon",
                &listener,
                inject.priority,
                &inject.id,
            ),
            Access::Api => {
                let point = routed.point;
                let mut manifest = horizon_mod_package::Manifest::parse(
                    "id = 'chem_addon'\nname = 'Addon'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n",
                )
                .unwrap();
                manifest.inject = vec![inject.clone()];
                let emitted = horizon_mod_point_emit::Point {
                    name: point.name.clone(),
                    target: point.target.clone(),
                    at_return: point.at_return,
                    api_refusal: point.api_refusal.clone(),
                    returns: point.returns.clone(),
                    host: point.host.clone(),
                    raw_shape: None,
                    return_ty: Some(point.return_ty.clone()),
                    table: Some(table.into()),
                };
                horizon_mod_point_emit::api(&manifest, &[emitted], "chem_addon::")
                    .unwrap()
                    .remove(0)
            }
        });
    }
    let api = repo().join("crates/horizon-mod-api");
    std::fs::write(
        probe.join("Cargo.toml"),
        format!(
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nhorizon-mod-api = {{ path = {:?} }}\nchemlib = {{ path = \"../chemlib\" }}\nchem-addon = {{ path = \"../chem_addon\" }}\n",
            api.display().to_string()
        ),
    )
    .unwrap();
    std::fs::write(
        probe.join("src/main.rs"),
        PROBE.replace("/* SLOTS */", &slots.join(", ")),
    )
    .unwrap();
    let output = Command::new(env!("CARGO"))
        .args(["run", "--quiet", "-p", "probe"])
        .current_dir(&scratch)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        // The staged library, the addon and the generated table build without a warning,
        // `unexpected_cfgs` included.
        .env("RUSTFLAGS", "-D warnings")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Unpatched: 10 moved. The api head listener doubled the rate (20 moved, so the second
    // tank holds 20); the internals return listener saw the 20; the opted-out audit and the
    // internals-only ledger point ran as written; the api listener at the private `flow` saw
    // the doubled rate.
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "10 20 20 20 100 1 20"
    );
}

/// Requires the pinned shared std built by `cargo horizon-mod wasm-std`.
#[test]
#[ignore = "requires HORIZON_WASM_STD and the pinned wasm toolchain"]
fn wasm_addon_routes_api_and_internals_into_library_registry() {
    let scratch = repo().join("target/mod-points-wasm-e2e");
    let library = scratch.join("chemlib");
    let addon = scratch.join("chem_addon");
    // Preserve compiler artifacts, not source or package metadata, between fixture runs.
    let cache = scratch.join("build-cache");
    if addon.join("target").is_dir() {
        std::fs::rename(addon.join("target"), &cache).unwrap();
    }
    copy("chemlib", &library);
    copy("chem_addon", &addon);
    if cache.is_dir() {
        std::fs::rename(cache, addon.join("target")).unwrap();
    }
    std::fs::write(
        scratch.join("Cargo.toml"),
        "[workspace]\nmembers=['chemlib','chem_addon']\nresolver='3'\n",
    )
    .unwrap();
    let catalog =
        horizon_mod_api_gen::points::mod_crate::mod_points(&library, &Default::default()).unwrap();
    for root in [&library, &addon] {
        let path = root.join("horizon-mod.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, text.split("[native]").next().unwrap()).unwrap();
        // The regular guest entry points are emitted by the hooks macro.
        let source = root.join("src/lib.rs");
        let mut text = std::fs::read_to_string(&source).unwrap();
        text.push_str("\n#[derive(Default)] pub struct Mod;\n#[mod_api::horizon_mod] impl mod_api::Hooks for Mod {}\n");
        std::fs::write(source, text).unwrap();
    }
    let path = addon.join("horizon-mod.toml");
    let mut manifest = std::fs::read_to_string(&path).unwrap();
    let hash = &catalog
        .points
        .iter()
        .find(|p| p.target == "chemlib::transfer")
        .unwrap()
        .hash;
    manifest.push_str(&format!("\n[[inject]]\nid='double'\nat='chemlib::transfer'\nkind='head'\naccess='api'\nwith='double_rate'\nhash={hash:?}\n[[inject]]\nid='observe'\nat='chemlib::transfer'\nkind='return'\naccess='internals'\nwith='saw_moved'\nhash={hash:?}\n"));
    std::fs::write(&path, &manifest).unwrap();
    let source = addon.join("src/lib.rs");
    let mut text = std::fs::read_to_string(&source).unwrap();
    text.push_str(
        r#"
#[mod_api::no_point]
pub fn probe() -> u32 {
    let chemical = chemlib::Chemical { id: 1, viscosity: 1 };
    let mut from = chemlib::Tank { chemical, amount: 100, capacity: 100 };
    let mut to = chemlib::Tank { chemical, amount: 0, capacity: 100 };
    let moved = chemlib::transfer(&mut from, &mut to, 10);
    moved * 1000 + MOVED.load(Ordering::Relaxed)
}
"#,
    );
    text = text.replace("impl mod_api::Hooks for Mod {}", "impl mod_api::Hooks for Mod { fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) { assert_eq!(probe(), 20020); } }");
    std::fs::write(source, text).unwrap();
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .args(["horizon-mod", "build", "--wasm"])
            .current_dir(&addon)
            .output()
            .unwrap()
    };
    let built = build();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let std_bytes = std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap();
    let lib_bytes = std::fs::read(library.join("mod.wasm")).unwrap();
    let addon_bytes = std::fs::read(addon.join("mod.wasm")).unwrap();
    use horizon_mod_wasm::link::{Group, Member};
    let engine = wasmtime::Engine::default();
    let mut group = Group::link_with(
        wasmtime::Store::new(&engine, ()),
        &[
            Member {
                name: "std",
                module: &std_bytes,
            },
            Member {
                name: "chemlib",
                module: &lib_bytes,
            },
            Member {
                name: "chem_addon",
                module: &addon_bytes,
            },
        ],
        |store, module, name| {
            if module != "horizon" {
                return None;
            }
            Some(wasmtime::Extern::Func(match name {
                "host" => {
                    wasmtime::Func::wrap(store, |_: u32, _: u32, _: u32, _: u32, _: u32| -> u32 {
                        panic!("unexpected host call")
                    })
                }
                "host_reply" => wasmtime::Func::wrap(store, |_: u32| -> wasmtime::Result<()> {
                    Err(wasmtime::Error::msg("unexpected host reply"))
                }),
                _ => panic!("unexpected import {name}"),
            }))
        },
    )
    .unwrap();
    for member in ["chemlib", "chem_addon"] {
        let mut result = [wasmtime::Val::I32(0)];
        group
            .call(member, "horizon_mod_points_register", &[], &mut result)
            .unwrap();
        assert_eq!(result[0].i32(), Some(0));
    }
    for member in ["chemlib", "chem_addon"] {
        group
            .call(member, "horizon_mod_points_seal", &[], &mut [])
            .unwrap();
    }
    // Sealed registries refuse the addon's first injection, reported as index plus one.
    let mut refused = [wasmtime::Val::I32(0)];
    group
        .call(
            "chem_addon",
            "horizon_mod_points_register",
            &[],
            &mut refused,
        )
        .unwrap();
    assert_eq!(refused[0].i32(), Some(1));
    let export = wasmparser::Parser::new(0)
        .parse_all(&addon_bytes)
        .filter_map(|p| match p.unwrap() {
            wasmparser::Payload::ExportSection(exports) => Some(
                exports
                    .into_iter()
                    .map(|e| e.unwrap().name.to_owned())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .find(|name| {
            rustc_demangle::try_demangle(name)
                .is_ok_and(|s| format!("{s:#}") == "chem_addon::probe")
        })
        .unwrap();
    let mut result = [wasmtime::Val::I32(0)];
    group.call("chem_addon", &export, &[], &mut result).unwrap();
    assert_eq!(result[0].i32(), Some(20020));
    // The production loader must finish both phases before the addon's register hook probes.
    let packages = [(&library, lib_bytes), (&addon, addon_bytes)]
        .into_iter()
        .map(|(root, module)| horizon_mod_wasm::WasmPackage {
            manifest: horizon_mod_package::Manifest::parse(
                &std::fs::read_to_string(root.join("horizon-mod.toml")).unwrap(),
            )
            .unwrap(),
            module,
        })
        .collect();
    let loaded = horizon_mod_wasm::load(&std_bytes, packages);
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert!(loaded.initialize(&[]).is_ok());
    // A pinned signature change refuses before compiling any guest code.
    let current = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, current.replace(hash, "0000000000000000")).unwrap();
    let refused = build();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("E_INJECT_CHANGED"));
}
