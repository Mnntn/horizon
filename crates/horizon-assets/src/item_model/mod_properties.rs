//! The item model properties, tint sources and node types native mods register (NeoForge's
//! `RegisterRangeSelectItemModelPropertyEvent`, `RegisterSelectItemModelPropertyEvent`,
//! `RegisterColorHandlersEvent.ItemTintSources` and `RegisterItemModelsEvent`). A
//! `range_dispatch` or `select` on a registered property, a tint of a registered source and a node
//! of a registered type read their answer from the host per stack; a property or tint nobody
//! registered stays foreign (`foreign.rs`), and a node type nobody registered fails to decode.
use super::identifier;
use std::{collections::BTreeSet, sync::RwLock};

static RANGE: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static SELECT: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static TINT: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());
static NODE: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// Keeps the numeric `range_dispatch` properties native mods registered, before
/// resources load. An id that is not `<namespace>:<path>` in a namespace other than
/// vanilla's is logged and left out. Returns the ids kept, in registration order.
pub fn register_range_properties(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&RANGE, "range select item model property", ids)
}

/// Keeps the categorical `select` properties native mods registered, before resources
/// load, checked as [`register_range_properties`] checks its ids.
pub fn register_select_properties(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&SELECT, "select item model property", ids)
}

/// Keeps the item tint sources native mods registered, before resources load, checked as
/// [`register_range_properties`] checks its ids.
pub fn register_tint_sources(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&TINT, "item tint source", ids)
}

/// Adds to `table` the ids a mod registered as `kind`: an id that is not `<namespace>:<path>` in a
/// namespace other than vanilla's, or one registered already, is logged and left out. Returns the
/// ids kept, in registration order.
pub fn keep_mod_ids(table: &RwLock<BTreeSet<String>>, kind: &str, ids: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut set = table.write().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if !is_mod_id(&id) {
            log::warn!("{kind} {id:?} is not <namespace>:<path> in a mod's namespace");
        } else if !set.insert(id.clone()) {
            log::warn!("{kind} {id} is registered twice");
        } else {
            kept.push(id);
        }
    }
    kept
}

/// Whether `id` is `<namespace>:<path>`, written in full, in a namespace other than vanilla's: an
/// id a mod may register.
pub fn is_mod_id(id: &str) -> bool {
    identifier(id).is_ok_and(|parsed| parsed == id) && !id.starts_with("minecraft:")
}

/// Whether a native mod registered `id` as a numeric `range_dispatch` property.
pub fn is_range_property(id: &str) -> bool {
    RANGE.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// Whether a native mod registered `id` as a categorical `select` property.
pub fn is_select_property(id: &str) -> bool {
    SELECT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
}

/// Keeps the item model node types native mods registered, before resources load, checked as
/// [`register_range_properties`] checks its ids.
pub fn register_item_models(ids: Vec<String>) -> Vec<String> {
    keep_mod_ids(&NODE, "item model type", ids)
}

/// Whether a native mod registered `id` as an item model node type.
pub fn is_item_model_type(id: &str) -> bool {
    NODE.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}

/// Whether a native mod registered `id` as an item tint source.
pub fn is_tint_source(id: &str) -> bool {
    TINT.read().unwrap_or_else(|e| e.into_inner()).contains(id)
}
