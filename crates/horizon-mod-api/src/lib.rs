//! Mod-facing API, re-exporting the engine vocabulary.
extern crate self as mod_api;

pub use mod_vocab::*;
include!(concat!(env!("OUT_DIR"), "/mod_api_facade_exports_root.rs"));

/// Mod-side installation and generated-code contracts.
#[doc(hidden)]
pub mod __private {
    pub use mod_vocab::__private::*;
    include!(concat!(
        env!("OUT_DIR"),
        "/mod_api_facade_exports_private.rs"
    ));
}

/// Shared Wire values and the mod-side transport.
pub mod wire {
    pub use mod_vocab::wire::*;
    /// The mod side of the WebAssembly boundary.
    #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
    pub mod wire_guest;
    #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
    pub use wire_guest as guest;
}
