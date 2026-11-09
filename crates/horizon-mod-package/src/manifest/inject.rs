//! Unified `[[inject]]` declarations and routing, independent of source staging or catalog I/O.
use serde::{Deserialize, Serialize};
use std::fmt;

const RESERVED: &str = concat!(
    "_ as async await break const continue crate dyn else enum extern false fn for if ",
    "impl in let loop match mod move mut pub ref return self Self static struct super ",
    "trait true type unsafe use where while abstract become box do final gen macro ",
    "override priv try typeof unsized virtual yield"
);

/// A listener in the mod's crate root. Freshness fields are written by the inject tool.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inject {
    pub id: String,
    pub at: String,
    pub kind: Kind,
    pub with: String,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub priority: i32,
    pub hash: String,
    pub call: Option<String>,
    pub nth: Option<usize>,
    pub ctx: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Head,
    Return,
    ModifyArg,
    Replace,
    BeforeCall,
    AfterCall,
    ModifyCallArg,
    ModifyValue,
    WrapCall,
    WrapIf,
    ModifyLocal,
    Field,
    Expression,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Api,
    Internals,
}

impl Kind {
    fn slot(self) -> Option<SlotKind> {
        Some(match self {
            Self::Head | Self::ModifyArg | Self::Replace => SlotKind::Head,
            Self::Return => SlotKind::Return,
            Self::BeforeCall
            | Self::AfterCall
            | Self::ModifyCallArg
            | Self::ModifyValue
            | Self::WrapCall
            | Self::WrapIf => SlotKind::Site,
            Self::ModifyLocal => SlotKind::Local,
            Self::Field => SlotKind::Extension,
            Self::Expression => return None,
        })
    }

    fn needs_call(self) -> bool {
        matches!(self.slot(), Some(SlotKind::Site | SlotKind::Local))
    }
}

impl Inject {
    pub(super) fn check_all(entries: &[Self], mod_id: &str, code: bool) -> Result<(), InjectError> {
        let mut ids = std::collections::HashSet::new();
        for entry in entries {
            let invalid = |field, fix| InjectError::Invalid {
                id: format!("{mod_id}:{}", entry.id),
                field,
                fix,
            };
            if !code {
                return Err(invalid("inject", "declare [native] or [wasm]"));
            }
            if entry.id.trim().is_empty() || !ids.insert(&entry.id) {
                return Err(invalid("id", "use a nonempty, unique inject id"));
            }
            if entry.at.trim().is_empty() || entry.hash.trim().is_empty() {
                return Err(invalid("at/hash", "run inject add for the pinned game"));
            }
            let name = entry.with.as_bytes();
            if RESERVED.split_whitespace().any(|word| word == entry.with)
                || name.is_empty()
                || name[0].is_ascii_digit()
                || !name.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                return Err(invalid("with", "name a function in the mod's crate root"));
            }
            let call = entry.call.as_ref().is_some_and(|s| !s.trim().is_empty());
            let ctx = entry.ctx.as_ref().is_some_and(|s| !s.trim().is_empty());
            if if entry.kind.needs_call() {
                !call || entry.nth.is_none() || !ctx
            } else {
                entry.call.is_some() || entry.nth.is_some() || entry.ctx.is_some()
            } {
                return Err(invalid("call/nth/ctx", "calls/locals only: all three"));
            }
        }
        Ok(())
    }
}

/// Compiled listener forms; replace uses Head with forced cancellation and a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Head,
    Return,
    Site,
    Local,
    Extension,
}

/// Resolved catalog places, including unlisted ones; calls use resolved paths, not source text.
#[derive(Clone, Debug)]
pub struct Place {
    pub at: String,
    pub hash: String,
    pub slots: Vec<SlotKind>,
    pub calls: Vec<CallPlace>,
    pub native_only: bool,
}

#[derive(Clone, Debug)]
pub struct CallPlace {
    pub call: String,
    pub nth: usize,
    pub ctx: String,
    pub slots: Vec<SlotKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Native,
    Wasm,
}

/// The package and resolved declaration; resolving a callee is not a routing operation.
pub struct Entry<'a> {
    pub inject: &'a Inject,
    pub target: Target,
    pub mod_id: &'a str,
    pub resolved_call: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoSlotReason {
    NotListed,
    KindHasNoSlot,
    NativeOnly,
    WasmSitesPending,
    LocalSlotsPending,
    ExtensionSlotsPending,
    ApiSitesPending,
    SourcePending,
    ReplacePending,
    NoApiRecord,
    ReplaceWithoutValue,
}

