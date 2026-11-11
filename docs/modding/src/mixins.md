# Native mixins

Hooks and Parts are the shared API for native and wasm mods. Implement `mod_api::Hooks` or the appropriate Part method when it covers the behaviour you need.

A native mod can change game code with `[[native.mixin]]` rows. The launcher compiles a patched copy of each targeted function against the pinned prebuilt game and installs that copy. It does not rebuild the game. Wasm mods cannot declare native code transformations.

Each row has `id`, `target`, `kind` and a `helper` Rust file under `src/`. Use the pinned horizon-mixins vocabulary: HEAD, RETURN, BEFORE_CALL, AFTER_CALL, AFTER_ASSIGN, MODIFY_ARG, MODIFY_ARGS, MODIFY_CONSTANT, MODIFY_VARIABLE, MODIFY_EXPRESSION_VALUE, WRAP_OPERATION or OVERWRITE. Bindings and selectors describe the values the helper needs. Unsupported selectors or control flow refuse the build; do not silently choose another call.

```toml
[[native.mixin]]
id = "longer_autosave"
target = "horizon_client::game::Game::next_autosave"
kind = "MODIFY_EXPRESSION_VALUE"
helper = "src/longer.rs"
```

Native draw functions such as `horizon_mod_render::render::world_drawn`, `hand_drawn` and `world_views` are reached with a HEAD mixin. Engine hook functions named `native_*` are replaceable too. Mixins operate on native Rust values, including live mutable references, not API copies.

The patch-library support tables are `[[native.bridge]]` and `[[native.module]]`. They do not
compile game crates. Unsupported bridge or module declarations are refused by name.
`[[native.state]]` is refused with `E_MIXIN_STATE_LAYOUT`: the prebuilt game's layout is fixed;
use attachments for per-object mod data. Declare private engine items through the fingerprinted
`internals` list. Pin the game and tool revisions and rebuild the mod when those pins change.

The pinned commit needs a published SDK (`E_NATIVE_NO_RELEASE`) for a supported host
(`E_NATIVE_HOST`). A mod's own build script is `E_NATIVE_BUILD_SCRIPT`; ship generated source
instead. Custom mod or SDK feature selections are `E_NATIVE_FEATURES`; use the SDK's feature set.

Prefer a hook request when the behaviour should also work on wasm. A native mixin is trusted code, not a sandbox.

## Pinned tool documentation

The vocabulary is horizon-mixins' pinned vocabulary. Read its [Mixin API](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/mixin-api.md), [advanced injections](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/advanced-injections.md), and [scopes and bridges](https://github.com/horizon-rust/horizon-mixins/blob/e59da9094f0c2fdda6c627fe9eff22dec6a906f0/docs/scopes-and-bridges.md) for selector and binding contracts.
