//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnEntry {
    pub entity: String,
    pub weight: u32,
    pub min: u32,
    pub max: u32,
    /// A `ConstantInt` count (sampling draws nothing).
    pub constant: bool,
}
/// A biome's colours as a mod may change them (`native_biome_colors`), each
/// 0xRRGGBB: what tints grass (before the dark forest's modifier; the swamp's
/// noise colours replace it), leaves and vines, dry foliage (leaf litter) and
/// water.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeColors {
    pub grass: u32,
    pub foliage: u32,
    pub dry_foliage: u32,
    pub water: u32,
}
/// A biome's climate, as its worldgen file declares it (`Biome.ClimateSettings`):
/// what `level_biome_properties` reads. The temperature is the biome's base one; the
/// height and frozen-ocean adjustments of `Biome.getHeightAdjustedTemperature` are not applied.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeProperties {
    /// `temperature`: the base temperature.
    pub temperature: f32,
    /// `downfall`.
    pub downfall: f32,
    /// `has_precipitation`: whether it rains or snows here at all.
    pub has_precipitation: bool,
    /// `temperature_modifier: frozen` (the frozen oceans).
    pub frozen: bool,
}
