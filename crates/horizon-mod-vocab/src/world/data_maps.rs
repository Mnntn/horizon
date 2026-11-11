//! Typed reads of data maps (NeoForge's `DataMapType` and `IWithData.getData`): the value the
//! server's data packs and mods' files gave a registry entry, after every pack was applied.
use crate::{Backend, BlockKey, DataMapType, Host, ItemKey, NoBackend};
use horizon_core::nbt::Tag;

/// A data map a mod registers and reads, typed (NeoForge's `DataMapType<R, T>`): its id, the
/// registry its entries are in (through [`DataMap::Entry`]), and how a value decodes.
///
/// Register it in `data_map_types` with [`DataMap::ty`], then read an entry's value with
/// [`ItemKey::data`] or [`BlockKey::data`]. A value arrives as the NBT form of the JSON a file
/// held: a whole number is an `Int`, or a `Long` past `i32`, a fraction a `Double`, a boolean a
/// `Byte`, an object a `Compound` and an array a `List`. Integers outside signed
/// 64-bit range and `null` at any nesting depth return [`DataMapError::Undecodable`]
/// rather than changing the value. Fractional/exponent JSON tokens use the parser's
/// `f64` value; empty and heterogeneous arrays retain their elements.
///
/// ```
/// use mod_api::{DataMap, ItemKey, Tag};
/// /// `projecte:emc`: an item's EMC value.
/// struct Emc;
/// impl DataMap for Emc {
///     type Entry = ItemKey;
///     type Value = i64;
///     fn id(&self) -> &str {
///         "projecte:emc"
///     }
///     fn decode(&self, value: &Tag) -> Option<i64> {
///         match value {
///             Tag::Int(value) => Some(i64::from(*value)),
///             Tag::Long(value) => Some(*value),
///             _ => None,
///         }
///     }
/// }
/// assert_eq!(Emc.ty().registry, "minecraft:item");
/// ```
pub trait DataMap {
    /// The registry entries the map attaches values to: [`ItemKey`] or [`BlockKey`].
    type Entry: DataMapEntry;
    /// The value an entry holds.
    type Value;
    /// The map's id, `mod_id:name`, not in the `minecraft` namespace, as registered.
    fn id(&self) -> &str;
    /// Decode an entry's value, refusing one of another shape.
    fn decode(&self, value: &Tag) -> Option<Self::Value>;
    /// The type to push in `data_map_types`: this map's id, for its entries' registry.
    fn ty(&self) -> DataMapType {
        DataMapType {
            id: self.id().to_owned(),
            registry: <Self::Entry as entry::Entry>::REGISTRY.to_owned(),
        }
    }
}

/// A registry entry a data map can attach a value to: [`ItemKey`] (`minecraft:item`) and
/// [`BlockKey`] (`minecraft:block`). Sealed: the game holds maps for these registries only.
pub trait DataMapEntry: entry::Entry {}
impl DataMapEntry for ItemKey {}
impl DataMapEntry for BlockKey {}

mod entry {
    use super::*;

    /// The registry an entry is in, and its raw read through the game's backend.
    pub trait Entry: Copy {
        /// The registry's id.
        const REGISTRY: &'static str;
        /// The entry's value in map `map`, before decoding.
        fn value(self, backend: &dyn Backend, map: &str) -> Result<Option<Tag>, DataMapError>;
    }
    impl Entry for ItemKey {
        const REGISTRY: &'static str = "minecraft:item";
        fn value(self, backend: &dyn Backend, map: &str) -> Result<Option<Tag>, DataMapError> {
            backend.item_data_map_value(self, map)
        }
    }
    impl Entry for BlockKey {
        const REGISTRY: &'static str = "minecraft:block";
        fn value(self, backend: &dyn Backend, map: &str) -> Result<Option<Tag>, DataMapError> {
            backend.block_data_map_value(self, map)
        }
    }
}

/// Why a data map read has no answer. An entry the map gives no value is `Ok(None)`, not one of
/// these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DataMapError {
    /// No data maps are loaded here: a client (data maps are not synced to clients), a server
    /// before its world's data first loads, or a host with no game behind it.
    Unavailable,
    /// No mod registered the map for the entry's registry in the current data load
    /// (`data_map_types`).
    UnknownType,
    /// The entry's value is not one the map's [`DataMap::decode`] takes.
    Undecodable,
}
impl NoBackend for DataMapError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

