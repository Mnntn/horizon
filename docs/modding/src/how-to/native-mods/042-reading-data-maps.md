<!-- chapter: Reading data maps -->
<!-- separator: 1 -->

## Reading data maps

A data map attaches a value to the entries of a registry (NeoForge's `DataMapType`), here
`minecraft:item` and `minecraft:block`. Data packs and mods' files set the values; a mod reads
what the server made of them, so a server pack can rebalance a mod's numbers without a rebuild.

Describe the map once with `DataMap`: its id, the registry its entries are in (`Entry`, an
`ItemKey` or a `BlockKey`), and how a value decodes. Values arrive as the NBT form of the JSON a
file held: a whole number is an `Int`, or a `Long` past `i32`, a fraction a `Double`, a boolean a
`Byte`, an object a `Compound` and an array a `List`. Integer tokens must fit signed
64-bit NBT `Long`; larger unsigned integers return `DataMapError::Undecodable`,
never a rounded `Double`. Fractional/exponent tokens use the JSON parser's `f64`
value (including an integral value written as `1.0`). A JSON `null` anywhere in a
value also refuses the entire read as `Undecodable`; it is not an empty string.
Empty and heterogeneous arrays preserve their elements as in-memory `List` values.
These rules are identical for native and wasm readers.

```rust,ignore
use mod_api::{DataMap, DataMapTypesEvent, ItemKey, ServerHost, Tag};

/// `mymod:emc`: an item's value.
struct Emc;
impl DataMap for Emc {
    type Entry = ItemKey;
    type Value = i64;
    fn id(&self) -> &str {
        "mymod:emc"
    }
    fn decode(&self, value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
}

// In `impl Hooks`: register it each time the server loads its data.
fn data_map_types(&self, _: &mut dyn ServerHost, event: &mut DataMapTypesEvent<'_>) {
    event.types.push(Emc.ty());
}
```

Its values are `data/mymod/data_maps/item/emc.json` in any data pack, and files the mod pushes
through `data_pack_files` under `data_maps/mymod:item/emc`:

```json
{ "values": { "minecraft:coal": 10, "#minecraft:logs": 32 } }
```

Read an entry with `item.data(host, &Emc)` or `block.data(host, &map)`, from any hook with a host:

- `Ok(Some(value))`: the merged value. The newest file wins, a `#tag` gives every entry in it the
  value, `replace` drops what earlier files set and `remove` takes entries away.
- `Ok(None)`: the map gives this entry no value.
- `Err(DataMapError::UnknownType)`: no mod registered the map for that registry in this load.
- `Err(DataMapError::Undecodable)`: the value is not one `decode` takes.
- `Err(DataMapError::Unavailable)`: no maps are loaded here. Data maps live on the server (the
  single player server thread included) and are not synced to clients, so a client always
  answers this, as does a server before its world's data first loads.

The server merges every map when its data loads: a world's first open and every `/reload`. A
read sees the last load whole, never part of one, and a value a reload drops is gone at once.
`data_maps_updated` fires after each load, the place to rebuild anything computed from the
values; reads also work in the world. A lookup reads the loaded map: it parses nothing and
touches no file.

Wasm mods read data maps the same way, with the same answers.
