//! A mod's own injection points (#2339), as the `mod-points.json` its build writes lists them, and the
//! routing of an addon's `[[inject]]` to them. Mod points mirror the game's: the same declarations,
//! `api` and `internals` levels and refusals, plus a signature hash an addon pins, checked when it
//! loads: a library rebuilt with a changed function refuses the addon instead of running it
//! against another signature. The native launcher and the wasm loader route through this one
//! function, and an edge across targets is refused by [`cross_target`] where it names a reason more
//! precise than the mixed-target refusal.
use super::{Entry, Inject, InjectError, NoSlotReason, Place, Route, SlotKind, Target};
use point_shape::catalog::{Catalog, Point};
use std::{fmt, io::Read};

/// The file a mod's build writes its points to, at the package root.
pub const FILE: &str = "mod-points.json";
const MAX_BYTES: u64 = 8 << 20;

/// An installed library, built for the addon's target, whose points the addon may target.
pub struct Library<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub catalog: &'a Catalog,
}

/// Reads a package's `mod-points.json` bytes, bounded: package metadata is untrusted.
pub fn read_bytes(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{FILE}: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{FILE}: exceeds 8 MiB"));
    }
    Ok(bytes)
}

/// A wasm package's catalog, checked against the hash its `[wasm]` section pins: a missing,
/// unpinned or changed `mod-points.json` is refused, so an addon routes only through the catalog the
/// library's build wrote. No pin and no file: a package without mod points.
pub fn bound(bytes: Option<&[u8]>, pinned: Option<&str>) -> Result<Option<Catalog>, String> {
    match (bytes, pinned) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(format!(
            "{FILE}: [wasm] pins no points; only the build writes {FILE}"
        )),
        (None, Some(_)) => Err(format!("{FILE}: missing; [wasm] pins one")),
        (Some(bytes), Some(pinned)) => {
            if super::super::sha512_of(bytes) != pinned {
                return Err(format!(
                    "{FILE}: does not match the hash [wasm] pins: changed after the build"
                ));
            }
            Catalog::read(bytes).map(Some)
        }
    }
}

/// Where an injection listens in its library's table.
#[derive(Debug, PartialEq, Eq)]
pub struct Routed<'a> {
    pub point: &'a Point,
    /// The place's name: the point's, or its `_return`.
    pub place: String,
    pub access: super::Access,
}

/// Why an addon's injection at a mod point doesn't load.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A routing refusal shared with the game's points (missing, no slot, no API record).
    Inject(InjectError),
    /// The library's function changed since the addon pinned it.
    Changed(Box<Changed>),
    /// The function exists but its author opted it out with `#[mod_api::no_point]`.
    OptedOut { id: String, at: String },
    /// The library's crate isn't named after its mod id, so no manifest can name its points.
    LibraryCrate {
        id: String,
        library: String,
        krate: String,
    },
    /// `internals` reaches the library's own types, so it needs the library's target.
    CrossTargetInternals { id: String, library: String },
    /// An `api` record crosses targets only with a `Wire` form for every parameter.
    CrossTargetNoWire {
        id: String,
        library: String,
        reason: String,
    },
}

