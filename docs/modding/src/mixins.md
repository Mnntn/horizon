# Mixins

A `[[native.mixin]]` is a source-level edit made by horizon-mixins. Its helper is compiled
inside a game function, where Rust privacy and explicit bindings determine what
it can use. The game is rebuilt on the player's computer. Mixins are trusted like
internals and are not sandboxed. Only the tables of `[native]` compile game code, and a
manifest is `[native]` or `[wasm]`, never both, so a wasm mod cannot declare one: it is
refused with that reason.

## Declaration

Each row in `horizon-mod.toml` becomes one upstream `[[injection]]` row:

| Key | Meaning |
| --- | --- |
| `id` | Unique declaration ID |
| `target` | Upstream function ID |
| `kind` | Uppercase upstream kind |
| `helper` | Relative Rust helper file |
| Other upstream keys | Forwarded unchanged; validated by horizon-mixins |

Pass-through keys include `cancellable`, `argument`, `bindings`, `priority`,
`selector`, `ordinal`, `require`, `fingerprint`, `slice`, `variable` and `result`.
`hook` and `hook_version` are refused, not translated. Invalid declarations report
`E_MIXIN_INVALID` or the upstream tool's diagnostic.

The vocabulary is upstream's: HEAD, RETURN, BEFORE_CALL, AFTER_CALL, AFTER_ASSIGN,
MODIFY_ARG, MODIFY_ARGS, MODIFY_CONSTANT, MODIFY_VARIABLE, MODIFY_EXPRESSION_VALUE,
WRAP_OPERATION and OVERWRITE. Read the pinned upstream documentation rather than
assuming Java annotation semantics:

- [Mixin API](https://github.com/horizon-rust/horizon-mixins/blob/5a64af938ec86b00d53d16a408aabc0038fb3e57/docs/mixin-api.md)
- [Advanced injections](https://github.com/horizon-rust/horizon-mixins/blob/5a64af938ec86b00d53d16a408aabc0038fb3e57/docs/advanced-injections.md)
- [Scopes, slices and bridges](https://github.com/horizon-rust/horizon-mixins/blob/5a64af938ec86b00d53d16a408aabc0038fb3e57/docs/scopes-and-bridges.md)
- [Effects and state](https://github.com/horizon-rust/horizon-mixins/blob/5a64af938ec86b00d53d16a408aabc0038fb3e57/docs/effects-and-state.md)
- [Boundary HEAD and modules](https://github.com/horizon-rust/horizon-mixins/blob/5a64af938ec86b00d53d16a408aabc0038fb3e57/docs/boundary-head-and-modules.md)

The launcher pins this tool with `MIXINS_REV`. `[[native.bridge]]`,
`[[native.state]]` and `[[native.module]]` become upstream `[[bridge]]`, `[[state]]`
and `[[module]]`, respectively, with their keys unchanged. These provide
accessor/invoker bridges, per-instance state and self-contained support modules.
State does not automatically connect to world saves; prefer attachments for
persistent mod data. Module code contains items without an outer module wrapper.
These tables also require trusted native builds.

## Example: observing a command

This manifest fragment inserts a helper at the head of the pinned game's command
method. Check the target ID against that game's source catalog.

```toml
[[native.mixin]]
id = "observe_command"
target = "horizon_client::game::commands::Game<'_>::run_command"
kind = "HEAD"
helper = "src/observe_command.rs"
bindings = [{ name = "command", ty = "&str", value = "command" }]
```

`src/observe_command.rs` contains one private helper function:

```rust,ignore
fn observe(command: &str) {
    eprintln!("command: {command}");
}
```

The binding supplies the target parameter explicitly; target locals are not
implicit helper context. The launcher writes private tool input, runs the
upstream planner and checks the staged game with Cargo. Changed selectors,
unsupported control flow and type or lifetime errors stop the build; they do not
fall back to another target or to an inject.

## Which do I use?

| Mechanism | Use it for | Compiles game code | Targets |
| --- | --- | --- | --- |
| Hook or Part method | A documented named moment or object operation | No | Native and wasm |
| `[[inject]]` | A compiled function point or listed call site | Never | API function points on native and wasm; engine internals and sites on native |
| `[[native.mixin]]` | A source edit supported by horizon-mixins | Yes | Native only |

Prefer the named hook. When none exists, [request one](injections.md#ask-for-the-hook)
even if an inject or mixin supplies the moment today.

Capabilities that are not compiled inject slots use these forms:

- Head or return at an unlisted function: mixin HEAD or RETURN.
- Unlisted call site: BEFORE_CALL, AFTER_CALL, MODIFY_ARG(S),
  MODIFY_EXPRESSION_VALUE or WRAP_OPERATION, or request a listed slot.
- Local modification: MODIFY_VARIABLE, not inject `modify_local`.
- Constants and expression results: MODIFY_CONSTANT or MODIFY_EXPRESSION_VALUE,
  not inject `expression`.
- Added fields: `[[native.state]]`, not inject `field`.

There is a real gap: a call site upstream refuses (for example with
`E_UNSUPPORTED_CONTROL_FLOW`) and that is not yet a listed slot has no route.
It must wait until its `slots/<mod_id>.txt` line ships in a Nightly. Request it
with `cargo horizon-mod inject sync --repo /path/to/horizon`; this writes the
request, not a slot into an already installed game.
