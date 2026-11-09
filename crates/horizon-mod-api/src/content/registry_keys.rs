//! Load-owned declarations and resolved mappings for registration-slot keys.
use crate::{BlockEntityTypeKey, BlockKey, DimensionKey, EffectKey, FluidKey, ItemKey};

impl BlockKey {
    /// Validate an incoming slot against declarations, not a world's resolved mappings.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            registering
                .map(|requests| &requests.blocks)
                .unwrap_or_else(|| block_names())
                .id(slot)
                .ok_or("unknown BlockKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl FluidKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.fluids,
                None => crate::parts::resolved_fluids()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().fluids),
            };
            names.id(slot).ok_or("unknown FluidKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl ItemKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.items,
                None => resolved_items()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().items),
            };
            names.id(slot).ok_or("unknown ItemKey slot")?;
            Ok(Self(slot))
        })
    }
}

fn resolved_items() -> &'static std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> {
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::registry_keys::Names> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_items)
}

/// Complete the load-owned item table with all registry entries, preserving requested slots.
/// Missing forward references are refused before publishing any keys.
pub fn resolve_items<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<(), Vec<crate::parts::ItemKeyError>> {
    let names = crate::parts::requested_keys()
        .items
        .complete(ids, |mod_id, id| crate::parts::ItemKeyError {
            mod_id,
            id: id.to_owned(),
        })?;
    let _ = resolved_items().set(names);
    Ok(())
}

fn resolved_block_keys()
-> &'static std::sync::OnceLock<horizon_mod_runtime::block_definitions::Blocks> {
    static EMPTY: std::sync::OnceLock<horizon_mod_runtime::block_definitions::Blocks> =
        std::sync::OnceLock::new();
    crate::parts::installed().map_or(&EMPTY, |installed| &installed.resolved_blocks)
}

fn block_names() -> &'static horizon_mod_runtime::registry_keys::Names {
    resolved_block_keys()
        .get()
        .map(|blocks| &blocks.names)
        .unwrap_or(&crate::parts::requested_keys().blocks)
}

/// Publish completed identities and immutable definitions together before constructing mesh tables.
/// World-number mappings remain owning-thread data.
pub fn resolve_block_keys<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    definition: impl FnMut(
        &str,
    )
        -> Result<horizon_mod_runtime::block_definitions::BlockDefinition, String>,
) -> Result<(), Vec<String>> {
    let names = crate::parts::requested_keys()
        .blocks
        .complete(ids, |mod_id, id| {
            format!("mod {mod_id} requested unknown block {id}")
        })?;
    let blocks = horizon_mod_runtime::block_definitions::Blocks::prepare(names, definition)?;
    let _ = resolved_block_keys().set(blocks);
    Ok(())
}

/// Find a block in the completed registry without allocating at a hook boundary.
pub fn block_key(id: &str) -> Option<BlockKey> {
    Some(BlockKey(*resolved_block_keys().get()?.names.slots.get(id)?))
}

/// Borrow the immutable definitions published with completed block identities.
pub fn block_definitions() -> Option<&'static horizon_mod_runtime::block_definitions::Blocks> {
    resolved_block_keys().get()
}

/// Find an item in the completed registry without allocating at a hook boundary.
pub fn item_key(id: &str) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.slots.get(id)?))
}

/// Resolve a typed identifier at a load boundary without formatting it as a string.
pub fn item_key_by_identifier(id: &crate::Identifier) -> Option<ItemKey> {
    Some(ItemKey(*resolved_items().get()?.identifiers.get(id)?))
}

impl EffectKey {
    /// Resolved effects also expose registry entries no mod explicitly requested.
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.effects,
                None => crate::parts::resolved_effects()
                    .get()
                    .map_or(&crate::parts::requested_keys().effects, |effects| {
                        &effects.names
                    }),
            };
            names.id(slot).ok_or("unknown EffectKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl BlockEntityTypeKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|registering| {
            let names = match registering {
                Some(requests) => &requests.block_entity_types,
                None => crate::parts::resolved_block_entity_types()
                    .get()
                    .unwrap_or(&crate::parts::requested_keys().block_entity_types),
            };
            names.id(slot).ok_or("unknown BlockEntityTypeKey slot")?;
            Ok(Self(slot))
        })
    }
}

