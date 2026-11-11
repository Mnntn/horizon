//! An entity's `Entity.noPhysics` for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level
//! handles"): whether its moves pass through blocks. Offered only for the
//! kinds whose flag has one value between ticks; an item recomputes its own
//! inside its tick, and an armor stand's depends on how it was loaded, so
//! they read None, as do the other kinds, a stale handle and a client.
use super::{Game, commands::Target};
use horizon_client::projectiles::ProjectileKind;
use mod_api::EntityRef;

impl Game<'_> {
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is false (a vex sets it only for
    /// the length of its own tick); an arrow's or a trident's is its
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. The
    /// ender dragon, an area-effect cloud, an ominous item spawner and a
    /// shulker bullet set it in their constructors and never clear it.
    pub(super) fn entity_no_physics(&self, entity: EntityRef) -> Option<bool> {
        match self.native_entity_target(entity)? {
            Target::Player => Some(self.player.as_ref()?.spectator),
            Target::Guest(conn) => Some(self.command_guest(conn)?.motion.as_deref()?.spectator),
            Target::Mob(_) => Some(false),
            Target::Dragon(_) | Target::Cloud(_) | Target::Spawner(_) => Some(true),
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                match (&p.kind, &p.arrow) {
                    (ProjectileKind::ShulkerBullet { .. }, _) => Some(true),
                    (_, Some(arrow)) => Some(arrow.no_physics),
                    _ => None,
                }
            }
            Target::Trident(id) => Some(self.tridents.iter().find(|t| t.id == id)?.no_physics),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn no_physics_reads_supported_entities_and_refuses_others() {
        use super::super::native_entity_refs::tests::with_game;
        use super::super::trident::ThrownTrident;
        use horizon_client::mobs::Mob;
        use horizon_client::motion::PlayerMotion;
        use horizon_client::projectiles::{Projectile, ProjectileKind};
        use horizon_core::geometry::Vec3;
        use mod_api::WorldEpoch;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            crate::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
            let player = game.entity(20).unwrap();
            game.player.as_mut().unwrap().spectator = false;
            assert_eq!(player.no_physics(game), Some(false));
            game.player.as_mut().unwrap().spectator = true;
            assert_eq!(player.no_physics(game), Some(true));

            let kind = game.res.mob_types().get("minecraft:vex").unwrap().clone();
            game.mobs.push(Mob::new(42, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(42).unwrap();
            assert_eq!(mob.no_physics(game), Some(false));

            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::ZERO,
                None,
            );
            arrow.id = game.take_entity_id();
            let arrow_id = arrow.id;
            game.creatures.projectiles.push(arrow);
            let arrow = game.entity(arrow_id).unwrap();
            assert_eq!(arrow.no_physics(game), Some(false));
            game.creatures
                .projectiles
                .update(0, |p| p.arrow.as_mut().unwrap().no_physics = true);
            assert_eq!(arrow.no_physics(game), Some(true));

            let trident_id = game.take_entity_id();
            game.tridents
                .push(ThrownTrident::remote(trident_id, Vec3::ZERO, 0.0, 0.0));
            let trident = game.entity(trident_id).unwrap();
            assert_eq!(trident.no_physics(game), Some(false));
            game.tridents[0].no_physics = true;
            assert_eq!(trident.no_physics(game), Some(true));

            let mut snowball = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
            snowball.id = game.take_entity_id();
            let snowball_id = snowball.id;
            game.creatures.projectiles.push(snowball);
            let snowball = game.entity(snowball_id).unwrap();
            assert_eq!(snowball.no_physics(game), None);

            let mut bullet = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::ZERO,
                None,
            );
            bullet.id = game.take_entity_id();
            let bullet_id = bullet.id;
            game.creatures.projectiles.push(bullet);
            let bullet = game.entity(bullet_id).unwrap();
            assert_eq!(bullet.no_physics(game), Some(true));

            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = game.take_entity_id();
            let dragon_id = dragon.id;
            game.dragon.replace(dragon);
            let dragon = game.entity(dragon_id).unwrap();
            assert_eq!(dragon.no_physics(game), Some(true));

            game.boats.push(super::super::boats::Boat::new(
                44,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            ));
            let boat = game.entity(44).unwrap();
            assert_eq!(boat.no_physics(game), None);
            let stale = EntityRef::new(WorldEpoch::new(mob.epoch().get() + 1), mob.id());
            assert_eq!(stale.no_physics(game), None);
        });
    }
}
