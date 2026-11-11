use crate::game::commands::items::Slot;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

fn with_game(check: impl FnOnce(&mut Game<'_>)) {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let scratch = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(scratch.path().to_path_buf());
    options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();

    game.set_allow_commands(true);
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    game.block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    check(&mut game);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn item_replace_jukebox_accepts_music_disc_cat() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 jukebox"));
        assert!(
            game.run_command(
                "item replace block 4 200 4 container.0 with minecraft:music_disc_cat"
            )
        );
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:music_disc_cat".into(), 1))
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn loot_empty_replace_jukebox_preserves_music_disc_13() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 jukebox"));
        assert!(
            game.run_command("item replace block 4 200 4 container.0 with minecraft:music_disc_13")
        );
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .unwrap()
                .item
                .as_ref(),
            "minecraft:music_disc_13"
        );
        assert!(game.run_command(r#"loot replace block 4 200 4 container.0 1 loot {"pools":[]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:music_disc_13".into(), 1))
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn loot_replace_furnace_fuel_refuses_non_fuel() {
    with_game(|game| {
        assert!(game.run_command("setblock 4 200 4 furnace"));
        assert!(game.run_command("item replace block 4 200 4 container.1 with minecraft:coal 3"));
        assert!(game.run_command(r#"loot replace block 4 200 4 container.1 1 loot {"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 1))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:coal".into(), 3))
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn item_commands_bypass_block_acceptance() {
    with_game(|game| {
        for slot in [1, 2] {
            // Destroy first so repeated rows also place a fresh container.
            assert!(game.run_command("setblock 4 200 4 furnace destroy"));
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.{slot} with minecraft:diamond 2"
            )));
            assert_eq!(
                game.slot_get(&Slot::Block([4, 200, 4], slot))
                    .map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 2))
            );
            assert!(game.run_command(&format!(
                r#"item modify block 4 200 4 container.{slot} {{"type":"minecraft:set_count","count":3}}"#
            )));
            assert_eq!(
                game.slot_get(&Slot::Block([4, 200, 4], slot))
                    .map(|s| (s.item.to_string(), s.count)),
                Some(("minecraft:diamond".into(), 3))
            );
        }
        for modify in [false, true] {
            assert!(game.run_command("setblock 4 200 4 jukebox destroy"));
            assert!(game.run_command(
                "item replace block 4 200 4 container.0 with minecraft:music_disc_cat"
            ));
            let command = if modify {
                r#"item modify block 4 200 4 container.0 {"type":"minecraft:set_count","count":0}"#
            } else {
                "item replace block 4 200 4 container.0 with minecraft:air"
            };
            assert!(game.run_command(command));
            assert!(game.slot_get(&Slot::Block([4, 200, 4], 0)).is_none());
        }
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn empty_loot_preserves_restricted_slots_but_clears_chest() {
    with_game(|game| {
        for (block, slot, item) in [
            ("furnace", 1, "coal"),
            ("furnace", 2, "iron_ingot"),
            ("brewing_stand", 0, "potion"),
            ("chiseled_bookshelf", 0, "book"),
            ("crafter{disabled_slots:[I;0]}", 0, "diamond"),
            ("chest", 0, "diamond"),
        ] {
            // An unchanged block state makes `setblock ... replace` fail.
            assert!(game.run_command(&format!("setblock 4 200 4 {block} destroy")));
            assert!(game.run_command(&format!(
                "item replace block 4 200 4 container.{slot} with minecraft:{item}"
            )));
            assert!(game.run_command(&format!(
                r#"loot replace block 4 200 4 container.{slot} 1 loot {{"pools":[]}}"#
            )));
            let actual = game
                .slot_get(&Slot::Block([4, 200, 4], slot))
                .map(|s| (s.item.to_string(), s.count));
            let expected = if block == "chest" {
                None
            } else {
                Some((format!("minecraft:{item}"), 1))
            };
            assert_eq!(actual, expected, "{block} slot {slot}");
        }
        assert!(game.run_command("setblock 4 200 4 chest destroy"));
        assert!(game.run_command(r#"loot replace block 4 200 4 container.0 1 loot {"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#));
        assert_eq!(
            game.slot_get(&Slot::Block([4, 200, 4], 0))
                .map(|s| (s.item.to_string(), s.count)),
            Some(("minecraft:diamond".into(), 1))
        );
    });
}
