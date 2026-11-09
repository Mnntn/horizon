use super::Game;
use super::api_values::{TryIntoApi, TryIntoEngine};
use mod_api::{AttachmentAccess, BlockPos, Direction};
use {
    horizon_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound},
    horizon_world::level::Ticks,
};

fn world_attachments(
    world: &mut horizon_client::world::World,
    holder: mod_api::Holder,
) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
    use mod_api::{AttachmentError, Holder};
    if holder == Holder::Level {
        return Ok(&mut world.attachments);
    }
    if let Holder::BlockEntity([x, y, z]) = holder {
        let pos = (x, y, z);
        let chunk = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .ok_or(AttachmentError::MissingHolder)?;
        attachment_compound(
            chunk
                .saved
                .as_ref()
                .and_then(|e| e.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
        )?;
        let map = world
            .block_entities
            .attachments_mut(pos)
            .ok_or(AttachmentError::MissingHolder)?;
        world.modified.insert((x >> 4, z >> 4));
        return Ok(map);
    }
    let Holder::Chunk([x, z]) = holder else {
        return Err(AttachmentError::MissingHolder);
    };
    let map = world.chunks.attachments_mut(&(x, z))?;
    world.modified.insert((x, z));
    Ok(map)
}

impl Game<'_> {
    /// Whether entity `id` is in the level, as a holder of attachments: one walk
    /// over the entity lists (a kind added there holds attachments too), then the
    /// kinds that are not on it.
    fn attachment_entity_exists(&self, id: u32) -> bool {
        use super::entity::{Entity as _, entity_lists_of, for_each_entity};
        let mut found = false;
        for_each_entity!(entity_lists_of!(self), entity => {
            found |= entity.id() == id && !entity.is_removed();
        });
        found || self.entity_ids_off_walk().any(|e| e == id)
    }

    /// The attachments of entities that left the level, dropped: each tick
    /// while any entity has some. Only those entities are looked at, and
    /// nothing else of the entities' memory changes.
    pub(super) fn prune_entity_attachments(&mut self) {
        let alive = self.live_entity_ids();
        let gone: Vec<[i32; 4]> = self
            .persist
            .uuids
            .iter()
            .filter(|(id, uuid)| {
                self.entity_attachments.contains_key(*uuid) && !alive.contains(*id)
            })
            .map(|(_, uuid)| *uuid)
            .collect();
        for uuid in gone {
            self.forget_entity_initialization(uuid);
            self.entity_attachments.remove(&uuid);
        }
    }

    pub(super) fn native_authoritative(&self) -> bool {
        self.serving.is_some() || self.options.dedicated.is_some()
    }

    /// Access a holder's attachment map, to lend to a mod: a server's, whose
    /// synced changes are sent at the pass's end, or a client's copy of what
    /// its server synced.
    pub(super) fn attachment_map(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if self.client_attachment_entity_exists(holder) {
            self.initialize_entity_on_access(holder);
        }
        if self.native_authoritative() {
            self.world.attachment_syncs.insert(holder);
            return self.attachment_map_quiet(holder, access);
        }
        let Some(remote) = self.remote.as_mut() else {
            return Err(mod_api::AttachmentError::MissingHolder);
        };
        remote.attachments.entry(holder).or_default();
        self.bind_remote_attachment(holder);
        let remote = self.remote.as_mut().unwrap();
        Ok(remote.attachments.get_mut(&holder).unwrap())
    }

    /// `attachment_map` without recording the holder for the sync. Inside a transaction, the
    /// map is kept as it was on its first access, so an abort puts it back.
    pub(super) fn attachment_map_quiet(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative() {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        self.journal_attachments(holder);
        self.attachment_slot(holder, access)
    }

    /// Where `holder`'s attachment map lives now: an entity's by its UUID, a player's where
    /// their state is (swapped in for a guest's turn, or kept beside), the world's others.
    pub(super) fn attachment_slot(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        use mod_api::AttachmentError;
        if let mod_api::Holder::Entity(id) = holder {
            if self.entity_initialization.active != Some(holder)
                && !self.attachment_entity_exists(id)
            {
                return Err(AttachmentError::MissingHolder);
            }
            self.initialize_entity_on_access(holder);
            attachment_compound(
                self.persist
                    .raw
                    .get(&id)
                    .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
            )?;
            let painting = if access == AttachmentAccess::Store {
                self.paintings.iter().position(|painting| painting.id == id)
            } else {
                None
            };
            if painting.is_some()
                && self
                    .persist
                    .removals
                    .contains(&super::commands::Target::Painting(id))
            {
                return Err(AttachmentError::MissingHolder);
            }
            let level = painting.and_then(|_| self.level(self.dim.id()));
            let uuid = self.uuid_for(id);
            let map = self.entity_attachments.entry(uuid).or_default();
            if let Some(i) = painting {
                let level = level.ok_or(AttachmentError::MissingHolder)?;
                if !self.paintings[i].attachment_lifetime.is_bound() {
                    self.paintings[i].attachment_lifetime.bind(
                        map,
                        self.attachment_indexes.entry(self.dim).or_default(),
                        mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(
                            level.epoch(),
                            id,
                        )),
                    );
                }
            }
            return Ok(map);
        }
        if let mod_api::Holder::Player(uuid) = holder {
            if self.entity_initialization.active != Some(holder)
                && self.lan_acting_uuid != Some(uuid)
                && self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.holder_guest(uuid, self.dim.id()))
                    .is_none()
            {
                return Err(AttachmentError::MissingHolder);
            }
            self.initialize_entity_on_access(holder);
            if self.lan_acting_uuid == Some(uuid) {
                if access == AttachmentAccess::Store
                    && let Some(conn) = self.lan_acting
                {
                    self.bind_player_attachments(conn, true);
                }
                return Ok(&mut self.player_attachments);
            }
            let conn = self
                .lan
                .as_ref()
                .and_then(|host| {
                    if self.entity_initialization.active == Some(holder) {
                        host.hub.guests.iter().find(|guest| guest.uuid == uuid)
                    } else {
                        host.hub.holder_guest(uuid, self.dim.id())
                    }
                })
                .map(|guest| guest.conn)
                .ok_or(AttachmentError::MissingHolder)?;
            if self.lan_guest_swaps.contains(&conn) {
                return Err(AttachmentError::MissingHolder);
            }
            if access == AttachmentAccess::Store {
                self.bind_player_attachments(conn, true);
            }
            return Ok(self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
                .ok_or(AttachmentError::MissingHolder)?);
        }
        world_attachments(&mut self.world, holder)
    }
}