impl DimensionKey {
    /// `minecraft:overworld`.
    pub const OVERWORLD: Self = Self(0);
    /// `minecraft:the_nether`.
    pub const NETHER: Self = Self(1);
    /// `minecraft:the_end`.
    pub const END: Self = Self(2);

    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        let key = Self(slot);
        dimension_id(key).ok_or("unknown DimensionKey slot")?;
        Ok(key)
    }
}

/// The key of dimension `id`, numbering it when the game meets it for the first time.
pub fn dimension_key(id: &str) -> DimensionKey {
    DimensionKey(horizon_mod_runtime::registry_keys::dimension_slot(id))
}

/// The name of a dimension's key, None for a slot the game never gave out.
pub fn dimension_id(key: DimensionKey) -> Option<&'static str> {
    horizon_mod_runtime::registry_keys::dimension_id(key.0)
}

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// A fluid requested by a mod but absent from the completed registry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UnknownFluid {
    /// Mod that requested the fluid.
    pub mod_id: &'static str,
    /// Canonical fluid identifier.
    pub id: String,
}

impl std::fmt::Display for UnknownFluid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod {} requested unknown fluid {}", self.mod_id, self.id)
    }
}

/// Resolve fluid requests against the loaded behavior registry.
pub fn resolve_fluids(ids: &[&str], world_fluids: &[&str]) -> Result<(), Vec<UnknownFluid>> {
    let requests = crate::parts::requested_keys();
    let mut names = requests.fluids.clone();
    let mut errors = Vec::new();
    for entry in &names.entries {
        if !ids.contains(&entry.id.as_str()) && !requests.declared_fluids.contains_key(&entry.id) {
            errors.extend(entry.mods.iter().map(|mod_id| UnknownFluid {
                mod_id,
                id: entry.id.clone(),
            }));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    for id in ids {
        names
            .request("horizon", id)
            .expect("registry owns valid ids");
    }
    for (slot, entry) in names.entries.iter().enumerate() {
        names
            .utf16_slots
            .insert(entry.id.encode_utf16().collect(), slot as u32);
        if let Some(path) = entry.id.strip_prefix("minecraft:") {
            names
                .utf16_slots
                .insert(path.encode_utf16().collect(), slot as u32);
        }
    }
    names.world_fluids = world_fluids
        .iter()
        .map(|name| names.slots.get(*name).copied())
        .collect();
    let _ = crate::parts::resolved_fluids().set(names);
    Ok(())
}

/// Resolve an engine fluid kind through the load-owned mapping.
pub fn world_fluid_key(index: usize) -> Option<FluidKey> {
    crate::parts::resolved_fluids()
        .get()?
        .world_fluids
        .get(index)
        .copied()
        .flatten()
        .map(FluidKey)
}

/// Find a fluid key at a data-loading boundary; a bare name means `minecraft:`.
pub fn fluid_key(id: &str) -> Option<FluidKey> {
    let names = crate::parts::resolved_fluids().get()?;
    let slot = if id.contains(':') {
        names.slots.get(id)
    } else {
        names.slots.get(&format!("minecraft:{id}"))
    };
    Some(FluidKey(*slot?))
}

/// Resolve an NBT UTF-16 fluid name without allocating.
pub fn fluid_key_utf16(id: &[u16]) -> Option<FluidKey> {
    Some(FluidKey(
        *crate::parts::resolved_fluids().get()?.utf16_slots.get(id)?,
    ))
}

/// The canonical registry name of a resolved fluid key.
pub fn fluid_id(key: FluidKey) -> Option<&'static str> {
    crate::parts::resolved_fluids().get()?.id(key.0)
}

/// Resolve all block entity types after registration and content loading.
pub fn resolve_block_entity_types<'a, T>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<T>,
) -> Result<(), Vec<String>> {
    let names = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &crate::parts::requested_keys().block_entity_types,
        ids,
        resolve,
    )?;
    let _ = crate::parts::resolved_block_entity_types().set(names);
    Ok(())
}

/// Lookup an engine-owned name after the registry is resolved.
pub fn block_entity_type_key(id: &str) -> Option<BlockEntityTypeKey> {
    Some(BlockEntityTypeKey(
        *crate::parts::resolved_block_entity_types()
            .get()?
            .slots
            .get(id)?,
    ))
}

