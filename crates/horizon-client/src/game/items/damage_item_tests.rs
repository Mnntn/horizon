#![cfg(test)]
//! `ItemPart::damage_item` on the wear of a player's held tool, and the part the tests of the other wear
//! paths (the dispenser, the shield, the mace) share. Each installs process-wide parts: run alone with the
//! prepared resources.
use super::*;
use mod_api::{DamageItemEvent, Flow, Hooks, ItemPart, ModEntry, Registrar, RegistrationContext};
use std::sync::{Arc, Mutex};

/// What the part of an item does to the wear it is asked about.
#[derive(Clone, Copy)]
pub(in crate::game) enum Rule {
    /// Half the points, rounded down.
    Halve,
    /// This many times the points.
    Times(i32),
    /// Replace the wear, including boundary values.
    Set(i32),
    /// No wear at all.
    Cancel,
}

/// The part of one item: it applies its `rule` and records the points it was asked about.
#[derive(Clone)]
pub(in crate::game) struct Wear {
    item: &'static str,
    pub(in crate::game) rule: Arc<Mutex<Rule>>,
    pub(in crate::game) asked: Arc<Mutex<Vec<i32>>>,
}

#[mod_api::horizon_part]
impl ItemPart for Wear {
    fn damage_item(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut DamageItemEvent<'_>,
    ) -> Flow {
        self.asked.lock().unwrap().push(event.amount);
        match *self.rule.lock().unwrap() {
            Rule::Halve => event.amount /= 2,
            Rule::Times(factor) => event.amount *= factor,
            Rule::Set(amount) => event.amount = amount,
            Rule::Cancel => return Flow::Cancel,
        }
        Flow::Continue
    }
}

struct Mod(Wear);
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key(self.0.item);
        r.item_part(key, self.0.clone());
    }
}

impl Wear {
    /// The points asked about since the last call.
    pub(in crate::game) fn take_asked(&self) -> Vec<i32> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
    pub(in crate::game) fn set(&self, rule: Rule) {
        *self.rule.lock().unwrap() = rule;
    }
}

/// Runs `test` on a serving game whose `item` has a part, starting with `rule`.
pub(in crate::game) fn with_wear_part(
    item: &'static str,
    rule: Rule,
    test: impl FnOnce(&mut Game<'_>, &Wear),
) {
    let wear = Wear {
        item,
        rule: Arc::new(Mutex::new(rule)),
        asked: Arc::default(),
    };
    let mods: mod_api::Mods = vec![ModEntry::new("wear", Box::new(Mod(wear.clone())))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game(|game| test(game, &wear));
    });
}

/// Uses `tool` once for `points` on the player, and the damage it has afterwards.
fn used(game: &mut Game<'_>, tool: &str, points: u32) -> u32 {
    game.creative = false;
    game.selected = 0;
    game.inventory[0] = Some(ItemStack::new(tool, 1));
    game.damage_held(points);
    game.inventory[0]
        .as_ref()
        .map_or(u32::MAX, |stack| stack.damage)
}

