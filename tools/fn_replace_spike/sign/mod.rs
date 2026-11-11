//! Farmer's Delight's canvas sign editor sprite (port row 42), as a `with` function.
use mod_api::{self, Hooks};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {}

pub fn canvas_sign(value: String, wood: &str) -> String {
    if wood == "farmersdelight:canvas" { "farmersdelight:gui/signs/canvas".into() } else { value }
}
