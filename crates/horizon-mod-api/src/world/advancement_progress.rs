//! Owned advancement progress captured at a criterion's grant or revoke.

/// One criterion in an advancement, including criteria not yet obtained.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct CriterionProgress {
    /// The criterion's name within the advancement.
    pub name: String,
    /// The stored grant date, or `None` when this criterion is not obtained.
    /// Dates use the advancement save format (`yyyy-MM-dd HH:mm:ss Z`);
    /// capturing progress preserves the original date rather than reading a clock.
    pub obtained_at: Option<String>,
}

impl CriterionProgress {
    /// Build a criterion snapshot from its name and existing grant date.
    pub fn new(name: String, obtained_at: Option<String>) -> Self {
        Self { name, obtained_at }
    }
}

/// An owned, read-only snapshot of one advancement's complete progress.
///
/// [`crate::PlayerAdvancementProgressEvent::progress`] captures this after the
/// criterion changes and before completion rewards. Retaining or changing a
/// clone does not change the player's progress. The same representation can
/// describe progress outside an event without borrowing the game or a player.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct AdvancementProgress {
    /// Every criterion in definition order, including unobtained criteria.
    pub criteria: Vec<CriterionProgress>,
    /// Requirement groups: at least one criterion in each group must be obtained.
    /// All groups must be satisfied; an empty list is not complete.
    pub requirements: Vec<Vec<String>>,
    /// Number of requirement groups satisfied, not the number of obtained criteria.
    pub completed_requirements: u32,
    /// Whether the owning progress considers this advancement complete.
    pub done: bool,
}

impl AdvancementProgress {
    /// Build a snapshot from the owning progress's criteria and requirement results.
    pub fn new(
        criteria: Vec<CriterionProgress>,
        requirements: Vec<Vec<String>>,
        completed_requirements: u32,
        done: bool,
    ) -> Self {
        Self {
            criteria,
            requirements,
            completed_requirements,
            done,
        }
    }
}
