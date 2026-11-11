# Your first native mod

A native mod is one flat Rust project that implements `mod_api::Hooks`, built by the launcher into the game it runs
in. You need the launcher and a Horizon commit to build against.

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
   Add `--profile developer` while developing for an unoptimized game crate. The choice is remembered for that instance.