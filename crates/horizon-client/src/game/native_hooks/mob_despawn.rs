use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `Mob.checkDespawn` is about to run for mob `entity` (NeoForge's
    /// `MobDespawnEvent`): set `result` to force or forbid the despawn. `kind`
    /// and `pos` are the mob's, given as data because a mob taken out for its
    /// tick does not resolve through its handle (a #29 follow-up).
    pub(in crate::game) fn native_mob_despawn(
        &self,
        dimension: &str,
        mob: MobRef,
        kind: &str,
        pos: [f64; 3],
        result: &mut Despawn,
    ) {
        self.fire_mob_despawn(dimension, mob, kind, pos, result);
    }
}
