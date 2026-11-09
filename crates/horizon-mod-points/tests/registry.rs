//! The registry's lifecycle, registering, sealing and stopping, on the real points: an integration
//! test, so the generated points compile once for the tests.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_points::__private::{POINTS, RawKind, SlotListener, seal};
use mod_points::{PointId, RegisterError, register, stop};
use std::sync::atomic::Ordering;

fn tick(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    assert_eq!(
        mod_api::content::crash::current_native_mod(),
        Some("tick_mod")
    );
    args.cancel = true;
}

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Quiet {}

fn raw_listener() {}

unsafe fn erased_listener(place: usize, receiver: *mut (), raw: *mut ()) {
    assert_eq!(place, PointId::Tick as usize);
    assert!(receiver.is_null() && raw.is_null());
}

/// `raw_listener` at `place`: the test's slots never call it.
fn raw_at(id: &'static str, place: PointId) -> SlotListener {
    // SAFETY: nothing fires a raw place here with a call into the function.
    unsafe { SlotListener::new(id, place, raw_listener as *const ()) }
}

/// Stopping from the mod's subscription callback still applies to slots registered later.
struct SelfStop;
impl mod_api::Hooks for SelfStop {
    fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        stop("self_stopping");
        &[]
    }
}

#[test]
fn one_list_registered_sealed_and_stopped() {
    mod_api::__private::install(
        ["tick_mod", "raw_mod", "stopped_early", "self_stopping"]
            .into_iter()
            .map(|id| {
                mod_api::ModEntry::new(
                    id,
                    if id == "self_stopping" {
                        Box::new(SelfStop) as Box<dyn mod_api::Hooks>
                    } else {
                        Box::new(Quiet)
                    },
                )
            })
            .collect(),
    );
    assert!(mod_api::__private::set_mod_enabled("tick_mod", false));
    assert!(mod_api::__private::set_mod_enabled("raw_mod", false));
    assert_eq!(
        register(&mod_points::listeners::tick("tick_mod", 0, "head", tick)),
        Ok(())
    );
    assert_eq!(
        register(&mod_points::listeners::tick_return(
            "tick_mod", 0, "tail", tick
        )),
        Ok(())
    );
    assert_eq!(
        register(&mod_points::listeners::tick(
            "stopped_early",
            0,
            "head",
            tick
        )),
        Ok(())
    );
    stop("stopped_early");
    assert_eq!(
        register(&mod_points::listeners::tick(
            "self_stopping",
            0,
            "head",
            tick
        )),
        Ok(())
    );
    let raw = PointId::Tick as usize + 2;
    let place = PointId::Tick;
    assert_eq!(register(&raw_at("raw_mod", place)), Ok(()));
    // SAFETY: the callback checks its place and never dereferences the pointers.
    let erased = unsafe { SlotListener::erased("raw_mod", place, erased_listener) };
    assert_eq!(register(&erased), Ok(()));
    assert_eq!(register(&raw_at("raw_mod", place)), Ok(()));
    assert!(
        !POINTS[PointId::Tick as usize].load(Ordering::Acquire),
        "nothing listens before the seal"
    );
    seal();
    assert_eq!(register(&raw_at("late", place)), Err(RegisterError::Sealed));
    assert_eq!(
        register(&mod_points::listeners::tick("late", 0, "head", tick)),
        Err(RegisterError::Sealed)
    );
    assert!(POINTS[PointId::Tick as usize].load(Ordering::Acquire));
    assert!(POINTS[PointId::TickReturn as usize].load(Ordering::Acquire));
    assert!(!POINTS[raw].load(Ordering::Acquire));
    let (mut heard, mut raw_heard) = (0, 0);
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |_| raw_heard += 1);
    assert_eq!((heard, raw_heard), (0, 0), "disabled before sealing");
    assert!(mod_api::__private::set_mod_enabled("tick_mod", true));
    assert!(mod_api::__private::set_mod_enabled("raw_mod", true));
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    let mut order = Vec::new();
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |kind| {
        match kind {
            RawKind::Typed(function) => {
                assert_eq!(function, raw_listener as *const ());
                order.push("typed");
            }
            RawKind::Erased(function) => {
                assert_eq!(function as usize, erased_listener as *const () as usize);
                order.push("erased");
            }
        }
        raw_heard += 1;
    });
    assert_eq!(order, ["typed", "erased", "typed"]);
    assert_eq!((heard, raw_heard), (1, 3));
    let mut args = mod_points::TickArgs::__new();
    mod_points::dispatch::tick(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(
        args.cancel,
        "the checked function runs under its owner's guard and writes back"
    );
    assert_eq!(mod_api::content::crash::current_native_mod(), None);
    assert!(mod_api::__private::set_mod_enabled("tick_mod", false));
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |_| raw_heard += 1);
    assert_eq!(
        (heard, raw_heard),
        (1, 6),
        "only the API listener is disabled"
    );
    assert!(mod_api::__private::set_mod_enabled("raw_mod", false));
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |_| raw_heard += 1);
    assert_eq!(
        (heard, raw_heard),
        (1, 6),
        "all raw listener kinds are disabled"
    );
    assert!(mod_api::__private::set_mod_enabled("tick_mod", true));
    assert!(mod_api::__private::set_mod_enabled("raw_mod", true));
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |_| raw_heard += 1);
    assert_eq!(
        (heard, raw_heard),
        (2, 9),
        "listeners resume without resealing"
    );
    stop("tick_mod");
    stop("raw_mod");
    for id in ["tick_mod", "raw_mod", "stopped_early", "self_stopping"] {
        assert!(mod_api::__private::set_mod_enabled(id, false));
        assert!(mod_api::__private::set_mod_enabled(id, true));
    }
    mod_points::__private::fire_api(PointId::Tick, &mut |_| heard += 1);
    mod_points::__private::fire_raw(PointId::Tick, "tick", &mut |_| raw_heard += 1);
    assert_eq!(
        (heard, raw_heard),
        (2, 9),
        "an enable switch cannot undo a stop"
    );
}
