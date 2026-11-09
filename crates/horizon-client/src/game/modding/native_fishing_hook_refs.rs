//! Fishing-hook handles for native server mods. A cast hook lives with the
//! player who cast it, not in a level list, so a handle finds it through them.
use super::{Game, commands::Target};
use mod_api::{EntityRef, FishingHookRef};

impl Game<'_> {
    /// Narrows an entity to a cast fishing hook. None for another kind, a
    /// stale handle, a client, or a hook taken out for its tick.
    pub(super) fn fishing_hook(&self, entity: EntityRef) -> Option<FishingHookRef> {
        matches!(self.native_entity_target(entity)?, Target::Hook(_))
            .then(|| FishingHookRef::new(entity))
    }

    /// `FishingHook.getPlayerOwner`: the player who cast it. None for a stale
    /// handle.
    pub(super) fn fishing_hook_player_owner(&self, hook: FishingHookRef) -> Option<EntityRef> {
        self.fishing_hook(hook.entity())?;
        let (_, owner) = self.fishing_hook_by_id(hook.entity().id())?;
        let id = match owner {
            // The one whose turn it is: their handle is the acting entity.
            Target::Player => self.lan_acting_entity?,
            guest => u32::try_from(self.network_entity_id(guest)?).ok()?,
        };
        self.entity(id)
    }

    /// `FishingHook.getHookedIn`: what it holds. None when it holds nothing,
    /// what it holds is gone, or for a stale handle.
    pub(super) fn fishing_hook_hooked_in(&self, hook: FishingHookRef) -> Option<EntityRef> {
        self.fishing_hook(hook.entity())?;
        let (found, _) = self.fishing_hook_by_id(hook.entity().id())?;
        self.entity(found.hooked_in()?)
    }

    /// `FishingHook.isOpenWaterFishing`, which the `fishing` loot's
    /// `in_open_water` predicate reads. None for a stale handle.
    pub(super) fn fishing_hook_open_water(&self, hook: FishingHookRef) -> Option<bool> {
        self.fishing_hook(hook.entity())?;
        let (found, _) = self.fishing_hook_by_id(hook.entity().id())?;
        Some(found.open_water())
    }
}

#[cfg(test)]
mod tests {
    use super::super::{fishing::Hook, native_entity_refs::tests::with_game};
    use super::*;
    use horizon_client::{mobs::Mob, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use mod_api::WorldEpoch;

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_cast_hook_narrows_reads_its_catch_and_goes_when_discarded() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();
            assert!(game.fishing_hook(mob).is_none());

            let id = game.take_entity_id();
            game.hook = Some(Hook::remote(id, Vec3::new(3.0, 64.0, 3.0)));
            let entity = game.entity(id).unwrap();
            let hook = game.fishing_hook(entity).unwrap();
            assert_eq!(hook.entity(), entity);
            assert_eq!(hook.hooked_in(game), None);

            game.hook.as_mut().unwrap().set_hooked_in(99);
            assert_eq!(hook.hooked_in(game), Some(mob));
            game.mobs.clear();
            assert_eq!(hook.hooked_in(game), None);

            // A box query finds it, as `Level.getEntities` does.
            let level = game.level(game.dim.id()).unwrap();
            let found = game
                .level_entities_in(level, [0.0, 60.0, 0.0], [6.0, 70.0, 6.0])
                .unwrap();
            assert_eq!(found, vec![entity]);

            let stale = FishingHookRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                id,
            ));
            assert_eq!(stale.open_water(game), None);

            assert!(entity.discard(game));
            assert!(game.fishing_hook(entity).is_none());
            assert_eq!(hook.open_water(game), None);
            game.native_removals_flush();
            assert!(game.hook.is_none());
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_live_hook_keeps_its_uuid_across_memory_pruning() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let id = game.take_entity_id();
            game.hook = Some(Hook::remote(id, Vec3::ZERO));
            let entity = game.entity(id).unwrap();
            let before = game.entity_uuid(entity).unwrap();
            game.prune_entity_memory();
            assert!(game.entity(id).is_some());
            assert_eq!(game.entity_uuid(entity), Some(before));
            assert_eq!(game.entity_uuid(entity), Some(before));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_hook_belongs_to_the_player_who_cast_it() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.lan_acting_uuid = Some(123);
            game.lan_acting_entity = Some(20);
            let player = game.entity(20).unwrap();
            let id = game.take_entity_id();
            game.hook = Some(Hook::remote(id, Vec3::ZERO));
            let hook = game.fishing_hook(game.entity(id).unwrap()).unwrap();
            assert_eq!(hook.player_owner(game), Some(player));
            assert_eq!(hook.open_water(game), Some(true));
        });
    }
}
