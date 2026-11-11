use super::*;

#[test]
#[ignore = "requires prepared game resources"]
fn identical_instant_stacks_report_the_selected_interaction_hand() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "hand_observer",
        Box::new(observer.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_api::testing::with_mods(mods, || {
        with_interaction_game(false, |game| {
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            for hand in [Hand::Main, Hand::Off] {
                *observer.0.lock().unwrap() = Controls {
                    expected_hand: Some(hand),
                    ..Controls::default()
                };
                game.swing = Default::default();
                assert!(game.using.is_none());
                game.guest_use = Some(super::super::super::GuestUse::Item(hand));
                assert_eq!(game.use_item_as_player().unwrap(), Some(hand));
                assert_eq!(game.swing.attack_hand(), hand);
                assert!(game.using.is_none());
                assert_eq!(
                    observer.0.lock().unwrap().steps,
                    [Step::Air(LogicalSide::Server, hand), Step::ItemPart]
                );
            }
            // The integrated server's local action also enters the same scoped hand context.
            *observer.0.lock().unwrap() = Controls {
                expected_hand: Some(Hand::Main),
                ..Controls::default()
            };
            game.guest_use = None;
            assert_eq!(game.use_item_as_player().unwrap(), Some(Hand::Main));
        });
    });
}
