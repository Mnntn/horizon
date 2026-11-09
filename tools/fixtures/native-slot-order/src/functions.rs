// One unchanged callback source compiled against both listed slots and private source points.
use internals::{game::Game, raw::*};
use std::sync::atomic::{AtomicBool, Ordering};
static HIGH: AtomicBool = AtomicBool::new(false);
pub fn head(_: &mut Game<'_>, _: &mut __OnTickRateChangedRaw<'_>) {
    assert_eq!(
        mod_api::content::crash::current_native_mod(),
        Some("native_slot_order")
    );
    eprintln!("SLOT ORDER HEAD");
}
pub fn return_value(_: &Game<'_>, value: &mut i64) {
    eprintln!("SLOT ORDER SOURCE RETURN: {value} -> {}", *value + 19);
    *value += 19;
}
pub fn high(game: &mut Game<'_>) {
    HIGH.store(true, Ordering::SeqCst);
    eprintln!(
        "SLOT ORDER HIGH ticks_until_autosave={}",
        game.ticks_until_autosave
    );
}
pub fn low(game: &mut Game<'_>) {
    assert!(HIGH.swap(false, Ordering::SeqCst), "priority order");
    assert_eq!(game.ticks_until_autosave, 319);
    eprintln!("SLOT ORDER LOW APPLIED ticks_until_autosave=319");
}
#[derive(Default)]
struct Mod;
#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}
