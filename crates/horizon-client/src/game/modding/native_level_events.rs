//! Events, explosions and lightning in a level for native server mods,
//! through a `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"):
//! `Level.levelEvent`, `Level.gameEvent`, `ServerLevel.blockEvent`,
//! `Level.explode` and a summoned lightning bolt. Like the level's block
//! writes, they act on the level in place, the one the hook runs in; a parked
//! level, a stale handle or a client returns false. Reaching a parked level, as `play_sound` and the
//! reads do through `native_level_world`, is a follow-up.
use super::{Game, GameLevel, native_explosions::NativeBlast};
use horizon_world::level::{BlockEventData, LevelEvent};
use mod_api::{BlockKey, ExplosionInteraction, LevelRef};

/// A position inside the world's horizontal and vertical bounds.
fn in_bounds(at: [f64; 3]) -> bool {
    at.iter().all(|v| v.is_finite())
        && at[0].abs() < 30_000_000.0
        && at[2].abs() < 30_000_000.0
        && at[1].abs() < 20_000_000.0
}

/// Whether level event `id` can play with `data`: the events that carry a
/// block state (2001, 2014, 3008) need `data` to be one of the `states`.
fn level_event_data_ok(id: i32, data: i32, states: usize) -> bool {
    !matches!(id, 2001 | 2014 | 3008) || usize::try_from(data).is_ok_and(|d| d < states)
}

impl GameLevel<'_> {
    /// `Level.levelEvent(null, id, pos, data)` for a hook that holds the
    /// level's write host: the event joins the ones the block behaviour
    /// emits, in order, and plays when the write that runs the hook is
    /// done (`process_level_events`), with the guests told as for any event
    /// the behaviour raises. False off an authoritative server, or for an
    /// event that carries a block state (see `level_level_event`)
    /// when `data` isn't one.
    pub(super) fn native_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        if !self.native_authoritative || !level_event_data_ok(id, data, self.facts.states.len()) {
            return false;
        }
        self.events.push(LevelEvent::LevelEvent {
            id,
            pos: horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source: None,
        });
        true
    }
}

