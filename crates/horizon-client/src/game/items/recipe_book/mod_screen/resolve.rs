//! Resource-backed resolution of mod recipe displays.
use crate::game::{Game, api_values::TryIntoEngine};
use horizon_client::{crafting::Ingredient, items::ItemStack, menu::place::Stacked};
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};

pub(super) struct Resolver<'a, 'w> {
    game: &'a Game<'w>,
    stocks: &'a [ItemStack],
}

impl<'a, 'w> Resolver<'a, 'w> {
    pub(super) fn new(game: &'a Game<'w>, stocks: &'a [ItemStack]) -> Self {
        Self { game, stocks }
    }

    pub(super) fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        let items = self.game.items();
        match slot {
            Slot::Empty => Vec::new(),
            Slot::Item(id) => items
                .get(id)
                .map(|_| ItemStack::new(id, 1))
                .into_iter()
                .collect(),
            Slot::ItemStack(stack) => stack.try_into_engine(items).ok().into_iter().collect(),
            Slot::AnyFuel => items
                .registry_order
                .iter()
                .filter(|id| items.get(id).is_some_and(|i| i.fuel.is_some()))
                .map(|id| ItemStack::new(id, 1))
                .collect(),
            Slot::Tag(tag) => items
                .item_tag_order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|id| ItemStack::new(id, 1))
                .collect(),
            Slot::Composite(children) => children
                .iter()
                .flat_map(|child| self.resolve(child))
                .collect(),
            Slot::WithRemainder { input, .. } => self.resolve(input),
            Slot::WithAnyPotion(source) => self
                .resolve(source)
                .into_iter()
                .flat_map(|stack| {
                    horizon_client::brewing::POTIONS
                        .iter()
                        .map(move |p| stack.clone().with_potion(&format!("minecraft:{}", p.id)))
                })
                .collect(),
            Slot::OnlyWithComponent { source, component } => self
                .resolve(source)
                .into_iter()
                .filter(|stack| stack.carries_component(items.get(&stack.item), component))
                .collect(),
            Slot::Dyed { dye, target } => {
                let targets = self.resolve(target);
                self.resolve(dye)
                    .iter()
                    .filter_map(|stack| {
                        if !stack.carries_component(items.get(&stack.item), "minecraft:dye") {
                            return None;
                        }
                        let name = stack
                            .data()
                            .and_then(|data| data.other("minecraft:dye"))
                            .and_then(|tag| match tag {
                                horizon_core::nbt::Tag::String(value) => {
                                    Some(value.to_string_lossy())
                                }
                                _ => None,
                            })
                            .or_else(|| items.get(&stack.item).and_then(|info| info.dye.clone()))?;
                        horizon_client::block_entity_render::DYES
                            .iter()
                            .find(|(id, _)| *id == name)
                            .map(|(_, color)| *color as i32)
                    })
                    .flat_map(|color| {
                        targets.iter().cloned().map(move |mut stack| {
                            stack.count = 1;
                            stack.data_mut().dyed_color = Some(color);
                            stack
                        })
                    })
                    .take(16)
                    .collect()
            }
            Slot::SmithingTrim {
                base,
                material,
                pattern,
            } => {
                let materials = self.resolve(material);
                let pattern =
                    horizon_client::crafting::RecipeTrimPattern::Reference(pattern.clone());
                self.resolve(base)
                    .iter()
                    .flat_map(|base| {
                        materials.iter().filter_map(|material| {
                            horizon_client::stations::apply_trim(
                                &self.game.stations().enchantments,
                                base,
                                material,
                                &pattern,
                            )
                        })
                    })
                    .collect()
            }
        }
    }

    pub(super) fn craftable(&self, _recipe: &str, display: &Display) -> bool {
        let slots: Vec<&Slot> = match display {
            Display::Shaped { ingredients, .. } | Display::Shapeless { ingredients, .. } => {
                ingredients
                    .iter()
                    .filter(|slot| !matches!(slot, Slot::Empty))
                    .collect()
            }
            Display::Furnace { ingredient, .. } => vec![ingredient],
            Display::Stonecutter { input, .. } => vec![input],
            Display::Smithing {
                template,
                base,
                addition,
                ..
            } => vec![template, base, addition],
        };
        let ingredients: Vec<_> = slots
            .iter()
            .map(|slot| {
                Ingredient::of(
                    self.resolve(slot)
                        .into_iter()
                        .map(|stack| stack.item.to_string())
                        .collect(),
                )
            })
            .collect();
        can_craft(self.stocks, &ingredients)
    }

    pub(super) fn search_text(&self, stacks: &[ItemStack]) -> String {
        stacks
            .iter()
            .map(|stack| horizon_client::stations::hover_name(&self.game.pack.names, stack))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
}

fn can_craft(stocks: &[ItemStack], ingredients: &[Ingredient]) -> bool {
    let mut stacked = Stacked::default();
    for stack in stocks {
        stacked.account_stack(stack, stack.count);
    }
    stacked.can_craft(&ingredients.iter().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_inputs_consume_capacity_and_reassign_alternatives() {
        let ingredients = [
            Ingredient::of(vec!["test:a".into(), "test:b".into()]),
            Ingredient::of(vec!["test:a".into()]),
        ];
        assert!(!can_craft(&[ItemStack::new("test:a", 1)], &ingredients));
        assert!(can_craft(&[ItemStack::new("test:a", 2)], &ingredients));
        assert!(can_craft(
            &[ItemStack::new("test:a", 1), ItemStack::new("test:b", 1)],
            &ingredients
        ));
    }
}
