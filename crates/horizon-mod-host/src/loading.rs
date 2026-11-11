//! Installed native and wasm code loading.
use mod_api::__private::game::Fields;
/// Load installed native and wasm code, register its dependency-ordered entries,
/// and start the engine. Keep the validated startup lease alive for the whole run.
pub fn main_with(
    mods: Vec<mod_api::ModEntry>,
    built: &'static [horizon_mod_package::runtime::BuiltMod],
    startup: horizon_client::entry::Startup,
) {
    let exe = std::env::current_exe();
    let native: Result<_, Box<dyn std::error::Error + Send + Sync>> = exe
        .as_ref()
        .map_err(|e| e.to_string().into())
        .and_then(|exe| {
            horizon_mod_package::runtime::Runtime::load(exe, built).map_err(Into::into)
        });
    let native = match native {
        Ok(native) => native,
        Err(error) => {
            eprintln!("Horizon: {error}");
            std::process::exit(1);
        }
    };
    horizon_client::entry::start(
        native,
        |native, launch| {
            let mut wasm = horizon_mod_wasm::load_installed(
                &launch.context.game_dir,
                &launch.context.dev_mods,
            );
            let mut mods = mods;
            mods.extend(std::mem::take(&mut wasm.mods));
            // One installed dispatch order for both targets; ties retain bundle order.
            let manifests: Vec<_> = native
                .packages()
                .iter()
                .map(|p| &p.manifest)
                .chain(wasm.manifests.iter())
                .filter(|m| mods.iter().any(|entry| entry.fields().id == m.id))
                .collect();
            assert_eq!(
                manifests.len(),
                mods.len(),
                "loaded code has checked manifests"
            );
            let declared: Vec<_> = manifests
                .iter()
                .map(|m| horizon_mod_package::dependencies::from_manifest(&m.dependencies))
                .collect();
            let inputs: Vec<_> = manifests
                .iter()
                .zip(&declared)
                .map(|(m, dependencies)| horizon_mod_package::dependencies::Mod {
                    id: &m.id,
                    version: &m.version,
                    has_api: true,
                    dependencies,
                })
                .collect();
            let order = horizon_mod_package::dependencies::resolve(&inputs)
                .unwrap_or_else(|error| panic!("mod load refused: {error}"));
            // What every level.dat this game saves records (`fml.LoadingModList`).
            horizon_client::save::level_mods::set_loaded(Some(
                order
                    .order
                    .iter()
                    .map(|&i| horizon_client::save::level_mods::ModRecord {
                        id: manifests[i].id.clone(),
                        version: manifests[i].version.clone(),
                    })
                    .collect(),
            ));
            let mut entries: std::collections::BTreeMap<_, _> = mods
                .into_iter()
                .map(|entry| (entry.fields().id, entry))
                .collect();
            let mods: Vec<_> = order
                .order
                .into_iter()
                .map(|i| entries.remove(manifests[i].id.as_str()).unwrap())
                .collect();
            let ids: Vec<_> = mods.iter().map(|entry| entry.fields().id).collect();
            mod_api::__private::install(mods.into());
            (ids, wasm)
        },
        horizon_client::entry::CodeRuntime {
            initialize: horizon_mod_wasm::initialize,
            refusal: |id| horizon_mod_wasm::refusal(id).map(|reason| reason.to_string()),
            startup_refused: |id| {
                horizon_mod_wasm::refused()
                    .iter()
                    .any(|(owner, _)| owner == id)
            },
        },
        startup,
    );
}
