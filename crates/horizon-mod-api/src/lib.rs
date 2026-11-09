//! Trait mods implement `Hooks` and link into the game executable.
//! The executable passes the ordered `Mods` to `entry::main_with`.
//! Trait mods are process-wide, like mixin module statics.
//! Hooks use `&self + Send + Sync` for shared and reentrant calls; see `docs/modding/src/how-to/native-mods.md`.
//! Generate the trait and dispatch with `cargo run -p xtask -- mod-api`.
#![deny(missing_docs)]
extern crate self as mod_api;

use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value};
use std::time::Duration;

mod generated_host {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated_host.rs"));
}

// no wire: type alias inherits the Wire policy of its tuple or Result target.
/// Result of accessing or decoding an attachment.
pub type AttachmentResult<T> = std::result::Result<T, AttachmentError>;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
}

include!(concat!(env!("OUT_DIR"), "/mod_api_exports_root.rs"));
include!(concat!(env!("OUT_DIR"), "/wire_generated.rs"));

#[cfg(test)]
mod brewing_dispatch_tests {
    include!("item/brewing_dispatch_tests.rs");
}

#[cfg(test)]
mod item_model_dispatch_tests {
    include!("item/item_model_dispatch_tests.rs");
}
