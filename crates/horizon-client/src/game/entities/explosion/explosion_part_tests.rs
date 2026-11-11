use super::*;
use mod_api::{
    BlockPart, BreakBlockEvent, ExpDropEvent, Flow, Hooks, LevelHost, OnBlockExplodedEvent,
    Registrar, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

#[derive(Clone)]
struct Xp(Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Xp {
    fn exp_drop(&self, level: &dyn LevelHost, event: &mut ExpDropEvent<'_>) -> Option<i32> {
        if level.state_block(event.state) == Some("minecraft:chest") {
            let data = level
                .block_entity_data(event.block_entity.expect("live XP block entity"))
                .unwrap();
            assert!(
                matches!(data.get(&"Items".into()), Some(mod_api::Tag::List(items)) if items.len() == 1)
            );
        }
        Some(self.0.load(Ordering::Relaxed))
    }
}
#[derive(Clone, Default)]
struct Blast(Arc<Mutex<Vec<[i32; 3]>>>, Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Blast {
    fn on_block_exploded(&self, host: &mut dyn ServerHost, event: &mut OnBlockExplodedEvent<'_>) {
        if self.1.load(Ordering::Relaxed) == 1 {
            panic!("adversarial part failure");
        }
        if self.1.load(Ordering::Relaxed) == 3 {
            std::panic::panic_any("native trap-equivalent refusal");
        }
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let entity = event.block_entity.expect("live chest before removal");
        let data = entity.data(host).expect("readable live chest");
        let mod_api::Tag::List(items) = data.get(&"Items".into()).unwrap() else {
            panic!("chest items");
        };
        assert_eq!(items.len(), 1);
        let mod_api::Tag::Compound(item) = &items[0] else {
            panic!("saved stack");
        };
        assert_eq!(item.get(&"count".into()), Some(&mod_api::Tag::Int(3)));
        assert_eq!(event.explosion.center, [1.0, 65.0, 1.0]);
        assert_eq!(event.explosion.radius, 4.0);
        let level = host.level(event.explosion.dimension).unwrap();
        let state = host.block_state_id("minecraft:stone").unwrap();
        assert!(
            level
                .set_block(host, pos, state, mod_api::BlockUpdateFlags::CLIENTS)
                .unwrap()
        );
        if self.1.load(Ordering::Relaxed) == 2 {
            panic!("failure after reentrant replacement");
        }
        self.0.lock().unwrap().push(pos);
    }
}
struct Parts {
    xp: Xp,
    blast: Blast,
    add: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Option<u32>>>>,
    cancel_blast: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:diamond_ore",
            "minecraft:spawner",
            "minecraft:chest",
        ] {
            let key = r.block_key(name);
            r.block_part(key, self.xp.clone());
        }
        let key = r.block_key("minecraft:chest");
        r.block_part(key, self.blast.clone());
    }
    fn explosion_start(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::ExplosionStartEvent<'_>,
    ) -> Flow {
        let _ = event;
        if self.cancel_blast.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn break_block(&self, _: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        self.seen.lock().unwrap().push(*event.exp_to_drop);
        if self.add.load(Ordering::Relaxed) {
            *event.exp_to_drop = Some(event.exp_to_drop.unwrap() + 3);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "needs prepared resources and isolated native mod state"]
fn block_parts_xp_and_explosion_contract() {
    let amount = Arc::new(AtomicI32::new(7));
    let add = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let blast = Blast::default();
    let cancel_blast = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "block_parts",
        Box::new(Parts {
            xp: Xp(amount.clone()),
            blast: blast.clone(),
            add: add.clone(),
            seen: seen.clone(),
            cancel_blast: cancel_blast.clone(),
        }),
    )]
    .into();
    mod_api::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let spawner = registry.default_state("minecraft:spawner").unwrap();
            let pos = BlockPos::new(1, 64, 1);
            let drop = |game: &mut Game<'_>,
                        state,
                        tool: Option<&horizon_client::items::ItemStack>,
                        override_xp| {
                game.with_level(|_, level, _| {
                    level.block_dropped(pos, state, None, tool, Vec::new(), override_xp)
                })
                .experience
            };
            let emerald = registry.default_state("minecraft:emerald_ore").unwrap();
            game.world.random = LegacyRandom::new(7);
            assert_eq!(drop(game, emerald, None, None), 4);
            assert_eq!(drop(game, ore, None, None), 7);
            assert_eq!(drop(game, stone, None, None), 0);
            let mut canceled = false;
            let mut xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(xp, Some(7));
            assert_eq!(drop(game, ore, None, xp), 7);
            add.store(true, Ordering::Relaxed);
            xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(seen.lock().unwrap().as_slice(), &[Some(7), Some(7)]);
            assert_eq!(drop(game, ore, None, xp), 10);
            let mut silk = horizon_client::items::ItemStack::new("minecraft:diamond_pickaxe", 1);
            silk.extra
                .get_or_insert_with(Default::default)
                .enchantments
                .push(("minecraft:silk_touch".into(), 1));
            assert_eq!(drop(game, ore, Some(&silk), None), 0);
            assert_eq!(drop(game, ore, Some(&silk), xp), 10);
            *game.held_mut() = Some(silk.clone());
            xp = None;
            game.native_break_block([1, 64, 1], ore, &mut canceled, &mut xp);
            assert_eq!(seen.lock().unwrap().last(), Some(&Some(0)));
            assert_eq!(xp, Some(3));
            assert_eq!(drop(game, ore, Some(&silk), xp), 3);
            *game.held_mut() = None;
            assert_eq!(drop(game, spawner, Some(&silk), None), 7);
            for (answer, expected) in [(-1, 0), (i32::MAX, 32767), (7, 7)] {
                amount.store(answer, Ordering::Relaxed);
                assert_eq!(drop(game, ore, None, None), expected);
            }
            let chest = registry.default_state("minecraft:chest").unwrap();
            let start = ExplosionStart::from(mod_api::__private::built::ExplosionStart {
                dimension: "minecraft:overworld",
                center: [1.0, 65.0, 1.0],
                radius: 4.0,
                fire: false,
                interaction: mod_api::BlockInteraction::Destroy,
                damage_type: "minecraft:explosion",
                source: None,
                owner: None,
            });
            for x in 1..=3 {
                game.level_set([x, 64, 1], if x == 1 { chest } else { stone })
                    .unwrap();
            }
            let mut contents = vec![None; 27];
            contents[0] = Some(horizon_client::items::ItemStack::new(
                "minecraft:diamond",
                3,
            ));
            game.world.block_entities.insert(
                (1, 64, 1),
                horizon_client::containers::BlockEntity::Chest(contents),
            );
            assert_eq!(drop(game, chest, None, None), 7);
            cancel_blast.store(true, Ordering::Relaxed);
            game.server_explode(start.center, 4.0, BlockInteraction::Destroy, false)
                .unwrap();
            assert_eq!(game.world.block(1, 64, 1), chest);
            assert!(blast.0.lock().unwrap().is_empty());
            cancel_blast.store(false, Ordering::Relaxed);
            game.server_explode(start.center, 4.0, BlockInteraction::Keep, false)
                .unwrap();
            assert!(blast.0.lock().unwrap().is_empty());
            assert_eq!(game.world.block(1, 64, 1), chest);
            game.server_explode_as(
                start.center,
                4.0,
                BlockInteraction::Destroy,
                false,
                "minecraft:explosion",
                Some(([1, 64, 1], f32::MAX)),
            )
            .unwrap();
            assert!(blast.0.lock().unwrap().is_empty());
            assert_eq!(game.world.block(1, 64, 1), chest);
            for x in 2..=3 {
                game.level_set([x, 64, 1], stone).unwrap();
            }
            let chest_handle = game
                .block_entity(game.level("minecraft:overworld").unwrap(), [1, 64, 1])
                .unwrap();
            let mut drops = ExplosionDrops::default();
            for x in 1..=4 {
                game.explosion_hit_block(
                    [x, 64, 1],
                    4.0,
                    BlockInteraction::Destroy,
                    &start,
                    &mut drops,
                )
                .unwrap();
            }
            assert_eq!(blast.0.lock().unwrap().as_slice(), &[[1, 64, 1]]);
            for x in 1..=4 {
                assert_eq!(game.world.block(x, 64, 1), game.world.air);
            }
            assert!(!game.world.block_entities.contains_key(&(1, 64, 1)));
            game.with_level(|_, level, _| {
                assert!(mod_api::LevelHost::block_entity_data(level, chest_handle).is_none());
            });
            for mode in 1..=3 {
                game.level_set([1, 64, 1], chest).unwrap();
                let mut contents = vec![None; 27];
                contents[0] = Some(horizon_client::items::ItemStack::new(
                    "minecraft:diamond",
                    3,
                ));
                game.world.block_entities.insert(
                    (1, 64, 1),
                    horizon_client::containers::BlockEntity::Chest(contents),
                );
                blast.1.store(mode, Ordering::Relaxed);
                let mut drops = ExplosionDrops::default();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.explosion_hit_block(
                        [1, 64, 1],
                        4.0,
                        BlockInteraction::Destroy,
                        &start,
                        &mut drops,
                    )
                    .unwrap();
                }));
                assert_eq!(
                    game.world.block(1, 64, 1),
                    game.world.air,
                    "failed callback must not leave a collected block: mode {mode}"
                );
                assert!(!game.world.block_entities.contains_key(&(1, 64, 1)));
                assert!(
                    result.is_ok(),
                    "part failure must be contained: mode {mode}"
                );
            }
        });
    });
}
