use crate::game::native_entity_refs::tests::with_game;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "needs prepared resources"]
fn relative_item_teleports_keep_only_relative_horizontal_motion() {
    with_game(|game| {
        game.ui.registries.permission = 4;
        game.spawn_item_own(
            horizon_client::items::ItemStack::new("minecraft:stone", 1),
            [0.0, 64.0, 0.0],
            10,
        );
        game.dropped[0].velocity = Vec3::new(1.0, -2.0, 3.0);
        assert!(game.run_command("tp @e[type=minecraft:item] ~ 70 12"));
        assert_eq!(game.dropped[0].velocity, Vec3::new(1.0, 0.0, 0.0));
        game.dropped[0].velocity = Vec3::new(1.0, -2.0, 3.0);
        assert!(game.run_command("tp @e[type=minecraft:item] 12 ~ ~"));
        assert_eq!(game.dropped[0].velocity, Vec3::new(0.0, 0.0, 3.0));
        assert!(game.run_command("tp @e[type=minecraft:item] 10 70 10"));
        assert_eq!(game.dropped[0].velocity, Vec3::ZERO);
    });
}
