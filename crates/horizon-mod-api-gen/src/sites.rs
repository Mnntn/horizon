//! Unified slot lists and call-expression staging, activated after widening.
use crate::points::plain_path;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use syn::Expr;

/// A site refusal, kept typed until the staging boundary renders it.
#[derive(Debug)]
pub(crate) enum Cause {
    Io(std::io::Error),
    Syntax(syn::Error),
    Format(&'static str),
    Missing(&'static str),
    Ambiguous(&'static str),
    Signature(&'static str),
    Record(&'static str),
    Conflict(String),
    Source(String),
    Identity(identity::Refusal),
}

/// The declaration that failed and its machine-distinguishable cause.
#[derive(Debug)]
pub struct Error {
    pub origin: Option<String>,
    pub(crate) cause: Cause,
}
impl Error {
    pub fn at(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}
impl From<Cause> for Error {
    fn from(cause: Cause) -> Self {
        Self {
            origin: None,
            cause,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Cause::Io(error).into()
    }
}
impl From<syn::Error> for Error {
    fn from(error: syn::Error) -> Self {
        Cause::Syntax(error).into()
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(origin) = &self.origin {
            write!(f, "{origin}: ")?;
        }
        match &self.cause {
            Cause::Io(e) => write!(f, "{e}"),
            Cause::Syntax(e) => write!(f, "invalid syntax: {e}"),
            Cause::Format(s)
            | Cause::Missing(s)
            | Cause::Ambiguous(s)
            | Cause::Signature(s)
            | Cause::Record(s) => f.write_str(s),
            Cause::Source(message) => f.write_str(message),
            Cause::Identity(refusal) => write!(f, "{refusal}"),
            Cause::Conflict(other) => {
                write!(f, "conflicts with {other}: both select the same call")
            }
        }
    }
}
impl std::error::Error for Error {}

/// One requested call expression, with its list-file diagnostic location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub target: String,
    pub selector: String,
    pub ordinal: Option<usize>,
    pub ctx: String,
    pub origin: String,
}

pub(crate) const DIRECTORY: &str = "crates/horizon-mod-api-gen/slots";

/// The union of per-mod slot lists, with declaration locations for diagnostics.
#[derive(Default, Debug)]
pub struct Slots {
    pub functions: BTreeMap<String, String>,
    pub calls: Vec<Declaration>,
}

/// Read function and call slots with one parser. An absent directory has no listed slots.
pub fn slots(directory: &Path) -> Result<Slots, Error> {
    let contents = match fs::read_dir(directory) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Slots::default()),
        Err(e) => return Err(Error::from(e).at(directory.display().to_string())),
    };
    let mut files = contents
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    files.sort();
    let mut slots = Slots::default();
    for file in files
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
    {
        let text =
            fs::read_to_string(&file).map_err(|e| Error::from(e).at(file.display().to_string()))?;
        slots.extend(&file, &text)?;
    }
    Ok(slots)
}

impl Slots {
    /// Parse another list with the same syntax and conflict checks as on-disk slot lists.
    /// `file` is a diagnostic location, not read or written. Discard this value on error.
    pub fn extend(&mut self, file: &Path, text: &str) -> Result<(), Error> {
        let mut union: BTreeMap<_, Declaration> = self
            .calls
            .iter()
            .cloned()
            .map(|call| {
                (
                    (call.target.clone(), call.selector.clone(), call.ordinal),
                    call,
                )
            })
            .collect();
        for (i, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let origin = format!("{}:{}", file.display(), i + 1);
            let mut parse = || -> Result<_, Error> {
                let parts: Vec<_> = line.split('|').map(str::trim).collect();
                let target = parts[0].trim();
                syn::parse_str::<syn::Path>(target)?;
                let target = plain_path(target);
                if parts.len() == 1 {
                    self.functions.entry(target).or_insert(origin.clone());
                    return Ok(None);
                }
                let [_, selector, ordinal, ctx] = parts.as_slice() else {
                    return Err(Cause::Format("expected at or at | call | nth | ctx").into());
                };
                let call = if selector.starts_with('.') {
                    format!("__receiver{selector}()")
                } else {
                    format!("{selector}()")
                };
                if !matches!(
                    syn::parse_str::<Expr>(&call),
                    Ok(Expr::Call(_) | Expr::MethodCall(_))
                ) {
                    return Err(Cause::Format("selector must name a method or path call").into());
                }
                let ordinal = if ordinal.is_empty() {
                    None
                } else {
                    Some(
                        ordinal
                            .parse()
                            .map_err(|_| Cause::Format("ordinal must be a nonnegative integer"))?,
                    )
                };
                Ok(Some(Declaration {
                    target,
                    selector: selector.replace(' ', ""),
                    ordinal,
                    ctx: (*ctx).to_owned(),
                    origin: origin.clone(),
                }))
            };
            if let Some(declaration) = parse().map_err(|e| e.at(origin.clone()))? {
                let key = (
                    declaration.target.clone(),
                    declaration.selector.clone(),
                    declaration.ordinal,
                );
                if let Some(other) = union.get(&key) {
                    if other.ctx != declaration.ctx {
                        return Err(Error::from(Cause::Conflict(other.origin.clone())).at(origin));
                    }
                } else {
                    union.insert(key, declaration);
                }
            }
        }
        self.calls = union.into_values().collect();
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn slot_extension_matches_disk_parser() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/slot-extension-{}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let a = directory.join("a.txt");
    let b = directory.join("b.txt");
    let first = "# comment\nlower::run\ngame::tick | lower::call | 2 | context\n";
    let second = "lower::run\ngame::tick | lower::call | 2 | context\nother::run\n";
    fs::write(&a, first).unwrap();
    fs::write(&b, second).unwrap();
    let from_disk = slots(&directory).unwrap();
    let mut extended = Slots::default();
    extended.extend(&a, first).unwrap();
    extended.extend(&b, second).unwrap();
    assert_eq!(extended.functions, from_disk.functions);
    assert_eq!(extended.calls, from_disk.calls);
    assert_eq!(
        extended.functions.keys().collect::<Vec<_>>(),
        ["lower::run", "other::run"]
    );
    assert_eq!(extended.calls[0].ordinal, Some(2));
    assert_eq!(extended.calls[0].ctx, "context");
    let conflict = "game::tick | lower::call | 2 | changed\n";
    fs::write(&b, conflict).unwrap();
    let disk_error = slots(&directory).unwrap_err().to_string();
    let memory_error = extended.extend(&b, conflict).unwrap_err().to_string();
    assert_eq!(disk_error, memory_error);
    assert!(memory_error.contains("a.txt:3") && memory_error.contains("b.txt:1"));
    fs::remove_dir_all(directory).unwrap();
}

/// A compiled call site's raw form, written to `points.json` beside the head/return points.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Site {
    pub name: String,
    pub target: String,
    pub selector: String,
    pub ordinal: usize,
    pub hash: String,
    /// Canonical internals catalog path of the callee, independent of source aliases.
    pub call: String,
    /// Statement and nesting-path fingerprint, independent of ordinal drift.
    pub ctx: String,
    pub listener: String,
    /// Move record and consuming forwarding capability for native calls.
    pub once: bool,
    pub record: String,
    pub receiver: String,
    pub args: Vec<(String, String)>,
    pub output: String,
}

pub(crate) mod analysis;

pub mod identity;

pub(crate) mod resolve;
pub(crate) mod rewrite;
/// The sole facade name for this canonical call place, independent of written aliases.
pub fn facade_name(target: &str, callee: &str, ordinal: usize) -> String {
    rewrite::name(
        &Declaration {
            target: crate::points::plain_path(target),
            selector: crate::points::plain_path(callee),
            ordinal: Some(ordinal),
            ctx: String::new(),
            origin: String::new(),
        },
        ordinal,
    )
}

mod stage;
pub(crate) use stage::{ids, stage, tracked};

#[cfg(test)]
mod production_tests;
#[cfg(test)]
mod tests;

#[test]
fn facade_name_is_stable() {
    assert_eq!(
        facade_name("game::tick", "game::step", 2),
        "S7de17070633c86a1"
    );
}
