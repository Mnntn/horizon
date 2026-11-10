//! Unified `[[inject]]` declarations and routing, independent of source staging or catalog I/O.
use serde::{Deserialize, Serialize};
use std::fmt;

pub mod mod_points;

const RESERVED: &str = concat!(
    "_ as async await break const continue crate dyn else enum extern false fn for if ",
    "impl in let loop match mod move mut pub ref return self Self static struct super ",
    "trait true type unsafe use where while abstract become box do final gen macro ",
    "override priv try typeof unsized virtual yield"
);

/// A listener or transient source field. Freshness fields are written by the inject tool.
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
    pub local: Option<String>,
    /// Rust type of the selected local, expression value, or injected field.
    #[serde(rename = "type")]
    pub local_type: Option<String>,
    pub position: Option<Position>,
    /// An `expression`'s selector: a literal (`64.0`, `"name"`), a constant's path or a field
    /// read (`self.speed`), as written in the function, found among equal expressions by `nth`
    /// and `ctx`.
    pub expr: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    Before,
    After,
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
    ModifyLocal,
    Field,
    Expression,
}

impl Kind {
    const ALL: [Self; 11] = [
        Self::Head,
        Self::Return,
        Self::Replace,
        Self::BeforeCall,
        Self::AfterCall,
        Self::ModifyValue,
        Self::WrapCall,
        Self::WrapIf,
        Self::ModifyLocal,
        Self::Field,
        Self::Expression,
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
            Self::ModifyLocal => "modify_local",
            Self::Field => "field",
            Self::Expression => "expression",
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// A removed spelling is refused naming the form that replaced it; it is never read as that form.
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
            other => format!(
                "unknown kind `{other}`, expected one of {}",
                Self::ALL.map(Self::name).join(", ")
            ),
        }))
    }
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
            Self::Head | Self::Replace => SlotKind::Head,
            Self::Return => SlotKind::Return,
            Self::BeforeCall
            | Self::AfterCall
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
            for (field, present) in [
                ("local", entry.local.is_some()),
                ("position", entry.position.is_some()),
            ] {
                if present != (entry.kind == Kind::ModifyLocal) {
                    return Err(invalid(
                        field,
                        "local and position are required only for modify_local",
                    ));
                }
            }
            if entry.local_type.is_some()
                != matches!(
                    entry.kind,
                    Kind::ModifyLocal | Kind::Expression | Kind::Field
                )
            {
                return Err(invalid(
                    "type",
                    "type is required only for modify_local, expression and field",
                ));
            }
            if entry.expr.is_some() != (entry.kind == Kind::Expression)
                || entry
                    .expr
                    .as_ref()
                    .is_some_and(|expr| expr.trim().is_empty())
            {
                return Err(invalid(
                    "expr",
                    "expression needs expr, the expression as written; other kinds take none",
                ));
            }
            if entry.local.as_ref().is_some_and(|name| {
                name.is_empty()
                    || RESERVED.split_whitespace().any(|word| word == name)
                    || name.as_bytes()[0].is_ascii_digit()
                    || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }) {
                return Err(invalid("local", "name one local binding"));
            }
            if entry
                .local_type
                .as_ref()
                .is_some_and(|ty| ty.trim().is_empty())
            {
                return Err(invalid("type", "name the Rust type"));
            }
            let call = entry.call.as_ref().is_some_and(|s| !s.trim().is_empty());
            let ctx = entry.ctx.as_ref().is_some_and(|s| !s.trim().is_empty());
            if if entry.kind == Kind::Expression {
                entry.call.is_some() || entry.nth.is_none() || !ctx
            } else if entry.kind.needs_call() {
                !call || entry.nth.is_none() || !ctx
            } else {
                entry.call.is_some() || entry.nth.is_some() || entry.ctx.is_some()
            } {
                return Err(invalid(
                    "call/nth/ctx",
                    "calls/locals only: all three; expression: nth and ctx",
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
    WasmSitesUnsupported,
    LocalSourceOnly,
    FieldRequiresSource,
    ExpressionSourceOnly,
    ApiSitesUnsupported,
    SourcePending,
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
            Self::LocalSourceOnly => "modify_local requires native internals source rebuilding; use internals, or request a hook",
            Self::FieldRequiresSource => "field requires native internals source rebuild; use attachments for API, wasm, persistence or sync",
            Self::ExpressionSourceOnly => "expression requires native internals source rebuilding; use internals, or request a hook",
            Self::ApiSitesUnsupported => "API call-site injections require internals",
            Self::SourcePending => "source injects pending (U5)",
            Self::NoApiRecord => "no API record: a parameter has no API type yet",
            Self::ReplaceWithoutValue => "replace needs a return its record carries",
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
                "{id}: {reason}; slot declaration {declaration}; access = \"api\" never rebuilds the game; use access = \"internals\" where supported or request a hook"
            ),
        }
    }
}
impl std::error::Error for InjectError {}

