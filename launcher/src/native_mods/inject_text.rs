//! The launcher's words for an inject refusal: the package crate's `Display` is the
//! command-line form, this is the translated one players see.
use crate::i18n::{tr, trf};
use horizon_mod_package::manifest::inject::{InjectError, NoSlotReason};

pub(super) fn inject_error(error: &InjectError) -> String {
    match error {
        InjectError::Invalid { id, field, fix } => trf(
            "E_INJECT_INVALID: {id}: invalid {field}; {fix}",
            &[("id", id), ("field", field), ("fix", fix)],
        ),
        InjectError::Missing { id, at } => trf(
            "E_INJECT_MISSING: {id}: {at} gone; rerun inject add",
            &[("id", id), ("at", at)],
        ),
        InjectError::Changed { id, at, old, new } => trf(
            "E_INJECT_CHANGED: {id}: {at} {old} -> {new}; run inject update",
            &[("id", id), ("at", at), ("old", old), ("new", new)],
        ),
        InjectError::Moved { id, nth } => trf(
            "E_INJECT_MOVED: {id} now at nth={nth}; run inject update",
            &[("id", id), ("nth", &nth.to_string())],
        ),
        InjectError::Lost { id, at } => trf(
            "E_INJECT_LOST: {id}: {at} lost or ambiguous; rerun inject add",
            &[("id", id), ("at", at)],
        ),
        InjectError::NoSlot {
            id,
            reason,
            declaration,
        } => trf(
            "E_INJECT_NO_SLOT: {id}: {reason}; slot declaration {declaration}; use a native mod or a game build supporting this slot",
            &[
                ("id", id),
                ("reason", no_slot_reason(*reason)),
                ("declaration", declaration),
            ],
        ),
    }
}

fn no_slot_reason(reason: NoSlotReason) -> &'static str {
    match reason {
        NoSlotReason::NotListed => tr("not listed"),
        NoSlotReason::KindHasNoSlot => tr("kind has no slot"),
        NoSlotReason::NativeOnly => tr("native-only place"),
        NoSlotReason::WasmSitesPending => tr("wasm sites pending"),
        NoSlotReason::LocalSlotsPending => tr("local slots pending (I5)"),
        NoSlotReason::ExtensionSlotsPending => tr("extension slots pending (I6)"),
        NoSlotReason::ApiSitesPending => tr("API site listeners pending (U4c)"),
        NoSlotReason::SourcePending => tr("source injects pending (U5)"),
        NoSlotReason::ReplacePending => tr("internals replace listeners pending (U9)"),
        NoSlotReason::NoApiRecord => tr("no API record: a parameter has no API type yet"),
        NoSlotReason::ReplaceWithoutValue => tr("replace needs a return the API carries"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_matches_the_package_form_for_every_reason() {
        let reasons = [
            NoSlotReason::NotListed,
            NoSlotReason::KindHasNoSlot,
            NoSlotReason::NativeOnly,
            NoSlotReason::WasmSitesPending,
            NoSlotReason::LocalSlotsPending,
            NoSlotReason::ExtensionSlotsPending,
            NoSlotReason::ApiSitesPending,
            NoSlotReason::SourcePending,
            NoSlotReason::ReplacePending,
            NoSlotReason::NoApiRecord,
            NoSlotReason::ReplaceWithoutValue,
        ];
        let mut errors: Vec<InjectError> = reasons
            .into_iter()
            .map(|reason| InjectError::NoSlot {
                id: "m:i".into(),
                reason,
                declaration: "a::b: why".into(),
            })
            .collect();
        errors.extend([
            InjectError::Invalid {
                id: "m:i".into(),
                field: "at",
                fix: "name a function",
            },
            InjectError::Missing {
                id: "m:i".into(),
                at: "a::b".into(),
            },
            InjectError::Changed {
                id: "m:i".into(),
                at: "a::b".into(),
                old: "1".into(),
                new: "2".into(),
            },
            InjectError::Moved {
                id: "m:i".into(),
                nth: 3,
            },
            InjectError::Lost {
                id: "m:i".into(),
                at: "a::b".into(),
            },
        ]);
        for error in &errors {
            assert_eq!(inject_error(error), error.to_string());
        }
    }
}
