//! One invulnerability observation across a paused player hit.
use super::{Environment, HurtSource, Hurting};

impl Environment<'_> {
    pub(super) fn observe_hit_invulnerability(&mut self, vanilla: bool) -> bool {
        if let Some(answer) = self.invulnerability_checked {
            return answer;
        }
        let source = HurtSource {
            damage_type: &self.source.definition().name,
            causing: self.causing,
            direct: self
                .level
                .attack()
                .projectile_direct(&self.source.definition().name),
            position: None,
        };
        let answer = self
            .level
            .invulnerability_check(Hurting::Player(self.entity, source), vanilla);
        // Only the Entity-level decision is retained. Its inputs are captured at hit entry.
        // Player/Living/ServerPlayer predicates still evaluate their own guards on each call.
        self.invulnerability_checked = Some(answer);
        answer
    }
}
