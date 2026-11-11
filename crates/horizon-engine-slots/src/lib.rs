//! Engine-owned injection activity and the load-time seal entry point.
//! Listener types, ordering and dispatch belong to the provider, not this crate.
#![forbid(unsafe_code)]

use std::sync::{Mutex, OnceLock};

/// Point activity, populated by the listener provider when registration seals.
pub static POINTS: Slots = Slots::new();
/// Call-site activity, populated by the listener provider when registration seals.
pub static SITES: Slots = Slots::new();
// Process state: the provider and final registration boundary belong to this game process.
static LIFECYCLE: Lifecycle = Lifecycle::new();

/// A refused load-time mutation of the engine's sealed registration state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Sealed,
    AlreadyInstalled,
    AlreadyPublished,
}
impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Sealed => "engine registration is sealed",
            Self::AlreadyInstalled => "an engine seal provider is already installed",
            Self::AlreadyPublished => "engine activity is already published",
        })
    }
}
impl std::error::Error for Refusal {}

enum State {
    Open(Option<fn()>),
    Sealing,
    Sealed,
}
struct Lifecycle(Mutex<State>);
impl Lifecycle {
    const fn new() -> Self {
        Self(Mutex::new(State::Open(None)))
    }
    fn install(&self, callback: fn()) -> Result<(), Refusal> {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match &mut *state {
            State::Open(provider @ None) => {
                *provider = Some(callback);
                Ok(())
            }
            State::Open(Some(_)) => Err(Refusal::AlreadyInstalled),
            _ => Err(Refusal::Sealed),
        }
    }
    fn seal(&self) {
        let callback = {
            let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if !matches!(*state, State::Open(_)) {
                return;
            }
            let State::Open(callback) = std::mem::replace(&mut *state, State::Sealing) else {
                unreachable!()
            };
            callback
        };
        // The provider publishes activity here, before the final boundary closes.
        if let Some(callback) = callback {
            callback();
        }
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = State::Sealed;
    }
}

/// An index-addressed activity table. An unpopulated or unknown slot is inactive.
/// Once published, checking a slot allocates nothing and never scans listeners.
pub struct Slots {
    flags: OnceLock<Box<[bool]>>,
}
impl Slots {
    const fn new() -> Self {
        Self {
            flags: OnceLock::new(),
        }
    }
    /// Whether the provider published an active listener at this index.
    #[inline]
    pub fn active(&self, index: usize) -> bool {
        self.flags
            .get()
            .and_then(|flags| flags.get(index))
            .copied()
            .unwrap_or(false)
    }
    /// Publish activity at seal time. Late or duplicate publication is refused.
    pub fn publish(&self, activity: impl IntoIterator<Item = bool>) -> Result<(), Refusal> {
        self.publish_in(&LIFECYCLE, activity)
    }
    fn publish_in(
        &self,
        lifecycle: &Lifecycle,
        activity: impl IntoIterator<Item = bool>,
    ) -> Result<(), Refusal> {
        let state = lifecycle.0.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, State::Sealed) {
            return Err(Refusal::Sealed);
        }
        self.flags
            .set(activity.into_iter().collect())
            .map_err(|_| Refusal::AlreadyPublished)
    }
}
/// Install the provider before engine startup; late or duplicate installation is refused.
pub fn install_seal(callback: fn()) -> Result<(), Refusal> {
    LIFECYCLE.install(callback)
}
/// Finish listener registration once, including when no provider was loaded.
pub fn seal() {
    LIFECYCLE.seal();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activity_is_inert_until_published_and_publication_is_permanent() {
        let lifecycle = Lifecycle::new();
        let slots = Slots::new();
        assert!(!slots.active(0));
        assert_eq!(slots.publish_in(&lifecycle, [true, false, true]), Ok(()));
        assert!(slots.active(0));
        assert!(!slots.active(1));
        assert!(slots.active(2));
        assert!(!slots.active(usize::MAX));
        assert_eq!(
            slots.publish_in(&lifecycle, [false]),
            Err(Refusal::AlreadyPublished)
        );
        assert!(slots.active(2));
    }
    #[test]
    fn seal_without_provider_refuses_late_publication() {
        let lifecycle = Lifecycle::new();
        let slots = Slots::new();
        lifecycle.seal();
        assert_eq!(slots.publish_in(&lifecycle, [true]), Err(Refusal::Sealed));
        assert!(!slots.active(0));
    }
    #[test]
    fn seal_without_provider_refuses_late_installation() {
        let lifecycle = Lifecycle::new();
        lifecycle.seal();
        assert_eq!(
            lifecycle.install(|| panic!("late provider")),
            Err(Refusal::Sealed)
        );
        lifecycle.seal();
    }
    #[test]
    fn engine_seal_invokes_only_the_installed_provider() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let lifecycle = Lifecycle::new();
        assert_eq!(
            lifecycle.install(|| {
                CALLS.fetch_add(1, Ordering::Relaxed);
            }),
            Ok(())
        );
        assert_eq!(
            lifecycle.install(|| panic!("second provider")),
            Err(Refusal::AlreadyInstalled)
        );
        lifecycle.seal();
        lifecycle.seal();
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    }
}
