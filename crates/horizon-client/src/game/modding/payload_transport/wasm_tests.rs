//! Actual compiled guest -> executor -> Game root proof. No substitute context.
use super::*;
use mod_api::{Hooks, ModEntry, Registrar};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PUBLICATIONS: AtomicUsize = AtomicUsize::new(0);
static RECEIPTS: AtomicUsize = AtomicUsize::new(0);
static ROOTS: Mutex<Vec<(f32, f32, bool)>> = Mutex::new(Vec::new());
const TEST: &str = "game::payload_transport::wasm_tests::compiled_guest_runs_in_actual_game_roots";

pub(super) fn publication(server_reply: bool) {
    if server_reply && ACTIVE.load(Ordering::SeqCst) {
        PUBLICATIONS.fetch_add(1, Ordering::SeqCst);
    }
}
pub(super) fn health(game: &Game<'_>, endpoint: &Endpoint, id: &PayloadId) -> Option<f32> {
    if endpoint.side() != Side::Server
        || !matches!(
            id.0.as_str(),
            "wasm_fixture:game_main" | "wasm_fixture:game_network"
        )
        || !ACTIVE.load(Ordering::SeqCst)
    {
        return None;
    }
    let player = endpoint.transport().player()?;
    let host: &dyn mod_api::ServerHost = game;
    mod_api::LivingRef::new(player.entity()).health(host)
}
pub(super) fn observe(
    before: Option<f32>,
    game: &Game<'_>,
    endpoint: &Endpoint,
    id: &PayloadId,
    commit: bool,
) {
    if let (Some(before), Some(after)) = (before, health(game, endpoint, id)) {
        ROOTS.lock().unwrap().push((before, after, commit));
    }
}
struct Receipt;
impl mod_api::IPayloadHandler for Receipt {
    fn handle(&self, bytes: PayloadBytes, context: &mut dyn IPayloadContext) -> Result<(), String> {
        RECEIPTS.fetch_add(1, Ordering::SeqCst);
        assert_eq!(context.flow(), PayloadDirection::ToClient);
        assert_eq!(bytes.0.len(), 1 << 20);
        for (i, byte) in bytes.0.iter().enumerate() {
            assert_eq!(*byte, ((i * 31 + 7) % 251) as u8);
        }
        Ok(())
    }
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar.payload_handlers().registrar("1").play_to_client(
            PayloadId("raw_probe:receipt".into()),
            Some(Arc::new(Receipt)),
        );
    }
}
#[test]
#[ignore = "requires prepared resources and compiled HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn compiled_guest_runs_in_actual_game_roots() {
    if super::super::native_entity_refs::tests::run_fresh_cases(
        TEST,
        "HORIZON_WASM_GAME_CASE",
        &[
            "main",
            "network",
            "handler_trap",
            "work_trap",
            "network_handler_trap",
        ],
    ) {
        return;
    }
    use horizon_mod_package::manifest::Package;
    let case = std::env::var("HORIZON_WASM_GAME_CASE").unwrap();
    let network = matches!(case.as_str(), "network" | "work_trap");
    let trap = matches!(
        case.as_str(),
        "handler_trap" | "work_trap" | "network_handler_trap"
    );
    let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
    let path = std::path::Path::new(&path);
    let manifest = Package::read(path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let loaded = horizon_mod_wasm::load(
        &std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info]).unwrap();
    let mut mods = loaded.mods;
    mods.push(ModEntry::new("raw_probe", Box::new(Native)));
    mod_api::testing::with_mods(mods.into(), || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "wasm_raw_transport",
                Some("Wasm raw transport"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline, "integrated play not ready");
                std::thread::yield_now();
            }
            ROOTS.lock().unwrap().clear();
            PUBLICATIONS.store(0, Ordering::SeqCst);
            ACTIVE.store(true, Ordering::SeqCst);
            struct Scope;
            impl Drop for Scope {
                fn drop(&mut self) {
                    ACTIVE.store(false, Ordering::SeqCst);
                    ROOTS.lock().unwrap().clear();
                }
            }
            let _scope = Scope;
            let channel = if case == "network_handler_trap" {
                "wasm_fixture:network_trap"
            } else if network {
                "wasm_fixture:game_network"
            } else {
                "wasm_fixture:game_main"
            };
            let bytes = if trap {
                vec![1]
            } else {
                (0..1 << 20).map(|i| ((i * 31 + 7) % 251) as u8).collect()
            };
            let host: &mut dyn mod_api::ClientHost = game;
            assert_eq!(
                host.send_payload_to_server(&PayloadId(channel.into()), PayloadBytes(bytes)),
                Ok(PayloadAccepted::Queued)
            );
            loop {
                game.poll_remote().unwrap();
                let observed = !ROOTS.lock().unwrap().is_empty();
                if (trap && game.remote.is_none())
                    || (!trap && observed && RECEIPTS.load(Ordering::SeqCst) == 1)
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "actual wasm Game root did not finish"
                );
                std::thread::yield_now();
            }
            assert_eq!(
                PUBLICATIONS.load(Ordering::SeqCst),
                usize::from(!trap),
                "actual Game deferred publication must follow the root outcome, before teardown can suppress delivery"
            );
            let roots = ROOTS.lock().unwrap();
            if case == "network_handler_trap" {
                assert!(
                    roots.is_empty(),
                    "NETWORK handler trap must cancel staged main work before Game dispatch"
                );
                assert_eq!(RECEIPTS.load(Ordering::SeqCst), 0);
                return;
            }
            assert_eq!(
                roots.len(),
                1,
                "only the authoritative request/work root is observed"
            );
            let &(before, after, commit) = &roots[0];
            assert_eq!(commit, !trap);
            assert_eq!(
                after,
                if trap { before } else { before - 1.0 },
                "actual authoritative player mutation must follow root outcome"
            );
            assert_eq!(RECEIPTS.load(Ordering::SeqCst), usize::from(!trap));
            drop(roots);
            if game.remote.is_some() {
                game.leave_server(None).unwrap();
            }
        });
    });
}
