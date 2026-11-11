use super::*;

// Stage 2 acceptance row: cursor stone 3, empty slot, ordinary placement.
// Vanilla 26.3 AbstractContainerMenu.doClick (PICKUP) / Slot.safeInsert.
#[test]
fn dfa_baseline_cursor_placement() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        menu.carried = stack("stone", 3);
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, None);
        assert_eq!(menu.slot(0), stack("stone", 3).as_ref());
    });
}

// Stage 2 acceptance row: quick move stone 10 into room 4 and 6.
// Vanilla 26.3 AbstractContainerMenu.moveItemStackTo fills matching stacks first.
#[test]
fn dfa_baseline_quick_move() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        for slot in 0..27 {
            menu.set_slot(env, slot, stack("dirt", 64));
        }
        menu.set_slot(env, 0, stack("stone", 60));
        menu.set_slot(env, 1, stack("stone", 58));
        menu.set_slot(env, 27, stack("stone", 10));
        menu.click(env, 27, 0, Input::QuickMove).unwrap();
        assert_eq!(menu.slot(27), None);
        assert_eq!(menu.slot(0), stack("stone", 64).as_ref());
        assert_eq!(menu.slot(1), stack("stone", 64).as_ref());
        assert_eq!(menu.carried, None);
        for slot in 2..27 {
            assert_eq!(menu.slot(slot), stack("dirt", 64).as_ref());
        }
    });
}

// Stage 2 acceptance row: recipe needs two stone, source stone 2, output empty.
// Vanilla 26.3 ServerPlaceRecipe.placeRecipe stages one set; ResultSlot.onTake
// consumes one item per occupied grid cell, once, when the result is taken.
#[test]
#[ignore = "requires prepared resources"]
fn dfa_baseline_recipe_placement() {
    let mut ran = false;
    with_menu(Kind::Crafting, |env, menu| {
        ran = true;
        menu.creative = false;
        menu.set_player_item(0, stack("stone", 2));
        assert_eq!(menu.slot(0), None);
        menu.place_recipe(env, "minecraft:stone_pressure_plate", false, false, None);
        assert_eq!(menu.player_item(0), None);
        assert_eq!(menu.slot(0), stack("stone_pressure_plate", 1).as_ref());
        assert_eq!(
            (1..10)
                .filter_map(|s| menu.slot(s).cloned())
                .collect::<Vec<_>>(),
            vec![stack("stone", 1).unwrap(), stack("stone", 1).unwrap()]
        );
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, stack("stone_pressure_plate", 1));
        assert!((0..10).all(|s| menu.slot(s).is_none()));
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, stack("stone_pressure_plate", 1));
        assert_eq!(menu.player_item(0), None);
    });
    assert!(
        ran,
        "requires prepared resources in .local/horizon-client/26.3"
    );
}
