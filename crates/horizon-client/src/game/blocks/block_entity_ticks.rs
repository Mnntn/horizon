//! `Level.tickBlockEntities` for the block entities ticked together here
//! (hoppers, crafters, jukeboxes, beacons, creaking hearts, spawners, trial
//! spawners, vaults, beehives, furnaces and potent sulfur): only those whose state has a
//! ticker, in vanilla's list order (`horizon_client::block_entities`), in
//! chunks that tick blocks. Sculk, brewing stands and pistons tick in their
//! own passes.
use super::Game;
use horizon_client::{
    Result,
    block_entities::{Pos, Ticking},
    block_entity_anim::Ticker,
    containers::{BlockEntity, FurnaceKind, HOPPER_SLOTS},
    furnace::FurnaceCooking,
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    pub(super) fn tick_block_entities(&mut self) -> Result<()> {
        self.tick_mount_menu();
        self.tick_craft_table();
        self.tick_container_reach();
        self.load_entity_types();
        self.ensure_stateful_entity_table();
        if !self.world.block_entities.has_states() {
            let states = self.ticking_states();
            self.world.block_entities.set_states(states);
            let containers = self.native_containers();
            self.world.block_entities.set_mod_containers(containers);
            let fluids = self.native_fluid_tables();
            self.world.block_entities.set_mod_fluids(fluids);
            let energy = self.native_energy_tables();
            self.world.block_entities.set_mod_energy(energy);
        }
        // Only lingering cooldowns are dropped; a hopper that ticks checks
        // its own block. Once a second is soon enough for the rest.
        if self.world.game_time % 20 == 0 {
            self.forget_gone_hoppers();
        }
        let ticking = self.block_ticking_area();
        self.spawner_tick_positions.clear();
        self.trial_spawner_tick_positions.clear();
        self.item_columns = Some(Default::default());
        self.pass_mobs = Some(Default::default());
        let tickers = self.world.block_entity_tickers();
        let mut last_chunk = (i32::MIN, i32::MIN);
        let mut last_chunk_ticking = false;
        for slot in tickers.iter().copied() {
            let pos = slot.0;
            let chunk = (pos.0 >> 4, pos.2 >> 4);
            let chunk_ticking = if chunk == last_chunk {
                last_chunk_ticking
            } else {
                last_chunk = chunk;
                last_chunk_ticking = ticking(pos.0, pos.2);
                last_chunk_ticking
            };
            if !chunk_ticking {
                continue;
            }
            // Ticks before this one can remove it (`TickingBlockEntity.isRemoved`).
            if self.world.block_entities.is_asleep(&pos)
                || !self.world.block_entities.is_live(slot)
                || !self.border.contains(f64::from(pos.0), f64::from(pos.2))
            {
                continue;
            }
            match self.world.block_entities.get(&pos) {
                Some(BlockEntity::Chest(items)) if items.len() == HOPPER_SLOTS => {
                    self.tick_hopper(pos)
                }
                Some(BlockEntity::Crafter(_)) => self.tick_crafter(pos),
                Some(BlockEntity::Jukebox(_)) => self.tick_jukebox(pos),
                Some(BlockEntity::Beacon(_)) => self.tick_beacon_at(pos),
                Some(BlockEntity::CreakingHeart(_)) => self.tick_creaking_heart(pos)?,
                Some(BlockEntity::Spawner(_)) => {
                    self.tick_spawner(pos)?;
                    self.spawner_tick_positions.push(pos);
                }
                Some(BlockEntity::TrialSpawner(_)) => {
                    self.tick_trial_spawner(pos)?;
                    self.trial_spawner_tick_positions.push(pos);
                }
                Some(BlockEntity::Vault(_)) => self.tick_vault(pos)?,
                Some(BlockEntity::Beehive(_)) => self.tick_beehive(pos),
                Some(BlockEntity::Furnace(_)) => self.tick_furnace(pos)?,
                Some(BlockEntity::Brewing(_)) => self.tick_brewing_stand(pos)?,
                Some(BlockEntity::DaylightDetector) => self.tick_daylight_detector(pos),
                Some(BlockEntity::EndGateway(_)) => self.tick_end_gateway(pos),
                Some(BlockEntity::Campfire(_)) => self.tick_campfire(pos)?,
                Some(BlockEntity::Mod(_)) => self.tick_mod_block_entity(pos),
                Some(BlockEntity::PotentSulfur(_)) => {
                    self.tick_potent_sulfur(pos)?;
                    // A window that simulates the world itself is its own
                    // client; a server's clients tick it on their own.
                    if self.options.execution_role.is_client() {
                        self.client_tick_potent_sulfur(pos);
                    }
                }
                _ => {}
            }
        }
        // Items move between passes: the columns hold for this one only.
        self.item_columns = None;
        self.pass_mobs = None;
        self.update_song_volumes();
        self.scan_for_hearts();
        // The client ticks (particles, sounds, the turning mob) are for a
        // window; a dedicated or integrated server has none.
        if self.options.dedicated.is_none() {
            let spawners = std::mem::take(&mut self.spawner_tick_positions);
            self.client_tick_spawners(&spawners);
            self.spawner_tick_positions = spawners;
            let trial_spawners = std::mem::take(&mut self.trial_spawner_tick_positions);
            for key in trial_spawners.iter().copied() {
                self.client_tick_trial_spawner(key);
            }
            self.trial_spawner_tick_positions = trial_spawners;
        }
        // `ShulkerBoxBlockEntity.tick`: the lids of the boxes this level
        // animates. Their grown collision shape reaches the collision queries
        // through `World::shulkers`, and an opening lid pushes what it runs
        // into (`moveCollidedEntities`), as a moving piston does.
        let pushes = self.tick_shulkers();
        self.sync_shulkers();
        self.push_shulker_entities(pushes);
        Ok(())
    }
    /// `ShulkerBoxBlockEntity.tick` for the boxes in block-ticking chunks, in
    /// position order: the lid's new progress, and the swept box of an opening
    /// lid for `moveCollidedEntities`. A box whose block is gone is dropped.
    /// The boxes are those the level animates: a box opens (`start_open` and
    /// block event 1) before it has one, exactly as vanilla's block entity is
    /// there from the start.
    fn tick_shulkers(&mut self) -> Vec<super::piston_push::ShulkerPush> {
        let ticking = self.block_ticking_area();
        let mut pushes = Vec::new();
        let mut gone = Vec::new();
        let positions: Vec<[i32; 3]> = self.be_anims.shulkers.keys().copied().collect();
        for pos in positions {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let Some(Ticker::Shulker { facing }) = self.be_anims.ticker(state.0) else {
                gone.push(pos);
                continue;
            };
            if !ticking(pos[0], pos[2])
                || !self.border.contains(f64::from(pos[0]), f64::from(pos[2]))
            {
                continue;
            }
            let tick = self
                .be_anims
                .shulkers
                .get_mut(&pos)
                .map(|shulker| shulker.tick(pos, facing));
            if let Some(push) = tick.and_then(|tick| tick.push) {
                pushes.push((pos, facing, push));
            }
        }
        for pos in gone {
            self.be_anims.shulkers.remove(&pos);
        }
        pushes
    }
    /// Copies the shulker box lids into the world, where the collision queries
    /// read them (`SceneCollisions::shulker_collision_state`), as
    /// `sync_moving_pistons` does for pistons.
    pub(super) fn sync_shulkers(&mut self) {
        self.world.shulkers.clear();
        self.world
            .shulkers
            .extend(self.be_anims.shulkers.iter().map(|(p, s)| (*p, s.clone())));
    }
    /// `BlockState.getTicker` of every state, for the block entities ticked
    /// here. A creaking heart keeps its slot while uprooted (see
    /// `tick_creaking_heart`).
    pub(in crate::game) fn ticking_states(&self) -> Vec<Ticking> {
        let registry = self.pack.assets.registry;
        (0..registry.len())
            .map(|i| {
                let state = BlockStateId(i as u32);
                let name: &str = &self.res.facts().get(state).name;
                let factory = horizon_client::block_entity_types::factory_block(name);
                match factory {
                    "minecraft:hopper"
                    | "minecraft:crafter"
                    | "minecraft:beacon"
                    | "minecraft:creaking_heart"
                    | "minecraft:spawner"
                    | "minecraft:trial_spawner"
                    | "minecraft:vault"
                    | "minecraft:beehive"
                    | "minecraft:bee_nest"
                    | "minecraft:brewing_stand"
                    | "minecraft:daylight_detector"
                    | "minecraft:end_gateway"
                    // Lit, cookTick; unlit, cooldownTick.
                    | "minecraft:campfire"
                    | "minecraft:soul_campfire" => Ticking::Yes,
                    // Every state but dry has a ticker.
                    "minecraft:potent_sulfur" => {
                        let dry = registry
                            .state(state)
                            .and_then(|s| s.properties.get("potent_sulfur_state"))
                            .is_some_and(|v| v == "dry");
                        if dry { Ticking::Idle } else { Ticking::Yes }
                    }
                    "minecraft:jukebox" => {
                        let record = registry
                            .state(state)
                            .and_then(|s| s.properties.get("has_record"))
                            .is_some_and(|v| v == "true");
                        if record { Ticking::Yes } else { Ticking::Idle }
                    }
                    n if FurnaceKind::from_block(n).is_some() => Ticking::Yes,
                    _ if !name.starts_with("minecraft:") && self.native_ticks(name) => Ticking::Yes,
                    _ => Ticking::No,
                }
            })
            .collect()
    }
    /// `LevelChunk.isTicking` (block coordinates): a block-ticking chunk from
    /// every player's tickets (`update_chunk_tickets`), its entities loaded
    /// (`ServerLevel.areEntitiesLoaded`).
    pub(super) fn block_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        let has_pending = !self.world.entities_pending.is_empty();
        let pending = has_pending.then(|| self.world.entities_pending.clone());
        move |x, z| {
            levels.block_ticking_at(x, z)
                && pending
                    .as_ref()
                    .is_none_or(|p| !p.contains(&(x >> 4, z >> 4)))
        }
    }
    /// `DaylightDetectorBlock.tickEntity` of the detector at `pos`.
    fn tick_daylight_detector(&mut self, pos: Pos) {
        // It updates every 20 game ticks (the behaviour checks this too).
        if self.world.game_time % 20 != 0 {
            return;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.tick_daylight_detector(level, ticks, at));
        let _ = self.process_level_events();
    }
    /// `AbstractFurnaceBlockEntity.serverTick` of the furnace at `pos`, and
    /// its block's `lit` following it.
    fn tick_furnace(&mut self, pos: Pos) -> Result<()> {
        // What the hooks need from before the tick, without cloning stacks: the
        // fuel's id only while the furnace is out (it can light this tick), the
        // input's id (a last item is gone after it smelts) and the result count.
        let before = match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Furnace(f)) => (
                f.cooking_timer,
                (f.lit_remaining == 0)
                    .then(|| f.items[1].as_ref().map(|s| s.item.clone()))
                    .flatten(),
                f.items[0].as_ref().map(|s| s.item.clone()),
                f.items[2].as_ref().map_or(0, |s| s.count),
                f.lit_remaining,
            ),
            _ => return Ok(()),
        };
        let mut drops = Vec::new();
        let (lit, burning, smelted, smelting, fueled) = {
            let needs_part = mod_api::__private::has_part_hook(mod_api::HookId::CraftingRemainder)
                && matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Furnace(f))
                    if f.lit_remaining <= 1 && f.items[1].as_ref().is_some_and(|s|
                        mod_api::__private::item_listens(mod_api::HookId::CraftingRemainder, &s.item)));
            let lit = if needs_part {
                let Some(BlockEntity::Furnace(original)) = self.world.block_entities.get(&pos)
                else {
                    return Ok(());
                };
                let mut furnace = original.clone();
                let lit = furnace.tick(
                    &self.loop_context,
                    world_recipes!(self),
                    world_items!(self),
                    self.res.providers(),
                    &mut drops,
                    &|s| self.crafting_remainder(s),
                );
                self.world
                    .block_entities
                    .insert(pos, BlockEntity::Furnace(furnace));
                lit
            } else {
                let Some(BlockEntity::Furnace(furnace)) = self.world.block_entities.get_mut(&pos)
                else {
                    return Ok(());
                };
                let items = world_items!(self);
                furnace.tick(
                    &self.loop_context,
                    world_recipes!(self),
                    items,
                    self.res.providers(),
                    &mut drops,
                    &|s| crate::crafting::CraftingRemainder::crafting_remainder(items, s),
                )
            };
            let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) else {
                return Ok(());
            };
            let burning = before.1.filter(|_| f.lit_remaining > 0);
            let smelted = f.items[2]
                .as_ref()
                .filter(|after| after.count > before.3)
                .map(|after| (before.2.clone(), after.item.clone()));
            let started = before.0 == 0 && f.cooking_timer > 0;
            let smelting = started
                .then(|| f.items[0].as_ref().map(|item| item.item.clone()))
                .flatten();
            // The tick takes one off `lit_remaining`; only consuming fuel
            // sets it higher (a lava bucket leaves a bucket of the same count).
            let fueled = f.lit_remaining > before.4.saturating_sub(1);
            (lit, burning, smelted, smelting, fueled)
        };
        // `AbstractFurnaceBlockEntity.serverTick`: `burn` and consuming fuel
        // set the changed flag, so a completed smelt or a refuel marks the
        // chunk and tells comparators even though the block state stays LIT.
        if smelted.is_some() || fueled {
            self.container_changed(pos);
        }
        if let Some(item) = smelting {
            self.native_furnace_start_smelt([pos.0, pos.1, pos.2], &item);
        }
        if let Some(fuel) = burning {
            self.native_furnace_burn([pos.0, pos.1, pos.2], &fuel);
        }
        if let Some((Some(item), result)) = smelted {
            self.native_furnace_smelt([pos.0, pos.1, pos.2], &item, &result);
        }
        for stack in drops {
            self.campfire_drop(pos, stack);
        }
        let Some(lit) = lit else {
            if matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Furnace(f)) if f.idle())
            {
                self.world.block_entities.sleep(pos);
            }
            return Ok(());
        };
        let state = self.world.block(pos.0, pos.1, pos.2);
        if let Some(mut s) = self.pack.assets.registry.state(state).cloned()
            && s.properties.contains_key("lit")
        {
            s.properties.insert("lit".into(), lit.to_string());
            if let Ok(id) = self.pack.assets.registry.resolve(&s) {
                self.level_set([pos.0, pos.1, pos.2], id)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod jukebox_stop_tests {
    use super::Game;
    use horizon_client::items::ItemStack;

    /// A windowless game from the prepared resources with a resident chunk and
    /// a jukebox at (0, 64, 0), recording its game events; `None` without the
    /// resources. `f` gets the game and the disc.
    fn with_jukebox(f: impl FnOnce(&mut Game<'_>, &ItemStack)) -> Option<Vec<String>> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
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
        .ok()?;
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        let jukebox = game
            .pack
            .assets
            .registry
            .default_state("minecraft:jukebox")?;
        game.world.set(0, 64, 0, jukebox).unwrap();
        let disc = ItemStack::new("minecraft:music_disc_cat", 1);
        crate::game::dispense_fixture::start_recording();
        f(&mut game, &disc);
        Some(crate::game::dispense_fixture::stop_recording())
    }

    fn stops(lines: &[String]) -> Vec<&String> {
        lines
            .iter()
            .filter(|l| l.ends_with("minecraft:jukebox_stop_play"))
            .collect()
    }

    /// `JukeboxSongPlayer.stop` posts `JUKEBOX_STOP_PLAY` at the jukebox, and
    /// `popOutTheItem` (the disc taken out by hand) and the end of a song both
    /// reach it. Horizon stopped the sound and told the parrots but posted the
    /// vibration only when the block itself was broken, so pulling the disc out
    /// was silent to a sculk sensor beside it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_jukebox_that_stops_tells_the_listeners() {
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    horizon_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines),
            vec![&"G 0,64,0 minecraft:jukebox_stop_play".to_owned()],
            "{lines:?}"
        );

        // The end of a song is the same stop: tick until the song is over.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            for _ in 0..40_000 {
                let playing = matches!(
                    game.world.block_entities.get(&(0, 64, 0)),
                    Some(horizon_client::containers::BlockEntity::Jukebox(j)) if j.playing
                );
                if !playing {
                    break;
                }
                game.tick_jukebox((0, 64, 0));
            }
            // JukeboxSongPlayer.stop posts only while a song plays: taking
            // the finished disc out is silent.
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    None,
                    horizon_world::level::UseBranch::WithoutItem,
                ) == Some(mod_api::InteractionResult::Success),
                "the empty hand pops it out"
            );
        }) else {
            return;
        };
        assert_eq!(
            stops(&lines).len(),
            1,
            "the song's own end posts it once, its removal after it nothing: {lines:?}"
        );

        // A broken jukebox posts it from the world's spill instead, and a
        // jukebox that never held a disc posts nothing at all.
        let Some(lines) = with_jukebox(|game, disc| {
            assert!(
                game.use_jukebox(
                    [0, 64, 0],
                    game.world.block(0, 64, 0),
                    Some(disc),
                    horizon_world::level::UseBranch::ItemOn,
                ) == Some(mod_api::InteractionResult::Success),
                "the disc goes in"
            );
            game.jukebox_removed([0, 64, 0]);
        }) else {
            return;
        };
        assert!(stops(&lines).is_empty(), "not twice: {lines:?}");
        let Some(lines) = with_jukebox(|game, _| {
            for _ in 0..41 {
                game.tick_jukebox((0, 64, 0));
            }
        }) else {
            return;
        };
        assert!(lines.is_empty(), "{lines:?}");
    }
}

