use internals::{game::Game, raw::*};
use std::sync::atomic::{AtomicBool, Ordering};
static API_HEAD: AtomicBool = AtomicBool::new(false);
static HIGH: AtomicBool = AtomicBool::new(false);
pub fn head(_: &mut Game<'_>, _: &mut __OnTickRateChangedRaw<'_>) {
    assert!(
        API_HEAD.swap(false, Ordering::SeqCst),
        "API before internals"
    );
    eprintln!("SLOT ORDER HEAD");
}
pub fn high(game: &mut Game<'_>) {
    HIGH.store(true, Ordering::SeqCst);
    let value = game.ticks_until_autosave;
    eprintln!("SLOT ORDER HIGH ticks_until_autosave={value}");
}
pub fn low(game: &mut Game<'_>) {
    assert!(HIGH.swap(false, Ordering::SeqCst), "priority order");
    assert_eq!(game.ticks_until_autosave, 319);
    eprintln!("SLOT ORDER LOW APPLIED ticks_until_autosave=319");
}
pub fn value(_: (), value: i64) -> i64 {
    eprintln!("SLOT ORDER MODIFY_VALUE: {value} -> {}", value + 7);
    value + 7
}
pub fn wrap<'g, A>(
    r: &Game<'g>,
    args: &mut A,
    original: &mut dyn FnMut(&Game<'g>, &mut A) -> i64,
) -> i64 {
    let value = original(r, args);
    eprintln!("SLOT ORDER WRAP_CALL: {value} -> {}", value + 11);
    value + 11
}
#[derive(Default)]
struct Mod;
#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

pub fn api_head(_: mod_api::HostSide<'_>, args: &mut mod_points::OnTickRateChangedArgs) {
    assert!(!args.cancel);
    assert_eq!(
        mod_api::content::crash::current_native_mod(),
        Some("native_slot_order")
    );
    API_HEAD.store(true, Ordering::SeqCst);
    eprintln!("SLOT ORDER API HEAD");
}
pub fn api_return(_: &dyn mod_api::Host, args: &mut mod_points::NextAutosaveIntervalArgs) {
    let value = args.result.as_mut().expect("the return has its value");
    eprintln!("SLOT ORDER API RETURN: {value} -> {}", *value + 1);
    *value += 1;
}