/// Canonical name of a load-owned block entity type slot.
pub fn block_entity_type_name(key: BlockEntityTypeKey) -> Option<&'static str> {
    crate::parts::resolved_block_entity_types()
        .get()
        .unwrap_or(&crate::parts::requested_keys().block_entity_types)
        .id(key.0)
}

pub(crate) fn block_entity_type_display_name(key: BlockEntityTypeKey) -> String {
    crate::parts::with_registering_keys(|registering| {
        registering
            .and_then(|r| r.block_entity_types.id(key.0))
            .or_else(|| block_entity_type_name(key))
            .unwrap_or("<invalid>")
            .to_owned()
    })
}

/// Resolve requested effects once, after every mod's content has been registered.
pub fn resolve_effects<'a>(
    ids: impl IntoIterator<Item = &'a str>,
    resolve: impl FnMut(&str) -> Option<usize>,
) -> Result<(), Vec<String>> {
    let effects = horizon_mod_runtime::registry_keys::resolve_effects(
        &crate::parts::requested_keys().effects,
        ids,
        resolve,
    )
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    })?;
    let _ = crate::parts::resolved_effects().set(effects);
    Ok(())
}

/// Resolve an effect operation to its load-owned engine registry index.
pub fn effect_index(key: EffectKey) -> Option<usize> {
    crate::parts::resolved_effects()
        .get()?
        .indices
        .get(key.0 as usize)
        .copied()
}

/// Canonical diagnostic name, unavailable until the process effect registry is resolved.
pub fn effect_id(key: EffectKey) -> Option<&'static str> {
    Some(&crate::parts::resolved_effects().get()?.names.entries[key.0 as usize].id)
}

/// Canonical diagnostic name of a trusted, load-owned slot, independent of world resolution.
pub fn block_id(key: BlockKey) -> Option<&'static str> {
    block_names().id(key.0)
}

/// Canonical diagnostic name retained by an item registration slot.
pub fn item_id(key: ItemKey) -> Option<&'static str> {
    resolved_items()
        .get()
        .unwrap_or(&crate::parts::requested_keys().items)
        .id(key.0)
}

/// Find an effect by its full registry name in the completed registry, without allocating.
pub fn effect_key(id: &str) -> Option<EffectKey> {
    Some(EffectKey(
        *crate::parts::resolved_effects()
            .get()?
            .names
            .slots
            .get(id)?,
    ))
}

/// The API key for an engine registry index, also for entries not explicitly requested by mods.
pub fn effect_key_by_index(index: usize) -> Option<EffectKey> {
    crate::parts::resolved_effects()
        .get()?
        .keys
        .get(index)
        .copied()
        .map(EffectKey)
}

impl crate::BoolGameRuleKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        crate::parts::with_registering_keys(|keys| {
            keys.unwrap_or(crate::parts::requested_keys())
                .game_rules
                .id(slot)
                .ok_or("unknown bool rule slot")?;
            Ok(Self(slot))
        })
    }
}

