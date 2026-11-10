//! Native `[[inject]]` slot routing. The catalog owns validation; generation consumes checked entries.
use super::{inject::PointInjection, internals::Catalog, sites::Site};
use horizon_mod_package::{
    manifest::inject::{
        route, Access, CallPlace, Entry, Inject, InjectError, Kind, NoSlotReason, Place, Route,
        SlotKind, Target,
    },
    Manifest,
};

pub(super) fn points<'a>(
    listed: &'a [super::inject::ListedPoint],
    manifest: &'a Manifest,
) -> impl Iterator<Item = PointInjection> + 'a {
    manifest
        .inject
        .iter()
        .filter(|entry| {
            entry.kind != Kind::Field
                && entry.access == Access::Internals
                && entry.call.is_none()
                && listed.iter().any(|point| {
                    super::internals::plain_path(&point.target)
                        == super::internals::plain_path(&entry.at)
                })
        })
        .map(|entry| PointInjection {
            mod_id: manifest.id.clone(),
            id: entry.id.clone(),
            target: entry.at.clone(),
            method: entry.with.clone(),
            at_return: entry.kind == Kind::Return,
        })
}

pub(super) fn sites<'a>(
    sites: &'a [Site],
    manifest: &'a Manifest,
) -> impl Iterator<Item = (&'a Inject, &'a Site)> {
    manifest
        .inject
        .iter()
        .filter(|entry| entry.call.is_some() || entry.expr.is_some())
        .map(|entry| (entry, selected(sites, entry).expect("slot route checked")))
}

fn selected<'a>(sites: &'a [Site], entry: &Inject) -> Option<&'a Site> {
    sites.iter().find(|site| {
        super::internals::plain_path(&site.target) == super::internals::plain_path(&entry.at)
            && Some(&site.call)
                == entry
                    .call
                    .clone()
                    .or_else(|| {
                        horizon_platform::mod_build::api_gen::sites::expression::spelling(
                            entry.expr.as_ref()?,
                        )
                        .ok()
                    })
                    .as_ref()
            && site.expression.as_ref().map(|e| &e.ty)
                == entry.local_type.as_ref().filter(|_| entry.expr.is_some())
            && Some(&site.ctx) == entry.ctx.as_ref()
            && Some(site.ordinal) == entry.nth
            && site.local.as_ref().map(|l| (&l.binding, &l.ty, l.after))
                == entry.local.as_ref().map(|name| {
                    (
                        name,
                        entry.local_type.as_ref().expect("validated type"),
                        entry.position
                            == Some(horizon_mod_package::manifest::inject::Position::After),
                    )
                })
    })
}

/// `manifest`'s injections at the game's own functions: not another mod's points.
fn engine<'a>(catalog: &'a Catalog, manifest: &'a Manifest) -> impl Iterator<Item = &'a Inject> {
    manifest
        .inject
        .iter()
        .filter(|inject| catalog.mod_owner(&inject.at).is_none())
}

pub(super) fn check(catalog: &Catalog, manifest: &Manifest) -> Result<(), InjectError> {
    routes(catalog, manifest).map(|_| ())
}

