use super::Game;
use horizon_client::entities::ItemEntity;

impl Game<'_> {
    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.dropped.remove(i);
        self.native_entity_left(e.id);
        e
    }
}
