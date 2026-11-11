use super::*;
use horizon_client::save::level_mods;

impl Game<'_> {
    /// Dispatch the saved list at the earliest Game Host boundary on either side.
    /// Each resolution keeps its first resolving listener across dispatch.
    pub(in crate::game) fn native_mod_mismatch(
        &mut self,
        mismatches: &[mod_api::ModMismatch],
        resolved: &mut Vec<mod_api::ModMismatchResolution>,
    ) {
        self.fire_mod_mismatch(mismatches, resolved);
    }

    /// A server opened a world: its saved mod list against the loaded mods.
    /// The mods hear the differences once, and each one nobody resolved is
    /// a warning.
    pub(in crate::game) fn check_mod_mismatch(&mut self) {
        let loaded = level_mods::loaded().unwrap_or_default();
        let mismatches = self.level.mod_mismatches(&loaded);
        if self.level.saved_mods.is_none() {
            return;
        }
        let mut resolved = Vec::new();
        self.native_mod_mismatch(&mismatches, &mut resolved);
        let shown = |version: &str| -> String {
            if version.is_empty() {
                "an unknown version".to_owned()
            } else {
                version.to_owned()
            }
        };
        let mut ordered: Vec<_> = mismatches.iter().collect();
        ordered.sort_by(|a, b| a.mod_id.cmp(&b.mod_id));
        for mismatch in ordered {
            let current = mismatch
                .current_version
                .as_deref()
                .map_or_else(|| "missing".to_owned(), shown);
            if let Some(resolution) = resolved.iter().find(|r| r.mod_id == mismatch.mod_id) {
                log::debug!(
                    "Horizon: mod {} was {}, now {}; resolved by {} ({})",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current,
                    resolution.resolver,
                    if resolution.resolver == mismatch.mod_id {
                        "self"
                    } else {
                        "other mod"
                    }
                );
            } else {
                log::warn!(
                    "Horizon: mod {} was {} when the world was saved, now {}",
                    mismatch.mod_id,
                    shown(&mismatch.saved_version),
                    current
                );
            }
        }
        if mismatches
            .iter()
            .any(|m| !resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::warn!("Horizon: changed mods may cause the world to work incorrectly");
        }
        if mismatches
            .iter()
            .any(|m| resolved.iter().any(|r| r.mod_id == m.mod_id))
        {
            log::debug!("Horizon: changed mods may cause the world to work incorrectly");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Hooks, HostSide, ModEntry, ModMismatchEvent};
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn existing_lists_fire_on_client_and_dedicated_even_without_differences() {
        struct Listener(Arc<Mutex<usize>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert!(event.mismatches.is_empty());
                *self.0.lock().unwrap() += 1;
            }
        }
        let heard = Arc::new(Mutex::new(0));
        mod_api::testing::with_mods(
            vec![ModEntry::new("listener", Box::new(Listener(heard.clone())))].into(),
            || {
                for role in [
                    execution_role::ExecutionRole::Client,
                    execution_role::ExecutionRole::DedicatedServer,
                ] {
                    super::super::super::native_entity_refs::tests::with_game(|game| {
                        game.options.execution_role = role;
                        game.level.saved_mods = None;
                        game.check_mod_mismatch();
                        game.level.saved_mods = Some(vec![]);
                        game.check_mod_mismatch();
                    });
                }
            },
        );
        assert_eq!(*heard.lock().unwrap(), 2);
    }
}
