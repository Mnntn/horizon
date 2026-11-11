//! A client block notification is not a save mutation.
use super::Game;

impl Game<'_> {
    pub(super) fn notify_block_clients(&mut self, pos: (i32, i32, i32)) {
        if let Some(log) = self.world.block_log.as_mut()
            && !log.contains(&pos)
        {
            log.push(pos);
        }
    }
}
