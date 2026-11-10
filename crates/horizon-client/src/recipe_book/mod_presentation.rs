//! Owned custom-book views for the shared recipe-book painter. These are display
//! identities only: they must never enter vanilla placement or ghost-slot paths.
use super::ModBookPanel;
use crate::{
    items::ItemStack,
    recipe_book_screen::{ButtonView, OverlayRecipe, TabView},
};
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};

/// Resource and inventory boundary implemented by the screen owner. Slot
/// resolution must preserve components and alternatives for every API slot form.
pub trait DisplayResolver {
    fn resolve(&self, slot: &Slot) -> Vec<ItemStack>;
    /// Match the actual inventory against the recipe, not merely its icons.
    fn craftable(&self, recipe: &str, display: &Display) -> bool;
    /// Localized names and search terms of the resolved result stacks.
    fn search_text(&self, results: &[ItemStack]) -> String;
}

/// One button and the alternatives shown on right click. Recipe ids are never
/// vanilla display ids. The caller paginates these buttons in groups of twenty.
pub struct PresentedRecipe {
    pub recipe: String,
    pub button: ButtonView,
    pub overlay: Vec<OverlayRecipe>,
}

pub struct ModPresentation {
    /// Search first, followed by category ids in stable panel order.
    pub tab_ids: Vec<String>,
    pub tabs: Vec<TabView>,
    pub recipes: Vec<PresentedRecipe>,
}

impl ModBookPanel<'_> {
    /// A stale category selects no recipes. Search always traverses only this
    /// panel, even when a query or selection survived a different menu.
    pub fn presentation(
        &self,
        category: Option<&str>,
        query: &str,
        filtering: bool,
        resolver: &dyn DisplayResolver,
    ) -> ModPresentation {
        let mut view = ModPresentation {
            tab_ids: vec![self.search_group().to_owned()],
            tabs: vec![TabView {
                icons: ("minecraft:compass".into(), None),
                selected: category.is_none(),
                anim: 0.0,
            }],
            recipes: Vec::new(),
        };
        let query = query.to_lowercase();
        for cat in self.categories() {
            view.tab_ids.push(cat.id.clone());
            view.tabs.push(TabView {
                icons: (cat.icon.clone(), None),
                selected: category == Some(cat.id.as_str()),
                anim: 0.0,
            });
            if category.is_some_and(|id| id != cat.id) {
                continue;
            }
            for recipe in &cat.displays {
                let mut alternatives = Vec::new();
                for display in &recipe.displays {
                    let craftable = resolver.craftable(&recipe.recipe, display);
                    if filtering && !craftable {
                        continue;
                    }
                    let results = resolver.resolve(result(display));
                    // The painter cycles nonempty alternatives; unresolved dynamic
                    // displays are not drawable until their resources exist.
                    if results.is_empty()
                        || !resolver
                            .search_text(&results)
                            .to_lowercase()
                            .contains(&query)
                    {
                        continue;
                    }
                    alternatives.push((
                        results,
                        OverlayRecipe {
                            craftable,
                            cells: cells(display, resolver),
                        },
                    ));
                }
                alternatives.sort_by_key(|(_, overlay)| !overlay.craftable);
                if alternatives.is_empty() {
                    continue;
                }
                let has_craftable = alternatives.iter().any(|(_, overlay)| overlay.craftable);
                let (entries, overlay) = alternatives.into_iter().unzip();
                view.recipes.push(PresentedRecipe {
                    recipe: recipe.recipe.clone(),
                    button: ButtonView {
                        has_craftable,
                        entries,
                        anim: 0.0,
                    },
                    overlay,
                });
            }
        }
        view
    }
}

fn result(display: &Display) -> &Slot {
    match display {
        Display::Shaped { result, .. }
        | Display::Shapeless { result, .. }
        | Display::Furnace { result, .. }
        | Display::Stonecutter { result, .. }
        | Display::Smithing { result, .. } => result,
    }
}

fn cells(display: &Display, resolver: &dyn DisplayResolver) -> Vec<(i32, i32, Vec<ItemStack>)> {
    let mut out = Vec::new();
    let mut push = |x, y, slot| {
        let stacks = resolver.resolve(slot);
        if !stacks.is_empty() {
            out.push((x, y, stacks));
        }
    };
    match display {
        Display::Shaped {
            width,
            height,
            ingredients,
            ..
        } => {
            for (i, slot) in ingredients.iter().enumerate() {
                push(
                    (i % *width as usize) as i32 + i32::from(*width == 1),
                    (i / *width as usize) as i32 + i32::from(*height == 1),
                    slot,
                );
            }
        }
        Display::Shapeless { ingredients, .. } => {
            for (i, slot) in ingredients.iter().enumerate() {
                push((i % 3) as i32, (i / 3) as i32, slot);
            }
        }
        Display::Furnace { ingredient, .. }
        | Display::Stonecutter {
            input: ingredient, ..
        } => push(1, 1, ingredient),
        Display::Smithing {
            template,
            base,
            addition,
            ..
        } => {
            push(0, 1, template);
            push(1, 1, base);
            push(2, 1, addition);
        }
    }
    out
}

#[cfg(test)]
#[path = "mod_presentation_tests.rs"]
mod tests;
