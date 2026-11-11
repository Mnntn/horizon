//! Exercise the actual world-open and command-reload publication coordinator.
use super::*;
use mod_api::{
    DatapackField, DatapackFieldKind, DatapackRegistrySync, DatapackValue, Registrar,
    RegistrationContext,
};
struct RegistryMod;
#[mod_api::__subscriptions]
impl Hooks for RegistryMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::ServerOnly,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
fn power(game: &crate::game::Game<'_>) -> DatapackValue {
    let set = game.mods.datapack_registries.as_ref().unwrap();
    set.get(
        &set.handle("registry_test:spell", "registry_test:fire")
            .unwrap(),
    )
    .unwrap()
    .get("power")
    .unwrap()
    .clone()
}
#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn datapack_game_open_reload_and_exhaustion_are_atomic() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for (name, value) in [("base", 3), ("override", 7)] {
        let pack = root.join("datapacks").join(name);
        write(
            &pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
        );
        write(
            &pack.join("data/registry_test/registry_test/spell/fire.json"),
            &format!(r#"{{"power":{value}}}"#),
        );
    }
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base","file/override"],"disabled":[]}"#,
    );
    let winning = root.join("datapacks/override/data/registry_test/registry_test/spell/fire.json");
    let tag = root.join("datapacks/base/data/registry_test/tags/block/publication.json");
    write(&tag, r#"{"values":["minecraft:stone"]}"#);
    write(
        &root.join("datapacks/base/data/registry_test/registry_test/spell/conditioned.json"),
        r#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"missing_condition_fixture"}],"power":"invalid"}"#,
    );
    mod_api::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                assert_eq!(power(game), DatapackValue::Int(7));
                assert_eq!(
                    game.mods
                        .datapack_registries
                        .as_ref()
                        .unwrap()
                        .entries("registry_test:spell")
                        .unwrap(),
                    ["registry_test:fire"]
                );
                let first = game
                    .mods
                    .datapack_registries
                    .as_ref()
                    .unwrap()
                    .handle("registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    1
                );
                write(&winning, "{bad");
                assert!(!game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(7));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    1
                );
                write(&winning, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(
                    horizon_client::datapack_net::served(),
                    game.mods.datapack_registries
                );
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    2
                );
                assert!(game.run_command("reload"));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    3
                );
                let features = game.level.enabled_features.clone();
                game.reload_datapacks(true, &features).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    4
                );
                assert!(game.run_command("datapack disable \"file/override\""));
                assert_eq!(power(game), DatapackValue::Int(3));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    5
                );
                assert!(game.run_command("datapack enable \"file/override\" last"));
                assert_eq!(power(game), DatapackValue::Int(9));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    6
                );
                game.reload_datapacks(false, &features).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    6
                );
                std::sync::Arc::make_mut(game.mods.datapack_registries.as_mut().unwrap())
                    .exhaust_generation_for_testing();
                let old_set = game.mods.datapack_registries.clone();
                let old_files = game
                    .world_tables
                    .as_ref()
                    .map(|tables| tables.files.clone());
                write(&winning, r#"{"power":11}"#);
                write(&tag, r#"{"values":["minecraft:dirt"]}"#);
                assert!(
                    !game.run_command("reload"),
                    "generation exhaustion must refuse the whole publication"
                );
                assert_eq!(game.mods.datapack_registries, old_set);
                assert_eq!(
                    horizon_client::datapack_net::served().unwrap().generation(),
                    6
                );
                assert_eq!(
                    game.world_tables
                        .as_ref()
                        .map(|tables| tables.files.clone()),
                    old_files
                );
                game.finish_close_world().unwrap();
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                game.load_world(request(root)).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().get(&first),
                    Err(mod_api::DatapackReadError::WrongSession)
                );
            });
        },
    );
}

#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn remote_reload_never_decodes_local_registry_files() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    write(
        &root.join("datapacks/local/pack.mcmeta"),
        r#"{"pack":{"description":"remote poison"}}"#,
    );
    write(
        &root.join("datapacks/local/data/registry_test/registry_test/spell/fire.json"),
        "{malformed",
    );
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["file/local"],"disabled":[]}"#,
    );
    mod_api::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(pipe, "DatapackRemote").unwrap();
                game.world_root = root.to_owned();
                game.reload_datapacks(true, &[]).unwrap();
                assert!(game.mods.datapack_registries.is_none());
                assert!(game.mods.datapack_candidate.is_none());
            });
        },
    );
}
#[path = "datapack_host_read_tests.rs"]
mod datapack_host_read_tests;

#[path = "datapack_game_delivery_tests.rs"]
mod datapack_game_delivery_tests;
