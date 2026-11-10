//! Internals `replace` listeners (native only) at real points, through
//! `mod_points::replace!(internals …)`, the expression the launcher's generated `main` emits: the
//! body never runs, the listener's value reaches the caller, it runs after every other head
//! listener whatever its priority, a second replace of either tier is refused, and a stopped
//! owner hands the function back.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_points::{__private::SlotListener, PointId, RegisterError, register};
use std::sync::atomic::{AtomicUsize, Ordering};

static SHUTDOWN_BODY: AtomicUsize = AtomicUsize::new(0);
static AVERAGE_BODY: AtomicUsize = AtomicUsize::new(0);
static REPLACED: AtomicUsize = AtomicUsize::new(0);

/// A unit-returning point with a raw form: its raw record has `cancel`.
#[horizon_mod_api_macros::point(name = "game_server_lifecycle_remember_shutdown", raw, free)]
fn remember_shutdown(failure: &mut Option<String>, message: String) {
    SHUTDOWN_BODY.fetch_add(1, Ordering::Relaxed);
    if failure.is_none() {
        *failure = Some(message);
    }
}

/// A value-returning point with a raw form: its raw record has `result`.
#[horizon_mod_api_macros::point(name = "game_beacon_average", raw, free)]
fn average(a: u32, b: u32) -> u32 {
    AVERAGE_BODY.fetch_add(1, Ordering::Relaxed);
    (a + b) / 2
}

// What the staging pass declares beside each function.
pub struct __GameServerLifecycleRememberShutdownRaw<'r, T0, T1> {
    pub failure: &'r mut T0,
    pub message: &'r mut T1,
    pub cancel: &'r mut bool,
}
pub type __GameServerLifecycleRememberShutdownListener = for<'r> fn(
    &'r mut __GameServerLifecycleRememberShutdownRaw<'r, &'_ mut Option<String>, String>,
);
pub type __GameServerLifecycleRememberShutdownReturnListener = fn();
pub struct __GameBeaconAverageRaw<'r, T0, T1, R> {
    pub a: &'r mut T0,
    pub b: &'r mut T1,
    pub result: &'r mut Option<R>,
}
pub type __GameBeaconAverageListener =
    for<'r> fn(&'r mut __GameBeaconAverageRaw<'r, u32, u32, u32>);
pub type __GameBeaconAverageReturnListener = for<'r> fn(&'r mut u32);

/// The mod facade's `raw` constructors, as the launcher generates them.
mod raw {
    use super::*;
    pub const fn game_beacon_average(
        id: &'static str,
        f: __GameBeaconAverageListener,
    ) -> SlotListener {
        // SAFETY: `f` has this place's listener type, checked by the signature.
        unsafe { SlotListener::new(id, PointId::GameBeaconAverage.place(), f as *const ()) }
    }
    pub const fn game_server_lifecycle_remember_shutdown(
        id: &'static str,
        f: __GameServerLifecycleRememberShutdownListener,
    ) -> SlotListener {
        // SAFETY: `f` has this place's listener type, checked by the signature.
        unsafe {
            SlotListener::new(
                id,
                PointId::GameServerLifecycleRememberShutdown.place(),
                f as *const (),
            )
        }
    }
}

/// Leaves `cancel` alone: the replace contract forces it.
fn own_shutdown(
    raw: &mut __GameServerLifecycleRememberShutdownRaw<'_, &mut Option<String>, String>,
) {
    REPLACED.fetch_add(1, Ordering::Relaxed);
    raw.message.push('!');
}

/// Produces the function's value from the raw arguments.
fn own_average(raw: &mut __GameBeaconAverageRaw<'_, u32, u32, u32>) -> u32 {
    (*raw.a).max(*raw.b) * 10
}

fn other_average(_: &mut mod_points::GameBeaconAverageArgs) -> u32 {
    0
}

fn other_raw_average(_: &mut __GameBeaconAverageRaw<'_, u32, u32, u32>) -> u32 {
    1
}

/// An API head listener: it runs first, and the replace sees its change.
fn raise_b(args: &mut mod_points::GameBeaconAverageArgs) {
    args.b += 1;
}

