//! Recipe book declarations supplied by a custom recipe type.
#[path = "recipe_display.rs"]
mod display;
#[allow(unused_imports)]
use crate::*;
pub use display::{RecipeBookDisplay, RecipeBookDisplayEntry, RecipeSlotDisplay};
/// A mod's book category and displays for one custom recipe type.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct RecipeBookCategory {
    /// A namespaced category id outside `minecraft:`.
    pub id: String,
    /// A custom type present in the current recipe table.
    pub recipe_type: String,
    /// A registered item id used as the category's icon.
    pub icon: String,
    /// The menu whose screen owns this category.
    pub target_menu: String,
    /// Registered book identity, separate from category identity and settings.
    /// Declaring a category does not itself register a book or change its flags.
    pub book: RecipeBookTypeKey,
    /// Search category aggregating exactly this book's categories.
    pub search_group: String,
    /// Recipe-type supplied displays, rebuilt with this declaration on reload.
    pub displays: Vec<RecipeBookDisplayEntry>,
}

/// A namespaced custom recipe book identity returned during registration.
/// This owns no settings; player settings and synchronization are separate.
#[derive(Clone, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
// exhaustive: its private identity field prevents external construction or destructuring
pub struct RecipeBookTypeKey {
    pub(crate) id: String,
}

impl RecipeBookTypeKey {
    /// Whether this process has installed this identity in its frozen registry.
    pub fn is_registered(&self) -> bool {
        crate::parts::installed()
            .is_some_and(|registry| registry.recipe_book_types.contains_key(self.as_str()))
    }

    /// The stable namespaced identity used in saves and negotiated payloads.
    pub fn as_str(&self) -> &str {
        &self.id
    }
}
