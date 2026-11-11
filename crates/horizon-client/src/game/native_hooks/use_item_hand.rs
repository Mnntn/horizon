use super::*;

impl Game<'_> {
    pub(in crate::game) fn client_item_interaction(
        &mut self,
        hand: Hand,
        stack: &ItemStack,
    ) -> Option<bool> {
        if let Some(answer) = self.right_click_item(hand, stack) {
            if answer == mod_api::InteractionResult::Success {
                self.with_hand(hand, Game::swing_arm);
            }
            return Some(answer.consumes_action());
        }
        self.with_hand(hand, |g| g.use_item_in_hand(stack))
    }

    /// Dispatch the item Part and apply the result on this logical side.
    pub(in crate::game) fn use_item_in_hand(&mut self, stack: &ItemStack) -> Option<bool> {
        use mod_api::InteractionResult as Result;
        let mut result = None;
        if mod_api::__private::has_part_hook(mod_api::HookId::UseItem)
            && let Ok(snapshot) = stack.try_into_api()
        {
            self.native_use_item(mod_api::__private::UseItemCall {
                stack: &snapshot,
                hand: self.hand,
                result: &mut result,
            });
        }
        match result? {
            Result::Pass | Result::TryWithEmptyHand => None,
            Result::Success => {
                if self.remote.is_some() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::SuccessServer => {
                if self.remote.is_none() {
                    self.swing_arm();
                }
                Some(true)
            }
            Result::Consume => Some(true),
            Result::Fail => Some(false),
            _ => None,
        }
    }
}
