//! Shared equipment durability mutation.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_core::nbt::Tag;
use horizon_world::item_durability::{self, DurabilityStack, DurabilityWorld};
use std::convert::Infallible;

struct World<'a, 'g> {
    game: &'a mut Game<'g>,
    original: &'a ItemStack,
    broken: Option<ItemStack>,
}

impl DurabilityWorld for World<'_, '_> {
    type Error = Infallible;
    type Stack = DurabilityStack;

    fn storage_error(error: Infallible) -> Infallible {
        error
    }
    fn infinite_materials(&mut self, _: &mut DurabilityStack) -> Result<bool, Infallible> {
        Ok(self.game.creative)
    }
    fn process_enchantments(
        &mut self,
        _: &mut DurabilityStack,
        amount: i32,
    ) -> Result<i32, Infallible> {
        let loss = self.game.durability_loss(self.original, amount as u32);
        Ok(loss.min(i32::MAX as u32) as i32)
    }
    fn durability_changed(
        &mut self,
        _: &mut DurabilityStack,
        damage: i32,
    ) -> Result<(), Infallible> {
        self.game.trigger_item_durability(self.original, damage);
        Ok(())
    }
    fn broken(&mut self, _: &mut DurabilityStack, copy: DurabilityStack) -> Result<(), Infallible> {
        let mut stack = self.original.clone();
        stack.damage = copy.damage.unwrap_or(0) as u32;
        self.broken = Some(stack);
        Ok(())
    }
}

impl Game<'_> {
    /// Compute the server durability update without firing equip effects.
    pub(super) fn damage_stack(
        &mut self,
        stack: &ItemStack,
        amount: i32,
        player: bool,
    ) -> (Option<ItemStack>, Option<ItemStack>) {
        let component = |name| stack.data().and_then(|data| data.other(name));
        let item = self.items().get(&stack.item);
        let max = match component("minecraft:max_damage") {
            Some(Tag::Int(value)) => Some(*value),
            _ => item.and_then(|item| (item.max_damage > 0).then_some(item.max_damage as i32)),
        };
        let mut projected = DurabilityStack {
            count: stack.count as i32,
            air: &*stack.item == "minecraft:air",
            max_damage: if component("!minecraft:max_damage").is_some() {
                None
            } else {
                max
            },
            damage: if component("!minecraft:damage").is_some() {
                None
            } else if component("minecraft:damage").is_some()
                || item.is_some_and(|item| {
                    item.components
                        .iter()
                        .any(|component| component == "minecraft:damage")
                })
            {
                Some(stack.damage as i32)
            } else {
                None
            },
            unbreakable: component("!minecraft:unbreakable").is_none()
                && (component("minecraft:unbreakable").is_some()
                    || item.is_some_and(|item| {
                        item.components
                            .iter()
                            .any(|component| component == "minecraft:unbreakable")
                    })),
        };
        let mut world = World {
            game: self,
            original: stack,
            broken: None,
        };
        item_durability::hurt_and_break(&mut projected, &mut world, amount, player).unwrap();
        let mut updated = stack.clone();
        updated.count = projected.count.max(0) as u32;
        if let Some(damage) = projected.damage {
            updated.damage = damage as u32;
        }
        ((updated.count > 0).then_some(updated), world.broken)
    }
}
