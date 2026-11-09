//! Behavioural proofs of the broadcast events around scoped interaction Parts.
use super::super::{
    Game, Hit, execution_role::ExecutionRole, lan::LanSettings,
    native_entity_refs::tests::with_game,
};
use horizon_client::{
    chunk::Chunk,
    first_person::Hand,
    items::ItemStack,
    lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    motion::PlayerMotion,
};
use horizon_core::geometry::Vec3;
use horizon_core::nbt::Tag;
use horizon_protocol::{connection::Pipe, packets::configuration::ClientInformation};
use mod_api::InteractionResult as Answer;
use mod_api::{
    self, BlockPart, ENTITY_ITEMS, Flow, Hooks, HostSide, HostTransaction, ItemPart, ItemResource,
    LogicalSide, Registrar, RightClickBlockEvent, RightClickItemEvent, ServerHost, TriState,
    UseItemEvent, UseItemOnBlockEvent, UseItemOnEvent, UseOnEvent, UsePhase,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};

#[derive(Debug, PartialEq, Eq)]
enum Step {
    Phase(UsePhase, LogicalSide),
    BlockPart,
    UseOnPart,
    FirstPart,
    Air(LogicalSide, Hand),
    ItemPart,
}

#[derive(Default)]
struct Controls {
    steps: Vec<Step>,
    /// Whether the onItemUseFirst moment is recorded in `steps`.
    record_first: bool,
    first_answer: Option<Answer>,
    first_part_answer: Option<Answer>,
    block_answer: Option<Answer>,
    item_on_answer: Option<Answer>,
    air_answer: Option<Answer>,
    consume_item: bool,
    expected_hand: Option<Hand>,
    /// Every hand `use_item_on_block` fired for, in order, instead of asserting one.
    hands: Option<Vec<Hand>>,
    use_block: TriState,
    use_item: TriState,
}

#[derive(Clone, Default)]
struct Observer(Arc<Mutex<Controls>>);

fn side(host: &HostSide<'_>) -> LogicalSide {
    match host {
        HostSide::Server(_) => LogicalSide::Server,
        HostSide::Client(_) => LogicalSide::Client,
    }
}

