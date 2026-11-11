# Mod extensions

Mods declare required and optional mod dependencies in `[dependencies]`. Libraries expose typed Rust functions, traits and explicit callback registration to their dependents. Native and wasm use the same dependency graph and mod versions.

Use hooks and Parts for engine behaviour. A library controls its own extension contracts; consumers call its published interface. Native game changes use [mixins](../mixins.md). Wasm mods use hooks and Parts.
