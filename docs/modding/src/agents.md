# Rules for agents

1. Implement `mod_api::Hooks` and Parts first. Native and wasm share this API.
2. For native game code changes, use [mixins](mixins.md). The launcher compiles patched functions against the prebuilt game, without rebuilding it.
3. Request a typed hook when wasm needs a capability the API does not expose.
4. Keep `horizon-mod.toml`, `Cargo.toml`, `src/`, `assets/` and `data/` at the project root. Mod identity comes from the manifest.
5. Pin native game and tool revisions. Declare engine internals explicitly. Unknown manifest fields are refused.
6. Read assets on reload, not every tick. Hooks with NeoForge counterparts use their NeoForge names.