/// A part that halves the wear makes its tool lose half the durability per use, a part that cancels
/// it stops the wear, and a tool without a part wears as vanilla and is never asked.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; process-wide parts, run this test alone"]
fn a_part_changes_the_wear_of_its_own_tool_on_the_player() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Halve, |game, wear| {
        assert_eq!(used(game, "minecraft:iron_pickaxe", 10), 5);
        assert_eq!(wear.take_asked(), [10]);
        // 7 points are halved to 3, as the integer halving a part does.
        assert_eq!(used(game, "minecraft:iron_pickaxe", 7), 3);
        assert_eq!(wear.take_asked(), [7]);

        wear.set(Rule::Cancel);
        assert_eq!(used(game, "minecraft:iron_pickaxe", 10), 0);
        assert_eq!(wear.take_asked(), [10]);

        // A pickaxe of another material has no part: no stack conversion or dispatch.
        let dispatches = game.damage_item_dispatches();
        assert_eq!(used(game, "minecraft:diamond_pickaxe", 10), 10);
        assert!(wear.take_asked().is_empty());
        assert_eq!(game.damage_item_dispatches(), dispatches);

        // A creative player's tools take no wear, so the part is not asked.
        game.creative = true;
        game.inventory[0] = Some(ItemStack::new("minecraft:iron_pickaxe", 1));
        game.damage_held(10);
        assert_eq!(game.inventory[0].as_ref().unwrap().damage, 0);
        assert!(wear.take_asked().is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; process-wide parts, run this test alone"]
fn part_amount_boundaries_and_threshold_crossing() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Set(0), |game, wear| {
        for amount in [i32::MIN, -1, 0] {
            wear.set(Rule::Set(amount));
            assert_eq!(used(game, "minecraft:iron_pickaxe", 1), 0);
            assert_eq!(wear.take_asked(), [1]);
        }
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::new(0.0, 64.0, 0.0),
            )
            .unwrap(),
        );
        game.lan_acting = Some(1);
        for amount in [250, i32::MAX] {
            game.guest_equipment_breaks.clear();
            let broken_before = game.stats.get("broken", "minecraft:iron_pickaxe");
            wear.set(Rule::Set(amount));
            assert_eq!(used(game, "minecraft:iron_pickaxe", 1), u32::MAX);
            assert_eq!(wear.take_asked(), [1]);
            assert_eq!(game.guest_equipment_breaks, [47]);
            assert_eq!(
                game.stats.get("broken", "minecraft:iron_pickaxe"),
                broken_before + 1
            );
            game.damage_held(1);
            assert!(wear.take_asked().is_empty());
            assert_eq!(game.guest_equipment_breaks, [47]);
            assert_eq!(
                game.stats.get("broken", "minecraft:iron_pickaxe"),
                broken_before + 1
            );
        }
        wear.set(Rule::Set(249));
        assert_eq!(used(game, "minecraft:iron_pickaxe", 1), 249);
        assert_eq!(wear.take_asked(), [1]);
        wear.set(Rule::Set(1));
        game.damage_held(1);
        assert!(game.inventory[0].is_none());
        assert_eq!(wear.take_asked(), [1]);
    });
}

/// The part is asked with what Unbreaking leaves, not the points the use asked for: a pickaxe of
/// Unbreaking III saves a point three times in four, the part hears each wear that remains, and it
/// is never asked about a point Unbreaking saved.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; process-wide parts, run this test alone"]
fn a_part_is_asked_after_unbreaking() {
    with_wear_part("minecraft:iron_pickaxe", Rule::Times(1), |game, wear| {
        game.world.random = horizon_core::random::LegacyRandom::new(12345);
        game.creative = false;
        game.selected = 0;
        let mut pickaxe = ItemStack::new("minecraft:iron_pickaxe", 1);
        pickaxe.enchant("minecraft:unbreaking", 3);
        game.inventory[0] = Some(pickaxe);
        for _ in 0..200 {
            game.damage_held(1);
        }
        let asked = wear.take_asked();
        let damage = game.inventory[0].as_ref().unwrap().damage;
        // One point each, and each worn point is one the part was asked about.
        assert!(asked.iter().all(|points| *points == 1));
        assert_eq!(asked.len() as u32, damage);
        // A quarter of 200 points is 50: far from none and from all of them.
        assert!((20..=90).contains(&damage), "{damage}");

        // Two points at once reach the part as what Unbreaking left of them.
        wear.set(Rule::Halve);
        game.inventory[0].as_mut().unwrap().damage = 0;
        for _ in 0..100 {
            game.damage_held(2);
        }
        let asked = wear.take_asked();
        assert!(asked.iter().all(|points| (1..=2).contains(points)));
        let expected: i32 = asked.iter().map(|points| points / 2).sum();
        assert_eq!(game.inventory[0].as_ref().unwrap().damage as i32, expected);
    });
}
