use super::Game;

impl Game<'_> {
    /// Run equipment continuations with the acting player installed on the real host.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hurt_player_equipment(
        &mut self,
        source: horizon_world::damage_source::DamageTypeId,
        amount: f32,
        blocking: Option<horizon_client::health::Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        attack: &horizon_client::health::Attack,
    ) -> horizon_client::Result<horizon_client::health::BlockOutcome> {
        let rules = self.world.health_rules;
        let helmet = self.armor[0]
            .as_ref()
            .is_some_and(|s| s.count > 0 && &*s.item != "minecraft:air");
        let mut hit = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .start_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                source,
                amount,
                blocking,
                living_cause,
                causing,
                creative_source,
                helmet,
                !self.combat.refuse_player_hit,
            )?;
        if let Some(damage) = hit.helmet_damage() {
            self.hurt_helmet(source, damage);
            self.refresh_armor_protection();
        }
        let armor_damage = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .select_equipment_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                &mut hit,
            )?;
        if let Some(damage) = armor_damage {
            self.hurt_armor(source, damage);
            self.refresh_armor_protection();
        }
        self.player
            .as_mut()
            .ok_or("missing damage target")?
            .finish_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                hit,
            )
    }
}
