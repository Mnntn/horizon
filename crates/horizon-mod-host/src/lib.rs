//! Installed mod code loading above the engine (docs/layers.md, #2534).
//! The platform prepares the engine, loads mods here, then makes one engine start call.
#![forbid(unsafe_code)]

mod loading;
pub use loading::main_with;

pub use horizon_mod_wasm::compile_child;
