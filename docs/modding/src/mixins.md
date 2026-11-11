# Native mixins

Hooks and Parts are the shared API for native and wasm mods. Implement `mod_api::Hooks` or the appropriate Part method when it covers the behaviour you need.

A native mod can change game code with `[[mixin]]` rows. The launcher compiles a patched copy of each targeted function against the pinned prebuilt game and installs that copy. It does not rebuild the game. Wasm mods cannot declare native code transformations.

Each row has `id`, `target`, `kind` and a `helper` Rust file under `src/`. Use the pinned horizon-mixins vocabulary: HEAD, RETURN, BEFORE_CALL, AFTER_CALL, AFTER_ASSIGN, MODIFY_ARG, MODIFY_ARGS, MODIFY_CONSTANT, MODIFY_VARIABLE, MODIFY_EXPRESSION_VALUE, WRAP_OPERATION or OVERWRITE. Bindings and selectors describe the values the helper needs. Unsupported selectors or control flow refuse the build; do not silently choose another call.

```toml
[[mixin]]
id = "longer_autosave"
target = "horizon_client::game::Game::next_autosave"
kind = "MODIFY_EXPRESSION_VALUE"
helper = "src/longer.rs"
```

Native draw functions such as `horizon_mod_render::render::world_drawn`, `hand_drawn` and `world_views` are reached with a HEAD mixin. Engine hook functions named `native_*` are replaceable too. Mixins operate on native Rust values, including live mutable references, not API copies.

The `[native]` support tables are `[[native.bridge]]`, `[[native.state]]` and `[[native.module]]`. They provide access bridges, state and support modules. Declare private engine items through the fingerprinted `internals` list. Pin the game and tool revisions in the manifest and rebuild the mod when those pins change.

Prefer a hook request when the behaviour should also work on wasm. A native mixin is trusted code, not a sandbox.
