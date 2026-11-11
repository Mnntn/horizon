//! The separately stored dragon participates in Level.getEntities; parts follow the walk.
use super::*;

impl Game<'_> {
    pub(super) fn append_dragon_query_ids(&self, bounds: Aabb, parts: bool, ids: &mut Vec<u32>) {
        let Some(dragon) = self.dragon.as_ref().filter(|d| d.id != 0) else {
            return;
        };
        if parts {
            ids.extend(self.dragon.parts_table().iter().filter(|&id| {
                bounds.intersects(dragon.part_geometry((id - dragon.id - 1) as usize).1)
            }));
        } else {
            // EntityType.ENDER_DRAGON dimensions: 16 wide, 8 high.
            let p = dragon.position;
            let body = Aabb::new(
                Vec3::new(p.x - 8.0, p.y, p.z - 8.0),
                Vec3::new(p.x + 8.0, p.y + 8.0, p.z + 8.0),
            );
            if bounds.intersects(body) {
                ids.push(dragon.id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{dragon::Dragon, native_entity_refs::tests::with_game};
    use horizon_client::mobs::Mob;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn all_entities_uses_lookup_order_without_bounds_or_parts() {
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(
                90,
                kind.clone(),
                Vec3::new(1.0e12, 64.0, 0.0),
                0.0,
            ));
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            game.mobs.push(Mob::new(3, kind, Vec3::ZERO, 0.0));
            let level = game.level(game.dim.id()).unwrap();
            let ids: Vec<_> = game
                .level_all_entities(level)
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .filter(|id| [90, 20, 3].contains(id) || (21..=28).contains(id))
                .collect();
            assert_eq!(ids, vec![90, 20, 3]);
            use horizon_client::join_seq::Joined;
            let join = game.mobs[0].join_seq();
            let extracted = game.mobs.extract(join).unwrap();
            assert_eq!(game.level_all_entities(level), None);
            game.mobs.put_back(0, extracted);
            assert!(game.level_all_entities(level).is_some());
            assert_eq!(
                game.level_all_entities(LevelRef::new(
                    mod_api::WorldEpoch::new(level.epoch().get() + 1),
                    level.dimension()
                )),
                None
            );
            assert_eq!(
                game.level_all_entities(LevelRef::new(level.epoch(), DimensionKey::NETHER)),
                None
            );
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            assert_eq!(game.level_all_entities(level), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn ordinary_entities_precede_dragon_parts() {
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            let level = game.level(game.dim.id()).unwrap();
            // Parts start at the origin, away from the dragon's own body.
            let ids = game
                .level_entities_in(level, [-10.0, -1.0, -10.0], [10.0, 10.0, 10.0])
                .unwrap()
                .into_iter()
                .map(|e| e.id())
                .collect::<Vec<_>>();
            assert_eq!(ids.first(), Some(&12));
            // This test pins only the ordinary/parts boundary, not part order.
            let mut parts = ids[1..].to_vec();
            parts.sort_unstable();
            assert_eq!(parts, (21..=28).collect::<Vec<_>>());
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn entities_in_includes_dragon_and_separate_parts() {
        with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.mobs
                .push(Mob::new(12, kind, Vec3::new(5.0, 64.0, 5.0), 0.0));
            let level = game.level(game.dim.id()).unwrap();
            let query = |game: &Game<'_>, lo, hi| {
                game.level_entities_in(level, lo, hi)
                    .unwrap()
                    .into_iter()
                    .map(|e| e.id())
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                query(game, [-10.0, 60.0, -10.0], [10.0, 100.0, 10.0]),
                vec![12]
            );
            let mut dragon = Dragon::new(Vec3::new(0.0, 80.0, 0.0), 0.0, 0);
            dragon.id = 20;
            game.dragon.replace(dragon);
            assert_eq!(
                query(game, [-10.0, 60.0, -10.0], [10.0, 100.0, 10.0]),
                vec![12, 20]
            );
            assert_eq!(
                query(game, [20.0, 60.0, 20.0], [30.0, 70.0, 30.0]),
                Vec::<u32>::new()
            );
            assert_eq!(query(game, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]), vec![12]);
            let entity = game.entity(20).unwrap();
            assert_eq!(
                game.living_health(game.living(entity).unwrap()),
                Some(200.0)
            );
            assert_eq!(
                query(game, [-100.0, -10.0, -100.0], [100.0, 200.0, 100.0]),
                vec![12, 20, 25, 27, 28, 24, 26, 23, 22, 21]
            );
            // Vanilla parts begin at the origin until the first dragon tick.
            assert_eq!(
                query(game, [-10.0, -1.0, -10.0], [10.0, 10.0, 10.0]),
                vec![25, 27, 28, 24, 26, 23, 22, 21]
            );
        });
    }
}