/// Route a validated entry against a resolved catalog, without I/O or source inspection.
/// Freshness failures never fall back to source. Wasm call-site injections are permanently unsupported.
pub fn route(entry: Entry<'_>, slots: &[Place]) -> Result<Route, InjectError> {
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
    if inject.kind == Kind::Field
        && inject.access == Access::Internals
        && entry.target == Target::Native
    {
        return Ok(Route::Source(NoSlotReason::FieldRequiresSource));
    }
    if let Some(reason) = match inject.kind {
        Kind::ModifyLocal
            if entry.target != Target::Native || inject.access != Access::Internals =>
        {
            Some(NoSlotReason::LocalSourceOnly)
        }
        Kind::Field => Some(NoSlotReason::FieldRequiresSource),
        Kind::Expression
            if entry.target != Target::Native || inject.access != Access::Internals =>
        {
            Some(NoSlotReason::ExpressionSourceOnly)
        }
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
    if inject.kind == Kind::ModifyLocal {
        return Ok(Route::Source(NoSlotReason::LocalSourceOnly));
    }
    // A constant or field read has no compiled slot: native internals rebuild the game.
    if inject.kind == Kind::Expression {
        return Ok(Route::Source(NoSlotReason::ExpressionSourceOnly));
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
        NoSlotReason::WasmSitesUnsupported
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
            (Kind::Head, Some(Head)),
            (Kind::Return, Some(Return)),
            (Kind::Replace, Some(Head)),
            (Kind::BeforeCall, Some(Site)),
            (Kind::AfterCall, Some(Site)),
            (Kind::ModifyValue, Some(Site)),
            (Kind::WrapCall, Some(Site)),
            (Kind::WrapIf, Some(Site)),
            (Kind::ModifyLocal, Some(Local)),
            (Kind::Field, Some(Extension)),
            (Kind::Expression, None),
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
                    let source_only = match kind {
                        Kind::ModifyLocal => Some(NoSlotReason::LocalSourceOnly),
                        Kind::Expression => Some(NoSlotReason::ExpressionSourceOnly),
                        _ => None,
                    };
                    if let Some(only) = source_only.filter(|_| access == Access::Internals) {
                        assert_eq!(
                            run(&i, Target::Native, &[p.clone()]),
                            Ok(Route::Source(only))
                        );
                        assert!(matches!(
                            run(&i, Target::Wasm, &[p]),
                            Err(InjectError::NoSlot { reason, .. }) if reason == only
                        ));
                        continue;
                    }
                    if let Some(pending) = match kind {
                        Kind::ModifyLocal => Some(NoSlotReason::LocalSourceOnly),
                        Kind::Field => Some(NoSlotReason::FieldRequiresSource),
                        Kind::Expression => Some(NoSlotReason::ExpressionSourceOnly),
                        _ => None,
                    } {
                        for target in [Target::Native, Target::Wasm] {
                            if kind == Kind::Field
                                && access == Access::Internals
                                && target == Target::Native
                            {
                                assert_eq!(
                                    run(&i, target, &[p.clone()]),
                                    Ok(Route::Source(NoSlotReason::FieldRequiresSource))
                                );
                                continue;
                            }
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
                        Some(NoSlotReason::WasmSitesUnsupported)
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
    fn modify_local_manifest_fields_and_permanent_api_refusal() {
        let native = format!(
            "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'",
            "a".repeat(40),
            "a".repeat(40)
        );
        let decl = DECL.replace("kind='head'", "kind='modify_local'\naccess='internals'\ncall='sink'\nnth=0\nctx='context'\nlocal='n'\ntype='i32'\nposition='before'");
        let m = parse(&decl, &native).unwrap();
        assert_eq!(m.inject[0].position, Some(Position::Before));
        assert_eq!(m.inject[0].local.as_deref(), Some("n"));
        assert_eq!(m.inject[0].local_type.as_deref(), Some("i32"));
        assert!(parse(&decl.replace("'before'", "'after'"), &native).is_ok());
        assert!(parse(&decl.replace("'before'", "'during'"), &native).is_err());
        for (field, line) in [
            ("local", "local='n'\n"),
            ("type", "type='i32'\n"),
            ("position", "position='before'\n"),
        ] {
            assert!(
                parse(&decl.replace(line, ""), &native)
                    .unwrap_err()
                    .contains(field)
            );
            let extra = format!("{DECL}{line}");
            assert!(parse(&extra, &native).unwrap_err().contains(field));
        }
        for kind in [
            Kind::Head,
            Kind::Return,
            Kind::Replace,
            Kind::BeforeCall,
            Kind::AfterCall,
            Kind::ModifyValue,
            Kind::WrapCall,
            Kind::WrapIf,
            Kind::Expression,
        ] {
            for field in ["local", "type", "position"] {
                if kind == Kind::Expression && field == "type" {
                    continue;
                }
                let mut other = m.inject[0].clone();
                other.kind = kind;
                other.local = (field == "local").then(|| "n".into());
                other.local_type = (field == "type").then(|| "i32".into());
                other.position = (field == "position").then_some(Position::Before);
                assert!(
                    matches!(Inject::check_all(&[other], "sample", true), Err(InjectError::Invalid { field: actual, .. }) if actual == field)
                );
            }
        }
        let mut i = m.inject[0].clone();
        i.at = "game::tick".into();
        i.hash = Some("signature".into());
        i.access = Access::Api;
        assert_eq!(
            run(&i, Target::Native, &[place()]).unwrap_err().to_string(),
            "E_INJECT_NO_SLOT: sample:ground: modify_local requires native internals source rebuilding; use internals, or request a hook; slot declaration slots/sample.txt: game::tick; access = \"api\" never rebuilds the game; use access = \"internals\" where supported or request a hook"
        );
    }

    #[test]
    fn expression_takes_expr_type_nth_and_ctx_and_only_native_internals() {
        let native = format!(
            "[native]\ngame='26.3'\nhorizon='{0}'\nmixins='{0}'",
            "a".repeat(40)
        );
        let decl = DECL.replace(
            "kind='head'",
            "kind='expression'\naccess='internals'\nexpr='64.0'\ntype='f64'\nnth=1\nctx='context'",
        );
        let m = parse(&decl, &native).unwrap();
        let i = &m.inject[0];
        assert_eq!(
            (
                i.expr.as_deref(),
                i.local_type.as_deref(),
                i.nth,
                i.call.as_deref()
            ),
            (Some("64.0"), Some("f64"), Some(1), None)
        );
        for (field, without, with) in [
            ("expr", "expr='64.0'\n", "expr=''\n"),
            ("type", "type='f64'\n", "type='f64'\nlocal='n'\n"),
            ("call/nth/ctx", "nth=1\n", "nth=1\ncall='sink'\n"),
            ("call/nth/ctx", "ctx='context'\n", "ctx='context'\n"),
        ] {
            assert!(
                parse(&decl.replace(without, ""), &native)
                    .unwrap_err()
                    .contains(field),
                "{field}"
            );
            if with != without {
                assert!(
                    parse(&decl.replace(without, with), &native).is_err(),
                    "{with}"
                );
            }
        }
        // Another kind with `expr` is refused naming it.
        assert!(
            parse(&format!("{DECL}expr='64.0'\n"), &native)
                .unwrap_err()
                .contains("expr")
        );
        let mut api = m.inject[0].clone();
        (api.at, api.hash, api.access) =
            ("game::tick".into(), Some("signature".into()), Access::Api);
        assert!(run(&api, Target::Native, &[place()]).unwrap_err().to_string().contains(
            "expression requires native internals source rebuilding; use internals, or request a hook"
        ));
    }

    #[test]
    fn wasm_call_site_refusal_is_permanent() {
        let i = inject(Kind::BeforeCall);
        let error = run(&i, Target::Wasm, &[place()]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "E_INJECT_NO_SLOT: sample:ground: wasm does not support internals or call-site injections; use a hook; slot declaration slots/sample.txt: game::tick | self.callee | 0 | context; access = \"api\" never rebuilds the game; use access = \"internals\" where supported or request a hook"
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
            assert_eq!(
                run(&i, target, &[p.clone()]),
                Ok(Route::Slot(SlotKind::Head))
            );
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
    #[test]
    fn field_manifest_and_permanent_refusals() {
        let native = format!(
            "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'",
            "a".repeat(40),
            "a".repeat(40)
        );
        let decl = DECL.replace(
            "kind='head'",
            "kind='field'\ntype='u32'\naccess='internals'",
        );
        assert!(parse(&decl, &native).is_ok());
        let wasm = format!("[wasm]\nsha512='{}'\ntoolchain='nightly'", "a".repeat(128));
        assert_eq!(
            parse(&decl, &wasm).unwrap_err(),
            "E_INJECT_NO_SLOT: sample:ground: field requires native internals source rebuild; use attachments for API, wasm, persistence or sync; slot declaration slots/sample.txt: type game::tick; access = \"api\" never rebuilds the game; use access = \"internals\" where supported or request a hook"
        );

        assert!(
            parse(&decl.replace("type='u32'", ""), &native)
                .unwrap_err()
                .contains("type")
        );
        assert!(
            parse(
                &DECL.replace("kind='head'", "kind='head'\ntype='u32'"),
                &native
            )
            .unwrap_err()
            .contains("type")
        );
        let mut i = inject(Kind::Field);
        i.at = "game::S".into();
        i.local_type = Some("u32".into());
        let mut p = place();
        p.at = "game::S".into();
        let expected = "E_INJECT_NO_SLOT: sample:ground: field requires native internals source rebuild; use attachments for API, wasm, persistence or sync; slot declaration slots/sample.txt: type game::S; access = \"api\" never rebuilds the game; use access = \"internals\" where supported or request a hook";
        for target in [Target::Native, Target::Wasm] {
            assert_eq!(
                run(&i, target, &[p.clone()]).unwrap_err().to_string(),
                expected
            );
        }
        i.access = Access::Internals;
        assert_eq!(
            run(&i, Target::Wasm, &[p.clone()]).unwrap_err().to_string(),
            expected
        );
        assert_eq!(
            run(&i, Target::Native, &[p]),
            Ok(Route::Source(NoSlotReason::FieldRequiresSource))
        );
    }
    #[test]
    fn kind_specific_fields_share_one_type_key() {
        for kind in [
            Kind::Head,
            Kind::Return,
            Kind::Replace,
            Kind::BeforeCall,
            Kind::AfterCall,
            Kind::ModifyValue,
            Kind::WrapCall,
            Kind::WrapIf,
            Kind::ModifyLocal,
            Kind::Field,
            Kind::Expression,
        ] {
            let mut valid = inject(kind);
            valid.local_type = matches!(kind, Kind::ModifyLocal | Kind::Field | Kind::Expression)
                .then(|| "u32".into());
            valid.local = (kind == Kind::ModifyLocal).then(|| "n".into());
            valid.position = (kind == Kind::ModifyLocal).then_some(Position::Before);
            if kind == Kind::Expression {
                valid.expr = Some("1u32".into());
                valid.nth = Some(0);
                valid.ctx = Some("context".into());
            }
            Inject::check_all(&[valid.clone()], "sample", true).unwrap();
            for field in ["local", "type", "position"] {
                let mut invalid = valid.clone();
                match field {
                    "local" => {
                        invalid.local = if valid.local.is_some() {
                            None
                        } else {
                            Some("n".into())
                        }
                    }
                    "type" => {
                        invalid.local_type = if valid.local_type.is_some() {
                            None
                        } else {
                            Some("u32".into())
                        }
                    }
                    _ => {
                        invalid.position = if valid.position.is_some() {
                            None
                        } else {
                            Some(Position::Before)
                        }
                    }
                }
                assert!(
                    matches!(Inject::check_all(&[invalid], "sample", true), Err(InjectError::Invalid { field: actual, .. }) if actual == field),
                    "{kind:?}: {field}"
                );
            }
            if valid.local_type.is_some() {
                let serialized = toml::to_string(&valid).unwrap();
                assert!(serialized.lines().any(|line| line == "type = \"u32\""));
                assert!(!serialized.contains("field_type") && !serialized.contains("local_type"));
                let mut empty = valid;
                empty.local_type = Some(" ".into());
                assert!(matches!(
                    Inject::check_all(&[empty], "sample", true),
                    Err(InjectError::Invalid { field: "type", .. })
                ));
            }
        }
        let native = format!(
            "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'",
            "a".repeat(40),
            "a".repeat(40)
        );
        let field = DECL.replace("kind='head'", "kind='field'\ntype='u32'");
        assert!(
            parse(&format!("{field}field_type='u32'\n"), &native)
                .unwrap_err()
                .contains("field_type")
        );
        let local = DECL.replace("id='ground'", "id='local'").replace("kind='head'", "kind='modify_local'\nlocal='n'\ntype='i32'\nposition='after'\ncall='sink'\nnth=0\nctx='context'");
        assert_eq!(
            parse(&format!("{field}{local}"), &native)
                .unwrap()
                .inject
                .len(),
            2
        );
    }
}