#[cfg(test)]
mod add_blocks_tests {
    use super::*;
    use mod_api::*;
    use std::sync::{Arc, OnceLock};

    #[test]
    #[ignore = "needs prepared resources; isolates process-wide registries"]
    fn vanilla_template_membership_creates_ticks_and_saves() {
        const FLAG: &str = "HORIZON_ADD_BLOCKS_CASE";
        const BLOCK: &str = "add_blocks_test:vault";
        const SIGN: &str = "add_blocks_test:sign";
        const DECLARED: &str = "add_blocks_test:declared";
        if std::env::var_os(FLAG).is_none() {
            for case in ["valid", "quiet", "bad_class", "bad_block", "bad_type"] {
                let scratch = std::env::temp_dir()
                    .join(format!("horizon-add-blocks-{}-{case}", std::process::id()));
                std::fs::create_dir_all(&scratch).unwrap();
                let mut child = std::process::Command::new(std::env::current_exe().unwrap());
                child.args(["--ignored", "--exact", "game::block_entity_ticks::add_blocks_tests::vanilla_template_membership_creates_ticks_and_saves", "--nocapture"])
                    .current_dir(&scratch).env(FLAG, case).env("HORIZON_NO_AUDIO", "1").env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
                for key in [
                    "HORIZON_HOME",
                    "HORIZON_LAUNCHER_HOME",
                    "XDG_DATA_HOME",
                    "XDG_CONFIG_HOME",
                ] {
                    child.env(key, scratch.join(key));
                }
                let status = child.status().unwrap();
                std::fs::remove_dir_all(&scratch).unwrap();
                assert!(status.success(), "case {case}");
            }
            return;
        }
        let case = std::env::var(FLAG).unwrap();
        #[derive(Default)]
        struct Keys(
            OnceLock<(BlockEntityTypeKey, BlockEntityTypeKey, BlockKey, BlockKey)>,
            std::sync::atomic::AtomicUsize,
        );
        struct Listener(Arc<Keys>, String);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r.block_entity_type_key(if self.1 == "bad_type" {
                    "minecraft:missing"
                } else {
                    "minecraft:vault"
                });
                let extra = r.block_entity_type("extra").blocks(&[]).build();
                r.block_entity_type("own").blocks(&[DECLARED]).build();
                let declared = r.block_key(DECLARED);
                let block = r.block_key(if self.1 == "bad_block" {
                    "add_blocks_test:missing"
                } else {
                    BLOCK
                });
                assert_eq!(
                    *self.0.0.get_or_init(|| (kind, extra, block, declared)),
                    (kind, extra, block, declared)
                );
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                for (id, template) in [
                    (
                        BLOCK,
                        if self.1 == "bad_class" {
                            "minecraft:stone"
                        } else {
                            "minecraft:vault"
                        },
                    ),
                    (SIGN, "minecraft:oak_sign"),
                    (DECLARED, "minecraft:vault"),
                ] {
                    event.blocks.push(mod_api::Block::new(id, template));
                }
            }
            fn block_entity_type_add_blocks(
                &self,
                _: &RegistrationContext,
                event: &mut BlockEntityTypeAddBlocksEvent<'_>,
            ) {
                self.0.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let &(kind, extra, block, declared) = self.0.0.get().unwrap();
                event.modify(kind, &[block, declared]);
                event.modify_matching(|key| key == kind || key == extra, &[block]);
            }
        }
        struct Quiet(Listener);
        #[mod_api::__subscriptions]
        impl Hooks for Quiet {
            fn register(&self, r: &mut dyn Registrar, context: &RegistrationContext) {
                self.0.register(r, context);
            }
            fn blocks(&self, context: &RegistrationContext, event: &mut BlocksEvent<'_>) {
                self.0.blocks(context, event);
            }
        }
        let observed = Arc::new(Keys::default());
        let listener = Listener(observed.clone(), case.clone());
        let hooks: Box<dyn Hooks> = if case == "quiet" {
            Box::new(Quiet(listener))
        } else {
            Box::new(listener)
        };
        let mods: Mods = vec![ModEntry::new("add_blocks_test", hooks)].into();
        mod_api::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default());
        if case.starts_with("bad_") {
            let error = root
                .expect_err("invalid addition must fail startup")
                .to_string();
            assert!(
                error.contains(match case.as_str() {
                    "bad_class" => "must inherit VaultBlock",
                    "bad_block" => "unknown block",
                    _ => "unknown block entity type",
                }),
                "{error}"
            );
            return;
        }
        let root = root.unwrap();
        // Reload reuses frozen validity without redispatching registration.
        crate::game::mods::content_root(&base, &Default::default()).unwrap();
        assert_eq!(
            observed.1.load(std::sync::atomic::Ordering::SeqCst),
            usize::from(case != "quiet")
        );
        crate::game::native_entity_refs::tests::with_game_at(
            root.prepared().to_path_buf(),
            |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let registry = game.pack.assets.registry;
                let state = registry.default_state(BLOCK).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                assert!(
                    game.level_set_block(level, [1, 64, 1], state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(matches!(
                    BlockEntity::for_block(SIGN),
                    Some(BlockEntity::Sign(_))
                ));
                assert!(matches!(
                    BlockEntity::for_block("minecraft:vault"),
                    Some(BlockEntity::Vault(_))
                ));
                if case == "quiet" {
                    assert!(game.world.block_entities.get(&(1, 64, 1)).is_none());
                    return;
                }
                let states = game.ticking_states();
                assert_eq!(states[state.0 as usize], Ticking::Yes);
                game.world.block_entities.set_states(states);
                assert!(
                    game.world
                        .block_entity_tickers()
                        .iter()
                        .any(|slot| slot.0 == (1, 64, 1))
                );
                game.world.game_time = 100;
                game.tick_vault((1, 64, 1)).unwrap();
                let entity = game.world.block_entities.get(&(1, 64, 1)).unwrap();
                let BlockEntity::Vault(vault) = entity else {
                    panic!("must create vanilla vault")
                };
                assert_eq!(vault.state_updating_resumes_at, 120);
                let tag = horizon_client::save::encode_block_entity((1, 64, 1), entity, BLOCK);
                let (_, restored) =
                    horizon_client::save::decode_block_entity_for_block(&tag, BLOCK).unwrap();
                assert_eq!(
                    horizon_client::save::encode_block_entity((1, 64, 1), &restored, BLOCK),
                    tag
                );
                let BlockEntity::Vault(restored) = restored else {
                    panic!("saved type must stay vanilla")
                };
                assert_eq!(restored.state_updating_resumes_at, 120);
                assert!(matches!(
                    horizon_client::save::decode_block_entity_for_block(&tag, DECLARED),
                    Some((_, BlockEntity::Vault(_)))
                ));
                let own_state = registry.default_state(DECLARED).unwrap();
                assert!(
                    game.level_set_block(level, [3, 64, 1], own_state, BlockUpdateFlags::NONE)
                        .unwrap()
                );
                assert!(
                    matches!(game.world.block_entities.get(&(3, 64, 1)), Some(BlockEntity::Mod(entity)) if entity.id() == "add_blocks_test:own")
                );
                assert_eq!(
                    horizon_client::block_entity_types::valid(BLOCK, "minecraft:vault"),
                    Some(true)
                );
                assert_eq!(
                    horizon_client::block_entity_types::valid(BLOCK, "add_blocks_test:extra"),
                    Some(true)
                );
                assert_eq!(horizon_client::block_entity_types::type_for(BLOCK), None);
                assert_eq!(
                    horizon_client::block_entity_types::factory_block(BLOCK),
                    "minecraft:vault"
                );
            },
        );
    }
}

