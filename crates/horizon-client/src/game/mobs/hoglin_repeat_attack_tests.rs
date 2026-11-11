//! A hoglin that retaliates keeps attacking its own attacker when a
//! neighbouring hoglin is hit in the same tick: `HoglinAi.broadcastAttackTarget`
//! gives each visible adult hoglin the nearer of its current target and the
//! new one (`setAttackTargetIfCloserThanCurrent`)
//! (tools/test_hoglin_repeat_attack_parity.py measures the same on vanilla 26.3).
use super::commands::Target;
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

const NEAR_HOGLIN: &str = "00006093-0000-0000-0000-000000000003";
const NEAR_PIG: &str = "00006093-0000-0000-0000-00000000000d";
const HOGLIN: &str = "00006093-0000-0000-0000-000000000001";
const PIG: &str = "00006093-0000-0000-0000-00000000000b";

fn game<'a>(resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>) -> Game<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
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
    for cx in 0..2 {
        game.world
            .chunks
            .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
        game.block_ticks
            .add_empty_chunk(ChunkPos::new(cx, 0))
            .unwrap();
    }
    for command in [
        "difficulty normal",
        "forceload add 0 0 31 15",
        "fill 0 99 0 31 99 15 warped_nylium",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    // Two glass pens 16 blocks apart, each with a hoglin and a sturdy NoAI pig.
    for (x0, n) in [(0, 3), (16, 1)] {
        assert!(game.run_command(&format!("fill {} 100 4 {} 102 12 glass", x0 + 4, x0 + 10)));
        assert!(game.run_command(&format!("fill {} 100 5 {} 102 11 air", x0 + 5, x0 + 9)));
        assert!(game.run_command(&format!(
            "summon hoglin {}.5 100 8.5 {{PersistenceRequired:1b,IsImmuneToZombification:1b,Health:40f,UUID:[I;24723,0,0,{n}]}}",
            x0 + 6
        )));
        assert!(game.run_command(&format!(
            "summon pig {}.5 100 8.5 {{NoAI:1b,PersistenceRequired:1b,Silent:1b,attributes:[{{id:\"minecraft:max_health\",base:100d}}],Health:100f,UUID:[I;24723,0,0,{}]}}",
            x0 + 7,
            10 + n
        )));
    }
    game
}

fn health(game: &mut Game<'_>, uuid: &str) -> f32 {
    let target = game
        .target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap())
        .expect("the entity is in the level");
    match target {
        Target::Mob(id) => game.mobs.iter().find(|m| m.id == id).unwrap().health,
        other => panic!("{other:?} is not a mob"),
    }
}

