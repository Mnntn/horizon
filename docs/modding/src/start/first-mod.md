# Your first native mod

A native mod is one flat Rust project that implements `mod_api::Hooks`, compiled by the launcher against a prebuilt game SDK. You need the launcher and a Horizon commit
with a published SDK.

1. Make the project:

   ```sh
   cargo horizon-mod new my_mod --template native
   ```

   It writes `horizon-mod.toml` (the manifest, with the exact build pins under `[native]`), `Cargo.toml` and
   `src/lib.rs`, which implements one hook.

2. Look at the hook. Each method of `Hooks` is a hook point; implement the ones the mod needs. Every
   `impl Hooks` carries `#[mod_api::horizon_mod]`, which subscribes the mod to just the methods in that block:

   ```rust,ignore
   {{#include ../../examples/src/lib.rs:tick_hook}}
   ```

3. Build and run it in the launcher: `horizon-launcher native-build . --instance <instance> --trust-native-code`.
   The launcher compiles the mod and links its executable without compiling game crates.
   `E_NATIVE_NO_RELEASE` means the pinned commit needs a published SDK; choose a published commit.
   See [build refusals](../how-to/native-mods/028-what-the-launcher-does.md#build-refusals) for unsupported inputs.