//! The game's injection points (#589), for mods that listen at them. Every function of the game
//! module the prebuilt game's staging pass admits is a point, and so is each listed function of the
//! crates below it: a mod listens at its head (before the body runs) and at its return (after)
//! through manifest `[[inject]]` functions, with no game rebuild. Each listener gets the
//! point's record: the arguments the API carries, and `cancel` or `result` where the function's
//! return allows. Each declaration names the target, access and listener function.
//!
//! Depend on this crate only to listen at points: it is large (one record per point), and every
//! crate that depends on it compiles a few seconds slower. Hooks, handles and parts are in
//! `mod_api`.
#![deny(missing_docs)]
// Its tests are integration tests, so the unit-test build (`cargo test --lib`) is empty rather than
// a second compile of the generated points.
#![cfg(not(test))]

include!(concat!(env!("OUT_DIR"), "/points_generated.rs"));

mod registry;
#[cfg(target_arch = "wasm32")]
pub use guest::register;
#[cfg(not(target_arch = "wasm32"))]
pub use registry::register;
pub use registry::{RegisterError, stop};

/// A `replace` listener at a point's head: the function is the listener's, as with a mixin
/// overwrite, so the body never runs and the listener produces the outcome. It runs after the
/// place's other API head listeners whatever its priority (they may change its inputs, not its
/// outcome), and the point keeps its outcome through the internals listeners after it.
///
/// `replace!(constructor, returns, host, mod_id, priority, inject_id, listener)`: `constructor`
/// is the place's typed listener constructor in its table (`mod_points::listeners::tick`, or a mod
/// point's `<mod>::__horizon_points::listeners::…`). `returns` is the
/// point's `returns` in `points.json`: at a `cancel` point the listener returns `()` and the cancel
/// is forced; at a `result` point it returns the record's `result` type, which the function
/// returns. A point whose return has no API form (`none`) has no replace listener. `host` is `host`
/// when the point's API listeners take one, `none` otherwise. The slot is marked
/// [`__private::SlotListener::replacing`], so a second replace listener at the place is refused.
///
/// The launcher's generated `main` and `cargo horizon-mod build --wasm` both emit it, so native
/// and wasm mods share one contract.
///
/// An internals `replace` (native only) takes the same contract through the point's raw form:
/// `replace!(internals constructor, outcome, receiver, mod_id, priority, inject_id, listener)`,
/// where `constructor` is the mod facade's `raw::<point>` and `outcome` is the raw shape's
/// `cancel` or `result`. The listener gets the real receiver (`receiver`, or `none` for a free or
/// associated function) and the raw record, and returns the function's own return type. It runs
/// after every API and internals head listener at the place, and a second replace there, of
/// either tier, is refused.
#[macro_export]
macro_rules! replace {
    (internals $constructor:path, cancel, receiver, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor($id, |receiver, raw| {
            $listener(receiver, raw);
            *raw.cancel = true;
        })
        .ordered($priority, $inject)
        .replacing()
    };
    (internals $constructor:path, cancel, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor($id, |raw| {
            $listener(raw);
            *raw.cancel = true;
        })
        .ordered($priority, $inject)
        .replacing()
    };
    (internals $constructor:path, result, receiver, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor($id, |receiver, raw| {
            let value = $listener(receiver, raw);
            *raw.result = ::core::option::Option::Some(value);
        })
        .ordered($priority, $inject)
        .replacing()
    };
    (internals $constructor:path, result, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor($id, |raw| {
            let value = $listener(raw);
            *raw.result = ::core::option::Option::Some(value);
        })
        .ordered($priority, $inject)
        .replacing()
    };
    ($constructor:path, cancel, host, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(cancel, host, $listener),
        )
        .replacing()
    };
    ($constructor:path, cancel, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(cancel, none, $listener),
        )
        .replacing()
    };
    ($constructor:path, result, host, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(result, host, $listener),
        )
        .replacing()
    };
    ($constructor:path, result, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(result, none, $listener),
        )
        .replacing()
    };
}

#[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
pub mod guest;

// hidden: the game's side of the points, not part of the API a mod is written against
#[doc(hidden)]
pub mod __private {
    pub use crate::registry::{TABLE, seal};
    use crate::{PointId, SiteId};
    pub use mod_api::__private::points::{
        ApiKind, Forward, PointHost, RawKind, SlotListener, cold,
    };

    /// Whether an active `replace` listener owns `place`'s outcome. A point's head checks it
    /// around its internals listeners, which run after the API ones and may not undo it.
    #[inline]
    pub fn replaced(place: PointId) -> bool {
        TABLE.replaced(place as usize)
    }

    /// Runs the checked API functions and forwarding endpoints under their owner guards.
    #[inline]
    pub fn fire_api(place: PointId, call: &mut dyn FnMut(ApiKind)) {
        TABLE.fire_api(place as usize, call);
    }

    /// Runs the raw listeners at `place`, each through `call` with its kind and function. The
    /// slot casts a typed function back to the place's listener type, or calls an erased one
    /// with the place index and its receiver and raw record or return value pointers.
    ///
    /// Extracting an erased callback does not make it safe to call with arbitrary pointers:
    ///
    /// ```compile_fail,E0133
    /// use mod_points::{PointId, __private::{fire_raw, RawKind}};
    /// let place = PointId::Tick;
    /// fire_raw(place, &mut |kind| {
    ///     if let RawKind::Erased(function) = kind {
    ///         function(place as usize, std::ptr::null_mut(), std::ptr::null_mut());
    ///     }
    /// });
    /// ```
    ///
    /// A caller that provides the slot's pointer guarantees can invoke it explicitly:
    ///
    /// ```
    /// use mod_points::{PointId, __private::{fire_raw, RawKind}};
    /// // SAFETY: the caller must supply this place's exact receiver and record pointers,
    /// // valid for the callback's duration, as required by RawKind::Erased.
    /// unsafe fn forward(place: PointId, receiver: *mut (), raw: *mut ()) {
    ///     fire_raw(place, &mut |kind| {
    ///         if let RawKind::Erased(function) = kind {
    ///             // SAFETY: the caller supplies the slot's pointer guarantees.
    ///             unsafe { function(place as usize, receiver, raw) };
    ///         }
    ///     });
    /// }
    /// ```
    #[inline]
    pub fn fire_raw(place: PointId, call: &mut dyn FnMut(RawKind)) {
        TABLE.fire_raw(place as usize, call);
    }

    /// Invokes the next enabled listener of call site `place` (see [`mod_api::__private::points::Table::wrap`]).
    #[inline]
    pub fn wrap<O>(
        place: SiteId,
        from: usize,
        call: impl FnOnce(Option<(*const (), usize)>) -> O,
    ) -> O {
        TABLE.wrap(place as usize, from, call)
    }
}