/// A mod point an addon pinned, changed in the installed library.
#[derive(Debug, PartialEq, Eq)]
pub struct Changed {
    pub id: String,
    pub library: String,
    pub version: String,
    pub at: String,
    pub pinned: String,
    pub installed: String,
    pub signature: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inject(error) => error.fmt(f),
            Self::Changed(changed) => {
                let Changed {
                    id,
                    library,
                    version,
                    at,
                    pinned,
                    installed,
                    signature,
                } = &**changed;
                write!(
                    f,
                    "E_INJECT_CHANGED: {id}: {library} {version} changed {at}: pinned {pinned}, \
                 installed {installed} ({signature}); update the declaration with `cargo \
                 horizon-mod inject update` and rebuild against it"
                )
            }
            Self::OptedOut { id, at } => write!(
                f,
                "E_INJECT_MISSING: {id}: {at} has no point (opted out with #[mod_api::no_point])"
            ),
            Self::LibraryCrate { id, library, krate } => write!(
                f,
                "{id}: mod point library {library}'s crate is {krate}; a mod point library's crate \
                 is named after its mod id: {}",
                library.replace('-', "_")
            ),
            Self::CrossTargetInternals { id, library } => write!(
                f,
                "{id}: internals access to {library} needs the same target: the same binary on \
                 native, the same dependency group on wasm"
            ),
            Self::CrossTargetNoWire {
                id,
                library,
                reason,
            } => write!(
                f,
                "{id}: an api listener at {library} on the other target crosses through the \
                 game, and {reason}: add #[derive(mod_api::Wire)] to it"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// Routes `inject`, an `[[inject]]` of mod `mod_id` whose `at` the `library` owns, to its place, or
/// refuses it with the reason a player sees.
pub fn route<'a>(
    mod_id: &str,
    inject: &Inject,
    library: &Library<'a>,
) -> Result<Routed<'a>, Refusal> {
    let id = format!("{mod_id}:{}", inject.id);
    let plain = |path: &str| path.replace(' ', "");
    let catalog = library.catalog;
    // An addon's manifest names a library by mod id, and an `at` by crate, so both targets reach
    // a library's points through the one rule `Manifest::mod_point_dependency` applies.
    if catalog.krate != library.id.replace('-', "_") {
        return Err(Refusal::LibraryCrate {
            id,
            library: library.id.to_owned(),
            krate: catalog.krate.clone(),
        });
    }
    let Some(point) = catalog
        .points
        .iter()
        .find(|point| plain(&point.target) == plain(&inject.at))
    else {
        if catalog
            .opted_out
            .iter()
            .any(|path| plain(path) == plain(&inject.at))
        {
            return Err(Refusal::OptedOut {
                id,
                at: inject.at.clone(),
            });
        }
        return Err(Refusal::Inject(InjectError::Missing {
            id,
            at: inject.at.clone(),
        }));
    };
    // Another mod's function moves with that mod's version, not the game's pin, so nothing
    // records a hash for it: the declaration must carry one.
    let Some(pinned) = &inject.hash else {
        return Err(Refusal::Inject(InjectError::Invalid {
            id,
            field: "hash",
            fix: "an injection at another mod's point pins its hash; add it with `cargo horizon-mod inject add`",
        }));
    };
    if &point.hash != pinned {
        return Err(Refusal::Changed(Box::new(Changed {
            id,
            library: library.id.to_owned(),
            version: library.version.to_owned(),
            at: inject.at.clone(),
            pinned: pinned.clone(),
            installed: point.hash.clone(),
            signature: point.signature.clone(),
        })));
    }
    let api = inject.access == super::Access::Api;
    if let (true, Some(why)) = (api, &point.api_refusal) {
        return Err(Refusal::Inject(InjectError::NoSlot {
            id,
            reason: NoSlotReason::NoApiRecord,
            declaration: format!("{}: {why}", inject.at),
        }));
    }
    let mut slots = Vec::new();
    if api || point.listener.is_some() {
        slots.push(SlotKind::Head);
    }
    if (api && point.at_return) || (!api && point.return_listener.is_some()) {
        slots.push(SlotKind::Return);
    }
    let place = Place {
        at: inject.at.clone(),
        hash: point.hash.clone(),
        slots,
        calls: Vec::new(),
        native_only: false,
    };
    // A mod point has head and return places only: call-site kinds have none.
    if inject.kind.needs_call() {
        return Err(Refusal::Inject(InjectError::NoSlot {
            id,
            reason: if api {
                NoSlotReason::ApiSitesUnsupported
            } else {
                NoSlotReason::NotListed
            },
            declaration: format!("{}: a mod point has head and return places only", inject.at),
        }));
    }
    // An internals replace needs the point's raw outcome shape, which the mod point catalog
    // doesn't record; routed as a head it would run beside the body instead of replacing it.
    if !api && inject.kind == super::Kind::Replace {
        return Err(Refusal::Inject(InjectError::NoSlot {
            id,
            reason: NoSlotReason::KindHasNoSlot,
            declaration: format!(
                "{}: a mod point takes an api replace only; use access = \"api\"",
                inject.at
            ),
        }));
    }
    match super::route(
        Entry {
            inject,
            target: Target::Native,
            mod_id,
            resolved_call: None,
        },
        std::slice::from_ref(&place),
    )
    .map_err(Refusal::Inject)?
    {
        Route::Slot(SlotKind::Head) => Ok(Routed {
            point,
            place: point.name.clone(),
            access: inject.access,
        }),
        Route::Slot(_) => Ok(Routed {
            point,
            place: format!("{}_return", point.name),
            access: inject.access,
        }),
        Route::Source(reason) => Err(Refusal::Inject(InjectError::NoSlot {
            id,
            reason,
            declaration: inject.at.clone(),
        })),
    }
}

/// Why `inject`, an `[[inject]]` of mod `mod_id` at a point of `library` built for the other
/// target, can never cross: `internals` reaches the library's own types, and an `api` record needs a
/// `Wire` form for every parameter. `None` when the point exists and its record could cross, so the
/// edge gets the general mixed-target refusal.
pub fn cross_target(
    mod_id: &str,
    inject: &Inject,
    library: &str,
    catalog: &Catalog,
) -> Option<Refusal> {
    let plain = |path: &str| path.replace(' ', "");
    let point = catalog
        .points
        .iter()
        .find(|point| plain(&point.target) == plain(&inject.at))?;
    let id = format!("{mod_id}:{}", inject.id);
    let library = library.to_owned();
    match (inject.access, &point.api_refusal) {
        (super::Access::Internals, _) => Some(Refusal::CrossTargetInternals { id, library }),
        (super::Access::Api, Some(reason)) => Some(Refusal::CrossTargetNoWire {
            id,
            library,
            reason: reason.clone(),
        }),
        (super::Access::Api, None) => None,
    }
}

#[cfg(test)]
#[path = "mod_points_tests.rs"]
mod tests;