#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for block in ["minecraft:lever", "minecraft:stone", "minecraft:campfire"] {
            let block_key = r.block_key(block);
            r.block_part(block_key, self.clone());
        }
        for item in ["minecraft:stick", "minecraft:shield"] {
            let item = r.item_key(item);
            r.item_part(item, self.clone());
        }
    }
    fn right_click_block(&self, _: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        let state = self.0.lock().unwrap();
        *event.use_block = state.use_block;
        *event.use_item = state.use_item;
        Flow::Continue
    }
    fn use_item_on_block(&self, host: HostSide<'_>, event: &mut UseItemOnBlockEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 0]);
        assert_eq!(event.click.hit.face, [0, 1, 0]);
        if event.click.side == LogicalSide::Server {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::TryWithEmptyHand);
        let mut state = self.0.lock().unwrap();
        if let Some(hands) = &mut state.hands {
            hands.push(event.click.hand);
        } else {
            assert_eq!(event.click.hand, state.expected_hand.unwrap_or(Hand::Main));
        }
        if event.phase != UsePhase::ItemBeforeBlock || state.record_first {
            state.steps.push(Step::Phase(event.phase, event.click.side));
        }
        let answer = match event.phase {
            UsePhase::Block => state.block_answer,
            UsePhase::ItemAfterBlock => state.item_on_answer,
            UsePhase::ItemBeforeBlock => state.first_answer,
            _ => unreachable!("UsePhase: unhandled variant"),
        };
        if let Some(answer) = answer {
            if state.consume_item
                && event.phase == UsePhase::ItemAfterBlock
                && let HostSide::Server(host) = host
            {
                let player = host.player(event.click.player).unwrap();
                let items = player.entity().capability(host, &ENTITY_ITEMS, ()).unwrap();
                let slot = player.selected_slot(host).unwrap();
                let resource = ItemResource::of(event.click.stack.unwrap());
                host.transaction(|host, tx| {
                    assert_eq!(items.extract(host, tx, slot, &resource, 1), 1);
                    Ok::<_, ()>(())
                })
                .unwrap();
            }
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn right_click_item(&self, host: HostSide<'_>, event: &mut RightClickItemEvent<'_>) -> Flow {
        assert_eq!(side(&host), event.click.side);
        assert_eq!(event.click.pos, [0, 64, 3]);
        if event.click.side == LogicalSide::Server {
            assert_eq!(event.click.player, 12);
            assert_eq!(event.click.player_entity_id, 12);
        }
        assert_eq!(*event.cancellation_result, Answer::Pass);
        let mut state = self.0.lock().unwrap();
        state
            .steps
            .push(Step::Air(event.click.side, event.click.hand));
        if let Some(answer) = state.air_answer {
            *event.cancellation_result = answer;
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
#[mod_api::horizon_part]
impl BlockPart for Observer {
    fn use_item_on(&self, _: &mut dyn ServerHost, _: &mut UseItemOnEvent<'_>) -> Option<Answer> {
        let mut state = self.0.lock().unwrap();
        state.steps.push(Step::BlockPart);
        None
    }
}
#[mod_api::horizon_part]
impl ItemPart for Observer {
    fn on_item_use_first(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::OnItemUseFirstEvent<'_>,
    ) -> Option<Answer> {
        let mut state = self.0.lock().unwrap();
        if state.record_first {
            state.steps.push(Step::FirstPart);
        }
        state.first_part_answer
    }
    fn use_on(&self, _: &mut dyn ServerHost, _: &mut UseOnEvent<'_>) -> Flow {
        self.0.lock().unwrap().steps.push(Step::UseOnPart);
        Flow::Cancel
    }
    fn use_item(&self, _: &mut dyn ServerHost, _: &mut UseItemEvent<'_>) -> Flow {
        self.0.lock().unwrap().steps.push(Step::ItemPart);
        Flow::Cancel
    }
}

// The same real Game runs either its server path or its normal local-client
// connection/prediction path, as the existing remote menu fixture does.
fn with_interaction_game(client: bool, test: impl FnOnce(&mut Game<'_>)) {
    with_game(|game| {
        let _peer = if client {
            game.options.execution_role = ExecutionRole::Client;
            let (client, server) = Pipe::pair();
            game.connect_local(client, "InteractionTest").unwrap();
            Some(server)
        } else {
            game.options.dedicated = Some(Vec::new());
            None
        };
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 3.5)).unwrap());
        game.set_mode_id(1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        if client {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            test(game);
        } else {
            game.lan_host(
                LanSettings {
                    port: 0,
                    guest_commands: false,
                    force_game_mode: false,
                },
                NetConfig {
                    listen: false,
                    announce: false,
                    lod_distance: 0,
                    ..NetConfig::lan(0, String::new(), 2)
                },
                HubConfig {
                    default_game_mode: 1,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 2,
                    simulation_distance: 2,
                    dimension: game.dim.id().to_owned(),
                    allow_flight: true,
                    seed: 0,
                    max_players: 2,
                    integrated: true,
                    levels: game.level_shapes,
                },
            )
            .unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "interaction_guest".into(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [0.5, 64.0, 3.5],
                10,
            );
            guest.placed = true;
            guest.game_mode = 1;
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            game.as_guest(12, |game| {
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
                test(game);
            })
            .unwrap();
        }
    });
}

fn hit() -> Hit {
    Hit {
        pos: [0, 64, 0],
        face: [0, 1, 0],
        point: [0.5, 65.0, 0.5],
        inside: false,
    }
}

fn use_block(game: &mut Game<'_>, client: bool) -> bool {
    if client {
        game.remote_use_item_on(Hand::Main)
    } else {
        game.guest_use = Some(super::super::GuestUse::OnBlock(Hand::Main));
        game.use_hand(Some(&hit())).unwrap()
    }
}

#[test]
#[ignore = "requires prepared game resources"]
fn phase_events_replace_the_behaviour_before_its_part_on_both_sides() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "phase_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let lever = game
                    .with_props(
                        registry.default_state("minecraft:lever").unwrap(),
                        &[("face", "floor")],
                    )
                    .unwrap();
                game.world.set(0, 63, 0, stone).unwrap();
                game.world.set(0, 64, 0, lever).unwrap();
                game.forced_hit = Some(hit());
                *observer.0.lock().unwrap() = Controls::default();
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), lever);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
                    game.world.set(0, 64, 0, lever).unwrap();
                    *observer.0.lock().unwrap() = Controls {
                        block_answer: Some(answer),
                        ..Controls::default()
                    };
                    let swings = game.swings_started;
                    assert!(use_block(game, client));
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Phase(UsePhase::Block, side)]
                    );
                    let swings = game.swings_started - swings;
                    assert_eq!(
                        swings,
                        u32::from(
                            answer == Answer::Success
                                || (!client && answer == Answer::SuccessServer)
                        )
                    );
                }

                // TRY skips the item method/Part, but still delegates the
                // block's empty method on the main hand, on both sides.
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::TryWithEmptyHand),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), lever);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );

                for answer in [Answer::Pass, Answer::Fail] {
                    game.world.set(0, 64, 0, lever).unwrap();
                    *observer.0.lock().unwrap() = Controls {
                        block_answer: Some(answer),
                        item_on_answer: Some(Answer::Consume),
                        ..Controls::default()
                    };
                    assert!(use_block(game, client));
                    assert_eq!(game.world.block(0, 64, 0), lever);
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [
                            Step::Phase(UsePhase::Block, side),
                            Step::Phase(UsePhase::ItemAfterBlock, side),
                        ]
                    );
                }

                // A passing block gets the item phase, before the item Part.
                game.world.set(0, 64, 0, stone).unwrap();
                *observer.0.lock().unwrap() = Controls::default();
                let _ = use_block(game, client);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                expected.push(Step::Phase(UsePhase::ItemAfterBlock, side));
                if !client {
                    expected.push(Step::UseOnPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        item_on_answer: Some(answer),
                        ..Controls::default()
                    };
                    let swings = game.swings_started;
                    assert_eq!(
                        use_block(game, client),
                        answer.consumes_action() || answer == Answer::Fail
                    );
                    let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                    if !client {
                        expected.push(Step::BlockPart);
                    }
                    expected.push(Step::Phase(UsePhase::ItemAfterBlock, side));
                    assert_eq!(observer.0.lock().unwrap().steps, expected);
                    assert_eq!(
                        game.swings_started - swings,
                        u32::from(
                            answer == Answer::Success
                                || (!client && answer == Answer::SuccessServer)
                        )
                    );
                }

                // The phase event is later than RightClickBlock's choices.
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    use_item: TriState::False,
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert!(observer.0.lock().unwrap().steps.is_empty());

                // ItemStack.useOn passes before Item.useOn in adventure mode.
                game.set_mode_id(2);
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    ..Controls::default()
                };
                assert!(!use_block(game, client));
                assert!(observer.0.lock().unwrap().steps.is_empty());
                game.inventory[game.selected]
                    .as_mut()
                    .unwrap()
                    .data_mut()
                    .set_other(
                        "minecraft:can_place_on",
                        Some(Tag::Compound(
                            [("blocks".into(), Tag::String("minecraft:stone".into()))].into(),
                        )),
                    );
                *observer.0.lock().unwrap() = Controls {
                    use_block: TriState::False,
                    item_on_answer: Some(Answer::Success),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::ItemAfterBlock, side)]
                );
                game.set_mode_id(1);

                // In 26.3 dousing is CampfireBlock.useItemOn, not Item.useOn:
                // a canceled BLOCK cannot predict it or reach ITEM_AFTER_BLOCK.
                let campfire = registry.default_state("minecraft:campfire").unwrap();
                game.world.set(0, 64, 0, campfire).unwrap();
                game.inventory[game.selected] = Some(ItemStack::new("minecraft:iron_shovel", 1));
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::Success),
                    ..Controls::default()
                };
                assert!(use_block(game, client));
                assert_eq!(game.world.block(0, 64, 0), campfire);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );
                *observer.0.lock().unwrap() = Controls::default();
                assert!(use_block(game, client));
                assert_ne!(game.world.block(0, 64, 0), campfire);
                let mut expected = vec![Step::Phase(UsePhase::Block, side)];
                if !client {
                    expected.push(Step::BlockPart);
                }
                assert_eq!(observer.0.lock().unwrap().steps, expected);

                game.offhand = Some(ItemStack::new("minecraft:stick", 1));
                *observer.0.lock().unwrap() = Controls {
                    block_answer: Some(Answer::Success),
                    expected_hand: Some(Hand::Off),
                    ..Controls::default()
                };
                let used = if client {
                    game.remote_use_item_on(Hand::Off)
                } else {
                    game.guest_use = Some(super::super::GuestUse::OnBlock(Hand::Off));
                    game.with_hand(Hand::Off, |g| g.use_hand(Some(&hit())).unwrap())
                };
                assert!(used);
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Phase(UsePhase::Block, side)]
                );
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn canceled_item_results_keep_the_stack_wrappers_side_effects() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "wrapper_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        let cooling_stack = |item: &str| {
            let mut stack = ItemStack::new(item, 1);
            stack.data_mut().set_other(
                "minecraft:use_cooldown",
                Some(Tag::Compound(
                    [
                        ("seconds".into(), Tag::Float(1.0_f32.to_bits())),
                        (
                            "cooldown_group".into(),
                            Tag::String("test:item_events".into()),
                        ),
                    ]
                    .into(),
                )),
            );
            stack
        };
        for client in [false, true] {
            with_interaction_game(client, |game| {
                game.set_mode_id(0);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    game.cooldowns.remove("test:item_events");
                    game.inventory[game.selected] = Some(cooling_stack("minecraft:milk_bucket"));
                    *observer.0.lock().unwrap() = Controls {
                        use_block: TriState::False,
                        item_on_answer: Some(answer),
                        consume_item: answer.consumes_action(),
                        ..Controls::default()
                    };
                    let used = game.stats.get("used", "minecraft:milk_bucket");
                    let _ = use_block(game, client);
                    assert_eq!(
                        game.stats.get("used", "minecraft:milk_bucket"),
                        used + if !client && answer.consumes_action() {
                            1
                        } else {
                            0
                        }
                    );
                    assert_eq!(
                        game.cooldowns.is_on_cooldown("test:item_events"),
                        answer.consumes_action()
                    );
                    assert_eq!(
                        game.held().unwrap().item.as_ref(),
                        if !client && answer.consumes_action() {
                            "minecraft:bucket"
                        } else {
                            "minecraft:milk_bucket"
                        }
                    );
                }
                game.forced_hit = None;
                game.guest_use = None;
                for item in ["minecraft:stick", "minecraft:shield"] {
                    for answer in [Answer::Success, Answer::SuccessServer, Answer::Consume] {
                        game.cooldowns.remove("test:item_events");
                        game.inventory[game.selected] = Some(cooling_stack(item));
                        *observer.0.lock().unwrap() = Controls {
                            air_answer: Some(answer),
                            ..Controls::default()
                        };
                        let used = game.stats.get("used", item);
                        if client {
                            game.remote_use_item(Hand::Main);
                        } else {
                            game.use_hand(None).unwrap();
                        }
                        // ItemStack.use applies components only to instant use,
                        // and unlike useOn does not award the used statistic.
                        assert_eq!(game.stats.get("used", item), used);
                        assert_eq!(
                            game.cooldowns.is_on_cooldown("test:item_events"),
                            item == "minecraft:stick"
                        );
                        assert!(game.using.is_none());
                    }
                }
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn air_event_skips_the_item_and_non_success_client_answers_try_the_other_hand() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "air_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        for client in [false, true] {
            with_interaction_game(client, |game| {
                let side = if client {
                    LogicalSide::Client
                } else {
                    LogicalSide::Server
                };
                for answer in [
                    Answer::Success,
                    Answer::SuccessServer,
                    Answer::Consume,
                    Answer::Pass,
                    Answer::Fail,
                    Answer::TryWithEmptyHand,
                ] {
                    *observer.0.lock().unwrap() = Controls {
                        air_answer: Some(answer),
                        ..Controls::default()
                    };
                    let used = if client {
                        game.remote_use_item(Hand::Main)
                    } else {
                        game.use_hand(None).unwrap()
                    };
                    assert_eq!(
                        used,
                        if client {
                            answer == Answer::Success
                        } else {
                            answer.consumes_action()
                        }
                    );
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Air(side, Hand::Main)]
                    );
                }
                if client {
                    *observer.0.lock().unwrap() = Controls {
                        air_answer: Some(Answer::Consume),
                        ..Controls::default()
                    };
                    game.offhand = Some(ItemStack::new("minecraft:shield", 1));
                    game.remote_use_hands(mod_api::InteractionAttempt::Press);
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Air(side, Hand::Main), Step::Air(side, Hand::Off)]
                    );
                    assert!(game.using.is_none());
                } else {
                    *observer.0.lock().unwrap() = Controls::default();
                    assert!(game.use_hand(None).unwrap());
                    assert_eq!(
                        observer.0.lock().unwrap().steps,
                        [Step::Air(side, Hand::Main), Step::ItemPart]
                    );
                }
                observer.0.lock().unwrap().steps.clear();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                // Even a passing block remains a block target for Item.use.
                assert!(
                    game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1), true)
                        .is_none()
                );
                assert!(observer.0.lock().unwrap().steps.is_empty());
                game.forced_hit = None;
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                game.mobs.push(horizon_client::mobs::Mob::new(
                    42,
                    kind,
                    Vec3::new(0.5, 64.0, 5.5),
                    0.0,
                ));
                assert!(game.entity_targeted());
                assert!(
                    game.right_click_item(Hand::Main, &ItemStack::new("minecraft:stick", 1), false)
                        .is_none()
                );
                assert!(observer.0.lock().unwrap().steps.is_empty());
            });
        }
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn on_item_use_first_runs_before_the_block_and_ends_the_click() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "first_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let lever = game
                .with_props(
                    registry.default_state("minecraft:lever").unwrap(),
                    &[("face", "floor")],
                )
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.forced_hit = Some(hit());
            let server = LogicalSide::Server;
            let first = Step::Phase(UsePhase::ItemBeforeBlock, server);

            // A Part's answer ends the click before the block is asked.
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_eq!(game.world.block(0, 64, 0), lever);
            assert_eq!(observer.0.lock().unwrap().steps, [first, Step::FirstPart]);

            // The broadcast event goes first: its cancel stops the Part.
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_answer: Some(Answer::Success),
                first_part_answer: Some(Answer::Success),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_eq!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Phase(UsePhase::ItemBeforeBlock, server)]
            );

            // A pass goes on: the block's phase and the lever flip.
            *observer.0.lock().unwrap() = Controls {
                record_first: true,
                first_part_answer: Some(Answer::Pass),
                ..Controls::default()
            };
            assert!(use_block(game, false));
            assert_ne!(game.world.block(0, 64, 0), lever);
            assert_eq!(
                observer.0.lock().unwrap().steps[..3],
                [
                    Step::Phase(UsePhase::ItemBeforeBlock, server),
                    Step::FirstPart,
                    Step::Phase(UsePhase::Block, server),
                ]
            );
        });
    });
}

