use super::*;

/// The `HurtSource` of a death's blow. A melee hit's direct entity is the
/// one that caused it; a projectile's isn't known, so `direct` is 0.
pub(in crate::game) fn death_hurt_source(damage_type: &str, causing: u32) -> HurtSource<'_> {
    let direct = if matches!(
        damage_type,
        "minecraft:player_attack"
            | "minecraft:mob_attack"
            | "minecraft:mob_attack_no_aggro"
            | "minecraft:sting"
    ) {
        causing as i32
    } else {
        0
    };
    HurtSource {
        damage_type,
        causing: causing as i32,
        direct,
        position: None,
    }
}