pub(super) fn level_side<'a>(level: &'a super::GameLevel<'_>) -> mod_api::LevelSide<'a> {
    if horizon_world::level::LevelAccess::is_client_side(level) {
        mod_api::LevelSide::Client(level)
    } else {
        mod_api::LevelSide::Server(level)
    }
}

impl mod_api::LevelHost for super::GameLevel<'_> {
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.block_registries, self.registry, name)?
            .try_into_api()
            .ok()
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        horizon_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
            .try_into_api()
            .ok()
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.native_state_block(state.try_into_engine(&()).ok()?)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.registry
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn height(&self) -> i32 {
        self.world.shape.height
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        if !self.native_authoritative {
            return None;
        }
        super::native_world_read::block_in_tag(block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        let [x, y, z] = pos;
        if !(self.world.shape.contains_y(y) && self.world.loaded(x, z)) {
            return None;
        }
        super::native_world_read::state_fluid(
            self.behaviours,
            self.registry,
            self.world.block(x, y, z),
        )
    }
    fn state_fluid_state(&self, state: mod_api::BlockState) -> Option<mod_api::FluidState> {
        super::native_world_read::state_fluid(
            self.behaviours,
            self.registry,
            state.try_into_engine(&()).ok()?,
        )
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        self.native_authoritative
            .then(|| super::native_world_read::fluid_in_tag(self.items, fluid, tag))
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        self.native_state_with_property(state.try_into_engine(&()).ok()?, key, value)?
            .try_into_api()
            .ok()
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        super::native_level_refs::world_brightness(self.world, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        horizon_world::level::raw_brightness(
            self,
            BlockPos::new(pos[0], pos[1], pos[2]),
            sky_darken,
        )
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        self.world.is_loaded_at(pos[0], pos[1], pos[2])
            && self
                .behaviours
                .is_raining_at(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::FeatureFlags::new(self.enabled_features.clone())
    }
}

impl mod_api::AttachmentHost for Game<'_> {
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        self.attachment_map(holder, access)
    }
}

