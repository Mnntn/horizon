use super::*;
use mod_api::{DatapackReadError as E, DimensionKey, Host, LevelRef, WorldEpoch};
#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn native_datapack_host_reads_enforce_level_session_and_generation() {
    let scratch = tempfile::tempdir().unwrap();
    mod_api::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(RegistryMod))].into(),
        || {
            with_game(|game| {
                game.load_world(request(scratch.path())).unwrap();
                let candidate = || {
                    mod_api::DatapackCandidate::build(
                        mod_api::__private::datapack_registry_declarations(),
                        &[
                            (
                                "data/registry_test/registry_test/spell/fire.json".into(),
                                vec![br#"{"power":7}"#.to_vec()],
                            ),
                            (
                                "data/addon/registry_test/spell/ice.json".into(),
                                vec![br#"{"power":3}"#.to_vec()],
                            ),
                        ],
                    )
                    .unwrap()
                };
                game.mods.datapack_registries =
                    Some(mod_api::DatapackRegistrySet::first(candidate(), 400));
                let level = game.level(game.dim.id()).unwrap();
                assert_eq!(
                    level.datapack_entries(game, "registry_test:spell"),
                    Some(vec!["addon:ice".into(), "registry_test:fire".into()])
                );
                let handle = level
                    .datapack_handle(game, "registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    level.datapack_read(game, &handle),
                    Ok(vec![("power".into(), DatapackValue::Int(7))])
                );
                assert_eq!(level.datapack_entries(game, "missing:registry"), None);
                assert_eq!(
                    level.datapack_handle(game, "missing:registry", "addon:ice"),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(
                    level.datapack_handle(game, "registry_test:spell", "addon:missing"),
                    Err(E::MissingEntry)
                );
                for refused in [
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension()),
                    LevelRef::new(level.epoch(), DimensionKey::NETHER),
                ] {
                    assert_eq!(refused.datapack_entries(game, "registry_test:spell"), None);
                    assert_eq!(
                        refused.datapack_handle(game, "registry_test:spell", "registry_test:fire"),
                        Err(E::UnknownRegistry)
                    );
                    assert_eq!(
                        refused.datapack_read(game, &handle),
                        Err(E::UnknownRegistry)
                    );
                }
                game.mods.datapack_registries = Some(
                    game.mods
                        .datapack_registries
                        .as_ref()
                        .unwrap()
                        .reload(candidate())
                        .unwrap(),
                );
                assert_eq!(level.datapack_read(game, &handle), Err(E::StaleGeneration));
                game.mods.datapack_registries =
                    Some(mod_api::DatapackRegistrySet::first(candidate(), 401));
                assert_eq!(level.datapack_read(game, &handle), Err(E::WrongSession));
                game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
                assert_eq!(level.datapack_entries(game, "registry_test:spell"), None);
                assert_eq!(
                    level.datapack_handle(game, "registry_test:spell", "registry_test:fire"),
                    Err(E::UnknownRegistry)
                );
                assert_eq!(level.datapack_read(game, &handle), Err(E::UnknownRegistry));
            });
        },
    );
}
