//! The `data_map_types` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0150

pub trait Hooks {
    /// The server is assembling its data tables, as for
    /// `native_data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`, `minecraft:item` or `minecraft:block`
    /// so far. Its values come from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry path>/<name>.json`, and from files
    /// you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry path>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values) and `remove`, a newer value
    /// overriding an older. Push a typed map's `DataMap::ty`
    /// and read its values with `ItemKey::data` and `BlockKey::data`. Server-side,
    /// not synced to clients.
    #[hook(
        side = "server",
        contract = "collect",
        out = "types",
        neoforge = "RegisterDataMapTypesEvent"
    )]
    fn data_map_types();
}