impl fmt::Display for NoSlotReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotListed => "not listed",
            Self::KindHasNoSlot => "kind has no slot",
            Self::NativeOnly => "native-only place",
            Self::WasmSitesPending => "wasm sites pending",
            Self::LocalSlotsPending => "local slots pending (I5)",
            Self::ExtensionSlotsPending => "extension slots pending (I6)",
            Self::ApiSitesPending => "API site listeners pending (U4c)",
            Self::SourcePending => "source injects pending (U5)",
            Self::ReplacePending => "internals replace listeners pending (U9)",
            Self::NoApiRecord => "no API record: a parameter has no API type yet",
            Self::ReplaceWithoutValue => "replace needs a return the API carries",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Slot(SlotKind),
    Source(NoSlotReason),
}

/// Escape-hatch refusals, never part of `mod_api`; `id` is qualified as `package:inject`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectError {
    Invalid {
        id: String,
        field: &'static str,
        fix: &'static str,
    },
    Missing {
        id: String,
        at: String,
    },
    Changed {
        id: String,
        at: String,
        old: String,
        new: String,
    },
    Moved {
        id: String,
        nth: usize,
    },
    Lost {
        id: String,
        at: String,
    },
    NoSlot {
        id: String,
        reason: NoSlotReason,
        declaration: String,
    },
}

impl fmt::Display for InjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::Invalid { .. } => "E_INJECT_INVALID",
            Self::Missing { .. } => "E_INJECT_MISSING",
            Self::Changed { .. } => "E_INJECT_CHANGED",
            Self::Moved { .. } => "E_INJECT_MOVED",
            Self::Lost { .. } => "E_INJECT_LOST",
            Self::NoSlot { .. } => "E_INJECT_NO_SLOT",
        };
        write!(f, "{code}: ")?;
        match self {
            Self::Invalid { id, field, fix } => write!(f, "{id}: invalid {field}; {fix}"),
            Self::Missing { id, at } => write!(f, "{id}: {at} gone; rerun inject add"),
            Self::Changed { id, at, old, new } => {
                write!(f, "{id}: {at} {old} -> {new}; run inject update")
            }
            Self::Moved { id, nth } => write!(f, "{id} now at nth={nth}; run inject update"),
            Self::Lost { id, at } => write!(f, "{id}: {at} lost or ambiguous; rerun inject add"),
            Self::NoSlot {
                id,
                reason,
                declaration,
            } => write!(
                f,
                "{id}: {reason}; slot declaration {declaration}; use a native mod or a game build supporting this slot"
            ),
        }
    }
}
impl std::error::Error for InjectError {}

