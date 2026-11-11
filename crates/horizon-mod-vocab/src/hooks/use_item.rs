//! The `use_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0037

pub trait Hooks {
    /// A player uses `stack` (right click) with `self.hand` on the block at
    /// `pos`, `face` the face clicked (`[0, 1, 0]` is the top), once the
    /// block itself let the click through (a chest still opens): where
    /// `ItemStack.useOn` starts. Set `claimed` to take the use: the hand
    /// swings and neither the item's `useOn` nor its `use` runs.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one); an item on cooldown is never used. `hand`, `hit`
    /// (`UseOnContext.getClickLocation`, `getClickedFace`, `isInside`) and
    /// `player` (the acting player's UUID) and `player_entity_id` (the server's
    /// entity id),.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        scope = "item",
        key = "stack",
        neoforge = "UseItemOnBlockEvent"
    )]
    fn use_on(
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        hand: Hand,
        hit: BlockHit,
        player: u128,
        player_entity_id: i32,
        claimed: &mut bool,
    );
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_on` let it through, where `Item.use`
    /// starts. Runs on both logical sides after the global right-click event.
    /// Return an interaction result: Success swings on the client, SuccessServer
    /// swings on the server, Consume handles without swinging. Fail skips this
    /// item's use without swinging, then the off hand may be tried.
    /// None lets later Parts answer; if none answers, the item's own use runs.
    /// Some(Pass) skips later Parts and continues to the item's own use.
    /// TryWithEmptyHand is block-phase-only and is treated as Pass here.
    /// Cooldown items are skipped.
    /// `hand` is the interaction hand of THIS click, including an instantaneous
    /// air use. Unlike this field, `living_used_item_hand` answers only during
    /// active item use. This is `PlayerInteractEvent.RightClickItem.getHand`.
    #[hook(
        side = "both",
        contract = "replace",
        out = "result",
        scope = "item",
        key = "stack",
        neoforge = "PlayerInteractEvent.RightClickItem"
    )]
    fn use_item(stack: &ItemStack, hand: Hand, result: &mut Option<InteractionResult>);
}
