include!("/box/work/shm-machine-scene/src/lib.rs");

struct EarlyLog;
impl log::Log for EarlyLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        eprintln!("EARLY {}: {}", record.level(), record.args());
    }
    fn flush(&self) {}
}
static EARLY_LOG: EarlyLog = EarlyLog;
struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
fn main() {
    if let Some(path) = std::env::var_os("SCENE_PROBE") {
        use mod_api::__private::game::Fields;
        let path = std::path::Path::new(&path);
        let manifest = horizon_mod_package::Package::read(path).unwrap().manifest;
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            vec![],
        );
        let module = horizon_mod_package::Package::read_wasm(path).unwrap();
        let loaded = horizon_mod_wasm::load(
            &std::fs::read(std::env::var("BENCH_STD").unwrap()).unwrap(),
            vec![horizon_mod_wasm::WasmPackage { manifest, module }],
        );
        eprintln!(
            "PROBE refused {:?}, mods {}",
            loaded.refused,
            loaded.mods.len()
        );
        loaded.initialize(&[info]).unwrap();
        let hooks = loaded.mods[0].fields().hooks;
        eprintln!(
            "PROBE container subscribed {}",
            hooks
                .subscribed()
                .iter()
                .any(|id| matches!(id, Some(mod_api::HookId::BlockEntityContainer)))
        );
        let mut event = mod_api::BlockEntityContainerEvent {
            block: "machine_sample:machine",
        };
        eprintln!(
            "PROBE container {:?}",
            hooks.block_entity_container(&Quiet, &mut event)
        );
        return;
    }
    let startup = horizon_client::entry::prepare();
    log::set_logger(&EARLY_LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    if std::env::var_os("BENCH_NATIVE").is_some() {
        static BUILT: &[horizon_mod_package::runtime::BuiltMod] =
            &[horizon_mod_package::runtime::BuiltMod {
                id: "machine_sample",
                version: "0.1.0",
                side: horizon_mod_package::Side::Both,
            }];
        horizon_mod_host::main_with(
            vec![mod_api::ModEntry::new("machine_sample", hooks())],
            &[],
            BUILT,
            startup,
        );
    } else {
        horizon_mod_host::main_with(Vec::new(), &[], &[], startup);
    }
}
