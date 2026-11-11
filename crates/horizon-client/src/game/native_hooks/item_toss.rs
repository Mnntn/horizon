use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `player` tosses `count` of `item` (`ItemTossEvent`, from
    /// `Player.drop`: the drop key, a menu's throw, what no longer fits),
    /// not what scatters when they die. Set `canceled` and no item entity
    /// is made; the items are already out of the inventory and are lost.
    /// Server-side.
    pub(in crate::game) fn native_item_toss(
        &mut self,
        player: u128,
        item: &str,
        count: u32,
        canceled: &mut bool,
    ) {
        self.fire_item_toss(player, item, count, canceled);
    }
}

#[cfg(test)]
mod item_toss_tests {
    use super::*;
    use mod_api::{Flow, Hooks, ItemTossEvent, ModEntry, ServerHost, testing::with_mods};
    use std::sync::{Arc, Mutex};

    /// Counts the tosses it hears and cancels each.
    struct Tosser(Arc<Mutex<u32>>);
    #[mod_api::__subscriptions]
    impl Hooks for Tosser {
        fn item_toss(&self, _host: &mut dyn ServerHost, _event: &mut ItemTossEvent<'_>) -> Flow {
            *self.0.lock().unwrap() += 1;
            Flow::Cancel
        }
    }

    /// A toss asks `item_toss` and a cancel stops the item entity; the
    /// random scatter of a death's drops is not a toss and is never asked.
    #[test]
    #[ignore = "needs prepared resources"]
    fn only_a_toss_fires_item_toss_not_the_death_scatter() {
        let heard = Arc::new(Mutex::new(0));
        let mods = vec![ModEntry::new("toss_probe", Box::new(Tosser(heard.clone())))];
        with_mods(mods.into(), || {
            crate::game::native_entity_refs::tests::with_frozen_game(|game| {
                let eye = [0.5, 65.0, 0.5];
                let stack = || ItemStack::new("minecraft:dirt", 1);
                assert!(game.throw_from_eyes(stack(), eye, false, true).is_none());
                assert_eq!(*heard.lock().unwrap(), 1);
                assert!(game.throw_from_eyes(stack(), eye, true, false).is_some());
                assert_eq!(*heard.lock().unwrap(), 1);
            });
        });
    }
}
