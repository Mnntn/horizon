//! API and area slot listeners. `src/count.rs` is the mixin's helper: horizon-mixins compiles it
//! inside the game function, so it is not a module of this crate.
mod area;
pub use area::area;
#[derive(Default)]
struct Check;

pub fn tick(_: mod_api::HostSide<'_>, _: &mut mod_points::TickArgs) {}

#[mod_api::horizon_mod]
impl mod_api::Hooks for Check {}
