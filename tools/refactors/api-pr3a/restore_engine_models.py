#!/usr/bin/env python3
"""Restore stack-bearing engine records; run once on the PR 3a base.

Declared edits: copied records use engine ItemStack; engine recipe snapshots have
Snapshot names, and client-library references use these names. Behaviour is moved
unchanged from the API Ingredient implementation. Shared ingredient walking stays
in the API temporarily (also used by fluid ingredients).
"""
from pathlib import Path
root = Path(__file__).resolve().parents[3]
def read(p): return (root / p).read_text()
def write(p, s): (root / p).write_text(s)
api = 'crates/horizon-mod-api/src/'
client = 'crates/horizon-client/src/'
s = read(api+'crafting.rs')
start = s.index('/// A set of accepted items')
end = s.index('/// A recipe in force')
ing = s[start:end]
a = ing.index('    /// `Ingredient.test`')
b = ing.index('    /// `test` with the loop context')
ing = ing[:a]+ing[b:]
ing = ing.replace('pub(crate) fn test_in', 'pub fn test_in')
a = ing.index('/// How a shape')
b = ing.index('impl Ingredient', a)
ing = ing[:a] + ing[b:]
# Common parsing is shared, not duplicated.
helpers = s[s.index('/// Where `Ingredient::resolve`'):s.index('/// The walk both ingredient kinds share')]
walk = s[s.index('/// The walk both ingredient kinds share'):start]
walk = walk.replace('pub(crate)', 'pub')
# Share walker through an explicitly internal module surface.
s = s[:s.index('/// The walk both ingredient kinds share')]+walk+s[start:]
write(api+'crafting.rs', s)
engine = ('//! Engine recipe ingredients and stack matching.\n'
          'use super::{ItemStack, Items};\n'
          'use horizon_world::native_load_context::LoadContext;\n'
          'use serde_json::Value;\nuse std::collections::HashSet;\n'
          'use mod_api::item::crafting::{WalkedIngredient, native_ingredient};\n'
          'pub use mod_api::item::crafting::Shape;\n'
          'use crate::native_stack_hooks::ingredient_test as native_ingredient_test;\n')
write(client+'crafting/ingredient.rs', engine+helpers+ing)
s = read(client+'crafting.rs').replace('use mod_api::__private::game::IngredientTest;\n','').replace('pub use mod_api::item::crafting::{Ingredient, Shape, native_ingredient, native_ingredient_test};', 'mod ingredient;\npub use ingredient::{Ingredient, Shape};')
write(client+'crafting.rs',s)
# The records carry stacks but no engine behaviour.
s = read(api+'stations.rs')
write(client+'stations/models.rs', '//! Engine station results.\nuse super::ItemStack;\n'+s[s.index('/// One offer row'):])
s = read(client+'stations.rs').replace('pub use mod_api::item::stations::{AnvilResult, EnchantmentRow};', 'mod models;\npub use models::{AnvilResult, EnchantmentRow};').replace('use mod_api::__private::game::IngredientTest;\n','')
write(client+'stations.rs',s)
s = read(api+'menu.rs')
s = s[s.index('/// Operations on the clicked slot'):]
s = s.replace('crate::stations::AnvilResult','crate::stations::AnvilResult')
write(client+'menu/models.rs', '//! Engine menu records and slot access.\nuse super::ItemStack;\n'+s)
s = read(client+'menu.rs').replace('use mod_api::__private::game::IngredientTest;\n','')
a=s.index('pub use mod_api::item::menu::{'); b=s.index('};',a)+2
s=s[:a]+'''pub use mod_api::item::menu::{LogicalSide, MenuButtonContext};
mod models;
pub use models::{AnvilCraft, AnvilUpdate, AnvilVanilla, CarriedAccess, GrindstonePlace, GrindstoneTake, SlotView, StackedOnOther};'''+s[b:]
write(client+'menu.rs',s)
