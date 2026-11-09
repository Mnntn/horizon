use super::*;
use horizon_core::nbt::Tag;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::Mutex;

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn lifecycle_carts_keep_registrations_through_commands_unload_park_and_remote() {
    use crate::game::entity::taken_pass_tests::{
        LifecycleFixture, RemoteLifecycle, SlotIdentities,
    };
    use horizon_protocol::{
        connection::Pipe,
        entity_data::{Value, serializer},
    };
    let mut identities = SlotIdentities::default();
    LifecycleFixture {
        seed: |game, entries| {
            game.carts.clear();
            for &(id, at) in entries {
                game.carts.push(Minecart::new(
                    id,
                    CartKind::Rideable,
                    Vec3::new(at[0], at[1], at[2]),
                ));
            }
        },
        ids: |game| game.carts.iter().map(|cart| cart.id).collect(),
        target: crate::game::commands::Target::Cart,
        unloads_with_chunk: true,
        remote: Some(RemoteLifecycle {
            prepare: |game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "Lifecycle").unwrap();
            },
            apply: |game, remove, update| {
                game.remote_object_data(update as i32, &[(12, serializer::INT, Value::Int(19))]);
                assert_eq!(
                    game.carts
                        .iter()
                        .find(|cart| cart.id == update)
                        .unwrap()
                        .custom_offset,
                    Some(19)
                );
                game.remove_remote_object(remove);
            },
        }),
    }
    .run_with(|game| {
        identities.observe(
            &mut game.carts,
            game.native_lifecycle.epoch().unwrap(),
            |cart| cart.id,
        )
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn connecting_to_a_remote_level_clears_the_existing_cart_list() {
    use horizon_protocol::connection::Pipe;
    super::super::native_entity_refs::tests::with_game(|game| {
        game.carts.push(Minecart::new(
            71,
            CartKind::Rideable,
            Vec3::new(0.5, 70.0, 0.5),
        ));
        assert_eq!(game.carts.len(), 1);
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "Cart reset").unwrap();
        // Observe the production reset before any fixture reseeding or cleanup.
        assert!(game.carts.is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn exploding_a_cart_removes_only_its_owner_and_keeps_a_bound_survivor() {
    bound_cart_case(CartKind::Rideable, |game| {
        let mut doomed = Minecart::new(72, CartKind::Tnt, Vec3::new(40.5, 70.0, 0.5));
        game.carts.push(doomed.clone());
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(72), 99).unwrap();
        game.carts.update(1, |cart| {
            cart.attachment_lifetime().bind(&mut values, &index, 72u32);
        });
        let key = ("test".into(), "cart_lifetime".into());
        let retained = index.lock().unwrap().snapshot(&key);
        assert_eq!(retained.len(), 1);
        game.explode_cart(&mut doomed, 0.0);
        assert_eq!(
            game.carts.iter().map(|cart| cart.id).collect::<Vec<_>>(),
            [71]
        );
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut values, Holder::Entity(72), 100).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(retained[0].entry.read(&VALUE), Ok(&99));
        // bound_cart_case also checks the survivor's registration and value.
    });
}

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "cart_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_cart_commits_the_normal_server_tick() {
    bound_cart_case(CartKind::Furnace, |game| {
        game.carts.update(0, |cart| cart.fuel = 6);
        game.tick_carts();
        assert_eq!(game.carts[0].fuel, 5);
        assert_eq!(game.carts[0].tick_count, 1);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_cart_commits_the_ridden_tick() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.carts.update(0, |cart| cart.in_water = true);
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                Vec3::new(0.5, 70.0 + SEAT, 0.5),
            )
            .unwrap(),
        );
        game.riding = Some(71);
        assert!(game.tick_cart_riding(Keys::default()));
        assert!(!game.carts[0].in_water);
        assert_eq!(game.carts[0].tick_count, 1);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_ticked_cart_publishes_its_moved_position_before_pushing() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.carts.update(0, |cart| cart.velocity.x = 0.4);
        game.carts.push(Minecart::new(
            72,
            CartKind::Rideable,
            Vec3::new(1.75, 70.0, 0.5),
        ));
        // Keep the other cart's own tick from masking the first cart's push.
        game.riding = Some(72);
        game.tick_carts();
        assert!(game.carts[0].position.x > 0.5);
        assert!(game.carts[1].velocity.x > 0.0);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_cart_commits_the_tnt_interaction() {
    bound_cart_case(CartKind::Tnt, |game| {
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:fire_charge", 1));
        assert!(game.interact_cart(Some(InteractTarget {
            id: 71,
            location: [0.5, 70.0, 0.5],
        })));
        assert_eq!(game.carts[0].fuse, 80);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_cart_commits_valid_nbt_and_refuses_an_invalid_load() {
    bound_cart_case(CartKind::Furnace, |game| {
        let target = super::super::commands::Target::Cart(71);
        game.carts.update(0, |cart| cart.fuel = 6);
        let uuid = game.uuid_for(71);
        assert!(
            game.apply_entity_nbt(target, Tag::Compound(Default::default()))
                .is_err()
        );
        assert_eq!(game.carts[0].fuel, 6);
        let (tag, _) = horizon_core::nbt::parse_snbt(
            "{Pos:[0.5d,70.0d,0.5d],Fuel:13s,UUID:[I;9,8,7,6],DisplayState:{Name:\"test:missing\"}}",
        ).unwrap();
        game.apply_entity_nbt(target, tag).unwrap();
        assert_eq!(game.carts[0].fuel, 13);
        assert_eq!(game.uuid_for(71), uuid);
        assert!(game.carts[0].custom_display.is_none());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_cart_survives_the_parked_level_swap() {
    bound_cart_case(CartKind::Rideable, |game| {
        game.enter_level(super::super::Dim::Nether).unwrap();
        assert!(game.carts.is_empty());
        assert_eq!(
            game.levels
                .parked_mut(super::super::Dim::Overworld)
                .unwrap()
                .carts[0]
                .id,
            71
        );
        game.enter_level(super::super::Dim::Overworld).unwrap();
        assert_eq!(game.carts[0].id, 71);
    });
}

fn bound_cart_case(kind: CartKind, action: impl FnOnce(&mut Game<'_>)) {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.tickets.update(None);
        game.carts
            .push(Minecart::new(71, kind, Vec3::new(0.5, 70.0, 0.5)));
        let holder = {
            let host: &dyn mod_api::ServerHost = game;
            host.entity(71).unwrap()
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.carts.update(0, |cart| {
            cart.attachment_lifetime().bind(&mut values, &index, holder);
            cart.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "cart_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        action(game);
        let current = game
            .carts
            .update(0, |cart| cart.attachment_lifetime().registration().unwrap());
        assert!(original.same_registration(&current));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(VALUE.get(&mut values, Holder::Entity(71)), Ok(10));
        let taken = game.take_entities();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.next_entity = game.next_entity.max(72);
        game.summon_with_nbt("minecraft:minecart", [8.5, 70.0, 0.5], "{}")
            .unwrap();
        let spawned = game.carts[0].id;
        assert_ne!(spawned, 71);
        let spawned_holder = game.entity(spawned).unwrap();
        let mut spawned_values = Attachments::default();
        VALUE
            .set(&mut spawned_values, Holder::Entity(spawned), 40)
            .unwrap();
        let spawned_registration = game.carts.update(0, |cart| {
            cart.attachment_lifetime()
                .bind(&mut spawned_values, &index, spawned_holder);
            cart.attachment_lifetime().registration().unwrap()
        });
        game.restore_entities(taken);
        assert_eq!(game.carts.last().unwrap().id, spawned);
        for (i, expected) in [(0, original), (game.carts.len() - 1, spawned_registration)] {
            let actual = game
                .carts
                .update(i, |cart| cart.attachment_lifetime().registration().unwrap());
            assert!(expected.same_registration(&actual));
        }
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&20)
        );
        assert_eq!(
            VALUE.get(&mut spawned_values, Holder::Entity(spawned)),
            Ok(40)
        );
        let live = game.entity(71).unwrap();
        assert!(game.entity_remove(live, mod_api::RemovalReason::Discarded));
        let clone = game.carts[0].clone();
        assert!(!game.store_cart_back(0, clone));
        game.native_removals_flush();
        assert!(!game.carts.iter().any(|cart| cart.id == 71));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        VALUE.set(&mut values, Holder::Entity(71), 30).unwrap();
        let surviving = index.lock().unwrap().snapshot(&key);
        assert_eq!(surviving.len(), 1);
        assert_eq!(surviving[0].holder, spawned_holder);
        let i = game
            .carts
            .iter()
            .position(|cart| cart.id == spawned)
            .unwrap();
        drop(game.carts.remove(i));
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&20)
        );
        assert_eq!(
            after
                .iter()
                .find(|row| row.holder == spawned_holder)
                .unwrap()
                .entry
                .read(&VALUE),
            Ok(&40)
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_bound_tnt_cart_keeps_its_join_when_damage_primes_it() {
    bound_cart_case(CartKind::Tnt, |game| {
        game.carts.push(Minecart::new(
            72,
            CartKind::Rideable,
            Vec3::new(4.5, 70.0, 0.5),
        ));
        let expected: Vec<_> = game.carts.iter().map(|c| (c.id, c.join)).collect();
        let join = game.carts[0].join;
        game.hurt_cart(0, 5.0, false, true);
        assert_eq!(
            game.carts
                .iter()
                .map(|c| (c.id, c.join))
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(game.carts[0].join, join);
        assert_eq!(game.carts.locate(join), Some(0));
        assert!((0..=38).contains(&game.carts[0].fuse));
    });
}
