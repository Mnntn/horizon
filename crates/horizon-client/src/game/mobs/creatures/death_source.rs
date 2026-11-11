use super::*;

/// Damage types of a projectile: its own entity is the direct one, and
/// the game keeps no id for it.
const PROJECTILE_DAMAGE: [&str; 9] = [
    "minecraft:arrow",
    "minecraft:trident",
    "minecraft:fireball",
    "minecraft:unattributed_fireball",
    "minecraft:wither_skull",
    "minecraft:mob_projectile",
    "minecraft:thrown",
    "minecraft:wind_charge",
    "minecraft:fireworks",
];

/// The `HurtSource` of a death's blow. A melee hit's direct entity is the
/// one that caused it; a projectile's isn't known, so `direct` is 0.
pub(in crate::game) fn death_hurt_source(damage_type: &str, causing: u32) -> HurtSource<'_> {
    let direct = if PROJECTILE_DAMAGE.contains(&damage_type) {
        0
    } else {
        causing as i32
    };
    HurtSource {
        damage_type,
        causing: causing as i32,
        direct,
        position: None,
    }
}
