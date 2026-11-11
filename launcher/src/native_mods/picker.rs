//! Prebuilt build eligibility, independent of UI preferences.
use super::Refusal;

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Prebuilt,
    Invalid { reason: String },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Routes {
    pub mods: Vec<(String, Route)>,
    pub unavailable: Option<Refusal>,
}
