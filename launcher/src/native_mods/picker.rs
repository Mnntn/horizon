//! Structural rebuild requirements, independent of a player's saved build preference.
use super::{Profile, SourceReason};
use crate::i18n::{tr, trf};

/// How a native mod can reach the game.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    /// Compiles against the prebuilt game without rebuilding it.
    Prebuilt,
    /// Requires a full game build for this structural reason.
    Game { reason: SourceReason },
    /// Invalid project input; rebuilding the game cannot repair it.
    Invalid { reason: String },
}

/// Owned planning results: the UI need not know how the worker obtains the game catalog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Routes {
    /// An ordered result for each native mod ID.
    pub mods: Vec<(String, Route)>,
    /// A separately probed release/computer limitation, or retryable download failure.
    pub unavailable: Option<SourceReason>,
}

impl Routes {
    /// Only the mods causing a rebuild, for the count and each row's warning tooltip.
    pub fn rebuild_mods(&self) -> impl Iterator<Item = (&str, &SourceReason)> {
        self.mods.iter().filter_map(|(id, route)| match route {
            Route::Game { reason } if reason.forces_source() => Some((id.as_str(), reason)),
            _ => None,
        })
    }

    /// Rebuild requirements, excluding malformed projects and retryable downloads.
    pub fn rebuild_reasons(&self) -> impl Iterator<Item = &SourceReason> {
        self.unavailable
            .iter()
            .filter(|reason| !self.mods.is_empty() && reason.forces_source())
            .chain(self.rebuild_mods().map(|(_, reason)| reason))
    }

    /// The count line; global limitations belong to the game, not any individual mod.
    pub fn rebuild_heading(&self) -> Option<String> {
        if self
            .rebuild_reasons()
            .any(|reason| matches!(reason, SourceReason::Host | SourceReason::NoRelease { .. }))
        {
            return Some(tr("No prebuilt game for this version").to_string());
        }
        match self.rebuild_mods().count() {
            0 => None,
            1 => Some(tr("1 mod requires a full game rebuild").to_string()),
            count => Some(trf(
                "{count} mods require a full game rebuild",
                &[("count", &count.to_string())],
            )),
        }
    }
}

impl SourceReason {
    /// Short per-mod warning text. Global limitations and retryable failures get no row warning.
    pub fn mod_warning(&self) -> Option<String> {
        Some(match self {
            Self::Mixins { .. } => tr("Mixins compile game code").to_string(),
            Self::BuildScript { .. } => tr("Has a build script").to_string(),
            Self::Features { .. } => tr("Uses Cargo features").to_string(),
            Self::Host
            | Self::NoRelease { .. }
            | Self::Download { .. }
            | Self::IncompatibleSdk { .. } => return None,
        })
    }
}

/// Show build modes only for structural rebuilds, folder-loaded mods or Developer mode.
pub fn needs_picker(routes: &Routes, folders: bool, developer_mode: bool) -> bool {
    folders || developer_mode || routes.rebuild_reasons().next().is_some()
}

impl Profile {
    /// A saved choice applies only while its picker is visible; otherwise use the prebuilt route.
    pub fn for_picker(self, routes: &Routes, folders: bool, developer_mode: bool) -> Self {
        if needs_picker(routes, folders, developer_mode) {
            self.picker_choice(developer_mode)
        } else {
            Self::Optimized
        }
    }

    /// Developer is offered only while its switch is on; keep the saved preference intact.
    pub fn picker_choice(self, developer_mode: bool) -> Self {
        match (self, developer_mode) {
            (Self::Developer, false) => Self::Balanced,
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_prebuilt_ignores_saved_modes_but_folder_and_developer_choices_apply() {
        for mods in [Vec::new(), vec![("api_mod".into(), Route::Prebuilt)]] {
            let routes = Routes {
                mods,
                unavailable: None,
            };
            for (saved, ordinary) in [
                (Profile::Optimized, Profile::Optimized),
                (Profile::Balanced, Profile::Balanced),
                (Profile::Developer, Profile::Balanced),
            ] {
                assert!(!needs_picker(&routes, false, false));
                assert_eq!(saved.for_picker(&routes, false, false), Profile::Optimized);
                for (folders, developer) in [(true, false), (false, true), (true, true)] {
                    assert!(needs_picker(&routes, folders, developer));
                    assert_eq!(
                        saved.for_picker(&routes, folders, developer),
                        if developer { saved } else { ordinary }
                    );
                }
            }
        }
        let empty = Routes {
            mods: Vec::new(),
            unavailable: Some(SourceReason::Host),
        };
        assert!(!needs_picker(&empty, false, false));
        assert!(empty.rebuild_heading().is_none());
    }

    #[test]
    fn every_structural_mod_reason_exposes_the_picker_and_a_row_warning() {
        for reason in [
            SourceReason::Mixins { mod_id: "a".into() },
            SourceReason::BuildScript { mod_id: "a".into() },
            SourceReason::Features { mod_id: "a".into() },
        ] {
            assert!(reason.mod_warning().is_some());
            let routes = Routes {
                mods: vec![("a".into(), Route::Game { reason })],
                unavailable: None,
            };
            assert!(needs_picker(&routes, false, false));
            assert_eq!(
                routes.rebuild_mods().map(|(id, _)| id).collect::<Vec<_>>(),
                ["a"]
            );
            assert!(routes.rebuild_heading().is_some());
            assert_eq!(
                Profile::Balanced.for_picker(&routes, false, false),
                Profile::Balanced
            );
        }
    }

    #[test]
    fn global_reasons_have_no_mod_warning_and_download_or_invalid_inputs_do_not_force_source() {
        for reason in [
            SourceReason::Host,
            SourceReason::NoRelease {
                revision: "revision".into(),
            },
        ] {
            assert!(reason.mod_warning().is_none());
            let routes = Routes {
                mods: vec![("api_mod".into(), Route::Prebuilt)],
                unavailable: Some(reason),
            };
            assert!(needs_picker(&routes, false, false));
            assert_eq!(routes.rebuild_mods().count(), 0);
            assert!(routes.rebuild_heading().is_some());
        }
        let offline = SourceReason::Download {
            why: "offline".into(),
        };
        assert!(offline.mod_warning().is_none());
        let mut routes = Routes {
            mods: vec![(
                "broken".into(),
                Route::Invalid {
                    reason: "invalid Cargo.toml".into(),
                },
            )],
            unavailable: Some(offline),
        };
        assert!(!needs_picker(&routes, false, false));
        assert!(routes.rebuild_heading().is_none());
        assert_eq!(
            Profile::Developer.for_picker(&routes, false, false),
            Profile::Optimized
        );
        for id in ["one", "two"] {
            routes.mods.push((
                id.into(),
                Route::Game {
                    reason: SourceReason::BuildScript { mod_id: id.into() },
                },
            ));
        }
        assert_eq!(
            routes.rebuild_mods().map(|(id, _)| id).collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(routes.rebuild_heading().unwrap().contains('2'));
    }
}
