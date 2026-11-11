//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `data_maps` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call `ItemKey::data` and `BlockKey::data`
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait DataMapsBackend {
    /// Side: both.
    /// Item `item`'s value in data map `map`, before the map decodes it: the merged JSON the
    /// server's data packs and mods' files gave it, as NBT. `Ok(None)` where it has none.
    #[access(PublishedResources)]
    fn item_data_map_value(&self, item: ItemKey, map: &str) -> Result<Option<Tag>, DataMapError>;
    /// Side: both.
    /// Block `block`'s value in data map `map`, as `item_data_map_value` answers for an item.
    #[access(PublishedResources)]
    fn block_data_map_value(&self, block: BlockKey, map: &str)
    -> Result<Option<Tag>, DataMapError>;
}
