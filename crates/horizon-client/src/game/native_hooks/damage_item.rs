use super::*;

#[cfg(test)]
thread_local! {
    static DISPATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Game<'_> {
    #[cfg(test)]
    pub(in crate::game) fn damage_item_dispatches(&self) -> usize {
        DISPATCHES.with(std::cell::Cell::get)
    }

    /// `IItemExtension.damageItem`: `stack` is about to lose `amount` points of
    /// durability (`ItemStack.hurtAndBreak`), after Unbreaking. Change `amount`
    /// (0 or less leaves the item whole) or set `canceled` for no wear.
    /// Server-side.
    pub(in crate::game) fn native_damage_item(
        &mut self,
        stack: &ItemStack,
        amount: &mut i32,
        canceled: &mut bool,
    ) {
        #[cfg(test)]
        DISPATCHES.with(|count| count.set(count.get() + 1));
        self.fire_damage_item(stack, amount, canceled);
    }
}
