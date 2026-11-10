//! Horizon: native Rust client for Minecraft Java Edition 26.3 gameplay.
#![forbid(unsafe_code)]
// Shared mod fixtures exercise the public facade; production uses vocabulary.
#[cfg(test)]
extern crate mod_api_facade as mod_api;

/// Identifies this executable as the game for launcher imports. `main` reads it so the
/// linker keeps it; it carries neither an API version nor a per-build id.
static GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] =
    horizon_mod_package::game_identity::MARK;

fn main() {
    let startup = horizon_client::entry::prepare();
    std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);
    std::hint::black_box(&GAME_IDENTITY);
    horizon_client::entry::main(startup);
}
