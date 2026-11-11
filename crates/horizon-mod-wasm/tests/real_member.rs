//! The build-tool fixture is a real shared module, not a component.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "fixture/src/matrix.rs"]
mod matrix;
#[path = "support/checks.rs"]
mod matrix_checks;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "fixture/src/replace.rs"]
mod replace;
#[path = "fixture/src/shutdown.rs"]
mod shutdown;
#[path = "fixture/src/turn.rs"]
mod turn;
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet, except(items));
impl mod_api::prelude::ItemsHost for Quiet {
    fn item_key(&self, id: &mod_api::Identifier) -> Option<mod_api::ItemKey> {
        mod_api::__private::item_key_by_identifier(id)
    }
}
impl mod_api::ClientHost for Quiet {}
mod_api::impl_client_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet, except(projectiles));
static OWNER_LAUNCH_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
impl mod_api::prelude::ProjectilesServerHost for Quiet {
    fn spawn_projectile_from_rotation(
        &mut self,
        owner: mod_api::LivingRef,
        kind: &mod_api::Identifier,
        stack: &mod_api::ItemStack,
        offset: f32,
        power: f32,
        spread: f32,
    ) -> Option<mod_api::ProjectileRef> {
        assert_eq!(
            owner,
            mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 21))
        );
        assert_eq!(kind.to_string(), "minecraft:snowball");
        assert_eq!(stack.count(), 16);
        assert_eq!((offset, power, spread), (0.0, 1.5, 1.0));
        OWNER_LAUNCH_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // This lower-layer harness has no Game and therefore no live owner.
        None
    }
}

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
    matrix_checks::baseline();
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
        // Independent expected values cross the actual wasm guest boundary. Game packet
        // and observer behavior must be checked at the higher client layer, not by Quiet.
        for key in [
            mod_api::InteractionKey::Attack,
            mod_api::InteractionKey::Use,
            mod_api::InteractionKey::PickBlock,
        ] {
            for (hand, attempt, expected_cancel, expected_swing) in [
                (
                    mod_api::Hand::Main,
                    mod_api::InteractionAttempt::Press,
                    false,
                    false,
                ),
                (
                    mod_api::Hand::Main,
                    mod_api::InteractionAttempt::Held,
                    false,
                    true,
                ),
                (
                    mod_api::Hand::Off,
                    mod_api::InteractionAttempt::Press,
                    true,
                    false,
                ),
                (
                    mod_api::Hand::Off,
                    mod_api::InteractionAttempt::Held,
                    true,
                    true,
                ),
            ] {
                let (mut swing, mut canceled) = (true, false);
                mod_api::dispatch_interaction_key_mapping_triggered(
                    &mut Quiet,
                    key,
                    hand,
                    attempt,
                    &mut swing,
                    &mut canceled,
                );
                assert_eq!(
                    (canceled, swing),
                    (expected_cancel, expected_swing),
                    "{key:?} {hand:?} {attempt:?}"
                );
            }
        }
        // Exercise the same three subscriptions as the native residency regression.
        // The guest only observes; terminal falling-block retirement belongs to
        // the engine and does not depend on the listener's target.
        let mut listened = false;
        mod_api::dispatch_entity_tick_listened(&mut Quiet, &mut listened);
        assert!(listened);
        let mut canceled = false;
        mod_api::dispatch_entity_tick_pre(
            &mut Quiet,
            "minecraft:overworld",
            71,
            "minecraft:item",
            &mut canceled,
        );
        assert!(!canceled);
        mod_api::dispatch_entity_tick_post(&mut Quiet, "minecraft:overworld", 71, "minecraft:item");
        let mut entity =
            mod_api::ModBlockEntity::new("wasm_fixture:counter".into(), Default::default());
        mod_api::dispatch_block_entity_tick(&mut Quiet, [0, 64, 0], &mut entity);
        assert_eq!(entity.data, [23]);
        mod_api::__private::resolve_items(["minecraft:snowball"]).unwrap();
        let mut launch =
            mod_api::ModBlockEntity::new("wasm_fixture:owner_launch".into(), Default::default());
        mod_api::dispatch_block_entity_tick(&mut Quiet, [0, 64, 0], &mut launch);
        assert_eq!(
            launch.data,
            [2],
            "the real wasm guest receives the launch refusal"
        );
        assert_eq!(
            OWNER_LAUNCH_CALLS.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the guest must cross the owner-launch host boundary exactly once"
        );
        for key in ["entity_pre_calls", "entity_post_calls"] {
            assert_eq!(
                entity.nbt.get(&key.into()),
                Some(&mod_api::Tag::Int(1)),
                "{key}"
            );
        }
        assert_eq!(
            entity.nbt.get(&"counter".into()),
            Some(&mod_api::Tag::Int(17))
        );
        // An answered collect run by the wasm guest: its entry comes back named after it.
        let mut types = Vec::new();
        {
            use mod_api::__private::game::RegistrationContextNew;
            mod_api::dispatch_register_json_animation_types(
                &mod_api::RegistrationContext::new(),
                &mut types,
            );
        }
        let named: Vec<_> = types
            .iter()
            .map(|entry| (entry.answer.clone(), entry.mod_id))
            .collect();
        assert_eq!(
            named,
            [(
                mod_api::JsonAnimationType::Target(
                    mod_api::Identifier::parse("wasm_fixture:pulse").unwrap()
                ),
                "wasm_fixture"
            )]
        );
    });
    mod_points::__private::seal();
    matrix_checks::check();
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
    // The guest's head listener (`turn::turn_angle`): the body runs with its argument.
    let _: mod_points::listeners::GameCommandsWrapDegreesListener = turn::turn_angle;
    for (angle, wrapped) in turn::CASES {
        assert_eq!(wrap_degrees(angle), wrapped, "{angle}");
    }
}

/// `horizon_client::game::commands::wrap_degrees` (`Mth.wrapDegrees`), as the staging pass
/// marks it.
#[horizon_mod_api_macros::point(name = "game_commands_wrap_degrees", free)]
fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
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
