use super::*;

impl Game<'_> {
    pub(super) fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }
}