impl mod_api::AttachmentHost for super::GameLevel<'_> {
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        _: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        if !self.native_authoritative {
            return Err(mod_api::AttachmentError::MissingHolder);
        }
        self.world.attachment_syncs.insert(holder);
        world_attachments(self.world, holder)
    }
}

impl mod_api::LevelRandomHost for super::GameLevel<'_> {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    fn random(&mut self) -> Option<mod_api::LegacyRandom> {
        self.native_level_random().cloned()
    }
    fn set_random(&mut self, random: mod_api::LegacyRandom) -> bool {
        self.native_level_random().map(|r| *r = random).is_some()
    }
}

impl mod_api::LevelHost for super::native_hooks::HitLevel<'_> {
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        super::native_block_states::parse(self.0.block_registries()?, self.0.registry(), name)?
            .try_into_api()
            .ok()
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        self.0.block_registries()?;
        super::api_values::default_state(block)?.try_into_api().ok()
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        self.0.tags()?;
        super::native_world_read::block_in_tag(block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        self
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        Some(super::native_world_read::fluid_in_tag(
            self.0.tags()?,
            fluid,
            tag,
        ))
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        self.0.block(pos).try_into_api().ok()
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(|s| &*s.name)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        self.0
            .registry()
            .state(state.try_into_engine(&()).ok()?)
            .map(super::native_block_states::serialize)
    }
    fn min_y(&self) -> i32 {
        self.0.shape().min_y
    }
    fn height(&self) -> i32 {
        self.0.shape().height
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        super::native_hooks::with_property(
            self.0.registry(),
            state.try_into_engine(&()).ok()?,
            key,
            value,
        )?
        .try_into_api()
        .ok()
    }
}

impl mod_api::AttachmentHost for super::native_hooks::HitLevel<'_> {
    fn attachments(
        &mut self,
        holder: mod_api::Holder,
        access: AttachmentAccess,
    ) -> Result<&mut mod_api::Attachments, mod_api::AttachmentError> {
        let _ = access;
        self.0.attachments(holder)
    }
}

impl mod_api::LevelRandomHost for super::native_hooks::HitLevel<'_> {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        self
    }
    fn random(&mut self) -> Option<mod_api::LegacyRandom> {
        Some(self.0.random().clone())
    }
    fn set_random(&mut self, random: mod_api::LegacyRandom) -> bool {
        *self.0.random() = random;
        true
    }
}

pub(super) struct LevelTicks<'a, L> {
    pub level: L,
    pub ticks: &'a mut Ticks,
}

impl<'a: 'b, 'b, L: std::ops::Deref<Target = super::GameLevel<'a>>> mod_api::LevelHost
    for LevelTicks<'b, L>
{
    fn block_state_id(&self, name: &str) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_id(&*self.level, name)
    }
    fn block_state_default(&self, block: mod_api::BlockKey) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state_default(&*self.level, block)
    }
    fn block_state(&self, pos: [i32; 3]) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::block_state(&*self.level, pos)
    }
    fn state_block(&self, state: mod_api::BlockState) -> Option<&str> {
        mod_api::LevelHost::state_block(&*self.level, state)
    }
    fn state_string(&self, state: mod_api::BlockState) -> Option<String> {
        mod_api::LevelHost::state_string(&*self.level, state)
    }
    fn min_y(&self) -> i32 {
        mod_api::LevelHost::min_y(&*self.level)
    }
    fn height(&self) -> i32 {
        mod_api::LevelHost::height(&*self.level)
    }
    fn block_in_tag(&self, block: mod_api::BlockKey, tag: &str) -> Option<bool> {
        mod_api::LevelHost::block_in_tag(&*self.level, block, tag)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        mod_api::LevelHost::tags(&*self.level)
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        mod_api::LevelHost::fluid_state(&*self.level, pos)
    }
    fn state_fluid_state(&self, state: mod_api::BlockState) -> Option<mod_api::FluidState> {
        mod_api::LevelHost::state_fluid_state(&*self.level, state)
    }
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool> {
        mod_api::LevelHost::fluid_in_tag(&*self.level, fluid, tag)
    }
    fn state_with_property(
        &self,
        state: mod_api::BlockState,
        key: &str,
        value: &str,
    ) -> Option<mod_api::BlockState> {
        mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
    fn brightness(&self, layer: mod_api::LightLayer, pos: [i32; 3]) -> i32 {
        mod_api::LevelHost::brightness(&*self.level, layer, pos)
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        mod_api::LevelHost::raw_brightness(&*self.level, pos, sky_darken)
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_loaded(&*self.level, pos)
    }
    fn is_raining_at(&self, pos: [i32; 3]) -> bool {
        mod_api::LevelHost::is_raining_at(&*self.level, pos)
    }
    fn enabled_features(&self) -> mod_api::FeatureFlags {
        mod_api::LevelHost::enabled_features(&*self.level)
    }
}

