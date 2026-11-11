mod parts;
#[cfg(test)]
pub(crate) use parts::install_parts;
#[cfg(test)]
pub(crate) use parts::mark_game_thread;
pub use parts::{
    Attached, BlockCapabilityProvider, BlockEntityProviders, BlockProvider, BlockProviderOf,
    CapabilityProviders, EntityCapabilityProvider, EntityProvider, EntityProviderOf,
    EntityProviders, ItemCapabilityProvider, ItemProvider, ItemProviderOf, OwnProvider, Parts,
    ProvidedBy, Registrar, block_entity_providers, entity_providers, has_entity_providers,
};
pub(crate) use parts::{InstalledProviders, answered, ask_providers, has_part_hook, parts};
mod __private {
    #[cfg(any(test, feature = "testing"))]
    pub use crate::parts::install_process_datapack_registry_declarations;
    pub use crate::parts::{
        AttributeDeclaration, BiomeModifierDeclaration, EntityAttributeDeclaration,
        EntityAttributeModification, HookParts, ItemKeyError, KnownIds, PartsRef,
        ResolvePartsError, SpawnPlacementDeclaration, TooltipAppenderPart, attribute_declarations,
        biome_modifier_declarations, block_entity_types, block_key_listens, clear_game_parts,
        datapack_registry_declarations, declared_world_fluids, entity_attribute_declarations,
        entity_attribute_modifications, entity_parts, game_rule_bool_slot, game_rule_categories,
        game_rules, has_entity_parts, has_part_hook, has_scoped_hook, install_parts,
        install_resolved_parts, item_listens, key_mappings, mark_game_thread, parts, prepare_parts,
        resolve_parts, setting, settings, spawn_placement_declarations, tooltip_appenders,
    };
    #[cfg(any(test, feature = "testing"))]
    pub use crate::{
        parts::tests as part_tests,
        world::phase::{with_test_packages, with_test_registration_data},
    };
}
