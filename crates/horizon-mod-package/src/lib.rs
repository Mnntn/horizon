//! Horizon's mod files (`docs/modding/src/how-to/content-mods.md`): content mods (`.hmod`: blocks, items, data and
//! assets, no code), modpacks and, with `index`, the mod registry's index. Code mods are
//! native (docs/modding/src/how-to/native-mods.md).
pub mod content;
pub mod dependencies;
pub mod enabled;
pub mod game_identity;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod holder;
pub mod icon;
#[cfg(feature = "index")]
pub mod index;
pub mod install_lock;
pub mod internals;
pub mod layouts;
#[cfg(all(
    feature = "runtime-process",
    any(target_os = "linux", target_os = "macos", windows)
))]
pub mod local_control;
#[cfg(feature = "runtime-process")]
pub mod locks;
pub mod manifest;
pub mod mod_list;
#[cfg(not(target_arch = "wasm32"))]
pub mod modpack;
#[cfg(feature = "runtime-process")]
pub mod output;
pub mod packages;
pub mod runtime;
#[cfg(feature = "runtime-process")]
pub mod runtime_leases;
pub mod selection;

pub use content::Content;
pub use manifest::{Capability, Manifest, Package, Side, find};

/// Filename of the versioned SDK tool catalog.
pub use point_shape::SDK_CATALOG_FILE;
