//! The `interaction_key_mapping_triggered` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0143

pub trait Hooks {
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. The projectile flies on and is not discarded, so a mod
    /// that cancels also decides what becomes of it. It continues through its
    /// whole motion and may hit something on a later tick. Covers what
    /// `tick_projectiles` flies: arrows, tridents, snowballs,
    /// eggs, pearls, potions, fireballs, wither skulls, wind charges, llama spit and
    /// shulker bullets, from players, mobs and dispensers. It does not fire for
    /// a breeze's deflection, for a hit on a wind charge, for fireworks or for
    /// fishing bobbers. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Heard only when a mod also sets
    /// `native_hears_projectile_impacts`. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ProjectileImpactEvent",
        cancellable = true
    )]
    fn projectile_impact(
        dimension: &str,
        projectile: u32,
        kind: &str,
        owner: u32,
        entity: Option<u32>,
        entity_kind: &str,
        block: [i32; 3],
        face: [i32; 3],
        at: [f64; 3],
        canceled: &mut bool,
    );
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding. Players hear boarding and dismounts by input, `/ride`, hits,
    /// bucking, teleport or spectator mode; mobs hear boarding and dismounts (jockeys,
    /// ravager raiders, `/ride`, hits), including removal cleanup. Live cancellation keeps
    /// the relationship. Actual boat/cart destruction currently unlinks remaining Mob
    /// edges after notification even if canceled, avoiding dangling vehicles; preserving
    /// canceled relationships is deferred (#2446). Loading a save does not notify. The
    /// acting player is the one whose action runs, a guest on a server. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityMountEvent",
        cancellable = true
    )]
    fn entity_mount(
        dimension: &str,
        entity: u32,
        kind: &str,
        vehicle: u32,
        vehicle_kind: &str,
        mounting: bool,
        canceled: &mut bool,
    );
    /// An interaction mapping is about to act on the tick's crosshair target. `attempt`
    /// distinguishes a queued press from held use or mining. Use asks for each hand tried,
    /// main first. Cancelling the main hand ends the click. A cancelled attack or use still
    /// swings unless the listener clears `swing_hand`. Fires on the window client.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.InteractionKeyMappingTriggered",
        version = 2,
        cancellable = true
    )]
    fn interaction_key_mapping_triggered();
}