#[test]
#[ignore = "requires prepared game resources"]
fn an_item_used_from_the_off_hand_reaches_the_use_hooks_on_the_server() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "off_hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            let side = LogicalSide::Server;
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));

            // An empty main hand passes, and the off hand uses its item.
            game.inventory[game.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A main hand item nothing answers for passes too: both hands' hooks, in order.
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stone", 1));
            *observer.0.lock().unwrap() = Controls::default();
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [
                    Step::Air(side, Hand::Main),
                    Step::Air(side, Hand::Off),
                    Step::ItemPart
                ]
            );

            // A guest's use packet names the off hand alone.
            game.inventory[game.selected] = None;
            *observer.0.lock().unwrap() = Controls::default();
            game.guest_use = Some(super::super::GuestUse::Item(Hand::Off));
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
            assert_eq!(
                observer.0.lock().unwrap().steps,
                [Step::Air(side, Hand::Off), Step::ItemPart]
            );

            // A block click: the off hand's `use_item_on_block` phases and its item Part's `use_on`,
            // for the local player and for a guest's use-on-block packet.
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, stone).unwrap();
            game.forced_hit = Some(hit());
            for guest in [None, Some(super::super::GuestUse::OnBlock(Hand::Off))] {
                game.inventory[game.selected] = None;
                *observer.0.lock().unwrap() = Controls {
                    hands: Some(Vec::new()),
                    ..Controls::default()
                };
                game.guest_use = guest;
                assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Off));
                let state = observer.0.lock().unwrap();
                assert!(
                    state
                        .hands
                        .as_ref()
                        .unwrap()
                        .iter()
                        .all(|h| *h == Hand::Off || guest.is_none())
                );
                assert_eq!(state.hands.as_ref().unwrap().last(), Some(&Hand::Off));
                assert_eq!(state.steps.last(), Some(&Step::UseOnPart));
            }
        });
    });
}