/// Route a validated entry against a resolved catalog, without I/O or source inspection.
/// Freshness failures never fall back to source. Wasm call/local slots await W3.
pub fn route(entry: Entry<'_>, slots: &[Place]) -> Result<Route, InjectError> {
    let inject = entry.inject;
    let id = format!("{}:{}", entry.mod_id, inject.id);
    let Some(place) = slots.iter().find(|p| p.at == inject.at) else {
        return Err(InjectError::Missing {
            id,
            at: inject.at.clone(),
        });
    };
    if place.hash != inject.hash {
        return Err(InjectError::Changed {
            id,
            at: inject.at.clone(),
            old: inject.hash.clone(),
            new: place.hash.clone(),
        });
    }
    if let Some(reason) = match inject.kind {
        Kind::ModifyLocal => Some(NoSlotReason::LocalSlotsPending),
        Kind::Field => Some(NoSlotReason::ExtensionSlotsPending),
        _ => None,
    } {
        return Err(InjectError::NoSlot {
            id,
            reason,
            declaration: format!(
                "slots/{}.txt: {}{}",
                entry.mod_id,
                if inject.kind == Kind::Field {
                    "type "
                } else {
                    ""
                },
                inject.at
            ),
        });
    }
    let available = if inject.kind.needs_call() {
        // An unlisted native internals call is checked against source by U2 before compilation.
        if entry.target == Target::Native
            && inject.access == Access::Internals
            && inject.kind.slot() == Some(SlotKind::Site)
            && !place
                .calls
                .iter()
                .any(|c| Some(c.call.as_str()) == entry.resolved_call)
        {
            return Ok(Route::Source(NoSlotReason::NotListed));
        }
        let mut candidates = place.calls.iter().filter(|c| {
            Some(c.call.as_str()) == entry.resolved_call
                && Some(c.ctx.as_str()) == inject.ctx.as_deref()
        });
        let current = candidates.clone().find(|c| Some(c.nth) == inject.nth);
        let (Some(call), None) = current.map_or_else(
            || (candidates.next(), candidates.next()),
            |call| (Some(call), None),
        ) else {
            return Err(InjectError::Lost {
                id,
                at: inject.at.clone(),
            });
        };
        if Some(call.nth) != inject.nth {
            return Err(InjectError::Moved { id, nth: call.nth });
        }
        &call.slots
    } else {
        &place.slots
    };
    let reason = if entry.target == Target::Wasm && place.native_only {
        NoSlotReason::NativeOnly
    } else if entry.target == Target::Wasm && inject.kind.needs_call() {
        NoSlotReason::WasmSitesPending
    } else if let Some(kind) = inject.kind.slot() {
        if available.contains(&kind) {
            return Ok(Route::Slot(kind));
        }
        NoSlotReason::NotListed
    } else {
        NoSlotReason::KindHasNoSlot
    };
    if entry.target == Target::Native {
        return Ok(Route::Source(reason));
    }
    let line = if let Some(call) = &inject.call {
        format!(
            "{} | {call} | {} | {}",
            inject.at,
            inject.nth.unwrap(),
            inject.ctx.as_deref().unwrap()
        )
    } else if inject.kind == Kind::Field {
        format!("type {}", inject.at)
    } else {
        inject.at.clone()
    };
    Err(InjectError::NoSlot {
        id,
        reason,
        declaration: format!("slots/{}.txt: {line}", entry.mod_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECL: &str =
        "[[inject]]\nid='ground'\nat='game::tick'\nkind='head'\nwith='on_tick'\nhash='signature'\n";

    fn parse(decl: &str, target: &str) -> Result<super::super::Manifest, String> {
        super::super::Manifest::parse(&format!(
            "id='sample'\nname='Sample'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n{target}\n{decl}"
        ))
    }

    fn inject(kind: Kind) -> Inject {
        let wasm = format!("[wasm]\nsha512='{}'\ntoolchain='nightly'", "a".repeat(128));
        let mut i = parse(DECL, &wasm).unwrap().inject.remove(0);
        i.kind = kind;
        if kind.needs_call() {
            i.call = Some("self.callee".into());
            i.nth = Some(0);
            i.ctx = Some("context".into());
        }
        i
    }

    fn place() -> Place {
        Place {
            at: "game::tick".into(),
            hash: "signature".into(),
            native_only: false,
            slots: vec![SlotKind::Head, SlotKind::Return, SlotKind::Extension],
            calls: vec![CallPlace {
                call: "game::callee".into(),
                nth: 0,
                ctx: "context".into(),
                slots: vec![SlotKind::Site, SlotKind::Local],
            }],
        }
    }

    fn run(i: &Inject, target: Target, places: &[Place]) -> Result<Route, InjectError> {
        route(
            Entry {
                inject: i,
                target,
                mod_id: "sample",
                resolved_call: Some("game::callee"),
            },
            places,
        )
    }

    #[test]
    fn only_absent_native_internals_calls_defer_identity_to_source() {
        let mut i = inject(Kind::BeforeCall);
        i.access = Access::Internals;
        let mut p = place();
        p.calls.clear();
        assert_eq!(
            run(&i, Target::Native, &[p.clone()]),
            Ok(Route::Source(NoSlotReason::NotListed))
        );
        assert!(matches!(
            run(&i, Target::Wasm, &[p.clone()]),
            Err(InjectError::Lost { .. })
        ));
        i.access = Access::Api;
        assert!(matches!(
            run(&i, Target::Native, &[p.clone()]),
            Err(InjectError::Lost { .. })
        ));
        i.access = Access::Internals;
        i.hash = "stale".into();
        assert!(matches!(
            run(&i, Target::Native, &[p]),
            Err(InjectError::Changed { .. })
        ));
        i.hash = "signature".into();
        i.ctx = Some("wrong".into());
        assert!(matches!(
            run(&i, Target::Native, &[place()]),
            Err(InjectError::Lost { .. })
        ));
        i.ctx = Some("context".into());
        i.nth = Some(1);
        assert!(matches!(
            run(&i, Target::Native, &[place()]),
            Err(InjectError::Moved { nth: 0, .. })
        ));
    }

    #[test]
    fn manifest_boundary() {
        let pin = "a".repeat(40);
        let native = format!("[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'");
        let m = parse(DECL, &native).unwrap();
        assert_eq!((m.inject[0].access, m.inject[0].priority), (Access::Api, 0));
        let call = "kind='wrap_call'\ncall='self.callee'\nnth=0\nctx='ctx'\naccess='internals'";
        assert!(parse(&DECL.replace("kind='head'", call), &native).is_ok());
        for (old, new) in [
            ("id='ground'", "id=''"),
            ("at='game::tick'", "at=''"),
            ("hash='signature'", "hash=''"),
            ("with='on_tick'", "with='src/lib.rs'"),
            ("kind='head'", "kind='HEAD'"),
            ("kind='head'", "kind='wrap_call'"),
            ("kind='head'", "kind='head'\nnth=0"),
            ("kind='head'", "kind='head'\naccess='raw'"),
            ("at='game::tick'", "target='game::tick'"),
        ] {
            assert!(parse(&DECL.replace(old, new), &native).is_err(), "{new}");
        }
        assert!(parse(&format!("{DECL}{DECL}"), &native).is_err());
        assert!(parse(DECL, "").is_err());
        for name in RESERVED.split_whitespace() {
            let mut i = inject(Kind::Head);
            i.with = name.into();
            assert!(
                matches!(Inject::check_all(&[i], "sample", true), Err(InjectError::Invalid { id, field: "with", .. }) if id == "sample:ground")
            );
        }
    }

    #[test]
    fn every_kind_access_target_and_listing() {
        use SlotKind::*;
        let rows = [
            (Kind::Head, Some(Head)),
            (Kind::Return, Some(Return)),
            (Kind::ModifyArg, Some(Head)),
            (Kind::Replace, Some(Head)),
            (Kind::BeforeCall, Some(Site)),
            (Kind::AfterCall, Some(Site)),
            (Kind::ModifyCallArg, Some(Site)),
            (Kind::ModifyValue, Some(Site)),
            (Kind::WrapCall, Some(Site)),
            (Kind::WrapIf, Some(Site)),
            (Kind::ModifyLocal, Some(Local)),
            (Kind::Field, Some(Extension)),
            (Kind::Expression, None),
        ];
        for (kind, expected) in rows {
            for access in [Access::Api, Access::Internals] {
                for (listed, native_only) in [(true, false), (false, false), (true, true)] {
                    let mut i = inject(kind);
                    i.access = access;
                    let mut p = place();
                    p.native_only = native_only;
                    if !listed {
                        p.slots.clear();
                        p.calls[0].slots.clear();
                    }
                    if let Some(pending) = match kind {
                        Kind::ModifyLocal => Some(NoSlotReason::LocalSlotsPending),
                        Kind::Field => Some(NoSlotReason::ExtensionSlotsPending),
                        _ => None,
                    } {
                        for target in [Target::Native, Target::Wasm] {
                            assert!(
                                matches!(run(&i, target, &[p.clone()]), Err(InjectError::NoSlot { reason, .. }) if reason == pending)
                            );
                        }
                        continue;
                    }
                    let reason = match expected {
                        None => Some(NoSlotReason::KindHasNoSlot),
                        Some(_) if !listed => Some(NoSlotReason::NotListed),
                        _ => None,
                    };
                    let native =
                        reason.map_or_else(|| Route::Slot(expected.unwrap()), Route::Source);
                    assert_eq!(run(&i, Target::Native, &[p.clone()]), Ok(native));
                    let reason = if native_only {
                        Some(NoSlotReason::NativeOnly)
                    } else if matches!(expected, Some(Site | Local)) {
                        Some(NoSlotReason::WasmSitesPending)
                    } else {
                        reason
                    };
                    let wasm = run(&i, Target::Wasm, &[p]);
                    if let Some(r) = reason {
                        assert!(
                            matches!(wasm, Err(InjectError::NoSlot { reason, declaration, .. }) if reason == r && declaration.starts_with("slots/sample.txt:") && declaration.contains("game::tick"))
                        );
                    } else {
                        assert_eq!(wasm, Ok(Route::Slot(expected.unwrap())));
                    }
                }
            }
        }
    }

    #[test]
    fn freshness_refuses_both_targets() {
        for target in [Target::Native, Target::Wasm] {
            for (case, code) in [
                ("missing", "E_INJECT_MISSING"),
                ("changed", "E_INJECT_CHANGED"),
                ("moved", "E_INJECT_MOVED"),
                ("ambiguous", "E_INJECT_LOST"),
                ("context", "E_INJECT_LOST"),
                ("callee", "E_INJECT_LOST"),
            ] {
                let mut i = inject(Kind::WrapCall);
                let mut places = vec![place()];
                match case {
                    "missing" => places.clear(),
                    "changed" => places[0].hash = "new".into(),
                    "moved" => {
                        places[0].calls[0].nth = 1;
                        let mut other = place().calls.remove(0);
                        other.ctx = "another statement".into();
                        places[0].calls.push(other);
                    }
                    "ambiguous" => {
                        i.nth = Some(4);
                        places[0].calls.push(place().calls.remove(0));
                    }
                    "context" => places[0].calls[0].ctx = "changed".into(),
                    "callee" => places[0].calls[0].call = "game::other".into(),
                    _ => unreachable!(),
                }
                let error = run(&i, target, &places).unwrap_err().to_string();
                assert!(error.starts_with(code), "{case}");
                assert!(error.contains("sample:ground"));
            }
        }
    }
}
