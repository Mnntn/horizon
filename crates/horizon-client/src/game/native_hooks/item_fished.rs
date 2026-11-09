use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player `player` (UUID) and `player_entity_id` reeled in a
    /// catch (`ItemFishedEvent`, in `FishingHook.retrieve` after the fishing
    /// loot rolled): `fished` holds the bobber and the items, and
    /// `rod_damage` the damage the rod takes (vanilla's 1, or 2 with the
    /// bobber on the ground), which the mods may set (`damageRodBy`, not
    /// negative). Set `cancel` and the player receives nothing: no items,
    /// experience or statistic, and no `fishing_rod_hooked` trigger; the
    /// bobber is gone and the rod
    /// still takes `rod_damage`. After `player_fish`'s reel-in. Server-side.
    pub(in crate::game) fn native_item_fished(
        &mut self,
        player: u128,
        player_entity_id: i32,
        fished: &ItemFished<'_>,
        rod_damage: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_item_fished(player, player_entity_id, fished, rod_damage, cancel);
    }
}
