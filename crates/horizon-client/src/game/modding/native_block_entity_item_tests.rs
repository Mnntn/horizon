use super::api_values::{StackReadError, TryIntoApi};
use super::native_live_holder_tests::{mod_holder, with_mod_parts};
use horizon_client::containers::{BlockEntity, ModBlockEntity, ModContainer};
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;
use mod_api::{
    BlockEntityItemError as Error, Host, HostTransaction, ITEMS, ItemResource, ServerHost,
    Transaction,
};
use std::sync::Arc;

fn commit(host: &mut dyn ServerHost, test: impl FnOnce(&mut dyn ServerHost, &Transaction)) {
    host.transaction(|host, tx| {
        test(host, tx);
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; runs in an isolated scratch process"]
fn internal_slots_preserve_automation_limits_and_nested_transactions() {
    const NAME: &str = "game::native_block_entity_item_tests::internal_slots_preserve_automation_limits_and_nested_transactions";
    with_mod_parts(NAME, |root| {
        super::native_entity_refs::tests::with_game_at(root, |game| {
            let entity = mod_holder(game);
            let get = |host: &dyn Host, slot| entity.item_slot(host, slot);
            let set = |host: &mut dyn ServerHost, tx: &Transaction, slot, stack| {
                entity.set_item_slot(host, tx, slot, stack)
            };
            let pos = (1, 64, 1);
            let stone = game.world.block(1, 64, 1);
            let granite = game
                .pack
                .assets
                .registry
                .default_state("minecraft:granite")
                .unwrap();
            let dirt = ItemStack::new("minecraft:dirt", 2).try_into_api().unwrap();
            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(get(game, 0), Err(Error::Unavailable));
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 0, Some(dirt.clone())),
                    Err(Error::Unavailable)
                );
            });
            let machine_slots = ModContainer {
                slots: 2,
                inputs: vec![0],
                outputs: vec![1],
                ..Default::default()
            };
            let replacement_slots = ModContainer {
                slots: 1,
                ..Default::default()
            };
            game.world.block_entities.set_mod_containers(
                [
                    (stone.0, Arc::new(machine_slots)),
                    (granite.0, Arc::new(replacement_slots)),
                ]
                .into_iter()
                .collect(),
            );
            assert_eq!(get(game, 0), Ok(None));
            assert_eq!(
                get(game, 2),
                Err(Error::SlotOutOfBounds { slot: 2, slots: 2 })
            );
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 1, None), Ok(false));
                assert_eq!(set(host, tx, 0, Some(dirt.clone())), Ok(true));
                let level = entity.level(host).unwrap();
                let external = level.capability(host, &ITEMS, [1, 64, 1], None).unwrap();
                assert_eq!(
                    external.insert(host, tx, 1, &ItemResource::of(&diamond), 1),
                    0
                );
            });
            let BlockEntity::Mod(holder) = game.world.block_entities.get_mut(&pos).unwrap() else {
                panic!()
            };
            holder
                .items
                .push(Some(ItemStack::new("minecraft:gold_ingot", 3)));
            game.world.modified.clear();
            let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, tx| {
                assert_eq!(
                    set(
                        host,
                        tx,
                        0,
                        Some(ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap())
                    ),
                    Ok(true)
                );
                host.transaction(|host, child| {
                    assert_eq!(set(host, child, 1, Some(diamond.clone())), Ok(true));
                    Ok(())
                })?;
                assert_eq!(get(host, 1), Ok(Some(diamond.clone())));
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert_eq!(get(game, 0), Ok(Some(dirt.clone())));
            assert_eq!(get(game, 1), Ok(None));
            commit(game, |host, tx| {
                assert_eq!(
                    set(
                        host,
                        tx,
                        0,
                        Some(ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap())
                    ),
                    Ok(true)
                );
                assert_eq!(set(host, tx, 1, Some(diamond.clone())), Ok(true));
            });
            assert_eq!(
                get(game, 0),
                Ok(Some(
                    ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap()
                ))
            );
            assert_eq!(get(game, 1), Ok(Some(diamond.clone())));
            assert!(!game.world.modified.is_empty());
            let BlockEntity::Mod(holder) = game.world.block_entities.get(&pos).unwrap() else {
                panic!()
            };
            assert_eq!(
                holder.items[2],
                Some(ItemStack::new("minecraft:gold_ingot", 3)),
                "saved excess is not truncated"
            );
            let before_revision = holder.revision();
            assert_eq!(
                ItemStack::new("missing:item", 1).try_into_api(),
                Err(StackReadError::UnknownItem)
            );
            assert_eq!(
                ItemStack::new("minecraft:dirt", 0).try_into_api(),
                Err(StackReadError::Empty)
            );
            game.world.modified.clear();
            let mut component = ItemStack::new("minecraft:diamond", 5);
            component
                .data_mut()
                .set_other("minecraft:max_stack_size", Some(Tag::Int(4)));
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 1, Some(diamond.clone())), Ok(false));
                assert_eq!(
                    set(host, tx, 2, None),
                    Err(Error::SlotOutOfBounds { slot: 2, slots: 2 })
                );
                for (stack, error) in [
                    (
                        ItemStack::new("minecraft:diamond", 65),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 64,
                        }),
                    ),
                    (
                        ItemStack::new("minecraft:diamond_sword", 2),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 1,
                        }),
                    ),
                    (
                        component.clone(),
                        Error::StackRejected(mod_api::StackWriteError::CountExceeded {
                            maximum: 4,
                        }),
                    ),
                    (ItemStack::new("minecraft:air", 1), Error::InvalidStack),
                ] {
                    let stack = stack.try_into_api().unwrap();
                    assert_eq!(set(host, tx, 1, Some(stack)), Err(error));
                }
            });
            let BlockEntity::Mod(holder) = game.world.block_entities.get(&pos).unwrap() else {
                panic!()
            };
            assert_eq!(holder.revision(), before_revision);
            assert_eq!(get(game, 1), Ok(Some(diamond)));
            assert!(game.world.modified.is_empty());
            component.count = 4;
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 1, Some(component.try_into_api().unwrap())),
                    Ok(true)
                );
                assert_eq!(get(host, 1), Ok(Some(component.try_into_api().unwrap())));
                assert_eq!(set(host, tx, 1, None), Ok(true));
            });
            assert_eq!(get(game, 1), Ok(None));
            let level = entity.level(game).unwrap();
            game.world.block_entities.remove(&pos);
            assert_eq!(get(game, 0), Err(Error::Stale));
            commit(game, |host, tx| {
                assert_eq!(set(host, tx, 0, None), Err(Error::Stale));
            });
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(vec![None; 27]));
            commit(game, |host, tx| {
                assert_eq!(get(host, 0), Err(Error::Stale));
                assert_eq!(set(host, tx, 0, None), Err(Error::Stale));
            });
            game.world.set(1, 64, 1, granite).unwrap();
            let replacement = ModBlockEntity::new("live_bench:ticker".into(), Default::default());
            game.world
                .block_entities
                .insert(pos, BlockEntity::Mod(Box::new(replacement)));
            assert_eq!(get(game, 0), Err(Error::Stale));
            let replacement = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(entity, replacement);
            let get = |host: &dyn Host, slot| replacement.item_slot(host, slot);
            let set = |host: &mut dyn ServerHost, tx: &Transaction, slot, stack| {
                replacement.set_item_slot(host, tx, slot, stack)
            };
            commit(game, |host, tx| {
                assert_eq!(
                    set(host, tx, 1, None),
                    Err(Error::SlotOutOfBounds { slot: 1, slots: 1 })
                );
                assert_eq!(set(host, tx, 0, Some(dirt.clone())), Ok(true));
            });
            assert_eq!(get(game, 0), Ok(Some(dirt)));
            game.native_lifecycle
                .begin_open(mod_api::Authority::IntegratedServer);
            assert_eq!(get(game, 0), Err(Error::Stale));
            assert!(game.transactions.is_empty());
        });
    });
}
