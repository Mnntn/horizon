//! Client-only surfaces: screens, input, sound, drawing and options.

// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod camera;
pub(crate) mod client_reload;
pub(crate) mod client_session;
pub(crate) mod debug_entries;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras_hud;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod first_person;
pub(crate) mod fluid_draw;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod fog;
pub(crate) mod gui_layers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod hud_layout;
// hidden: module of API-owned HUD values; mods use the re-export
#[doc(hidden)]
pub mod hud_mode;
pub(crate) mod key_mapping;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod mesher;
pub(crate) mod model_instance;
pub(crate) mod model_layers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod music;
pub(crate) mod pack_finders;
pub(crate) mod particle_keys;
pub(crate) mod particle_providers;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod particles;
pub(crate) mod posed_parts;
pub(crate) mod scene;
pub(crate) mod screenshot;
pub(crate) mod terrain_draw;
pub(crate) mod types;
/// Buttons and text fields for mod screens.
pub mod widgets;
pub(crate) mod world_draw;
