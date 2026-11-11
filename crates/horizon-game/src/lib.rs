//! The game as the platform runs it, for an executable to start (docs/layers.md, "How a Horizon
//! crate plugs in"). The official executable (`horizon-official`, Horizon layer) registers its
//! hooks (`horizon_platform::hooks`) and then calls [`main`]; it never names an engine crate.
//!
//! An executable built on it:
//!
//! ```ignore
//! static GAME_IDENTITY: [u8; horizon_game::GAME_IDENTITY.len()] = horizon_game::GAME_IDENTITY;
//!
//! fn main() {
//!     std::hint::black_box(&GAME_IDENTITY);
//!     my_crate::register();
//!     horizon_game::main();
//! }
//! ```
//!
//! The platform's own parts register here, in [`main`], as they leave the engine (layer plan M6).
#![forbid(unsafe_code)]

/// Identifies an executable as the game for launcher imports, without an API version or build id.
pub const GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] =
    horizon_mod_package::game_identity::MARK;

/// Runs the game with the hooks registered so far, and returns when it ends. Register hooks
/// (`horizon_platform::hooks::register`) before calling it.
pub fn main() {
    if let Some(code) = horizon_mod_host::compile_child(std::env::args_os().skip(1)) {
        std::process::exit(code);
    }
    let startup = horizon_client::entry::prepare();
    std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);
    horizon_mod_host::main_with(Vec::new(), &[], &[], startup);
}
