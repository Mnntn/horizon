//! The build-tool fixture is a real shared module, not a component.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "fixture/src/replace.rs"]
mod replace;
#[path = "fixture/src/shutdown.rs"]
mod shutdown;
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);

#[test]
#[ignore = "needs HORIZON_WASM_STD or HORIZON_WASM_FIXTURE_RAW; prints a member's sections"]
fn dump_members() {
    for (name, path) in [
        ("std", "HORIZON_WASM_STD"),
        ("fixture", "HORIZON_WASM_FIXTURE_RAW"),
    ] {
        let Ok(path) = std::env::var(path) else {
            println!("=== {name} === skipped: {path} is not set");
            continue;
        };
        let bytes = std::fs::read(path).unwrap();
        println!("=== {name} ===");
        for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
            match payload.unwrap() {
                wasmparser::Payload::CustomSection(s) if s.name() == "dylink.0" => {
                    println!("dylink.0: {:02x?}", s.data())
                }
                wasmparser::Payload::ImportSection(s) => {
                    for i in s.into_imports() {
                        println!("import: {:?}", i.unwrap());
                    }
                }
                wasmparser::Payload::ExportSection(s) => {
                    for e in s {
                        println!("export: {:?}", e.unwrap());
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD, which the wasm-mods workflow builds"]
fn real_member() {
    let (Ok(fixture), Ok(std)) = (
        std::env::var("HORIZON_WASM_FIXTURE"),
        std::env::var("HORIZON_WASM_STD"),
    ) else {
        panic!(
            "HORIZON_WASM_FIXTURE and HORIZON_WASM_STD must be set: run with the wasm-mods workflow's environment"
        );
    };
    let path = std::path::Path::new(&fixture);
    let manifest = Package::read(path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        manifest.dependencies.keys().cloned().collect(),
    );
    let package = WasmPackage {
        manifest,
        module: Package::read_wasm(path).unwrap(),
    };
    // load checks the layout and calls Hooks::subscribed through the real host boundary.
    let loaded = load(&std::fs::read(std).unwrap(), vec![package]);
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.mods.len(), 1);
    loaded
        .initialize(&[info])
        .expect("guest initialization and point registration");
    mod_api::testing::with_mods(loaded.mods.into(), || {
        mod_api::__private::install_parts(
            mod_api::__private::mods(),
            &mod_api::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        assert!(
            mod_api::__private::block_entity_types()
                .iter()
                .any(|kind| kind.id == "wasm_fixture:counter")
        );
        mod_api::__private::mark_game_thread();
        let mut entity =
            mod_api::ModBlockEntity::new("wasm_fixture:counter".into(), Default::default());
        mod_api::dispatch_block_entity_tick(&mut Quiet, [0, 64, 0], &mut entity);
        assert_eq!(entity.data, [23]);
        assert_eq!(
            entity.nbt.get(&"counter".into()),
            Some(&mod_api::Tag::Int(17))
        );
    });
    mod_points::__private::seal();
    let mut args = mod_points::TickArgs::__new();
    mod_points::dispatch::tick(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(
        args.cancel,
        "real wasm listener must modify the point record"
    );
    // The real point's expansion: its `&mut` place copied in and out of the wasm listener's
    // record, its by-value message changed, and its cancel skipping the body.
    // The guest built `shutdown::on_shutdown`, the source `native_listener` registers natively.
    let _: mod_points::listeners::GameServerLifecycleRememberShutdownListener =
        shutdown::on_shutdown;
    for (before, message, after) in shutdown::CASES {
        let mut failure = before.map(String::from);
        remember_shutdown(&mut failure, message.into());
        assert_eq!(failure.as_deref(), after, "{message}");
    }
    // The guest's `replace` (`replace::own_average`, emitted through `mod_points::replace!`):
    // its value reaches the caller across the boundary, and the body never runs.
    let _: fn(&mut mod_points::GameBeaconAverageArgs) -> u32 = replace::own_average;
    for (a, b, value) in replace::CASES {
        assert_eq!(average(a, b), value, "{a} {b}");
    }
}

/// `horizon_client::game::beacon::average`, as the staging pass marks it.
#[horizon_mod_api_macros::point(name = "game_beacon_average", free)]
fn average(a: u32, b: u32) -> u32 {
    panic!("a replaced body ran: {a} {b}")
}

/// `horizon_client::game::server_lifecycle::remember_shutdown`, as the staging pass marks it.
#[horizon_mod_api_macros::point(name = "game_server_lifecycle_remember_shutdown", free)]
fn remember_shutdown(failure: &mut Option<String>, message: String) {
    if failure.is_none() {
        *failure = Some(message);
    }
}
