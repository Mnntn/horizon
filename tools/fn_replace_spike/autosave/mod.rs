//! The mod: an ordinary native mod crate. `autosave` is its `[[inject]]` `with` function
//! (kind = "modify_value" at `autosave_interval`, nth 0, in `Game::next_autosave_interval`).
use mod_api::{self, Hooks};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {}

pub fn autosave(value: i64) -> i64 {
    let doubled = value * 2;
    eprintln!("fnr_autosave: autosave interval {value} -> {doubled} (listener called inline from the replaced copy)");
    doubled
}
