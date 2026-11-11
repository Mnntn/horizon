//! `LivingEntity.onClimbable` for the player: the block it stands in, and the mods' say on whether
//! that block is a ladder (`native_is_ladder`, in `motion`).
use super::*;

impl ClimbingWorld for MotionWorld<'_, '_, '_> {
    type Error = std::convert::Infallible;
    fn is_spectator(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.spectator)
    }
    fn block_position(&mut self) -> std::result::Result<BlockPos, Self::Error> {
        Ok(BlockPos::containing(self.player.position))
    }
    fn in_block_state(&mut self) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(BlockPos::containing(self.player.position)))
    }
    fn is_fall_flying(&mut self) -> std::result::Result<bool, Self::Error> {
        Ok(self.player.fall_flying)
    }
    fn block_state(
        &mut self,
        position: BlockPos,
    ) -> std::result::Result<BlockStateId, Self::Error> {
        Ok(self.block(position))
    }
    fn set_last_climbable_position(&mut self, position: BlockPos) {
        self.player.last_climbable = Some(position);
    }
    fn adjust_ladder(&mut self, position: BlockPos, state: BlockStateId, tagged: bool) -> bool {
        let mut ladder = tagged;
        native_is_ladder(
            self.loop_context,
            self.player.entity_id as u32,
            position,
            state,
            &mut ladder,
        );
        ladder
    }
}
