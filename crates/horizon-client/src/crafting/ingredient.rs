//! Engine recipe ingredients and stack matching.
use super::native_hooks::native_ingredient;
use super::{ItemStack, Items};
use crate::native_stack_hooks::ingredient_test as native_ingredient_test;
use horizon_world::native_load_context::LoadContext;
use mod_api::__private::{WalkedIngredient, dedup_keep_order, resolved_items};
pub use mod_api::item::crafting::Shape;
use serde_json::Value;
use std::collections::HashSet;
/// Where `Ingredient::resolve` leaves the hook's answer inside the occurrence
/// it resolved. Written and read only here; the loads that own the sources
/// never send or save it.
const RESOLVED_ITEMS: &str = "horizon:resolved_items";

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A mod's own kind: a namespaced type that is not minecraft's.
fn is_custom(kind: &str) -> bool {
    kind.contains(':') && !kind.starts_with("minecraft:")
}

/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order (`Ingredient.display`'s `TagSlotDisplay`).
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Ingredient {
    /// The items it accepts, however the source shape named them.
    pub items: HashSet<String>,
    /// The same items in `HolderSet` order (`display`).
    pub order: Vec<String>,
    /// A mod's own kind, for matching at craft time: the kind and its
    /// NBT compound, as the load-time hook saw them (`native_ingredient_test`).
    pub custom: Option<(String, horizon_core::nbt::Compound)>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
}
impl Ingredient {
    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
        }
    }
    /// Asks the `ingredient` hook once for each mod kind in `v`, nested ones
    /// included, and leaves each answer with its occurrence for `parse`: the
    /// one place the hook is called, so every table that reads the recipes
    /// shares the same answers. A file's own answer is dropped first, so data
    /// cannot forge one. `true`: `v` changed.
    // hidden: the load's half of the ingredient hook, not part of the mod surface
    #[doc(hidden)]
    pub fn resolve(v: &mut Value, context: &LoadContext) -> bool {
        match v {
            Value::Array(entries) => entries
                .iter_mut()
                .fold(false, |changed, e| Self::resolve(e, context) | changed),
            Value::Object(o) => {
                let mut changed = o.remove(RESOLVED_ITEMS).is_some();
                let kind = ingredient_kind(o).map(str::to_owned);
                match kind.as_deref() {
                    Some("neoforge:compound" | "neoforge:intersection") => {
                        for key in ["children", "ingredients"] {
                            if let Some(children) = o.get_mut(key) {
                                changed |= Self::resolve(children, context);
                            }
                        }
                    }
                    Some("neoforge:difference") => {
                        for key in ["base", "subtracted"] {
                            if let Some(part) = o.get_mut(key) {
                                changed |= Self::resolve(part, context);
                            }
                        }
                    }
                    Some(k) if is_custom(k) => {
                        let mut list = Vec::new();
                        if mod_api::__private::listened(mod_api::HookId::Ingredient)
                            && let Ok(data) = crate::json_nbt::json_to_compound(o)
                        {
                            native_ingredient(context, k, &data, &mut list);
                        }
                        dedup_keep_order(&mut list);
                        o.insert(RESOLVED_ITEMS.to_owned(), list.into());
                        changed = true;
                    }
                    Some(_) => {}
                    // A brewing slot's item predicate wraps its ingredient.
                    None => {
                        if let Some(item) = o.get_mut("item") {
                            changed |= Self::resolve(item, context);
                        }
                    }
                }
                changed
            }
            _ => false,
        }
    }
    /// Drops every answer `resolve` left anywhere in `v`: what a load sends
    /// or shows is the files' own JSON. `true`: `v` changed.
    // hidden: the load's half of the ingredient hook, not part of the mod surface
    #[doc(hidden)]
    pub fn forget_answers(v: &mut Value) -> bool {
        match v {
            Value::Array(entries) => entries
                .iter_mut()
                .fold(false, |changed, e| Self::forget_answers(e) | changed),
            Value::Object(o) => {
                let mut changed = o.remove(RESOLVED_ITEMS).is_some();
                for part in o.values_mut() {
                    changed |= Self::forget_answers(part);
                }
                changed
            }
            _ => false,
        }
    }
    pub fn parse(v: &Value, items: &Items) -> Option<Self> {
        // A mod's own kind: `resolve` asked the hook once at load and left
        // the item list it matches with the occurrence; one no load
        // resolved fails the file.
        Self::of_walked(WalkedIngredient::walk(
            v,
            &items.item_tag_order,
            &resolved_items,
            &|data| crate::json_nbt::json_to_compound(data).ok(),
        )?)
    }
    /// The walk of an item ingredient's answer: NeoForge's standard shapes
    /// keep their children, and a mod's own kind keeps its kind and NBT metadata,
    /// since the load-time list only names items and matching a concrete
    /// stack (its components) is the match-time hook's.
    fn of_walked(walked: WalkedIngredient) -> Option<Self> {
        // The shapes' children walk the same way; a child no wrapper
        // answers fails the file.
        let children = match walked.children {
            Some((shape, kids)) => Some((
                shape,
                kids.into_iter()
                    .map(Self::of_walked)
                    .collect::<Option<_>>()?,
            )),
            None => None,
        };
        (!walked.order.is_empty()).then(|| Self {
            items: walked.set,
            order: walked.order,
            custom: walked.custom,
            children,
        })
    }
    /// `test` with the loop context the game already holds, for the game's
    /// own matching.
    pub fn test_in(
        &self,
        context: &horizon_world::native_load_context::LoopContext,
        stack: &ItemStack,
    ) -> bool {
        match &self.children {
            // Each child its own `test`, as on NeoForge — a mod's kind
            // among them answers through its hook.
            Some((Shape::Compound, kids)) => kids.iter().any(|k| k.test_in(context, stack)),
            Some((Shape::Intersection, kids)) => kids.iter().all(|k| k.test_in(context, stack)),
            Some((Shape::Difference, kids)) => {
                kids.first().is_some_and(|k| k.test_in(context, stack))
                    && kids.get(1).is_none_or(|k| !k.test_in(context, stack))
            }
            None => {
                if !self.items.contains(&*stack.item) {
                    return false;
                }
                if let Some((kind, data)) = &self.custom {
                    let mut accept = true;
                    native_ingredient_test(context, kind, data, stack, &mut accept);
                    return accept;
                }
                true
            }
        }
    }
}

#[cfg(test)]
#[path = "custom_data_tests.rs"]
mod custom_data_tests;
