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
    mod_api::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                assert_eq!(power(game), DatapackValue::Int(7));
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
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    2
                );
                assert!(game.run_command("reload"));
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().generation(),
                    3
                );
                game.mods
                    .datapack_registries
                    .as_mut()
                    .unwrap()
                    .exhaust_generation_for_testing();
                let old_set = game.mods.datapack_registries.clone();
                let old_files = game
                    .world_tables
                    .as_ref()
                    .map(|tables| tables.files.clone());
                write(&winning, r#"{"power":11}"#);
                assert!(
                    !game.run_command("reload"),
                    "generation exhaustion must refuse the whole publication"
                );
                assert_eq!(game.mods.datapack_registries, old_set);
                assert_eq!(
                    game.world_tables
                        .as_ref()
                        .map(|tables| tables.files.clone()),
                    old_files
                );
                game.load_world(request(root)).unwrap();
                assert_eq!(
                    game.mods.datapack_registries.as_ref().unwrap().get(&first),
                    Err(mod_api::DatapackReadError::WrongSession)
                );
            });
        },
    );
}
