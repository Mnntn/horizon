//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `entities` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait EntitiesHost {
    /// Side: both.
    /// The entity `id` (the id events carry) as a handle for the `entity_*`
    /// methods: None with no world open, or for no such entity in the level
    /// in place or among the server's players. On a client (API
    /// @API:client_entity_refs@), any entity the server shows this window,
    /// this window's own player included. API 373.
    #[access(WorldOnly)]
    fn entity(&self, id: u32) -> Option<EntityRef>;
    /// Side: both.
    /// The level `dimension` (`minecraft:the_nether`) as a handle: on a client,
    /// the level it shows, whatever the server calls it. None with no world
    /// open, or for a dimension the server hasn't loaded. API 373.
    #[access(WorldOnly)]
    fn level(&self, dimension: &str) -> Option<LevelRef>;
}
/// The server host's methods of the `entities` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait EntitiesServerHost {
    /// Side: server.
    /// Spawns an entity of `kind` (`minecraft:item_display`) at `pos` in the
    /// level in place, `dimension`, from `nbt` (its compound, as `/summon
    /// <kind> <pos> <nbt>` takes it; passengers included): its id. None on a
    /// client's copy of the world, for another dimension, a position outside
    /// `/summon`'s bounds, a chunk that isn't loaded, a kind Horizon can't
    /// load, or one a mod kept out (`native_entity_join_level`).
    #[access(WorldOnly)]
    fn summon(&mut self, dimension: &str, kind: &str, pos: [f64; 3], nbt: Compound) -> Option<u32>;
    /// Side: server.
    /// Writes the experience orb `id`'s `value` (the points a pickup gives), `count`
    /// (orbs merged into it, each worth `value`) and `age` (ticks lived; an orb
    /// despawns at 6000); a `None` is left as it is. Signed values are preserved,
    /// including zero and negative values.
    /// False, and nothing written, for an orb not in the server level `dimension`,
    /// a `count` of 0, and on a client of a server.
    #[access(WorldOnly)]
    fn set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<i32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool;
    /// Side: server.
    /// The experience orbs of the server level `dimension` whose box meets
    /// the box `min`..`max`, in the level's order: what a mod reads before it
    /// cancels an orb in `native_entity_join_level`, to give its points with
    /// `PlayerRef::give_experience_points`. Empty for a level not in place and on a
    /// client of a server.
    #[access(WorldOnly)]
    fn experience_orbs(&self, dimension: &str, min: [f64; 3], max: [f64; 3]) -> Vec<ExperienceOrb>;
    /// Side: server.
    /// Throws a falling block of `state` from `pos` (its bottom centre) at
    /// `velocity` (blocks a tick) in the level in place, `dimension`: it
    /// falls, lands and places itself as sand does, or drops as an item
    /// where it can't. The block at `pos` is left as it is: take it away first
    /// with `native_set_block` to make a block fall out of the world. False,
    /// with nothing thrown, on a client's copy of the world, for another
    /// dimension, an air or unknown state, a chunk that isn't loaded, a value
    /// that isn't finite, or with 1024 falling blocks in the level already.
    /// Each axis of `velocity` is cut to 3.9.
    #[access(WorldOnly)]
    fn spawn_falling_block(
        &mut self,
        dimension: &str,
        state: BlockState,
        pos: [f64; 3],
        velocity: [f64; 3],
        falling: Falling,
    ) -> bool;
}
/// The handles' reads of the `entities` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait EntitiesBackend {
    /// Side: server.
    /// The entity's tags (`Entity.getTags`), as `/tag <entity> list` shows
    /// them. API 386.
    #[access(WorldOnly)]
    fn entity_tags(&self, entity: EntityRef) -> Option<Vec<String>>;
    /// Side: both.
    /// The entity's name as plain text (`Entity.getName`): its custom name,
    /// a player's name, else its type's name. On a client (API
    /// @API:client_entity_refs@), from the custom name the server synced
    /// (`Entity.DATA_CUSTOM_NAME`) and a player's profile name. API
    /// 386.
    #[access(WorldOnly)]
    fn entity_name(&self, entity: EntityRef) -> Option<String>;
    /// Side: server.
    /// Whether the entity's name shows over it at any distance
    /// (`Entity.isCustomNameVisible`); false for the kinds that keep no name.
    #[access(WorldOnly)]
    fn entity_custom_name_visible(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The category of the entity's type (`EntityType.getCategory`): a mob's
    /// from its type, `Monster` for the dragon, `Misc` for every other kind.
    #[access(WorldOnly)]
    fn entity_classification(&self, entity: EntityRef) -> Option<MobCategory>;
    /// Side: server.
    /// Whether the entity is in its level: from when `entity_join_level` let
    /// it in (false while that hook runs for it and before) until it
    /// leaves or a mod removes it. Players, and the kinds that hook doesn't
    /// tell (displays, markers, interactions, evoker fangs, area effect
    /// clouds, ominous item spawners), are in it while the handle names them.
    /// False for a stale handle or on a client.
    #[access(WorldOnly)]
    fn entity_added_to_level(&self, entity: EntityRef) -> bool;
    /// Side: both.
    /// The entity's position (`Entity.position`): its feet, the middle of
    /// its box for a painting or an item frame.
    #[access(WorldOnly)]
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// The block the entity's position is in (`Entity.blockPosition`). API
    /// 373.
    #[access(WorldOnly)]
    fn entity_block_position(&self, entity: EntityRef) -> Option<[i32; 3]>;
    /// Side: both.
    /// The entity's yaw and pitch in degrees (`Entity.getYRot`, `getXRot`); 0
    /// for the kinds that don't turn. API 373.
    #[access(WorldOnly)]
    fn entity_rotation(&self, entity: EntityRef) -> Option<[f32; 2]>;
    /// Side: server.
    /// The entity's type (`Entity.getType`): `minecraft:zombie`. API
    /// 373.
    #[access(WorldOnly)]
    fn entity_kind(&self, entity: EntityRef) -> Option<String>;
    /// Side: server.
    /// The level the entity is in (`Entity.level`): on a client, the
    /// client's level. API 373.
    #[access(WorldOnly)]
    fn entity_level(&self, entity: EntityRef) -> Option<LevelRef>;
    /// Side: server.
    /// `Entity.isPermanentlyInvulnerable`: the `Invulnerable` flag, which
    /// only damage that bypasses invulnerability (the void, `/kill`) or a
    /// creative player gets past. Mobs keep it on themselves (saved with
    /// them), players on their body (#916, not saved: Horizon's player save
    /// keeps no such field yet), the other kinds in the saved `Invulnerable`
    /// tag of their entity data (#916) — the tag the item damage path
    /// already refuses by.
    #[access(WorldOnly)]
    fn entity_permanently_invulnerable(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// `Entity.invulnerableTime` (#916): the entity-level invulnerability
    /// window, its own field on a mob and a player — never the living damage
    /// cooldown — and the saved `invulnerable_time` tag for the other kinds.
    /// Counted down by the base tick; `isTemporarilyInvulnerable` is it above
    /// zero. None for a stale handle or a kind that keeps none.
    #[access(WorldOnly)]
    fn entity_invulnerable_time(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// `LivingEntity.damageCooldownTime`: 20 after a hit, counting down;
    /// while it is above 10 a hit does only what it deals beyond the last
    /// one. Mobs and players.
    #[access(WorldOnly)]
    fn entity_damage_cooldown_time(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// `Entity.isInvulnerableTo` for a source of `damage_type`: #842's own
    /// decision, the guard every mob damage hook runs after — dead or
    /// removed, fire immunity, or the `Invulnerable` flag nothing but
    /// `#bypasses_invulnerability` gets past. None for a stale handle or a
    /// kind with no such decision (the player's paths answer their refusals
    /// at hurt time). #924.
    #[access(WorldOnly)]
    fn entity_invulnerable_to_damage(&self, entity: EntityRef, damage_type: &str) -> Option<bool>;
    /// Side: server.
    /// Whether `source`'s damage may hit `entity` through their teams
    /// (`Player.canHarmPlayer`, `Team.allowFriendlyFire`,
    /// `DamageSource.doTeamsAllowDamage`): allies without friendly fire
    /// cannot. Entities without a team always can. None for a stale handle.
    /// #924.
    #[access(WorldOnly)]
    fn entity_entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        source: EntityRef,
    ) -> Option<bool>;
    /// Side: server.
    /// `Entity.isInvulnerableToPiercingWeapon` (`isInvulnerable`): the
    /// `Invulnerable` flag; the temporary window `Entity.invulnerableTime`
    /// opens does not exist yet (#916), so no hit answers through it. None
    /// for a stale handle or a kind without the flag. #924.
    #[access(WorldOnly)]
    fn entity_invulnerable_to_piercing_weapon(&self, entity: EntityRef) -> Option<bool>;

    /// Side: both.
    /// The entity's velocity in blocks a tick (`Entity.getDeltaMovement`). A
    /// player's is the move its client last reported (`getKnownMovement`); an
    /// entity Horizon doesn't simulate (a display, a painting) has none, so
    /// zero. On a client, another entity's is the motion the server last
    /// sent it (`Entity.lerpMotion`). API 379.
    #[access(WorldOnly)]
    fn entity_velocity(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// Whether the entity stands on a block (`Entity.onGround`).
    /// Whether the entity stands on a block (`Entity.onGround`). API
    /// 379.
    #[access(WorldOnly)]
    fn entity_on_ground(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The entity's box (`Entity.getBoundingBox`): its lowest and highest
    /// corner. None for the kinds whose box Horizon doesn't keep (displays,
    /// markers, End crystals, the dragon); a dragon part's is its own. API 379.
    #[access(WorldOnly)]
    fn entity_bounding_box(&self, entity: EntityRef) -> Option<[[f64; 3]; 2]>;
    /// Side: server.
    /// The height of the entity's eyes above its feet (`Entity.getEyeHeight`).
    /// API 379.
    #[access(WorldOnly)]
    fn entity_eye_height(&self, entity: EntityRef) -> Option<f64>;
    /// Side: server.
    /// Where the entity's eyes are (`Entity.getEyePosition`). API
    /// 379.
    #[access(WorldOnly)]
    fn entity_eye_position(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// The unit vector the entity looks along (`Entity.getViewVector`, from
    /// its pitch and yaw). API 379.
    #[access(WorldOnly)]
    fn entity_view_vector(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: both.
    /// The unit vector that points up out of the entity's head
    /// (`Entity.getUpVector`): its view vector turned a quarter of a turn
    /// toward the zenith, so a level look gives straight up and a look down
    /// leans it forward. From the entity's pitch and yaw.
    #[access(WorldOnly)]
    fn entity_up_vector(&self, entity: EntityRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// The block the entity stands on (`Entity.getOnPos`): the supporting block
    /// of an entity on the ground (`Entity.mainSupportingBlockPos`, the solid
    /// block under its box nearest its position), else the block just under
    /// its feet. A dragon part, which never stands, has the block under its
    /// feet. None for a stale handle.
    #[access(WorldOnly)]
    fn entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]>;
    /// Side: server.
    /// A ray from the entity's eyes along its view vector, `distance` blocks
    /// long, against block outlines with an empty collision context and no
    /// fluids (`Entity.pick(distance, 1.0F, false)`): what it looks at. A
    /// miss is the hit with `miss` set. Entities are not clipped: that is
    /// `LevelRef::ray_trace_entities`. None for a stale handle, a
    /// `distance` that is negative or not finite, and where `LevelRef::clip`
    /// is None.
    #[access(WorldOnly)]
    fn entity_pick(&self, entity: EntityRef, distance: f64) -> Option<ClipHit>;
    /// Side: both.
    /// Whether the entity is made of parts that are entities of their own
    /// (`Entity.isMultipartEntity`): the ender dragon. Its parts are not the
    /// mod API's behaviour Parts. A part is not itself multipart. None for a
    /// stale handle.
    #[access(WorldOnly)]
    fn entity_is_multipart(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// The entities a multipart entity is made of (`Entity.getParts`), in
    /// `getSubEntities` order: for the dragon its head, neck, body, three
    /// tail segments and two wings. Each is a handle of the part's own entity
    /// id, the eight after the dragon's, with the dragon's type, its own
    /// position, rotation, velocity and box, and no living state; the
    /// handle stops resolving with the dragon. Empty for an entity that is not
    /// multipart, None for a stale handle.
    #[access(WorldOnly)]
    fn entity_parts(&self, entity: EntityRef) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// The square of the distance from the entity's feet to `pos`
    /// (`Entity.distanceToSqr`). API 379.
    #[access(WorldOnly)]
    fn entity_distance_to_sqr(&self, entity: EntityRef, pos: [f64; 3]) -> Option<f64>;
    /// Side: server.
    /// The yaw of the entity's head (`Entity.getYHeadRot`): a mob's or a
    /// player's own, zero for the kinds without a head.
    #[access(WorldOnly)]
    fn entity_head_yaw(&self, entity: EntityRef) -> Option<f32>;
    /// Side: server.
    /// The horizontal direction the entity faces (`Entity.getDirection`):
    /// `north`, `south`, `east` or `west`.
    #[access(WorldOnly)]
    fn entity_direction(&self, entity: EntityRef) -> Option<&'static str>;
    /// Side: server.
    /// The entity's width and height (`Entity.getBbWidth`, `getBbHeight`):
    /// its box's, so a baby's or a scaled entity's own.
    #[access(WorldOnly)]
    fn entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]>;
    /// Side: server.
    /// `Entity.noPhysics`: a player's is whether it spectates (`Player.tick`
    /// sets it from `isSpectator`); a mob's is false (a vex sets it only for
    /// the length of its own tick); an arrow's or a trident's is its
    /// `AbstractArrow.noPhysics`, set once a loyal trident turns back. The
    /// ender dragon, an area-effect cloud, an ominous item spawner and a
    /// shulker bullet set it in their constructors and never clear it.
    #[access(WorldOnly)]
    fn entity_no_physics(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// What the entity rides (`Entity.getVehicle`): `None` when it rides
    /// nothing, its actual vehicle is retained off-level and has no resident handle,
    /// or the supplied handle is stale. `is_passenger` still reports the relationship.
    #[access(WorldOnly)]
    fn entity_vehicle(&self, entity: EntityRef) -> Option<EntityRef>;
    /// Side: server.
    /// Whether the entity rides another (`Entity.isPassenger`), including a retained
    /// off-level vehicle that cannot be returned by `vehicle`. Stale handles return `None`.
    #[access(WorldOnly)]
    fn entity_is_passenger(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The lowest vehicle under the entity (`Entity.getRootVehicle`): the
    /// entity itself when it rides nothing. Traverses retained off-level intermediaries,
    /// but returns `None` if the actual root has no resident handle, the supplied handle
    /// is stale, or the relationship contains a cycle. Never substitutes a descendant.
    #[access(WorldOnly)]
    fn entity_root_vehicle(&self, entity: EntityRef) -> Option<EntityRef>;
    /// Side: server.
    /// The entities riding this one (`Entity.getPassengers`), in `addPassenger`'s order: each
    /// behind those aboard, except that a player goes first when the first is no player.
    /// Only resident handles are returned; retained off-level passengers are omitted.
    /// Dying passengers stay aboard until removed.
    #[access(WorldOnly)]
    fn entity_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// Whether anything rides the entity (`Entity.isVehicle`), including retained
    /// off-level passengers omitted from `passengers`. Dying is not removal.
    #[access(WorldOnly)]
    fn entity_is_vehicle(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Every entity riding this one, at any depth (`Entity.getIndirectPassengers`): each
    /// passenger in boarding order, followed by those riding it. Traverses retained
    /// off-level intermediaries, omitting only their handles from the result; resident
    /// descendants below them are still returned.
    #[access(WorldOnly)]
    fn entity_indirect_passengers(&self, entity: EntityRef) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// The first passenger, when it steers the entity (`Entity.getControllingPassenger`): a
    /// player in a boat, a player on a mount that is ready for one, or a mob on a mob with AI
    /// unless `#non_controlling_rider` names it. `None` when nothing steers it, or for a stale
    /// handle. The actual first passenger is chosen before filtering by residency:
    /// an off-level first passenger returns `None`, never a later resident substitute.
    #[access(WorldOnly)]
    fn entity_controlling_passenger(&self, entity: EntityRef) -> Option<LivingRef>;
    /// Side: server.
    /// Whether both entities share a root vehicle (`Entity.isPassengerOfSameVehicle`): true for
    /// an entity and itself, false for two distinct entities riding nothing. Compares
    /// actual object roots even when they are retained off-level and have no handle.
    /// `None` for stale handles or a cycle with no root.
    #[access(WorldOnly)]
    fn entity_is_passenger_of_same_vehicle(
        &self,
        entity: EntityRef,
        other: EntityRef,
    ) -> Option<bool>;
    /// Side: server.
    /// Where getting off this vehicle puts `passenger` as it stands now, riding it or not. This
    /// is the game's own placement, from the code `stop_riding` runs, so it can differ from
    /// `Entity.getDismountLocationForPassenger`. A player leaves a boat ahead of them or on top
    /// of it, a cart beside or on top of it, a strider by vanilla's search, a happy ghast on
    /// top, and any other mob beside or on top of it. A mob leaves a boat as a player does, a
    /// cart by vanilla's search, a horse, camel or llama at its side or feet, and any other mob
    /// on top: vanilla's searches for `Animal` and the strider don't reach mobs yet, and those
    /// for horses, `Animal` and the cart don't reach players. Nothing rides anything else, so
    /// there it is the top of the box. Reads only: no `mob_ride` or dismount veto is asked and
    /// the passenger's pose stays. `None` for a stale handle, a passenger other than a player
    /// or a mob, a player with no motion state, or a vehicle whose box Horizon does not keep.
    #[access(WorldOnly)]
    fn entity_dismount_location(
        &self,
        vehicle: EntityRef,
        passenger: LivingRef,
    ) -> Option<[f64; 3]>;
    /// Side: server.
    /// The live holder of a mob's or boat's lead (`Leashable.getLeashHolder`), or `Unleashed`
    /// when no lead exists. `None` means an entity that takes no lead, or that the entity,
    /// holder or saved holder is unavailable, stale or in a parked level. Does not load a level.
    #[access(WorldOnly)]
    fn entity_leash_holder(&self, entity: EntityRef) -> Option<LeashHolder>;
    /// Side: server.
    /// `Entity.getSoundSource`: the group an entity's sounds play under.
    /// Players are `Players`; `Monster`s, the dragon, hoglins, ghasts,
    /// phantoms, shulkers, slimes and a killer bunny are `Hostile`; dropped
    /// items and experience orbs `Ambient`; every other entity `Neutral`.
    #[access(WorldOnly)]
    fn entity_sound_source(&self, entity: EntityRef) -> Option<SoundSource>;
    /// Side: server.
    /// `Entity.isSpectator`: true for a player in spectator mode, false for
    /// every other entity.
    #[access(WorldOnly)]
    fn entity_spectator(&self, entity: EntityRef) -> Option<bool>;
    /// Side: both.
    /// `isSecondaryUseActive`: the local client's live semantic sneak state (not the
    /// physical key binding or last tick's crouching pose); the server's shift-key flag.
    /// False for non-player entities, None for a stale or unreachable handle.
    #[access(WorldOnly)]
    fn entity_is_shift_key_down(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity crouches (`Entity.isCrouching`: the crouching
    /// pose).
    #[access(WorldOnly)]
    fn entity_is_crouching(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity sprints (`Entity.isSprinting`): a player, or a
    /// cat or ocelot dashing after its target (their shared sprinting flag);
    /// false for the other kinds.
    #[access(WorldOnly)]
    fn entity_is_sprinting(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity touched water at its last fluid update
    /// (`Entity.isInWater`): players and mobs.
    #[access(WorldOnly)]
    fn entity_is_in_water(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity is in lava (`Entity.isInLava`): players and mobs.
    #[access(WorldOnly)]
    fn entity_is_in_lava(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Whether the entity's type never burns (`EntityType.fireImmune`):
    /// players and mobs.
    #[access(WorldOnly)]
    fn entity_fire_immune(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The ticks the entity burns for (`Entity.getRemainingFireTicks`):
    /// players, mobs, armor stands and items.
    #[access(WorldOnly)]
    fn entity_fire_ticks(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// `Entity.getPortalCooldown`: a player's own portal state, the falling
    /// block's, a pearl's End gateway state, or the level's for the other
    /// kinds its portal walk carries (zero when it keeps no state for one).
    #[access(WorldOnly)]
    fn entity_portal_cooldown(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// The entity's air (`Entity.getAirSupply`): players and mobs.
    #[access(WorldOnly)]
    fn entity_air_supply(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// The most air the entity holds (`Entity.getMaxAirSupply`): 300, a
    /// dolphin's 4800, an axolotl's 6000.
    #[access(WorldOnly)]
    fn entity_max_air_supply(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Whether the mob makes no sounds (`Entity.isSilent`, its `Silent`
    /// tag); false for a player.
    #[access(WorldOnly)]
    fn entity_is_silent(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the mob's persistent glowing tag (`Entity.hasGlowingTag`).
    #[access(WorldOnly)]
    fn entity_glowing(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the mob's no-gravity flag (`Entity.isNoGravity`).
    #[access(WorldOnly)]
    fn entity_no_gravity(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// Returns the entity's frozen ticks (`Entity.getTicksFrozen`).
    #[access(WorldOnly)]
    fn entity_ticks_frozen(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Ticks the entity has lived (`Entity.tickCount`): mobs, item entities,
    /// experience orbs and minecarts. Wraps as vanilla's `int` does.
    #[access(WorldOnly)]
    fn entity_tick_count(&self, entity: EntityRef) -> Option<i32>;
    /// Side: server.
    /// Whether the entity's model is hidden (`Entity.isInvisible`): the
    /// invisibility effect on a player or a mob, a mob's or an armor stand's
    /// flag.
    #[access(WorldOnly)]
    fn entity_is_invisible(&self, entity: EntityRef) -> Option<bool>;
    /// Side: server.
    /// The entity's pose (`Entity.getPose`) as `Pose` names it: `standing`,
    /// `crouching`, `swimming`, `sleeping`, a warden's `roaring`... Kinds
    /// with no pose stand.
    #[access(WorldOnly)]
    fn entity_pose(&self, entity: EntityRef) -> Option<&'static str>;
}
/// The handles' writes of the `entities` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait EntitiesBackendMut {
    /// Side: server.
    /// The entity's data as `/data get entity` shows it
    /// (`Entity.saveWithoutId`): its compound without `id`. An entity with no
    /// UUID yet gets the one it will be saved with. API 386.
    #[access(WorldOnly)]
    fn entity_nbt(&mut self, entity: EntityRef) -> Option<Compound>;
    /// Side: server.
    /// Merges `nbt` into the entity's data and loads the entity from it, as
    /// `/data merge entity` does (`EntityDataAccessor.setData`): true when it
    /// changed. False for a player (vanilla refuses them too), a kind Horizon
    /// can't reload yet (only mobs, dropped items, armor stands and item
    /// frames), data nested too deep, or data that changes nothing. API
    /// 386.
    #[access(WorldOnly)]
    fn entity_merge_nbt(&mut self, entity: EntityRef, nbt: Compound) -> bool;
    /// Side: server.
    /// Adds tag `tag` (`Entity.addTag`): false when it has it already, holds
    /// 1024, its kind can't store tags yet, or the handle is stale. API 386.
    #[access(WorldOnly)]
    fn entity_add_tag(&mut self, entity: EntityRef, tag: &str) -> bool;
    /// Side: server.
    /// Removes tag `tag` (`Entity.removeTag`): false when it didn't have it.
    /// API 386.
    #[access(WorldOnly)]
    fn entity_remove_tag(&mut self, entity: EntityRef, tag: &str) -> bool;
    /// Side: server.
    /// Names the entity (`Entity.setCustomName`), or takes its name away
    /// with `None`: its name, its saved `CustomName` and, for a mob or an
    /// armor stand, what the players tracking it are sent. False for a
    /// player, a kind whose name Horizon keeps nowhere yet (projectiles, the
    /// dragon and crystals, primed TNT, falling blocks, experience orbs,
    /// ominous item spawners), a stale handle or on a client.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn entity_set_custom_name(
        &mut self,
        entity: EntityRef,
        name: Option<Text>,
    ) -> Result<bool, TextFormError>;
    /// Side: server.
    /// Shows the entity's name at any distance, or only when looked at
    /// (`Entity.setCustomNameVisible`): saved, and sent to the players
    /// tracking a mob or an armor stand. False where `set_custom_name` is.
    #[access(WorldOnly)]
    fn entity_set_custom_name_visible(&mut self, entity: EntityRef, visible: bool) -> bool;
    /// Side: server.
    /// `Entity.resetFallDistance`: how far the entity has fallen is forgotten, so its next
    /// landing hurts it as a step does (a fan's or a launcher's push, a mod's soft block).
    /// Players too, the acting one and guests. False for a stale handle or an entity not in
    /// the level in place; true for a kind that keeps no fall distance, which has none to forget.
    #[access(WorldOnly)]
    fn entity_reset_fall_distance(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// The entity's UUID (`Entity.getUUID`), the one it is saved with; an
    /// entity that has none yet gets it now, as a new vanilla entity does.
    /// On a client, the one the server added it with (a player's is its
    /// profile's). API 373.
    #[access(WorldOnly)]
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128>;
    /// Side: server.
    /// `Entity.setInvulnerable`: sets a mob's `Invulnerable` flag, saved
    /// with it. False for the other kinds.
    #[access(WorldOnly)]
    fn entity_set_invulnerable(&mut self, entity: EntityRef, invulnerable: bool) -> bool;
    /// Side: server.
    /// Sets `LivingEntity.damageCooldownTime`, as mods do to let rapid hits
    /// through (0). Mobs and players.
    #[access(WorldOnly)]
    fn entity_set_damage_cooldown_time(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// `Entity.setInvulnerableTime` (#916): the write side of
    /// `entity_invulnerable_time`. False where the entity is gone or keeps
    /// no such field.
    #[access(WorldOnly)]
    fn entity_set_invulnerable_time(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// `Entity.hurtServer(level, source, amount)` of `amount` from a source
    /// of `damage_type` (`minecraft:generic`'s shape), `direct` the entity
    /// that dealt it (a shot arrow) and `causing` the one responsible for it
    /// (its shooter): the engine's own hurt paths take it — the damage type,
    /// the attacker, armor, the damage cooldown and every existing hurt hook
    /// kept, the damage event the hit already broadcasts is the broadcast —
    /// by kind: the player's and mobs' for the living, the item, vehicle,
    /// dragon and crystal branches for the damageable rest. True when the
    /// damage was taken. None for a stale handle, false for a kind that
    /// cannot be hurt or an unregistered damage type. Stale direct/causing
    /// handles are treated as absent sources. #924.
    #[access(WorldOnly)]
    fn entity_hurt(
        &mut self,
        entity: EntityRef,
        damage_type: &str,
        amount: f32,
        direct: Option<EntityRef>,
        causing: Option<EntityRef>,
    ) -> Option<bool>;
    /// Side: server.
    /// Sets the entity's velocity (`Entity.setDeltaMovement`); clients see it
    /// as they see the entity move. False for a player (its client moves
    /// it), an entity Horizon doesn't simulate, a value that isn't finite,
    /// or on a client. API 379.
    #[access(WorldOnly)]
    fn entity_set_velocity(&mut self, entity: EntityRef, velocity: [f64; 3]) -> bool;
    /// Side: server.
    /// Moves the entity to `pos` in its level (`Entity.teleportTo`), as
    /// `/tp` does: a player is placed there and told, a rider gets off. With
    /// `rotation` (yaw, pitch in degrees) it also turns. False for a position
    /// outside the world's bounds, for wall-mounted entities or dragon parts, or when the
    /// requested rotation cannot be stored.
    #[access(WorldOnly)]
    fn entity_teleport_to(
        &mut self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool;
    /// Side: server.
    /// Sends the entity to `pos` in the level `dimension` (`minecraft:the_nether`,
    /// or a level a mod registered), as `/execute in <dimension> run tp` does
    /// (`Entity.teleportTo(ServerLevel, ..)`): `pos` is in that level's own
    /// coordinates (nothing scales it by `coordinate_scale`), a level nobody has
    /// entered opens first, and a vehicle takes its passengers along. A player
    /// arrives after the current tick, and `player_teleported` and
    /// `player_changed_dimension` report the move. In the entity's own level
    /// it is `entity_teleport_to`. False for what `entity_teleport_to` refuses,
    /// a dimension that isn't registered, and kinds that cannot change level
    /// (all but a player, a mob, a boat or a minecart). A fake player stays
    /// where it is, and the call is still true. Nothing builds ground at the
    /// destination.
    #[access(WorldOnly)]
    fn entity_teleport_to_level(
        &mut self,
        entity: EntityRef,
        dimension: &str,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool;
    /// Side: server.
    /// Turns the entity to `yaw` and `pitch` in degrees (`Entity.setYRot`,
    /// `setXRot`): the pitch wraps and is clamped to -90..90. False for kinds
    /// whose rotation Horizon does not fully store, or an unavailable player.
    /// Does not teleport or dismount.
    #[access(WorldOnly)]
    fn entity_set_rotation(&mut self, entity: EntityRef, yaw: f32, pitch: f32) -> bool;
    /// Side: server.
    /// Turns a mob's head to `yaw` (`Entity.setYHeadRot`); its body follows
    /// as its AI turns it. Players' heads can turn too; false for other kinds
    /// or an unavailable player.
    #[access(WorldOnly)]
    fn entity_set_head_yaw(&mut self, entity: EntityRef, yaw: f32) -> bool;
    /// Side: server.
    /// Adds `delta` to the entity's velocity (`Entity.push`,
    /// `addDeltaMovement`). False for an unavailable player, an entity
    /// Horizon doesn't simulate, or a value that isn't finite.
    #[access(WorldOnly)]
    fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool;
    /// Side: server.
    /// Removes the entity with no death and no loot (`Entity.discard`): a
    /// chest boat or a minecart with a chest still drops what it holds, and
    /// a boat its lead, as vanilla's `remove` does. False for a player, a dragon
    /// part (it leaves with its dragon), or a stale handle.
    /// The tick scope below is the port's own transactional requirement:
    /// NeoForge adds no event, deferral or drop capture around a block entity's
    /// tick, and new item spawning stays immediate. With no scope open the
    /// engine behaves exactly as vanilla for these item operations: removal
    /// is immediate, no overlay is created, and nothing is staged.
    /// Dropped-item removal during a mod block-entity tick is staged with its
    /// final snapshot, and immediately hidden from that scope's reads. It is
    /// discarded on writeback rejection. Non-item removal and spawning are
    /// deliberately immediate and are not undone by rejection.
    #[access(WorldOnly)]
    fn entity_discard(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Removes the entity for `reason` (`Entity.remove`): `Discarded`, as
    /// `discard`, or `Killed`, which also runs a mob's effects that act on
    /// removal (Oozing's slimes, Weaving's cobwebs, Wind Charged's burst),
    /// still with no death and no loot. False for a player, a dragon part (it
    /// leaves with its dragon), or a stale handle.
    /// Dropped items share `entity_discard`'s block-entity tick staging;
    /// non-item removal is deliberately immediate.
    #[access(WorldOnly)]
    fn entity_remove(&mut self, entity: EntityRef, reason: RemovalReason) -> bool;
    /// Side: server.
    /// Drops `stack` at the entity's feet, `y_offset` above
    /// (`Entity.spawnAtLocation`), or 0.15 out along a frame or painting's
    /// horizontal facing (`HangingEntity.spawnAtLocation`), moving as a new item does, with the
    /// default pickup delay: the item entity. None for an empty stack or an
    /// unknown item, a stale handle, a player in another level, or an item a
    /// mod kept out.
    #[access(WorldOnly)]
    fn entity_spawn_at_location(
        &mut self,
        entity: EntityRef,
        stack: ItemStack,
        y_offset: f32,
    ) -> Result<Option<EntityRef>, StackWriteError>;
    /// Side: server.
    /// Attempts to board a vehicle through the shared server mount path.
    ///
    /// Players and mobs can ride a mob, boat, minecart or generic seat entity
    /// (an armor stand, display, marker or interaction). Returns false for
    /// unsupported entity pairs, stale or held-out handles, a full stored
    /// vehicle, a cycle, or a canceled mount or dismount. Mob vehicle capacity
    /// follows its species; minecarts store one passenger and boats store one
    /// mob and up to two passengers in total. Generic seats admit one passenger unless forced.
    /// `force` bypasses normal admission checks, not these storage limits.
    /// Player boarding cooldown and the strider's
    /// eye-in-lava admission check are not yet represented by the game.
    #[access(WorldOnly)]
    fn entity_start_riding(&mut self, entity: EntityRef, vehicle: EntityRef, force: bool) -> bool;
    /// Side: server.
    /// Gets the entity off its vehicle (`Entity.stopRiding`), where the
    /// vehicle lets it out, as `/ride ... dismount` does. False when it rode
    /// nothing, a mod's `mob_ride` hook kept it on, or for a stale handle.
    #[access(WorldOnly)]
    fn entity_stop_riding(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Gets every passenger off the entity (`Entity.ejectPassengers`), last
    /// first as vanilla walks the list, so `mob_ride` hears them in its
    /// order. False when nothing rode it, or for a stale handle.
    #[access(WorldOnly)]
    fn entity_eject_passengers(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Gives the holder the mob's or boat's lead (`Leashable.setLeashedTo`): the entity lets go
    /// of any other holder and gets off its vehicle, where a mod's `mob_ride` lets it. As
    /// there, neither `canBeLeashed` nor the distance is asked: a holder too far away snaps
    /// the lead on the next tick. False for a stale handle, the entity itself, a holder other
    /// than a player, mob or boat, or an entity that takes no lead.
    #[access(WorldOnly)]
    fn entity_leash_to(&mut self, entity: EntityRef, holder: EntityRef) -> bool;
    /// Side: server.
    /// Ties the mob's or boat's lead to the fence at `pos`, as `entity_leash_to` does to
    /// an entity; the knot is there while a lead hangs on it. False for a stale handle, an
    /// entity that takes no lead, or no fence at `pos`.
    #[access(WorldOnly)]
    fn entity_leash_to_knot(&mut self, entity: EntityRef, pos: [i32; 3]) -> bool;
    /// Side: server.
    /// Takes the lead off a mob or boat: the lead falls as an item with `drop_lead`
    /// (`Leashable.dropLeash`), and is gone without it (`removeLeash`). A mob's removal is heard
    /// by `entity_unleash`. False when no holder has the lead, for a stale handle, or for an
    /// entity that takes no lead. A holder still unresolved from a save is left alone.
    #[access(WorldOnly)]
    fn entity_detach_leash(&mut self, entity: EntityRef, drop_lead: bool) -> bool;
    /// Side: server.
    /// `Entity.playSound(sound, volume, pitch)`: plays sound event `sound` at
    /// the entity under its sound source, for the players within range. A
    /// silent mob plays nothing, as vanilla's check is `isSilent`; a player
    /// (`Player.playSound`) doesn't hear it themself, their own client plays
    /// it. True means queued, not heard; false for an unknown `minecraft:`
    /// event or a volume or pitch that isn't a finite non-negative number.
    #[access(WorldOnly)]
    fn entity_play_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        volume: f32,
        pitch: f32,
    ) -> bool;
    /// Side: server.
    /// `Entity.gameEvent(event)`: game event `event` (`minecraft:step`...)
    /// at the entity's position with the entity as its source, which sculk
    /// sensors, shriekers, wardens and allays in range hear. A player or mob
    /// is the event's source entity; other entities retain their own id.
    /// False for an event vanilla doesn't have, a dead guest and a guest while
    /// another one acts.
    #[access(WorldOnly)]
    fn entity_game_event(&mut self, entity: EntityRef, event: &str) -> bool;
    /// Side: server.
    /// `Level.broadcastEntityEvent(entity, id)`: sends entity event `id` (the
    /// byte `Entity.handleEntityEvent` reads: `2` is a hurt flash, `3` a
    /// death, `35` a totem...) to the players tracking the entity and to the
    /// entity itself. True means queued; false for an entity outside the
    /// level in place.
    #[access(WorldOnly)]
    fn entity_broadcast_event(&mut self, entity: EntityRef, id: u8) -> bool;
    /// Side: server.
    /// Sets the remaining fire ticks (`Entity.setRemainingFireTicks`).
    #[access(WorldOnly)]
    fn entity_set_remaining_fire_ticks(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// Clears positive fire ticks while retaining fire immunity (`Entity.clearFire`).
    #[access(WorldOnly)]
    fn entity_clear_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// `Entity.extinguishFire`: plays the ordinary extinguish sound when the
    /// entity is burning, then clears positive fire ticks. Fire-immune mobs
    /// and fire-resistant dropped stacks do not play a sound; negative
    /// fire-immunity countdowns are retained.
    /// Uses the engine's entity/cosmetic random and normal sound route.
    /// Supports players, guests, mobs, armor stands and dropped items. False
    /// for a stale or unsupported entity.
    /// Unlike this operation, `clear_fire` is always silent and draws no random.
    #[access(WorldOnly)]
    fn entity_extinguish_fire(&mut self, entity: EntityRef) -> bool;
    /// Side: server.
    /// Sets the entity's air (`Entity.setAirSupply`).
    #[access(WorldOnly)]
    fn entity_set_air_supply(&mut self, entity: EntityRef, air: i32) -> bool;
    /// Side: server.
    /// Silences the mob or lets it make sounds again (`Entity.setSilent`).
    /// False for the other kinds.
    #[access(WorldOnly)]
    fn entity_set_silent(&mut self, entity: EntityRef, silent: bool) -> bool;
    /// Side: server.
    /// Sets the mob's persistent glowing tag (`Entity.setGlowingTag`). Returns false for players and guests.
    #[access(WorldOnly)]
    fn entity_set_glowing(&mut self, entity: EntityRef, glowing: bool) -> bool;
    /// Side: server.
    /// Sets the mob's no-gravity flag (`Entity.setNoGravity`). Returns false for players and guests.
    #[access(WorldOnly)]
    fn entity_set_no_gravity(&mut self, entity: EntityRef, no_gravity: bool) -> bool;
    /// Side: server.
    /// Sets the entity's frozen ticks (`Entity.setTicksFrozen`).
    #[access(WorldOnly)]
    fn entity_set_ticks_frozen(&mut self, entity: EntityRef, ticks: i32) -> bool;
    /// Side: server.
    /// Sets the entity's shared invisibility flag (`Entity.setInvisible`). Returns false for players and guests.
    #[access(WorldOnly)]
    fn entity_set_invisible(&mut self, entity: EntityRef, invisible: bool) -> bool;
    /// Side: server.
    /// `Level.playSound(null, entity, sound, source, volume, pitch)`: plays
    /// sound event `sound` bound to the entity, so that it follows it, for
    /// the players within range (16 blocks, times the volume above 1) of
    /// where it is now, the entity's own player too. True means queued, not
    /// heard; false for an entity outside the level in place.
    #[access(WorldOnly)]
    fn entity_play_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool;
}
/// The handles' client writes of the `entities` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait EntitiesClientBackendMut {
    /// Side: client.
    /// Spawn sprint particles when called explicitly by a client mod. A
    /// valid entity returns true even if the block beneath it is invisible.
    /// Returns false for stale, untracked, server-side, or unsupported entities.
    #[access(WorldOnly)]
    fn entity_sprint_particles(&mut self, entity: EntityRef) -> bool;
}
