impl Game<'_> {
    pub(super) fn unscoped_native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if self.held_out.contains(&entity.id())
            || !self.is_logical_server()
            || self.native_lifecycle.epoch() != Some(entity.epoch())
        {
            return None;
        }
        let horizon_client::join_seq::residency::table::EntityLookup::Found { location, value } =
            self.server_resident_entity(entity.id())
        else {
            return None;
        };
        // Players are server-wide records. Ordinary body operations retain
        // their existing current-level domain; source membership is global.
        (matches!(value, Target::Player | Target::Guest(_))
            || location.owner.dimension == self.dim.key())
        .then_some(value)
    }
    #[cold]
    #[inline(never)]
    fn scoped_native_entity_target(&self, entity: EntityRef) -> Option<Target> {
        if self
            .tick_item_view(entity.id())
            .is_some_and(|stack| stack.is_none())
        {
            return None;
        }
        self.unscoped_native_entity_target(entity)
    }
}
