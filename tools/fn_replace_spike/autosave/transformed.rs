use horizon_client::game::Game;
use horizon_client::game::autosave_interval;
pub fn next_autosave_interval(this: &Game<'_>) -> i64 {
    let rate = if this.clock.sprinting() {
        1.0e9 / (this.clock.average_nanos() + 1) as f32
    } else {
        this.clock.rate()
    };
    ({
        fn autosave(value: i64) -> i64 {
            fnr_autosave::autosave(value)
        }
        autosave
    })(autosave_interval(rate))
}

/// The point's internals replace listener: the transformed copy owns the function's outcome.
pub fn replace(game: &Game<'_>, _raw: &mut horizon_client::game::autosave::__NextAutosaveIntervalRaw<'_, i64>) -> i64 {
    next_autosave_interval(game)
}

/// Registers a function of this place's exact listener type (as the facade's `raw::` does).
pub const fn listener(id: &'static str, f: horizon_client::game::autosave::__NextAutosaveIntervalListener) -> mod_points::__private::SlotListener {
    // SAFETY: `f` has this place's listener type, checked by the signature.
    unsafe { mod_points::__private::SlotListener::new(id, mod_points::PointId::NextAutosaveInterval.place(), f as *const ()) }
}
