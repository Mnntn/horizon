//! `[[inject]]` declarations and their compiled slots, independent of catalog I/O. An inject
//! never compiles game code: a place with no slot is refused, and [`super::mixin`] is the form
//! that edits source.
use serde::{Deserialize, Serialize};
use std::fmt;

pub mod mod_points;

const RESERVED: &str = concat!(
    "_ as async await break const continue crate dyn else enum extern false fn for if ",
    "impl in let loop match mod move mut pub ref return self Self static struct super ",
    "trait true type unsafe use where while abstract become box do final gen macro ",
    "override priv try typeof unsized virtual yield"
);

/// A listener at a compiled slot. Freshness fields are written by the inject tool.
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
    /// Optional pinning: the function's catalog fingerprint `inject add` writes. Absent, the
    /// build takes it from the pinned game's catalog, which `[native] horizon` fixes exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    pub call: Option<String>,
    pub nth: Option<usize>,
    pub ctx: Option<String>,
}

/// What a listener does at its place, spelled in snake case in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Head,
    Return,
    Replace,
    BeforeCall,
    AfterCall,
    ModifyValue,
    WrapCall,
    WrapIf,
}

impl Kind {
    pub const ALL: [Self; 8] = [
        Self::Head,
        Self::Return,
        Self::Replace,
        Self::BeforeCall,
        Self::AfterCall,
        Self::ModifyValue,
        Self::WrapCall,
        Self::WrapIf,
    ];

    /// The manifest spelling.
    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Return => "return",
            Self::Replace => "replace",
            Self::BeforeCall => "before_call",
            Self::AfterCall => "after_call",
            Self::ModifyValue => "modify_value",
            Self::WrapCall => "wrap_call",
            Self::WrapIf => "wrap_if",
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// A removed spelling is refused naming the form that replaced it; it is never read as that form.
/// The source-level kinds name the `[[native.mixin]]` form that compiles them.
impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = std::borrow::Cow::<str>::deserialize(deserializer)?;
        if let Some(kind) = Self::ALL.into_iter().find(|kind| kind.name() == name) {
            return Ok(kind);
        }
        Err(serde::de::Error::custom(match &*name {
            "modify_arg" => {
                "kind `modify_arg` was removed: use `head`, whose listener already gets \
                             every argument to change"
                    .to_owned()
            }
            "modify_call_arg" => "kind `modify_call_arg` was removed: use `before_call`, whose \
                                  listener gets the call's arguments mutably before it runs"
                .to_owned(),
            "modify_local" => source_level(&name, "`[[native.mixin]]` kind `MODIFY_VARIABLE`"),
            "expression" => source_level(
                &name,
                "`[[native.mixin]]` kind `MODIFY_CONSTANT` or `MODIFY_EXPRESSION_VALUE`",
            ),
            "field" => source_level(&name, "`[[native.state]]`"),
            other => format!(
                "unknown kind `{other}`, expected one of {}",
                Self::ALL.map(Self::name).join(", ")
            ),
        }))
    }
}

fn source_level(kind: &str, form: &str) -> String {
    format!(
        "E_INJECT_NO_SLOT: kind `{kind}` has no compiled slot and `[[inject]]` never compiles \
         game code: use {form} in a native mod"
    )
}

/// The manifest's `[[inject]]` rows. A row's kind is read before its other keys, so a removed
/// kind is refused by name even when keys that went with it follow.
pub(super) fn rows<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Inject>, D::Error> {
    Vec::<toml::Table>::deserialize(deserializer)?
        .into_iter()
        .map(|row| {
            if let Some(kind) = row.get("kind") {
                Kind::deserialize(kind.clone())?;
            }
            row.try_into()
        })
        .collect::<Result<_, toml::de::Error>>()
        .map_err(serde::de::Error::custom)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Api,
    Internals,
}

impl Kind {
    /// The compiled slot form this kind attaches to.
    pub fn slot(self) -> SlotKind {
        match self {
            Self::Head | Self::Replace => SlotKind::Head,
            Self::Return => SlotKind::Return,
            Self::BeforeCall
            | Self::AfterCall
            | Self::ModifyValue
            | Self::WrapCall
            | Self::WrapIf => SlotKind::Site,
        }
    }

    fn needs_call(self) -> bool {
        self.slot() == SlotKind::Site
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
            if entry.at.trim().is_empty() {
                return Err(invalid("at", "name the target function's canonical path"));
            }
            if entry
                .hash
                .as_ref()
                .is_some_and(|hash| hash.trim().is_empty())
            {
                return Err(invalid(
                    "hash",
                    "omit it, or run inject add for the pinned game",
                ));
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
                return Err(invalid(
                    "call/nth/ctx",
                    "call kinds take all three; other kinds take none",
                ));
            }
        }
        Ok(())
    }
}

