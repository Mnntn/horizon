//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// A native mod's book category for one custom recipe type.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct RecipeBookCategory {
    /// A namespaced category id outside `minecraft:`.
    pub id: String,
    /// A custom type present in the current recipe table.
    pub recipe_type: String,
    /// A registered item id used as the category's icon.
    pub icon: String,
}
