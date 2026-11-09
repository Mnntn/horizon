use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `ClientInformationUpdatedEvent`: a server player's client sent new
    /// client information (language, view distance, chat, skin parts, main hand,
    /// filtering, listing, particles) in play. `old` and `new` are the values before and
    /// after this packet, one call per packet; they can be equal. The join's own
    /// information (the configuration phase, `ServerPlayer`'s constructor) is not
    /// reported. The player is not swapped in. Server-side.
    pub(in crate::game) fn native_client_information_updated(
        &mut self,
        player: u128,
        player_entity_id: i32,
        name: &str,
        old: &mod_api::ClientInfo,
        new: &mod_api::ClientInfo,
    ) {
        self.fire_client_information_updated(player, player_entity_id, name, old, new);
    }
}