pub(super) fn routes(catalog: &Catalog, manifest: &Manifest) -> Result<Vec<Route>, InjectError> {
    let plain = super::internals::plain_path;
    let places = manifest
        .inject
        .iter()
        .filter_map(|entry| {
            let item = catalog.target(&entry.at)?;
            let point = catalog
                .points
                .iter()
                .find(|point| !point.private && plain(&point.target) == plain(&item.path));
            Some(Place {
                at: entry.at.clone(),
                hash: item.hash.clone(),
                native_only: false,
                slots: point
                    .map(|point| {
                        let mut slots = Vec::new();
                        if entry.access == Access::Api || (point.raw && point.listener.is_some()) {
                            slots.push(SlotKind::Head);
                        }
                        if (entry.access == Access::Api && point.at_return)
                            || (entry.access == Access::Internals
                                && point.raw
                                && point.return_listener.is_some())
                        {
                            slots.push(SlotKind::Return);
                        }
                        slots
                    })
                    .unwrap_or_default(),
                calls: catalog
                    .sites
                    .iter()
                    .filter(|site| !site.private && plain(&site.target) == plain(&item.path))
                    .map(|site| CallPlace {
                        call: site.call.clone(),
                        nth: site.ordinal,
                        ctx: site.ctx.clone(),
                        slots: vec![SlotKind::Site],
                    })
                    .collect(),
            })
        })
        .collect::<Vec<_>>();
    let mut decisions = Vec::new();
    // Injections at another mod's points route through that mod's table (`mod_points`).
    for inject in engine(catalog, manifest) {
        // An API listener needs the point's record: every parameter with an API type.
        let refusal = catalog
            .points
            .iter()
            .find(|point| plain(&point.target) == plain(&inject.at))
            .and_then(|point| point.api_refusal.as_ref())
            .filter(|_| inject.access == Access::Api && inject.call.is_none());
        if let Some(why) = refusal {
            return Err(InjectError::NoSlot {
                id: format!("{}:{}", manifest.id, inject.id),
                reason: NoSlotReason::NoApiRecord,
                declaration: format!("{}: {why}", inject.at),
            });
        }
        let route = route(
            Entry {
                inject,
                target: Target::Native,
                mod_id: &manifest.id,
                resolved_call: inject.call.as_deref(),
            },
            &places,
        )?;
        let pending = match (route, inject.kind, inject.access) {
            (Route::Source(reason), _, Access::Api) => Some(reason),
            (Route::Slot(SlotKind::Site), _, Access::Api) => {
                Some(NoSlotReason::ApiSitesUnsupported)
            }
            // An internals replace is built from the listed point's raw shape: an unlisted
            // function needs its slot row first, which the declaration names.
            (Route::Source(reason), Kind::Replace, Access::Internals) => Some(reason),
            (
                Route::Source(_),
                Kind::Field
                | Kind::Head
                | Kind::Return
                | Kind::BeforeCall
                | Kind::ModifyLocal
                | Kind::Expression
                | Kind::AfterCall
                | Kind::ModifyValue
                | Kind::WrapCall
                | Kind::WrapIf,
                Access::Internals,
            ) => None,
            _ => None,
        };
        if let Some(reason) = pending {
            return Err(InjectError::NoSlot {
                id: format!("{}:{}", manifest.id, inject.id),
                reason,
                declaration: format!("slots/{}.txt: {} ({route:?})", manifest.id, inject.at),
            });
        }
        // A replace produces the outcome: the same check the shared emitter makes, at load.
        let replace_refusal = catalog
            .points
            .iter()
            .find(|point| plain(&point.target) == plain(&inject.at))
            .filter(|_| inject.kind == Kind::Replace)
            .and_then(|point| match inject.access {
                Access::Api => {
                    horizon_mod_point_emit::replaceable(&inject.id, &inject.at, &point.emitted())
                        .err()
                }
                Access::Internals => horizon_mod_point_emit::internals_replace(
                    inject,
                    &manifest.id,
                    &point.emitted(),
                    "raw",
                    &inject.with,
                )
                .err(),
            });
        if let Some(why) = replace_refusal {
            return Err(InjectError::NoSlot {
                id: format!("{}:{}", manifest.id, inject.id),
                reason: NoSlotReason::ReplaceWithoutValue,
                declaration: why,
            });
        }
        decisions.push(route);
        if inject.kind == Kind::WrapIf
            && selected(&catalog.sites, inject).is_some_and(|site| site.output != "()")
        {
            return Err(InjectError::Invalid {
                id: format!("{}:{}", manifest.id, inject.id),
                field: "kind",
                fix: "wrap_if requires a unit-returning call",
            });
        }
    }
    Ok(decisions)
}

pub(super) fn report<'a>(
    catalog: &'a Catalog,
    manifest: &'a Manifest,
) -> impl Iterator<Item = String> + 'a {
    engine(catalog, manifest)
        .zip(routes(catalog, manifest).expect("checked routes"))
        .map(|(entry, route)| {
            if let Route::Source(reason) = route {
                return format!(
                    "{}:{}: {} -> source (private slot, {reason})",
                    manifest.id, entry.id, entry.at
                );
            }
            let form = if entry.call.is_some() {
                "site"
            } else if entry.kind == Kind::Return {
                "return"
            } else {
                "head"
            };
            format!(
                "{}:{}: {} -> slot ({form}, {})",
                manifest.id,
                entry.id,
                entry.at,
                if entry.access == Access::Api {
                    "api"
                } else {
                    "internals"
                }
            )
        })
}

#[cfg(test)]
mod tests;