/// Compiled listener forms; replace uses Head with forced cancellation or a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Head,
    Return,
    Site,
}

/// Resolved catalog places; calls use resolved paths, not source text.
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
    WasmSitesUnsupported,
    ApiSitesUnsupported,
    NoApiRecord,
    ReplaceWithoutValue,
}

impl fmt::Display for NoSlotReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotListed => "not listed",
            Self::KindHasNoSlot => "kind has no slot",
            Self::NativeOnly => "native-only place",
            Self::WasmSitesUnsupported => {
                "wasm does not support internals or call-site injections; use a hook"
            }
            Self::ApiSitesUnsupported => "API call-site injections require internals",
            Self::NoApiRecord => "no API record: a parameter has no API type yet",
            Self::ReplaceWithoutValue => "replace needs a return its record carries",
        })
    }
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
                "{id}: {reason}; slot declaration {declaration}; [[inject]] never compiles game code: use [[native.mixin]] in a native mod, or request the slot"
            ),
        }
    }
}
impl std::error::Error for InjectError {}

/// The compiled slot a validated entry attaches to, against a resolved catalog and without I/O.
/// A place or kind with no slot is refused on every target: nothing here rebuilds the game.
/// Wasm call-site injections are permanently unsupported.
pub fn route(entry: Entry<'_>, slots: &[Place]) -> Result<SlotKind, InjectError> {
    let inject = entry.inject;
    let id = format!("{}:{}", entry.mod_id, inject.id);
    let Some(place) = slots.iter().find(|p| p.at == inject.at) else {
        return Err(InjectError::Missing {
            id,
            at: inject.at.clone(),
        });
    };
    // An absent hash follows the catalog routed against: the pinned game's own.
    if let Some(old) = inject.hash.as_ref().filter(|hash| **hash != place.hash) {
        return Err(InjectError::Changed {
            id,
            at: inject.at.clone(),
            old: old.clone(),
            new: place.hash.clone(),
        });
    }
    let available = if inject.kind.needs_call() {
        // A native internals call to a callee with no listed call here was never a slot:
        // it names the line to request, where a listed call that moved is lost.
        if entry.target == Target::Native
            && inject.access == Access::Internals
            && !place
                .calls
                .iter()
                .any(|c| Some(c.call.as_str()) == entry.resolved_call)
        {
            return Err(InjectError::NoSlot {
                id,
                reason: NoSlotReason::NotListed,
                declaration: format!("slots/{}.txt: {}", entry.mod_id, line(inject)),
            });
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
        NoSlotReason::WasmSitesUnsupported
    } else if available.contains(&inject.kind.slot()) {
        return Ok(inject.kind.slot());
    } else {
        NoSlotReason::NotListed
    };
    Err(InjectError::NoSlot {
        id,
        reason,
        declaration: format!("slots/{}.txt: {}", entry.mod_id, line(inject)),
    })
}

/// The slot list line that would give `inject` its slot.
fn line(inject: &Inject) -> String {
    match (&inject.call, inject.nth, &inject.ctx) {
        (Some(call), Some(nth), Some(ctx)) => format!("{} | {call} | {nth} | {ctx}", inject.at),
        _ => inject.at.clone(),
    }
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
            slots: vec![SlotKind::Head, SlotKind::Return],
            calls: vec![CallPlace {
                call: "game::callee".into(),
                nth: 0,
                ctx: "context".into(),
                slots: vec![SlotKind::Site],
            }],
        }
    }

    fn run(i: &Inject, target: Target, places: &[Place]) -> Result<SlotKind, InjectError> {
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
    fn an_unlisted_native_internals_call_is_refused_naming_its_slot_line() {
        let mut i = inject(Kind::BeforeCall);
        i.access = Access::Internals;
        let mut p = place();
        p.calls.clear();
        assert_eq!(
            run(&i, Target::Native, &[p.clone()]),
            Err(InjectError::NoSlot {
                id: "sample:ground".into(),
                reason: NoSlotReason::NotListed,
                declaration: "slots/sample.txt: game::tick | self.callee | 0 | context".into(),
            })
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
        i.hash = Some("stale".into());
        assert!(matches!(
            run(&i, Target::Native, &[p]),
            Err(InjectError::Changed { .. })
        ));
        i.hash = Some("signature".into());
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
        // Removed spellings name their replacement; they are never read as it.
        for (old, new) in [
            ("modify_arg", "`head`"),
            ("modify_call_arg", "`before_call`"),
        ] {
            let error = parse(&DECL.replace("'head'", &format!("'{old}'")), &native).unwrap_err();
            assert!(
                error.contains(&format!("kind `{old}` was removed: use {new}")),
                "{error}"
            );
        }
        // A source-level kind names the form that compiles it, whatever keys follow it.
        for (old, new) in [
            ("modify_local", "`[[native.mixin]]` kind `MODIFY_VARIABLE`"),
            (
                "expression",
                "`[[native.mixin]]` kind `MODIFY_CONSTANT` or `MODIFY_EXPRESSION_VALUE`",
            ),
            ("field", "`[[native.state]]`"),
        ] {
            let row = DECL.replace(
                "'head'",
                &format!("'{old}'\nlocal='n'\ntype='i32'\nposition='before'\nexpr='1'"),
            );
            let error = parse(&row, &native).unwrap_err();
            assert!(
                error.contains(&format!(
                    "E_INJECT_NO_SLOT: kind `{old}` has no compiled slot and `[[inject]]` never \
                     compiles game code: use {new} in a native mod"
                )),
                "{error}"
            );
        }
        // Their keys went with them.
        for key in ["local='n'", "type='i32'", "position='before'", "expr='1'"] {
            let error = parse(&format!("{DECL}{key}\n"), &native).unwrap_err();
            assert!(error.contains("unknown field"), "{error}");
        }
        let error = parse(&DECL.replace("'head'", "'overwrite'"), &native).unwrap_err();
        assert!(
            error.contains("unknown kind `overwrite`, expected one of head, return, replace"),
            "{error}"
        );
        for kind in Kind::ALL {
            let value = toml::Value::try_from(kind).unwrap();
            assert_eq!(value.as_str(), Some(kind.name()));
            assert_eq!(value.try_into::<Kind>().unwrap(), kind);
        }
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
            (Kind::Head, Head),
            (Kind::Return, Return),
            (Kind::Replace, Head),
            (Kind::BeforeCall, Site),
            (Kind::AfterCall, Site),
            (Kind::ModifyValue, Site),
            (Kind::WrapCall, Site),
            (Kind::WrapIf, Site),
        ];
        assert_eq!(rows.len(), Kind::ALL.len());
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
                    // No slot is the same refusal on native as on wasm: nothing rebuilds.
                    let refused = |target, why| {
                        assert!(
                            matches!(run(&i, target, std::slice::from_ref(&p)), Err(InjectError::NoSlot { reason, declaration, .. }) if reason == why && declaration.starts_with("slots/sample.txt: game::tick")),
                            "{kind:?} {access:?} {target:?}"
                        );
                    };
                    if listed {
                        assert_eq!(
                            run(&i, Target::Native, std::slice::from_ref(&p)),
                            Ok(expected)
                        );
                    } else {
                        refused(Target::Native, NoSlotReason::NotListed);
                    }
                    if native_only {
                        refused(Target::Wasm, NoSlotReason::NativeOnly);
                    } else if expected == Site {
                        refused(Target::Wasm, NoSlotReason::WasmSitesUnsupported);
                    } else if listed {
                        assert_eq!(
                            run(&i, Target::Wasm, std::slice::from_ref(&p)),
                            Ok(expected)
                        );
                    } else {
                        refused(Target::Wasm, NoSlotReason::NotListed);
                    }
                }
            }
        }
    }

    #[test]
    fn wasm_call_site_refusal_is_permanent() {
        let i = inject(Kind::BeforeCall);
        let error = run(&i, Target::Wasm, &[place()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "E_INJECT_NO_SLOT: sample:ground: wasm does not support internals or call-site injections; use a hook; slot declaration slots/sample.txt: game::tick | self.callee | 0 | context; [[inject]] never compiles game code: use [[native.mixin]] in a native mod, or request the slot"
        );
    }

    #[test]
    fn an_absent_hash_follows_the_catalog_and_a_present_one_is_checked() {
        let native = format!(
            "[native]\ngame='26.3'\nhorizon='{0}'\nmixins='{0}'",
            "a".repeat(40)
        );
        let m = parse(&DECL.replace("hash='signature'\n", ""), &native).unwrap();
        let mut i = m.inject[0].clone();
        assert_eq!(i.hash, None);
        let mut p = place();
        p.hash = "whatever the pinned game says".into();
        for target in [Target::Native, Target::Wasm] {
            assert_eq!(run(&i, target, &[p.clone()]), Ok(SlotKind::Head));
            // Still refused where the function is gone.
            assert!(matches!(
                run(&i, target, &[]),
                Err(InjectError::Missing { .. })
            ));
        }
        i.hash = Some("stale".into());
        assert!(matches!(
            run(&i, Target::Native, &[p]),
            Err(InjectError::Changed { old, .. }) if old == "stale"
        ));
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