/// An internals head listener with a higher priority than the replace: it still runs before it.
fn raise_a(raw: &mut __GameBeaconAverageRaw<'_, u32, u32, u32>) {
    *raw.a += 100;
}

/// An internals head listener with a lower priority, trying to set the answer: the replace runs
/// last whatever its priority, so its value stands.
fn answer_average(raw: &mut __GameBeaconAverageRaw<'_, u32, u32, u32>) {
    *raw.result = Some(5);
}

#[test]
fn an_internals_replace_owns_the_function_and_only_one_may() {
    register(&mod_points::replace!(
        internals raw::game_beacon_average,
        result,
        none,
        "owner",
        -50,
        "own_average",
        own_average
    ))
    .unwrap();
    register(&mod_points::replace!(
        internals raw::game_server_lifecycle_remember_shutdown,
        cancel,
        none,
        "quitter",
        0,
        "own_shutdown",
        own_shutdown
    ))
    .unwrap();
    // A second replace is refused whatever its tier, naming both.
    assert_eq!(
        register(&mod_points::replace!(
            mod_points::listeners::game_beacon_average,
            result,
            none,
            "rival",
            5,
            "other_average",
            other_average
        )),
        Err(RegisterError::Replaced {
            first: ("owner", "own_average"),
            second: ("rival", "other_average"),
        })
    );
    assert_eq!(
        register(&mod_points::replace!(
            internals raw::game_beacon_average,
            result,
            none,
            "rival",
            5,
            "other_raw_average",
            other_raw_average
        )),
        Err(RegisterError::Replaced {
            first: ("owner", "own_average"),
            second: ("rival", "other_raw_average"),
        })
    );
    // API first, then internals: refused the same way. Registration only: this place's function
    // isn't compiled into the test, so neither listener is ever called.
    fn never() {}
    // SAFETY: never fired (see above).
    register(
        &unsafe {
            SlotListener::api(
                "api_owner",
                PointId::GameBeaconButtons.place(),
                never as *const (),
            )
        }
        .ordered(0, "own_buttons")
        .replacing(),
    )
    .unwrap();
    // SAFETY: never fired (see above).
    let internals = unsafe {
        SlotListener::new(
            "raw_rival",
            PointId::GameBeaconButtons.place(),
            never as *const (),
        )
    };
    assert_eq!(
        register(&internals.ordered(0, "raw_buttons").replacing()),
        Err(RegisterError::Replaced {
            first: ("api_owner", "own_buttons"),
            second: ("raw_rival", "raw_buttons"),
        })
    );
    register(&mod_points::listeners::game_beacon_average(
        "raiser", 0, "raise", raise_b,
    ))
    .unwrap();
    register(&raw::game_beacon_average("lifter", raise_a).ordered(100, "lift")).unwrap();
    register(&raw::game_beacon_average("answerer", answer_average).ordered(-100, "answer"))
        .unwrap();
    mod_points::__private::seal();

    // a lifted to 103 and b raised to 8 before the replace: 103 * 10. The lower-priority internals
    // head that sets `result = 5` runs before the replace, so the replace's value wins, as with an
    // API replace; the body never runs.
    assert_eq!(average(3, 7), 1030);
    assert_eq!(
        AVERAGE_BODY.load(Ordering::Relaxed),
        0,
        "the replaced body ran"
    );

    let mut failure = None;
    remember_shutdown(&mut failure, "disk full".into());
    assert_eq!(REPLACED.load(Ordering::Relaxed), 1);
    assert_eq!(
        SHUTDOWN_BODY.load(Ordering::Relaxed),
        0,
        "the replaced body ran"
    );
    assert_eq!(failure, None);

    // A stopped owner hands the function back: it runs as written again.
    mod_points::stop("quitter");
    remember_shutdown(&mut failure, "disk full".into());
    assert_eq!(REPLACED.load(Ordering::Relaxed), 1);
    assert_eq!(SHUTDOWN_BODY.load(Ordering::Relaxed), 1);
    assert_eq!(failure.as_deref(), Some("disk full"));

    // With the value replace's owner stopped, the other internals head's answer stands again.
    mod_points::stop("owner");
    assert_eq!(average(3, 7), 5);
    assert_eq!(AVERAGE_BODY.load(Ordering::Relaxed), 0);
}