fn step(game: &mut Game<'_>, ticks: u32) {
    for _ in 0..ticks {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_hoglin_keeps_attacking_its_attacker_beside_another_hit_hoglin() {
    let mut resources = None;
    let mut game = game(&mut resources);
    step(&mut game, 40);
    // The neighbour's hoglin first, then this one, in the same tick.
    assert!(game.run_command(&format!(
        "damage {NEAR_HOGLIN} 1 minecraft:mob_attack by {NEAR_PIG}"
    )));
    assert!(game.run_command(&format!("damage {HOGLIN} 1 minecraft:mob_attack by {PIG}")));
    step(&mut game, 5);
    let after_first = health(&mut game, PIG);
    assert!(
        after_first < 100.0,
        "the first retaliation hits ({after_first})"
    );
    let near_first = health(&mut game, NEAR_PIG);
    assert!(
        near_first < 100.0,
        "the neighbour retaliates ({near_first})"
    );
    // MeleeAttack's 40-tick cooldown: two more hits are due within 90 ticks.
    step(&mut game, 90);
    let near_later = health(&mut game, NEAR_PIG);
    assert!(
        near_later < near_first,
        "the neighbour hits again ({near_later})"
    );
    let later = health(&mut game, PIG);
    assert!(
        later < after_first,
        "this hoglin hits its attacker again ({later})"
    );
}

/// Added controls (after the frozen baseline above).
fn mob_id(game: &mut Game<'_>, uuid: &str) -> u32 {
    match game.target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap()) {
        Some(Target::Mob(id)) => id,
        other => panic!("{other:?}"),
    }
}

/// `setAttackTargetIfCloserThanCurrent` on the receiver: pacified ignores
/// the broadcast; a nearer current target stays; a nearer (or equally near)
/// incoming one replaces a farther one; none current takes the new one.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_broadcast_target_replaces_only_a_farther_one() {
    use horizon_client::mobs::{Who, piglin::receive_attack_target};
    use horizon_core::geometry::Vec3;
    let mut resources = None;
    let mut game = game(&mut resources);
    let receiver = mob_id(&mut game, HOGLIN);
    let (near, far) = (mob_id(&mut game, PIG), mob_id(&mut game, NEAR_PIG));
    let place = |game: &Game<'_>, id: u32| game.mobs.iter().find(|m| m.id == id).unwrap().position;
    let (near_at, far_at) = (place(&game, near), place(&game, far));
    let position = |who: Who, _current| match who {
        Who::Mob(id) if id == near => Some(near_at),
        Who::Mob(id) if id == far => Some(far_at),
        _ => None::<Vec3>,
    };
    let m = game.mobs.iter_mut().find(|m| m.id == receiver).unwrap();
    // No current target: the broadcast's is taken.
    m.ai.target = None;
    let proposal = receive_attack_target(m, Who::Mob(far), position);
    assert_eq!(proposal, Some(Who::Mob(far)), "no current target");
    assert_eq!(
        m.ai.target, None,
        "selection leaves committing to the event hook"
    );
    // Pacified: nothing changes.
    m.ai.target = None;
    m.extra.ints[0] = 200;
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), position),
        None,
        "pacified"
    );
    m.extra.ints[0] = 0;
    // A nearer current target stays.
    m.ai.target = Some(Who::Mob(near));
    assert_eq!(
        receive_attack_target(m, Who::Mob(far), position),
        None,
        "nearer current"
    );
    // A nearer incoming target replaces a farther current one.
    m.ai.target = Some(Who::Mob(far));
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), position),
        Some(Who::Mob(near)),
        "nearer incoming"
    );
    // A tie takes the incoming one (`getTargetNearestMe`: current only if strictly nearer).
    m.ai.target = Some(Who::Mob(far));
    assert_eq!(
        receive_attack_target(m, Who::Mob(near), |_, _| Some(near_at)),
        Some(Who::Mob(near)),
        "tie"
    );
}

/// The broadcast's receivers are the sensed visible adult hoglins
/// (`NEAREST_VISIBLE_ADULT_HOGLINS`): not a baby, not one behind stone.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_sensed_receivers_are_visible_adult_hoglins() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // Inside this hoglin's pen: an adult and a baby. Beyond a stone wall: an adult.
    assert!(game.run_command(
        "summon hoglin 24.5 100 10.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,NoAI:1b,UUID:[I;24723,0,0,6]}"
    ));
    assert!(game.run_command(
        "summon hoglin 24.5 100 6.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,Age:-24000,UUID:[I;24723,0,0,4]}"
    ));
    assert!(game.run_command("fill 27 100 0 27 104 15 stone"));
    assert!(game.run_command(
        "summon hoglin 29.5 100 8.5 {PersistenceRequired:1b,IsImmuneToZombification:1b,NoAI:1b,UUID:[I;24723,0,0,5]}"
    ));
    step(&mut game, 40);
    let receiver = mob_id(&mut game, HOGLIN);
    let near = mob_id(&mut game, "00006093-0000-0000-0000-000000000006");
    let baby = mob_id(&mut game, "00006093-0000-0000-0000-000000000004");
    let walled = mob_id(&mut game, "00006093-0000-0000-0000-000000000005");
    let adults = &game
        .mobs
        .iter()
        .find(|m| m.id == receiver)
        .unwrap()
        .extra
        .piglin
        .adults;
    assert!(
        adults.contains(&near),
        "the visible adult neighbour: {adults:?}"
    );
    assert!(!adults.contains(&baby), "not a baby: {adults:?}");
    assert!(
        !adults.contains(&walled),
        "not one behind stone: {adults:?}"
    );
}