impl<'a: 'b, 'b> mod_api::LevelRandomHost for LevelTicks<'b, &mut super::GameLevel<'a>> {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        mod_api::LevelRandomHost::attachments(&mut *self.level)
    }
    fn random(&mut self) -> Option<mod_api::LegacyRandom> {
        mod_api::LevelRandomHost::random(&mut *self.level)
    }
    fn set_random(&mut self, random: mod_api::LegacyRandom) -> bool {
        mod_api::LevelRandomHost::set_random(&mut *self.level, random)
    }
}

impl<'a: 'b, 'b, L: std::ops::Deref<Target = super::GameLevel<'a>>> mod_api::LevelTicksHost
    for LevelTicks<'b, L>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: mod_api::BlockKey, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(self.ticks, pos, fluid, delay)
    }
    fn signal(&self, pos: [i32; 3], direction: Direction) -> i32 {
        self.signals()
            .signal(BlockPos::new(pos[0], pos[1], pos[2]), direction)
    }
    fn direct_signal(&self, pos: [i32; 3], direction: Direction) -> i32 {
        self.signals()
            .direct_signal(BlockPos::new(pos[0], pos[1], pos[2]), direction)
    }
    fn direct_signal_to(&self, pos: [i32; 3]) -> i32 {
        self.signals()
            .direct_signal_to(BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn best_neighbor_signal(&self, pos: [i32; 3]) -> i32 {
        self.signals()
            .best_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn has_neighbor_signal(&self, pos: [i32; 3]) -> bool {
        self.signals()
            .has_neighbor_signal(BlockPos::new(pos[0], pos[1], pos[2]))
    }
}

impl<'a: 'b, 'b, L: std::ops::Deref<Target = super::GameLevel<'a>>> LevelTicks<'b, L> {
    /// The level in place read through the game's own `SignalGetter`, with this write's
    /// comparator outputs.
    fn signals(&self) -> horizon_world::level::Signals<'_, super::GameLevel<'a>> {
        self.level.behaviours.signals(&*self.level, &*self.ticks)
    }
}

impl<'a: 'b, 'b> mod_api::LevelWriteHost for LevelTicks<'b, &mut super::GameLevel<'a>> {
    fn set_block(
        &mut self,
        pos: [i32; 3],
        state: mod_api::BlockState,
        flags: mod_api::BlockUpdateFlags,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        self.level
            .native_set_block(self.ticks, pos, state, flags.bits())
    }

    fn remove_block(&mut self, pos: [i32; 3], moved: bool) -> bool {
        self.level.native_remove_block(self.ticks, pos, moved)
    }

    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(self.ticks, pos, key, value, flags)
    }

    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.level
            .native_play_sound(sound, at, source, volume, pitch)
    }

    fn send_particles(
        &mut self,
        options: &mod_api::ParticleOptions,
        state: Option<mod_api::BlockState>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        let Ok(state) = state.try_into_engine(&()) else {
            return false;
        };
        let Ok(options) = options.try_into_engine(&()) else {
            return false;
        };
        self.level
            .native_send_particles(&options, state, at, spread, speed, count)
    }

    fn level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        self.level.native_level_event(id, pos, data)
    }
}

super::native_tags::impl_tags!(
    super::GameLevel<'_>,
    level,
    level.native_authoritative.then_some(level.items)
);
super::native_tags::impl_tags!(super::native_hooks::HitLevel<'_>, level, level.0.tags());
