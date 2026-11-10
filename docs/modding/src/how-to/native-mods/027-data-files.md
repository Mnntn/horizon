<!-- chapter: Data files -->
<!-- separator: 1 -->

### Data files

A mod has one top-level `data/` tree and one `assets/` tree, with the same namespace and
content checks as a content mod. They are normal pack resources, not a second native-only data folder.
[Server data reload listeners](../../reference/hook-notes.md#server-data-reload-listeners) register through
`Hooks::add_server_reload_listeners`. Their `ServerReloadContext` lists namespaced IDs and reads the
winning bytes from selected datapacks and the mod's own `data/`. Parse once in prepare and publish in
apply, not on the tick. Server listener applies run before `data_pack_files`. A later refusal
keeps the old engine tables and Parts, but cannot undo effects of listeners or hooks that already
ran. Contributions share a list, so a failed file cannot be attributed to a particular mod.
The original pack trees remain separate from the launcher's code-only horizon-mixins input.

Client-side asset reads use [client asset reload listeners](../../reference/hook-notes.md#client-asset-reload-listeners); server
listeners cannot read `assets/`. Packaged assets also participate in the game's normal resource
packs.

Outside a listener, `host.resource(pack_type, &identifier)` reads one file from the selected
stack: `PackType::ServerData` takes `mymod:machines/furnace.json` from `data/`, and
`PackType::ClientResources` takes `mymod:models/block/furnace.json` from `assets/`. As with
`ResourceManager.getResource`, the highest selected pack that supplies the file wins; filters,
a world's datapacks and a mod's own root data all apply. The answer is the winning bytes, or
`None` for a missing, unreadable or over-64-MiB file. Use a reload listener for a whole folder
or for parsing at reload time; a single read does not reload packs. Directory-pack
symlinks follow vanilla pack behavior: linked resource files are read through the pack.

Code-only bytes belong inside the crate: use Cargo's ordinary `build.rs` to generate a table in
`OUT_DIR`, then compile it with `include_bytes!(concat!(env!("OUT_DIR"), "/table.bin"))`.
They are not runtime pack resources and do not participate in resource reloads.

**What a native bundle holds.** The one manifest, optional icon, root `Cargo.toml` and `build.rs`,
`.rs` and `.toml` files under `src/`, `assets/` and `data/`, and root text files (`README*`, `LICENSE*`,
`LICENCE*`, `COPYING*`, `NOTICE*`). Source and resource size limits are checked separately;
anything else refuses the archive ([the manifest](../../reference/manifest.md)).

**One `.hmod` for content and code.** Without root `Cargo.toml`, nothing compiles. With it and `[native]`,
the launcher builds the code and the game loads the same package's content through `horizon_mod_package::Content`.
Both kinds use `horizon-mod.toml` for their identity. Executable `mod.wasm` files are unsupported.
