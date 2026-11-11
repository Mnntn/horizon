//! Whether automatic updates may change the explicit base selection.
use super::*;

/// Whether a version installed as an update becomes the selected one: when the player had none,
/// was on a Nightly, or on the newest installed release (so a player of the release channel moves
/// to the Nightly at the first check, docs/release-flow.md). An older release picked by hand
/// stays, and so does an imported build (`build_import`); an instance pinned to a version keeps it
/// either way.
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) if crate::nightly::date_of(s).is_some() => true,
        Some(s)
            if crate::build_import::is_imported(s)
                || crate::developer_install::uses_developer(s) =>
        {
            false
        }
        Some(s) => installed_before
            .iter()
            .find(|t| is_game_tag(t))
            .is_none_or(|newest| newest == s),
    }
}