#[cfg(test)]
mod furnace_changed_tests {
    use crate::game::game_tests::tests::server_game;
    use horizon_client::{
        containers::{BlockEntity, Furnace, FurnaceKind},
        items::ItemStack,
    };
    use std::collections::HashMap;

    /// `AbstractFurnaceBlockEntity.serverTick`'s changed flag: `burn` sets it,
    /// and the old furnace calls `setChanged` at the end of the tick. A smelt
    /// that completes while the furnace stays LIT changes the output and the
    /// input without changing the block state, so nothing else marks it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_finished_smelt_marks_the_chunk() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        let pos = (8, 64, 8);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:raw_iron", 1)),
                    Some(ItemStack::new("minecraft:coal", 1)),
                    None,
                ],
                lit_remaining: 5,
                lit_total: 200,
                cooking_timer: 199,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }),
        );
        game.world.modified.clear();

        game.tick_furnace(pos).expect("the furnace ticks");

        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
            panic!("no furnace");
        };
        assert_eq!(
            f.items[2].as_ref().map(|s| &*s.item),
            Some("minecraft:iron_ingot"),
            "the smelt completed"
        );
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a finished smelt left the chunk unmarked"
        );
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_refuel_while_lit_marks_the_chunk() {
        // litTimeRemaining goes 1 -> 0 and the furnace takes new fuel in the
        // same tick: it stays LIT, and consuming the fuel sets changed.
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:furnace")
            .unwrap();
        let pos = (8, 64, 8);
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        game.world.block_entities.insert(
            pos,
            BlockEntity::Furnace(Furnace {
                kind: FurnaceKind::Furnace,
                items: [
                    Some(ItemStack::new("minecraft:raw_iron", 1)),
                    Some(ItemStack::new("minecraft:coal", 2)),
                    None,
                ],
                lit_remaining: 1,
                lit_total: 1600,
                cooking_timer: 10,
                cooking_total: 200,
                speed: 1.0,
                recipes_used: HashMap::new(),
            }),
        );
        game.world.modified.clear();

        game.tick_furnace(pos).expect("the furnace ticks");

        let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&pos) else {
            panic!("no furnace");
        };
        assert_eq!(f.items[1].as_ref().map(|s| s.count), Some(1), "fuel used");
        assert!(f.items[2].is_none(), "nothing smelted");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a refuel left the chunk unmarked"
        );
    }
}
