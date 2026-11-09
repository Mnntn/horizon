//! Compile with rustc --edition 2024 --test; exercises the real scalar converter without the game.
#![allow(dead_code)]
extern crate self as horizon_world;
extern crate self as mod_api;
#[path = "../../../../crates/horizon-mod-api/src/entity/health.rs"]
mod api_health;
pub use api_health::*;
#[path = "probe_game.rs"]
mod game;
#[path = "../../../../crates/horizon-world/src/living_damage.rs"]
pub mod living_damage;
