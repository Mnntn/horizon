use super::*;

impl Game<'_> {
    /// The game mode's item-phase trigger is separate from item-internal triggers.
    pub(super) fn finish_item_block_use(
        &mut self,
        hit: Option<&Hit>,
        original: &ItemStack,
    ) -> horizon_client::Result<bool> {
        if let Some(hit) = hit {
            self.trigger_block("minecraft:item_used_on_block", hit.pos, original);
        }
        Ok(true)
    }

    /// Complete one consuming block-use packet, after its inner criterion.
    /// Sees the hand after the use; a replacement stack is visible, unlike
    /// vanilla\'s original object.
    pub(super) fn finish_block_use(&mut self, pos: [i32; 3], hand: Hand) {
        let stack = self
            .with_hand(hand, |g| g.held().cloned())
            .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
        horizon_client::spans::time("use_advancements", || {
            self.trigger_any_block_use(pos, &stack);
        });
    }
}
