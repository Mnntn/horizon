use super::*;
use std::cell::{Cell, RefCell};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static CAPTURE: Cell<bool> = const { Cell::new(false) };
    static SEEN: RefCell<Vec<(String, Option<ItemStack>)>> = const { RefCell::new(Vec::new()) };
}
pub(super) fn record(name: &str, stack: Option<&ItemStack>) {
    if CAPTURE.get() {
        SEEN.with_borrow_mut(|seen| seen.push((name.to_owned(), stack.cloned())));
    }
}
fn counts() -> [usize; 3] {
    SEEN.with_borrow(|seen| {
        [
            "minecraft:default_block_use",
            "minecraft:item_used_on_block",
            "minecraft:any_block_use",
        ]
        .map(|name| seen.iter().filter(|(n, _)| n == name).count())
    })
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn consuming_cancellation_has_one_outer_on_local_and_packet_routes() {
    CAPTURE.set(true);
    struct Recording;
    impl Drop for Recording {
        fn drop(&mut self) {
            CAPTURE.set(false);
            SEEN.with_borrow_mut(Vec::clear);
        }
    }
    let _recording = Recording;
    struct Listener(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn right_click_block(
            &self,
            host: mod_api::HostSide,
            event: &mut mod_api::RightClickBlockEvent<'_>,
        ) -> mod_api::Flow {
            let mode = self.0.load(Ordering::SeqCst);
            if mode == 0 {
                return mod_api::Flow::Continue;
            }
            if let mod_api::HostSide::Server(host) = host {
                if mode == 1 {
                    let player = host.player(event.click.player).unwrap();
                    player.trigger_block_use(host, event.click.pos).unwrap();
                } else if mode == 4 {
                    let player = host.player(event.click.player).unwrap();
                    player
                        .trigger_item_used_on_block(
                            host,
                            event.click.pos,
                            event.click.stack.unwrap(),
                        )
                        .unwrap();
                }
            }
            *event.cancellation_result = match mode {
                2 => mod_api::InteractionResult::Fail,
                5 => mod_api::InteractionResult::Pass,
                6 => mod_api::InteractionResult::Success,
                7 => mod_api::InteractionResult::SuccessServer,
                _ => mod_api::InteractionResult::Consume,
            };
            mod_api::Flow::Cancel
        }
    }
    let mode = Arc::new(AtomicUsize::new(0));
    mod_api::__private::install(Arc::from([mod_api::ModEntry::new(
        "count-block-use",
        Box::new(Listener(mode.clone())),
    )]));
    crate::game::native_guest_turn_tests::with_guests(|g| {
        g.build_registries();
        g.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let state = g
            .pack
            .assets
            .registry
            .default_state("minecraft:oak_trapdoor")
            .unwrap();
        let hit = Hit {
            pos: [3, 64, 4],
            face: [0, 1, 0],
            point: [3.5, 65.0, 4.5],
            inside: false,
        };
        for packet in [false, true] {
            for (answer, expected) in [
                (0, [1, 0, 1]),
                (1, [1, 0, 1]),
                (2, [0, 0, 0]),
                (3, [0, 0, 1]),
                (4, [0, 1, 1]),
                (5, [0, 0, 0]),
                (6, [0, 0, 1]),
                (7, [0, 0, 1]),
            ] {
                mode.store(answer, Ordering::SeqCst);
                g.world.set(3, 64, 4, state).unwrap();
                g.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[0] =
                            (answer == 4).then(|| ItemStack::new("minecraft:stone", 2));
                        guest.offhand = None;
                    })
                    .unwrap();
                SEEN.with_borrow_mut(Vec::clear);
                if packet {
                    let gi = g
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .position(|guest| guest.conn == 12)
                        .unwrap();
                    assert!(g.guest_use_item_on(12, gi, hit, Hand::Main).is_ok());
                } else {
                    g.as_guest(12, |g| {
                        g.forced_hit = Some(hit);
                        g.use_hand(Some(&hit)).unwrap();
                        g.forced_hit = None;
                    })
                    .unwrap();
                }
                assert_eq!(counts(), expected, "packet={packet}, mode={answer}");
            }
        }
        // Item-internal triggers and the game-mode item trigger both run.
        mode.store(0, Ordering::SeqCst);
        for (block, item) in [
            ("minecraft:copper_block", "minecraft:honeycomb"),
            ("minecraft:cave_vines", "minecraft:shears"),
        ] {
            let initial = g.pack.assets.registry.default_state(block).unwrap();
            for packet in [false, true] {
                g.world.set(3, 64, 4, initial).unwrap();
                g.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.inventory[0] = Some(ItemStack::new(item, 1));
                    })
                    .unwrap();
                SEEN.with_borrow_mut(Vec::clear);
                if packet {
                    let gi = g
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .position(|guest| guest.conn == 12)
                        .unwrap();
                    assert!(g.guest_use_item_on(12, gi, hit, Hand::Main).is_ok());
                } else {
                    g.as_guest(12, |g| {
                        g.forced_hit = Some(hit);
                        g.use_hand(Some(&hit)).unwrap();
                        g.forced_hit = None;
                    })
                    .unwrap();
                }
                assert_ne!(
                    g.world.block(3, 64, 4),
                    initial,
                    "packet={packet}, item={item}"
                );
                assert_eq!(counts(), [0, 2, 1], "packet={packet}, item={item}");
            }
        }
        // Filling a cauldron replaces the water bucket: the accepted H divergence.
        mode.store(0, Ordering::SeqCst);
        let cauldron = g
            .pack
            .assets
            .registry
            .default_state("minecraft:cauldron")
            .unwrap();
        for packet in [false, true] {
            g.world.set(3, 64, 4, cauldron).unwrap();
            g.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                })
                .unwrap();
            SEEN.with_borrow_mut(Vec::clear);
            if packet {
                let gi = g
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .position(|guest| guest.conn == 12)
                    .unwrap();
                assert!(g.guest_use_item_on(12, gi, hit, Hand::Main).is_ok());
            } else {
                g.as_guest(12, |g| {
                    g.forced_hit = Some(hit);
                    g.use_hand(Some(&hit)).unwrap();
                    g.forced_hit = None;
                })
                .unwrap();
            }
            assert_eq!(counts()[2], 1);
            SEEN.with_borrow(|seen| {
                let stack = &seen
                    .iter()
                    .find(|(name, _)| name == "minecraft:any_block_use")
                    .unwrap()
                    .1;
                assert_eq!(&*stack.as_ref().unwrap().item, "minecraft:bucket");
            });
        }
    });
}
