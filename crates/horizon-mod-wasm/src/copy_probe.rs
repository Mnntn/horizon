//! Opt-in test accounting for successful host/guest shared-memory copies.
use std::cell::Cell;
#[derive(Clone, Copy, Debug, Default)]
pub struct Counts {
    pub to_guest_bytes: usize,
    pub from_guest_bytes: usize,
}
thread_local! { static COUNTS: Cell<Option<Counts>> = const { Cell::new(None) }; }
pub(crate) fn record(to_guest: bool, bytes: usize) {
    COUNTS.with(|cell| {
        if let Some(mut c) = cell.get() {
            if to_guest {
                c.to_guest_bytes += bytes;
            } else {
                c.from_guest_bytes += bytes;
            }
            cell.set(Some(c));
        }
    });
}
/// Measures actual runtime copies, not guest-internal serialization or allocator moves.
pub fn measure<R>(run: impl FnOnce() -> R) -> (R, Counts) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            COUNTS.with(|c| c.set(None));
        }
    }
    COUNTS.with(|c| {
        assert!(c.get().is_none());
        c.set(Some(Counts::default()));
    });
    let _guard = Guard;
    let result = run();
    (result, COUNTS.with(|c| c.get().unwrap()))
}
