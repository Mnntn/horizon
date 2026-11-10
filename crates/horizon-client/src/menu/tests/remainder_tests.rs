use super::*;

#[test]
fn crafting_remainder_consumption_and_quick_move() {
    use crate::crafting::{CraftingRemainder, Ingredient, Recipe, Template};
    struct Tool;
    impl CraftingRemainder for Tool {
        fn crafting_remainder(&self, s: &ItemStack) -> Option<ItemStack> {
            (s.damage < 4).then(|| ItemStack {
                count: 1,
                damage: s.damage + 1,
                ..s.clone()
            })
        }
    }
    bare_env(|base| {
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "tool".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["test:tool".into()])],
                result: Template {
                    item: Some("test:result".into()),
                    count: 1,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let env = Env {
            recipes: &recipes,
            crafting_remainder: Some(&Tool),
            ..*base
        };
        for (kind, cell) in [(Kind::Crafting, 4), (Kind::Inventory, 3)] {
            let mut menu = Menu::new(kind);
            menu.set_slot(
                &env,
                cell + 1,
                Some(ItemStack {
                    damage: 2,
                    ..ItemStack::new("test:tool", 1)
                }),
            );
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(cell + 1).map(|s| (s.damage, s.count)),
                Some((3, 1))
            );
            menu.carried = None;
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(cell + 1).map(|s| (s.damage, s.count)),
                Some((4, 1))
            );
            menu.carried = None;
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert!(menu.slot(cell + 1).is_none());
            let mut menu = Menu::new(kind);
            menu.set_slot(
                &env,
                cell + 1,
                Some(ItemStack {
                    damage: 2,
                    ..ItemStack::new("test:tool", 1)
                }),
            );
            menu.click(&env, 0, 0, Input::QuickMove).unwrap();
            assert!(menu.slot(cell + 1).is_none());
            assert_eq!(
                menu.containers[PLAYER]
                    .iter()
                    .flatten()
                    .filter(|s| &*s.item == "test:result")
                    .map(|s| s.count)
                    .sum::<u32>(),
                3
            );
        }
    });
}
