//! Serialized values shared by mod-point generation and package validation.
use serde::{Deserialize, Serialize};

/// A mod crate's points, as its build lists them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// The crate's library name: an injection's `at` starts with it.
    #[serde(rename = "crate")]
    pub krate: String,
    /// The slot table module at the crate's root.
    pub table: String,
    /// The functions `#[mod_api::no_point]` opted out, by path.
    #[serde(default)]
    pub opted_out: Vec<String>,
    pub points: Vec<Point>,
}

/// One mod point.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Point {
    /// Its place's name in the table: the head; the return adds `_return`.
    pub name: String,
    /// The function's path, as an injection's `at` names it.
    pub target: String,
    /// What an addon's declaration pins.
    pub hash: String,
    /// The signature the hash covers.
    pub signature: String,
    /// Whether it has an internals form.
    pub raw: bool,
    /// Whether its API listeners have a return place.
    pub at_return: bool,
    /// Why its API listeners are refused, naming the parameter and type.
    #[serde(default)]
    pub api_refusal: Option<String>,
    /// What its listeners may do with the return: `cancel`, `result` or `none`.
    pub returns: String,
    /// Its API listeners' host: `none` for every mod point.
    pub host: String,
    /// The function's return type as written.
    pub return_ty: String,
    /// Its internals head and return listener types, by path from the crate root.
    #[serde(default)]
    pub listener: Option<String>,
    #[serde(default)]
    pub return_listener: Option<String>,
}

impl Catalog {
    /// Parses a package's `mod-points.json`, strictly.
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| format!("mod-points.json: {e}"))
    }

    /// Whether `at` names a function of this crate.
    pub fn owns(&self, at: &str) -> bool {
        at.split("::").next() == Some(self.krate.as_str())
    }
}
