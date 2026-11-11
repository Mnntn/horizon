# Function replacement spike (branch fn-replace-spike, not for main)

Design note: `/box/shared/fn-replace/design.md`. Everything here is hand glue unless named otherwise.

Reproduce, from the checkout root, in `.local/fnr/`:

1. `python3 tools/prebuilt_game.py build .local/fnr/prebuilt`: the SDK of main 215d201fcd. Nothing here changes it.
2. `autosave/copy.rs` (hand-extracted `Game::next_autosave_interval`, `&self` made a parameter): put it in a one-crate
   workspace as `fnr_patch/src/lib.rs`, then run upstream horizon-mixins (e59da90) on it:
   `horizon-mixins apply --workspace ws --mods mixin-mods --out patched`, where `mixin-mods/autosave/` holds
   `autosave/horizon-mixin.toml` and `autosave/autosave.rs`. The output is `autosave/transformed.rs` (less the replace
   glue appended at its end, hand-written).
3. `build.sh` compiles the mod (`autosave/mod.rs`), the transformed copy and the executable (`autosave/main.rs`, the
   SDK's `native_main.rs.template` with one `replace!` line) with rustc against the SDK folder alone.
4. Put `autosave/horizon-mod.toml` and the mod's source in `out/horizon-native-mods/fnr_autosave/`, start
   `out/horizon-client --resources <prepared> --dedicated --nogui` and type `tick rate 40`:
   `server-log-excerpt.txt`.

`sign/`: the Farmer's Delight canvas sign row (editor_sprite in `draw_sign_editor`), extracted by `extract.py`.
Upstream horizon-mixins refuses this site (`E_UNSUPPORTED_CONTROL_FLOW`, a call beneath a reference), so
`sign/transformed.rs` is a hand edit in the shape a direct-call emitter would write; it compiles against the SDK.

`bench.rs`: the point `horizon_client::game::autosave_interval` called from outside the SDK in each mode;
`bench-results.txt` are its runs (100 M calls each, pinned to one core, on a shared 8-core box under load).
