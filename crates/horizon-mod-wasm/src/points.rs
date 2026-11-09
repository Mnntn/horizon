//! Points use the hook transport and register in the native points list.
use crate::{CallClass, Refusal, WasmMod, WeakWasmMod, boundary::Frame};
use horizon_mod_package::manifest::{
    Manifest,
    inject::{Access, Inject, Kind},
};
use mod_points::__private::{Forward, PointHost, SlotListener};
use mod_points::PointId;
use std::cell::RefCell;

struct Endpoint {
    owner: &'static str,
    module: WeakWasmMod,
    listener: u32,
}

impl Forward for Endpoint {
    fn forward(&self, id: u32, host: PointHost<'_>, record: &mut dyn mod_api::wire::Record) {
        let mut frame = Frame::default();
        let mut side = None;
        match host {
            PointHost::Read(host) => frame.set_host(host),
            PointHost::Server(host) => frame.set_server_host_mut(host),
            PointHost::Client(host) => frame.set_client_host_mut(host),
            PointHost::Side(mod_api::HostSide::Server(host)) => {
                side = Some(true);
                frame.set_server_host_mut(host);
            }
            PointHost::Side(mod_api::HostSide::Client(host)) => {
                side = Some(false);
                frame.set_client_host_mut(host);
            }
            PointHost::None => {}
        }
        self.invoke(
            frame,
            id,
            |out| {
                if let Some(server) = side {
                    out.put(&server);
                }
            },
            record,
        );
    }
}

impl Endpoint {
    fn invoke(
        &self,
        frame: Frame,
        id: u32,
        prefix: impl FnOnce(&mut mod_api::wire::Writer),
        record: &mut dyn mod_api::wire::Record,
    ) {
        let record = RefCell::new(record);
        self.module.call(
            CallClass::Hook,
            frame,
            id,
            None,
            |out| {
                out.put(&self.listener);
                prefix(out);
                record.borrow().encode(out);
            },
            |input| {
                input.take::<()>()?;
                let bytes = input.take_bytes(input.rest().len())?;
                record.borrow_mut().apply(bytes)
            },
        );
        if self.module.is_stopped() {
            mod_points::stop(self.owner);
        }
    }
}

/// Check subscriptions completely before registering any listener in the host's list. Internals
/// injections never reach here: `load` refuses them, a native-only tier.
pub(crate) fn listen(manifest: &Manifest, module: &WasmMod) -> Result<(), Refusal> {
    listen_checked(manifest, module).map_err(Refusal::Runtime)
}

fn listen_checked(manifest: &Manifest, module: &WasmMod) -> Result<(), String> {
    if manifest.inject.is_empty() {
        return Ok(());
    }
    let listed: Vec<(u32, String)> = module
        .call(
            CallClass::Load,
            Frame::default(),
            0xBFFF_FFFF,
            None,
            |_| {},
            |input| input.take(),
        )
        .ok_or("point subscriptions did not answer")?;
    let mut declared = manifest
        .inject
        .iter()
        .map(|inject| (inject.id.as_str(), inject))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut slots = Vec::new();
    for (index, (place, id)) in listed.into_iter().enumerate() {
        let inject = declared
            .remove(id.as_str())
            .ok_or_else(|| format!("undeclared point listener {id}"))?;
        let point = checked_place(&id, inject, place)?;
        slots.push((
            point,
            index as u32,
            inject.priority,
            id,
            inject.kind == Kind::Replace,
        ));
    }
    if let Some(id) = declared.keys().next() {
        return Err(format!("unfilled point listener {id}"));
    }
    let owner = Box::leak(manifest.id.clone().into_boxed_str());
    for (place, listener, priority, id, replace) in slots {
        let endpoint = Box::leak(Box::new(Endpoint {
            owner,
            module: module.downgrade(),
            listener,
        }));
        let id = Box::leak(id.into_boxed_str());
        let mut slot = SlotListener::forwarded(owner, place, endpoint, 0x8000_0000 + place as u32)
            .ordered(priority, id);
        // The guest's listener already owns the outcome (`mod_points::replace!`); the host's
        // slot carries the mark, so a second replace at the place is refused for either target.
        if replace {
            slot = slot.replacing();
        }
        mod_points::register(&slot).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The engine's API place `place` that a guest subscribed as its declared `inject`, checked
/// against the game's own point list, not what the module says of itself: the target, the phase,
/// and, for a `replace`, that its head returns `()` or an API value the listener can produce.
fn checked_place(id: &str, inject: &Inject, place: u32) -> Result<PointId, String> {
    let Some(&(point, target, at_return, replace_refusal)) = mod_points::API_PLACES
        .iter()
        .find(|(point, ..)| *point as u32 == place)
    else {
        return Err(format!("{id}: no API record for place {place}"));
    };
    if inject.access != Access::Api
        || !matches!(
            inject.kind,
            Kind::Head | Kind::ModifyArg | Kind::Replace | Kind::Return
        )
        || (inject.kind == Kind::Return) != at_return
        || horizon_mod_point_emit::plain_path(&inject.at)
            != horizon_mod_point_emit::plain_path(target)
    {
        return Err(format!(
            "{id}: listener does not match its declared API head/return target"
        ));
    }
    if let Some(ty) = replace_refusal.filter(|_| inject.kind == Kind::Replace) {
        return Err(format!(
            "{id}: no replace listener at {target}: a replace returns the function's value, and \
             `{ty}` has no API form"
        ));
    }
    Ok(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inject(kind: Kind, at: &str) -> Inject {
        Inject {
            id: "own".into(),
            at: at.into(),
            kind,
            with: "own".into(),
            access: Access::Api,
            priority: 0,
            hash: "pin".into(),
            call: None,
            nth: None,
            ctx: None,
        }
    }

    /// A module built without the build tool may subscribe a `replace` anywhere: the loader
    /// refuses one at a head whose return it can't produce, by the game's own point list.
    #[test]
    fn a_replace_is_checked_against_the_games_point_list_at_load() {
        let heads = || {
            mod_points::API_PLACES
                .iter()
                .filter(|(_, _, at_return, _)| !at_return)
        };
        let &(fallible, target, _, refusal) = heads()
            .find(|(.., refusal)| refusal.is_some_and(|ty| ty.starts_with("Result")))
            .expect("a game point returning Result<(), E> with an API record");
        let error =
            checked_place("own", &inject(Kind::Replace, target), fallible as u32).unwrap_err();
        assert!(
            error.contains(target) && error.contains(&format!("`{}`", refusal.unwrap())),
            "{error}"
        );
        // An ordinary head listener there is unaffected.
        assert_eq!(
            checked_place("own", &inject(Kind::Head, target), fallible as u32),
            Ok(fallible)
        );
        let &(owned, target, ..) = heads()
            .find(|(.., refusal)| refusal.is_none())
            .expect("a game point a replace can own");
        assert_eq!(
            checked_place("own", &inject(Kind::Replace, target), owned as u32),
            Ok(owned)
        );
    }
}