impl Game<'_> {
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    pub(super) fn level_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        if !level_event_data_ok(id, data, self.res.facts().states.len()) {
            return false;
        }
        self.level_event_at(id, pos, data);
        true
    }
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(event, at, context)`, with validated source and state.
    pub(super) fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: mod_api::GameEventContext,
    ) -> bool {
        let Some(event) = horizon_client::sculk::intern(event) else {
            return false;
        };
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        self.post_contextual_game_event(event, at, context)
    }
    /// Queues block event `a`, `b` for `block` at `pos` in the level in place
    /// (`ServerLevel.blockEvent`): `run_block_events` hands it to the block's
    /// `triggerEvent` after this tick's scheduled ticks if `pos` still holds
    /// `block`. False for an unresolved block key, a position outside the
    /// build height (it could never reach a block), a parked level, a stale
    /// handle or a client.
    pub(super) fn level_block_event(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        a: i32,
        b: i32,
    ) -> bool {
        let Some(block) = super::api_values::behavior_block(block, self.res.behaviours()) else {
            return false;
        };
        if !self.level_in_place(level) || !self.world.shape.contains_y(pos[1]) {
            return false;
        }
        self.block_ticks.queue_block_event(BlockEventData {
            pos: horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            block,
            param_a: a,
            param_b: b,
            note_override: None,
        });
        true
    }
    /// Explodes at `center` with no source entity, as `Level.explode(null,
    /// x, y, z, radius, fire, interaction)` does (`native_explode`): `fire`
    /// lights some of the air around, `interaction` says what happens to
    /// blocks: `None`, `Block` or `Tnt` (vanilla's `Mob` and `Trigger` aren't
    /// offered yet). False for a chunk that isn't loaded or a radius outside
    /// `0 < radius <= 32`.
    pub(super) fn level_explode(
        &mut self,
        level: LevelRef,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        let blast = match interaction {
            ExplosionInteraction::None => NativeBlast::Keep,
            ExplosionInteraction::Block => NativeBlast::Block,
            ExplosionInteraction::Tnt => NativeBlast::Tnt,
            _ => unreachable!("ExplosionInteraction: unhandled variant"),
        };
        self.native_explode(level.dimension_id(), center, radius, fire, blast)
    }
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    pub(super) fn level_strike_lightning(
        &mut self,
        level: LevelRef,
        at: [f64; 3],
        visual_only: bool,
    ) -> bool {
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        let chunk = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        if !self.world.chunks.contains_key(&chunk) {
            return false;
        }
        let block = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        if !self.lightning_strike_allowed(None, block, "mod") {
            return false;
        }
        self.spawn_bolt(at, visual_only);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::game::LevelDimension;
    use horizon_world::level::LevelEvent;
    use mod_api::{self, BlockPart, Flow, Hooks, RandomTickEvent, Registrar};

    /// A stone part whose random tick plays level events through the write host.
    struct Events;
    #[mod_api::horizon_part]
    impl BlockPart for Events {
        fn random_tick(
            &self,
            level: &mut dyn mod_api::LevelWriteHost,
            event: &mut RandomTickEvent,
        ) -> Flow {
            // 2001 carries a block state: `data` -1 is none, so it is refused.
            assert!(!level.level_event(2001, [1, 64, 1], -1));
            assert!(level.level_event(1500, [1, 64, 1], 0));
            assert!(level.level_event(2005, [2, 64, 2], 7));
            let _ = event;
            Flow::Continue
        }
    }
    struct EventsMod;
    #[mod_api::__subscriptions]
    impl Hooks for EventsMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let block_key = r.block_key("minecraft:stone");
            r.block_part(block_key, Events);
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_queued_block_event_runs_the_blocks_trigger_once() {
        use horizon_core::position::BlockPos;
        use mod_api::__private::part_tests::block_key;
        use mod_api::{LevelRef, WorldEpoch};
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let note = game
                .pack
                .assets
                .registry
                .default_state("minecraft:note_block")
                .unwrap();
            game.world.set(0, 64, 0, note).unwrap();
            let (note_key, stone_key) = (block_key("minecraft:note_block"), block_key("stone"));

            // Queued twice, it runs once; one queued for a block `pos` no longer holds is
            // dropped when it runs, as `runBlockEvents` checks the block.
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], stone_key, 0, 0));
            assert_eq!(game.block_ticks.block_events.len(), 2);
            let (bottom, height) = level.height(game).unwrap();
            assert!(!level.block_event(game, [0, bottom - 1, 0], note_key, 0, 0));
            assert!(
                !level.block_event(game, [0, bottom + height, 0], note_key, 0, 0),
                "above the build height"
            );
            assert!(!stale.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(
                !nether.block_event(game, [0, 64, 0], note_key, 0, 0),
                "a level not in place"
            );
            assert_eq!(
                game.block_ticks.block_events.len(),
                2,
                "refusals queue nothing"
            );

            game.level_events.clear();
            game.with_level(|b, level, ticks| b.run_block_events(level, ticks));
            assert!(game.block_ticks.block_events.is_empty());
            let handled: Vec<_> = game
                .level_events
                .iter()
                .filter(|e| matches!(e, LevelEvent::BlockEntityEvent { .. }))
                .collect();
            assert_eq!(
                handled,
                [&LevelEvent::BlockEntityEvent {
                    pos: BlockPos::new(0, 64, 0),
                    state: note,
                    a: 0,
                    b: 0,
                }],
                "the note block's triggerEvent handled it once, for its players"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn a_level_event_in_a_write_hook_joins_the_events_in_order() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("level-events", Box::new(EventsMod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                let before = LevelEvent::LevelEvent {
                    id: 1000,
                    pos: horizon_core::position::BlockPos::new(0, 64, 0),
                    data: 0,
                    source: None,
                };
                game.level_events.push(before.clone());
                let state = game.world.block(0, 64, 0);
                let mut handled = false;
                game.as_guest(12, |game| {
                    game.audio.capture = Some(Vec::new());
                    game.with_level(|_, level, ticks| {
                        level.native_random_tick(
                            ticks,
                            LevelDimension::Overworld,
                            horizon_core::position::BlockPos::new(0, 64, 0),
                            state,
                            &mut handled,
                        );
                    });
                    let events = game.level_events.clone();
                    game.lan_record_events(&events);
                    assert_eq!(
                        game.lan_events[1],
                        super::super::QueuedLevelEvent {
                            kind: 1500,
                            pos: [1, 64, 1],
                            data: 0,
                            source: None,
                        }
                    );
                    assert_eq!(
                        game.lan_events[2],
                        super::super::QueuedLevelEvent {
                            kind: 2005,
                            pos: [2, 64, 2],
                            data: 7,
                            source: None,
                        }
                    );
                })
                .unwrap();
                let at = |id, x, z, data| LevelEvent::LevelEvent {
                    id,
                    pos: horizon_core::position::BlockPos::new(x, 64, z),
                    data,
                    source: None,
                };
                assert_eq!(
                    game.level_events,
                    [before, at(1500, 1, 1, 0), at(2005, 2, 2, 7)]
                );
            });
        });
    }
}
