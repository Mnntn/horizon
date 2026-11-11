//! Spike bench (fn-replace-spike): the cost of one game function point,
//! `horizon_client::game::autosave_interval(rate: f32) -> i64`, called from outside the prebuilt
//! game, in each way a mod can change it. Built with rustc against the SDK folder alone.
//!
//! usage: bench <mode> [millions of calls]
//!   plain    a local copy of the body, never inlined: the call with no point at all
//!   idle     the game's function, nothing listening (head and return slot loads)
//!   head     an internals head listener that only observes (today's raw slot dispatch)
//!   return   an internals return listener doubling the value (today's closest form of a
//!            modify_value on this function's result)
//!   replace  an internals `replace` whose body is the transformed copy with its modify_value
//!            listener compiled inline (function replacement through today's replace slot)
//!   redirect        what an entry redirect would cost: a stand-in for the game function whose
//!                   head loads one function pointer and calls the inline-listener copy through it
//!   redirect_guard  the same, entering the mod's crash-attribution guard around the copy
//!   redirect_idle   the stand-in with no redirect installed (its idle cost against `plain`)
// The game is one dylib (horizon-mod-host); naming it links the game crates through it.
extern crate horizon_mod_host;
use horizon_client::game as g;
use mod_points::__private::SlotListener;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::Instant;

#[inline(never)]
fn plain(rate: f32) -> i64 {
    i64::from((rate * 300.0) as i32).max(100)
}

/// The mod's listener: what `with` names.
#[inline]
fn listener(value: i64) -> i64 {
    value * 2
}

/// The transformed copy: the body of `autosave_interval` with the listener at its value.
fn replacement(raw: &mut g::__GameAutosaveIntervalRaw<'_, f32, i64>) -> i64 {
    let rate = *raw.rate;
    listener(i64::from((rate * 300.0) as i32).max(100))
}

/// The entry redirect a replacement-only engine would keep per function: one pointer, null when
/// nothing replaces the function.
static REDIRECT: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static GUARDED: AtomicBool = AtomicBool::new(false);

#[inline(never)]
fn redirected(rate: f32) -> i64 {
    let to = REDIRECT.load(Ordering::Relaxed);
    if !to.is_null() {
        return mod_api::__private::points::cold(|| {
            // SAFETY: only `copy` is ever stored, and it has this signature.
            let to = unsafe { std::mem::transmute::<*mut (), fn(f32) -> i64>(to) };
            if GUARDED.load(Ordering::Relaxed) {
                let _guard = mod_api::__private::enter("bench", "replace");
                to(rate)
            } else {
                to(rate)
            }
        });
    }
    i64::from((rate * 300.0) as i32).max(100)
}

/// The transformed copy as the redirect calls it: its own signature, the listener inline.
fn copy(rate: f32) -> i64 {
    listener(i64::from((rate * 300.0) as i32).max(100))
}

fn observe(_raw: &mut g::__GameAutosaveIntervalRaw<'_, f32, i64>) {}

fn double(value: &mut i64) {
    *value = listener(*value);
}

const fn head(id: &'static str, f: g::__GameAutosaveIntervalListener) -> SlotListener {
    // SAFETY: `f` has this place's listener type.
    unsafe { SlotListener::new(id, mod_points::PointId::GameAutosaveInterval.place(), f as *const ()) }
}

const fn ret(id: &'static str, f: g::__GameAutosaveIntervalReturnListener) -> SlotListener {
    // SAFETY: `f` has this place's return listener type.
    unsafe { SlotListener::new(id, mod_points::PointId::GameAutosaveIntervalReturn.place(), f as *const ()) }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode");
    let calls: u64 = args.next().map_or(200, |n| n.parse().unwrap()) * 1_000_000;
    let slot = match mode.as_str() {
        "head" => Some(head("bench", observe)),
        "return" => Some(ret("bench", double)),
        "replace" => Some(mod_points::replace!(internals head, result, none, "bench", 0, "replace", replacement)),
        _ => None,
    };
    if let Some(slot) = slot {
        mod_points::register(&slot).expect("register");
    }
    mod_points::__private::seal();
    if mode == "redirect" || mode == "redirect_guard" {
        REDIRECT.store(copy as *mut (), Ordering::Relaxed);
        GUARDED.store(mode == "redirect_guard", Ordering::Relaxed);
    }
    let f: fn(f32) -> i64 = match mode.as_str() {
        "plain" => plain,
        m if m.starts_with("redirect") => redirected,
        _ => g::autosave_interval,
    };
    let check = f(1.0);
    let started = Instant::now();
    let mut sum = 0i64;
    for i in 0..calls {
        sum = sum.wrapping_add(f(black_box(20.0 + (i & 7) as f32)));
    }
    let elapsed = started.elapsed();
    println!(
        "{mode}: {:.2} ns/call (f(1.0) = {check}, sum {sum})",
        elapsed.as_nanos() as f64 / calls as f64
    );
}