/// `entry`'s value in `map`, decoded.
fn data<M: DataMap>(
    entry: M::Entry,
    host: &dyn Host,
    map: &M,
) -> Result<Option<M::Value>, DataMapError> {
    let backend = crate::backend_of(host).ok_or_else(DataMapError::no_backend)?;
    match entry::Entry::value(entry, backend, map.id())? {
        Some(value) => map
            .decode(&value)
            .map(Some)
            .ok_or(DataMapError::Undecodable),
        None => Ok(None),
    }
}

impl ItemKey {
    /// This item's value in data map `map` (`Holder.getData`), as the server's data packs and
    /// mods' files left it: the newest file wins, a `#tag` entry gives every item in the tag
    /// its value, `replace` and `remove` apply, and conditional files count only when their
    /// conditions hold. `Ok(None)` where the map gives this item no value.
    ///
    /// Server side: the server merges the maps when its data loads, on a world's first open
    /// and after every `/reload`, and a read sees the last load whole, never a mix of two.
    /// Readable from `data_maps_updated` on, in the world and while data reloads. A reload
    /// that drops a value drops it here too.
    pub fn data<M: DataMap<Entry = ItemKey>>(
        self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}

impl BlockKey {
    /// This block's value in data map `map` (`Holder.getData`), as for [`ItemKey::data`]: the
    /// server's merged value after every pack and mod file, `Ok(None)` where it has none.
    pub fn data<M: DataMap<Entry = BlockKey>>(
        self,
        host: &dyn Host,
        map: &M,
    ) -> Result<Option<M::Value>, DataMapError> {
        data(self, host, map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host whose data maps hold `test:emc` for items, coal at 10 and a stick at a string,
    /// and `test:hardness` for blocks, stone at 3; every other read answers no value.
    struct Maps;
    impl crate::host::data_maps::DataMapsBackend for Maps {
        fn item_data_map_value(
            &self,
            item: ItemKey,
            map: &str,
        ) -> Result<Option<Tag>, DataMapError> {
            match (map, item.0) {
                ("test:emc", 0) => Ok(Some(Tag::Int(10))),
                ("test:emc", 1) => Ok(Some(Tag::String("ten".into()))),
                ("test:emc", _) => Ok(None),
                _ => Err(DataMapError::UnknownType),
            }
        }
        fn block_data_map_value(
            &self,
            block: BlockKey,
            map: &str,
        ) -> Result<Option<Tag>, DataMapError> {
            match (map, block.0) {
                ("test:hardness", 0) => Ok(Some(Tag::Long(3))),
                ("test:hardness", _) => Ok(None),
                _ => Err(DataMapError::UnknownType),
            }
        }
    }
    impl Host for Maps {
        fn backend(&self) -> Option<crate::BackendRef<'_>> {
            Some(crate::BackendRef::of(self))
        }
    }
    crate::impl_host_areas!(Maps);
    crate::impl_backend_areas!(Maps, except(data_maps));
    impl Backend for Maps {}

    /// No game behind it.
    struct Bare;
    impl Host for Bare {}
    crate::impl_host_areas!(Bare);

    fn number(value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
    struct Emc;
    impl DataMap for Emc {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:emc"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    struct Hardness;
    impl DataMap for Hardness {
        type Entry = BlockKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:hardness"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    /// A map no mod registered.
    struct Unregistered;
    impl DataMap for Unregistered {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            "test:unregistered"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }

    #[test]
    fn a_read_decodes_the_value_and_tells_no_value_from_a_refusal() {
        assert_eq!(ItemKey(0).data(&Maps, &Emc), Ok(Some(10)));
        assert_eq!(BlockKey(0).data(&Maps, &Hardness), Ok(Some(3)));
        assert_eq!(ItemKey(2).data(&Maps, &Emc), Ok(None));
        assert_eq!(BlockKey(1).data(&Maps, &Hardness), Ok(None));
        assert_eq!(ItemKey(1).data(&Maps, &Emc), Err(DataMapError::Undecodable));
        assert_eq!(
            ItemKey(0).data(&Maps, &Unregistered),
            Err(DataMapError::UnknownType)
        );
        assert_eq!(ItemKey(0).data(&Bare, &Emc), Err(DataMapError::Unavailable));
        assert_eq!(
            BlockKey(0).data(&Bare, &Hardness),
            Err(DataMapError::Unavailable)
        );
    }

    #[test]
    fn a_maps_type_names_its_entries_registry() {
        assert_eq!(
            Emc.ty(),
            DataMapType {
                id: "test:emc".into(),
                registry: "minecraft:item".into(),
            }
        );
        assert_eq!(Hardness.ty().registry, "minecraft:block");
    }
}
