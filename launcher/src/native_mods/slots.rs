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
            entry.access == Access::Internals
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
        .filter(|entry| entry.call.is_some())
        .map(|entry| (entry, selected(sites, entry).expect("slot route checked")))
}

fn selected<'a>(sites: &'a [Site], entry: &Inject) -> Option<&'a Site> {
    sites.iter().find(|site| {
        super::internals::plain_path(&site.target) == super::internals::plain_path(&entry.at)
            && Some(&site.call) == entry.call.as_ref()
            && Some(&site.ctx) == entry.ctx.as_ref()
            && Some(site.ordinal) == entry.nth
    })
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
            let item = catalog.internals.iter().find(|item| {
                !item.shape_only
                    && plain(&item.path) == plain(&entry.at)
                    && matches!(item.kind.as_str(), "fn" | "method" | "struct")
            })?;
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
    for inject in &manifest.inject {
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
            (_, Kind::ModifyCallArg, _) => Some(NoSlotReason::SourcePending),
            (
                Route::Source(_),
                Kind::Head
                | Kind::Return
                | Kind::BeforeCall
                | Kind::AfterCall
                | Kind::ModifyValue
                | Kind::WrapCall
                | Kind::WrapIf,
                Access::Internals,
            ) => None,
            (Route::Source(_), _, _) => Some(NoSlotReason::SourcePending),
            (_, Kind::Replace, Access::Internals) => Some(NoSlotReason::ReplacePending),
            (Route::Slot(SlotKind::Site), _, Access::Api) => Some(NoSlotReason::ApiSitesPending),
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
            .and_then(|point| {
                horizon_mod_point_emit::replaceable(&inject.id, &inject.at, &point.emitted()).err()
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
    manifest
        .inject
        .iter()
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