#[cfg(test)]
mod block_tests {
    use super::*;
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            assert_eq!(registrar.block_key("stone").0, 0);
        }
    }

    #[test]
    fn completed_blocks_preserve_slots_and_bind_unrequested_states() {
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Requests))])
            .unwrap();
        let definition = |_: &str| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|error| format!("{error:?}"))
        };
        assert!(resolve_block_keys(["minecraft:air"], definition).is_err());
        assert!(block_key("minecraft:air").is_none());
        assert!(
            resolve_block_keys(["minecraft:stone", "minecraft:air"], |_| Err(
                "missing definition".into()
            ))
            .is_err()
        );
        assert!(block_key("minecraft:air").is_none());
        let names = [
            "test:machine",
            "minecraft:zombie_head",
            "minecraft:stone",
            "minecraft:air",
            "minecraft:birch_log",
            "minecraft:air",
        ];
        resolve_block_keys(names, definition).unwrap();
        let stone = block_key("minecraft:stone").unwrap();
        let air = block_key("minecraft:air").unwrap();
        assert_eq!(stone, BlockKey(0));
        assert_eq!(air, BlockKey(1));
        assert_eq!(BlockKey::from_slot(air.0), Ok(air));
        assert_eq!(block_key("minecraft:birch_log"), Some(BlockKey(2)));
        assert_eq!(block_key("minecraft:zombie_head"), Some(BlockKey(3)));
        assert_eq!(block_key("test:machine"), Some(BlockKey(4)));
        assert!(BlockKey::from_slot(5).is_err());
        for name in [
            "",
            "missing:block",
            "minecraft:不存在",
            &"x".repeat(100_000),
        ] {
            assert_eq!(block_key(name), None);
        }
        assert_eq!(block_id(air), Some("minecraft:air"));
        // Reversing the registry's enumeration cannot change any completed key.
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Requests))])
            .unwrap();
        resolve_block_keys(names.into_iter().rev(), definition).unwrap();
        for (name, slot) in [
            ("minecraft:stone", 0),
            ("minecraft:air", 1),
            ("minecraft:birch_log", 2),
            ("minecraft:zombie_head", 3),
            ("test:machine", 4),
        ] {
            assert_eq!(block_key(name), Some(BlockKey(slot)));
        }
        // Replacing Parts replaces the load-owned identities, not a second global registry.
        crate::install_parts(&[], &crate::parts::tests::rule_schema(), |_| false).unwrap();
        assert!(block_key("minecraft:air").is_none());
        assert!(BlockKey::from_slot(air.0).is_err());
    }
}

#[cfg(test)]
mod item_tests {
    use super::*;
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            assert_eq!(registrar.item_key("stick").0, 0);
        }
    }

    #[test]
    fn completed_items_preserve_forward_slots_and_refuse_unresolved_requests() {
        crate::install_parts(
            &[ModEntry::new("test", Box::new(Requests))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        assert!(resolve_items(["minecraft:stone"]).is_err());
        assert!(item_key("minecraft:stone").is_none());
        resolve_items(["minecraft:stone", "minecraft:stick", "minecraft:apple"]).unwrap();
        assert_eq!(item_key("minecraft:stick"), Some(ItemKey(0)));
        let apple = item_key("minecraft:apple").unwrap();
        assert_ne!(apple, ItemKey(0));
        assert_eq!(ItemKey::from_slot(apple.0), Ok(apple));
        assert_eq!(item_id(apple), Some("minecraft:apple"));
        assert!(ItemKey::from_slot(3).is_err());
    }
}

#[cfg(test)]
mod hot_call_key_tests {
    use crate::*;
    #[test]
    fn hot_call_signatures_and_unknown_slots_are_typed() {
        let _: fn(&dyn Host, ItemKey) -> Option<i32> = |h, k| h.item_burn_time(k);
        let _: fn(&dyn Host, ItemKey) -> Option<f32> = |h, k| h.item_compost_chance(k);
        let _: fn(&mut dyn ClientHost, ItemKey, u32) -> Result<(), String> = |h, k, n| h.give(k, n);
        let _: fn(&dyn ClientHost, &str, ItemKey) -> bool = |h, e, k| h.enchantment_supports(e, k);
        crate::testing::with_mods(Vec::new().into(), || {
            assert!(ItemKey::from_slot(u32::MAX).is_err());
            assert_eq!(__private::item_key_id(ItemKey(u32::MAX)), None);
            assert_eq!(__private::block_key_id(BlockKey(u32::MAX)), None);
        });
    }
}

#[cfg(test)]
mod fluid_mapping_tests {
    #[test]
    fn world_fluid_order_is_supplied_by_engine_not_key_order() {
        crate::parts::tests::install_parts_fixture(&[]).unwrap();
        super::resolve_fluids(
            &["minecraft:lava", "minecraft:empty", "minecraft:water"],
            &["minecraft:empty", "minecraft:water", "minecraft:lava"],
        )
        .unwrap();
        for (index, expected) in ["minecraft:empty", "minecraft:water", "minecraft:lava"]
            .iter()
            .enumerate()
        {
            let key = super::world_fluid_key(index).unwrap();
            assert_eq!(super::fluid_id(key), Some(*expected));
        }
        assert!(super::world_fluid_key(3).is_none());
    }
}