#[derive(Default)]
struct BroadcastTargetListener {
    state: Arc<
        std::sync::Mutex<(
            usize,
            Option<mod_api::LivingRef>,
            Option<mod_api::LivingRef>,
        )>,
    >,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for BroadcastTargetListener {
    fn living_change_target(
        &self,
        _host: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        assert!(event.is_brain, "hoglin targets use the brain event");
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        state.2 = event.original_target;
        if let Some(redirect) = state.1 {
            *event.new_target = Some(redirect);
            mod_api::Flow::Continue
        } else {
            mod_api::Flow::Cancel
        }
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn hoglin_broadcast_obeys_target_cancellation_and_redirection() {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::mobs::{Mob, MobEvent, Who};
    use horizon_core::geometry::Vec3;

    let state = Arc::new(std::sync::Mutex::new((0, None, None)));
    let mods = vec![mod_api::ModEntry::new(
        "hoglin_target_listener",
        Box::new(BroadcastTargetListener {
            state: state.clone(),
        }),
    )]
    .into();
    mod_api::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:hoglin")
                .unwrap()
                .clone();
            for (id, x) in [(9001, 0.0), (9002, 4.0), (9003, 2.0)] {
                game.mobs
                    .push(Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.0), 0.0));
            }
            let broadcast = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Mob(9002),
                    player: None,
                }]
            };
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 1);
            assert_eq!(
                game.mobs[0].ai.target, None,
                "cancellation preserves target"
            );

            let epoch = game.native_lifecycle.epoch().unwrap();
            state.lock().unwrap().1 = Some(mod_api::LivingRef::new(mod_api::EntityRef::new(
                epoch, 9003,
            )));
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            assert_eq!(
                game.mobs[0].ai.target,
                Some(Who::Mob(9003)),
                "listener redirect is committed"
            );

            // A nearer current target and a pacified receiver reject the proposal
            // before listener dispatch, preserving vanilla's broadcast selection.
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            game.mobs[0].ai.target = None;
            game.mobs[0].extra.ints[0] = 200;
            game.handle_mob_events(broadcast()).unwrap();
            assert_eq!(state.lock().unwrap().0, 2);
            assert_eq!(game.mobs[0].ai.target, None);

            use crate::game::native_item_places::tests::{guest, open_to_lan};
            open_to_lan(game);
            for conn in [12, 13] {
                let mut placed = guest(conn);
                placed.loaded = true;
                placed.position = [if conn == 12 { 4.0 } else { 2.0 }, 64.0, 0.0];
                game.lan.as_mut().unwrap().hub.guests.push(placed);
            }
            let attacker_id = game.guest_identity(12).unwrap().1;
            let attacker = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, attacker_id));
            game.mobs[0].extra.ints[0] = 0;
            game.lan.as_mut().unwrap().mob_players.insert(9001, 13);
            let broadcast_player = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Player,
                    player: Some(12),
                }]
            };
            state.lock().unwrap().1 = None;
            game.handle_mob_events(broadcast_player()).unwrap();
            assert_eq!(state.lock().unwrap().2, Some(attacker));
            assert_eq!(game.mobs[0].ai.target, None);
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
            state.lock().unwrap().1 = Some(attacker);
            game.handle_mob_events(broadcast_player()).unwrap();
            assert_eq!(state.lock().unwrap().2, Some(attacker));
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 12);
            // Concrete players must be compared, not the shared Who::Player tag.
            let nearer_mob = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, 9003));
            state.lock().unwrap().1 = Some(nearer_mob);
            let before = state.lock().unwrap().0;
            game.handle_mob_events(vec![MobEvent::HoglinAttackTarget {
                mob: 9001,
                target: Who::Mob(9003),
                player: None,
            }])
            .unwrap();
            assert_eq!(state.lock().unwrap().0, before + 1);
            assert_eq!(
                game.mobs[0].ai.target,
                Some(Who::Mob(9003)),
                "player at four loses to mob at two"
            );
            game.mobs[0].ai.target = Some(Who::Mob(9002));
            let nearer_player = mod_api::LivingRef::new(mod_api::EntityRef::new(
                epoch,
                game.guest_identity(13).unwrap().1,
            ));
            state.lock().unwrap().1 = Some(nearer_player);
            let broadcast_nearer_player = || {
                vec![MobEvent::HoglinAttackTarget {
                    mob: 9001,
                    target: Who::Player,
                    player: Some(13),
                }]
            };
            game.handle_mob_events(broadcast_nearer_player()).unwrap();
            assert_eq!(state.lock().unwrap().0, before + 2);
            assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
            game.lan.as_mut().unwrap().mob_players.insert(9001, 12);
            game.handle_mob_events(broadcast_nearer_player()).unwrap();
            assert_eq!(state.lock().unwrap().0, before + 3);
            assert_eq!(game.lan.as_ref().unwrap().mob_players[&9001], 13);
        });
    });
}
