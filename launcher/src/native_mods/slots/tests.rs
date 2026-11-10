use super::super::api;
use super::*;

fn fixture() -> (Manifest, Catalog) {
    let pin = "a".repeat(40);
    let manifest = Manifest::parse(&format!("id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n")).unwrap();
    let catalog = Catalog {
        wire: Default::default(),
        mods: Vec::new(),
        internals: serde_json::from_value(serde_json::json!([{
            "path":"game::run", "kind":"fn", "signature":"fn run()", "hash":"signature", "cfg":"", "shape_only":false
        }])).unwrap(),
        points: serde_json::from_value(serde_json::json!([{
            "name":"run", "target":"game::run", "raw":true, "at_return":true,
            "returns":"cancel", "host":"none", "return_ty":"()",
            "raw_shape":{"return_ty":"()", "outcome":"cancel", "receiver":{"kind":"none"}},
            "listener":"game::__RunListener", "return_listener":"game::__RunReturnListener"
        }])).unwrap(),
        sites: serde_json::from_value(serde_json::json!([{
            "name":"Sone", "target":"game::run", "selector":"self.step", "ordinal":0,
            "call":"game::step", "ctx":"context", "hash":"place", "listener":"game::__SoneListener", "record":"game::__SoneArgs", "output":"()"
        }])).unwrap(),
        discovery: false,
    };
    (manifest, catalog)
}

#[test]
fn api_head_without_prebuilt_slot_is_refused() {
    let (mut manifest, mut catalog) = fixture();
    catalog.points.clear();
    manifest.inject.push(Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "missing".into(),
        at: "game::run".into(),
        kind: Kind::Head,
        with: "listen".into(),
        access: Access::Api,
        priority: 0,
        hash: Some("signature".into()),
        call: None,
        nth: None,
        ctx: None,
    });
    let error = check(&catalog, &manifest).unwrap_err().to_string();
    assert!(error.contains("example:missing"));
    assert!(error.contains("game::run"));
    assert!(error.contains("access = \"api\" never rebuilds the game"));
    assert!(error.contains("request a hook"));
}

#[test]
fn every_native_kind_and_access_registers_or_refuses_by_name() {
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
        for access in [Access::Api, Access::Internals] {
            let (mut manifest, catalog) = fixture();
            let call = matches!(
                kind,
                Kind::BeforeCall
                    | Kind::AfterCall
                    | Kind::ModifyValue
                    | Kind::WrapCall
                    | Kind::WrapIf
                    | Kind::ModifyLocal
            );
            manifest.inject.push(Inject {
                local: None,
                local_type: (kind == Kind::Field).then(|| "u32".into()),
                position: None,
                expr: None,
                id: "test".into(),
                at: "game::run".into(),
                kind,
                with: "listen".into(),
                access,
                priority: 7,
                hash: Some("signature".into()),
                call: call.then(|| "game::step".into()),
                nth: call.then_some(0),
                ctx: call.then(|| "context".into()),
            });
            let source_only = match kind {
                Kind::ModifyLocal => Some(NoSlotReason::LocalSourceOnly),
                Kind::Expression => Some(NoSlotReason::ExpressionSourceOnly),
                _ => None,
            };
            if let Some(only) = source_only.filter(|_| access == Access::Internals) {
                assert_eq!(
                    routes(&catalog, &manifest).unwrap(),
                    vec![Route::Source(only)]
                );
                continue;
            }
            let pending = match (kind, access) {
                (Kind::ModifyLocal, Access::Api) => Some(NoSlotReason::LocalSourceOnly),
                (Kind::Field, Access::Api) => Some(NoSlotReason::FieldRequiresSource),
                (Kind::Field, Access::Internals) => None,
                (Kind::Expression, Access::Api) => Some(NoSlotReason::ExpressionSourceOnly),
                (_, Access::Api) if call => Some(NoSlotReason::ApiSitesUnsupported),
                _ => None,
            };
            if let Some(expected) = pending {
                assert!(
                    matches!(check(&catalog, &manifest), Err(InjectError::NoSlot { reason, .. }) if reason == expected),
                    "{kind:?} {access:?}"
                );
                continue;
            }
            check(&catalog, &manifest).unwrap();
            if kind == Kind::Field {
                let facade = catalog.facade(&manifest).unwrap();
                let main = api::main_rs("/* MOD_SLOTS */", &[&manifest], &catalog);
                assert!(!facade.source.contains("FieldCall"));
                assert!(!facade.source.contains("fn listen"));
                assert!(!main.contains("mod_0::listen"));
                assert!(points(&catalog.points, &manifest).next().is_none());
                assert!(report(&catalog, &manifest)
                    .next()
                    .unwrap()
                    .contains("source"));
                continue;
            }
            let facade = catalog.facade(&manifest).unwrap();
            let main = api::main_rs("/* MOD_SLOTS */", &[&manifest], &catalog);
            if access == Access::Api {
                assert!(
                    main.contains(match kind {
                        Kind::Return =>
                            "mod_points::listeners::run_return(\"example\", 7, \"test\",",
                        Kind::Replace =>
                            "mod_points::replace!(mod_points::listeners::run, cancel, none, \"example\", 7, \"test\",",
                        _ => "mod_points::listeners::run(\"example\", 7, \"test\",",
                    }),
                    "{main}"
                );
                assert!(main.contains("mod_0::listen"), "{main}");
                continue;
            }
            assert_eq!(main.matches("mod_0_internals::raw::").count(), 1, "{main}");
            assert!(main.contains("mod_0::listen"), "{main}");
            assert!(
                main.contains(if kind == Kind::Replace {
                    "mod_points::replace!(internals mod_0_internals::raw::run, cancel, none, \"example\", 7, \"test\", mod_0::listen)"
                } else if call {
                    "(\"example\", 7, \"test\","
                } else {
                    ".ordered(7, \"test\")"
                }),
                "{main}"
            );
            assert!(facade.source.contains(if call {
                "pub const fn s"
            } else if kind == Kind::Return {
                "pub const fn run_return"
            } else {
                "pub const fn run"
            }));
            let reports = report(&catalog, &manifest).collect::<Vec<_>>();
            assert_eq!(reports.len(), 1);
            assert!(reports[0].contains("example:test: game::run -> slot"));
        }
    }
}

#[test]
fn an_api_listener_at_a_point_without_an_api_record_is_refused_naming_the_type() {
    let (mut manifest, mut catalog) = fixture();
    catalog.points[0].api_refusal =
        Some("argument 1 `batch` (`&mut HudBatch`) has no API type yet".into());
    let inject = |access| Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "test".into(),
        at: "game::run".into(),
        kind: Kind::Head,
        with: "listen".into(),
        access,
        priority: 0,
        hash: Some("signature".into()),
        call: None,
        nth: None,
        ctx: None,
    };
    manifest.inject.push(inject(Access::Api));
    match check(&catalog, &manifest) {
        Err(
            error @ InjectError::NoSlot {
                reason: NoSlotReason::NoApiRecord,
                ..
            },
        ) => assert!(
            error.to_string().contains("`batch` (`&mut HudBatch`)"),
            "{error}"
        ),
        other => panic!("{other:?}"),
    }
    // The native internals tier still listens there.
    manifest.inject[0] = inject(Access::Internals);
    check(&catalog, &manifest).unwrap();
}

#[test]
fn a_replace_at_a_point_whose_return_has_no_api_form_is_refused_naming_it() {
    let (mut manifest, mut catalog) = fixture();
    catalog.points[0].returns = "none".into();
    catalog.points[0].raw_shape = Some(horizon_mod_point_emit::RawReturn {
        return_ty: "Frame<'_>".into(),
        ..Default::default()
    });
    manifest.inject.push(Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "own".into(),
        at: "game::run".into(),
        kind: Kind::Replace,
        with: "listen".into(),
        access: Access::Api,
        priority: 0,
        hash: Some("signature".into()),
        call: None,
        nth: None,
        ctx: None,
    });
    match check(&catalog, &manifest) {
        Err(
            error @ InjectError::NoSlot {
                reason: NoSlotReason::ReplaceWithoutValue,
                ..
            },
        ) => {
            let text = error.to_string();
            assert!(
                text.contains("game::run") && text.contains("`Frame<'_>`"),
                "{text}"
            );
        }
        other => panic!("{other:?}"),
    }
    // A `Result<(), E>` point cancels with `Ok(())`, so a replace there is refused by its type.
    catalog.points[0].returns = "cancel".into();
    catalog.points[0].return_ty = Some("Result<(),StoreError>".into());
    match check(&catalog, &manifest) {
        Err(
            error @ InjectError::NoSlot {
                reason: NoSlotReason::ReplaceWithoutValue,
                ..
            },
        ) => assert!(
            error.to_string().contains("`Result<(),StoreError>`"),
            "{error}"
        ),
        other => panic!("{other:?}"),
    }
    // A head listener at the same point is unaffected: it need not produce the value.
    manifest.inject[0].kind = Kind::Head;
    check(&catalog, &manifest).unwrap();
}

#[test]
fn an_internals_replace_needs_a_listed_point_whose_raw_record_carries_the_outcome() {
    let (mut manifest, mut catalog) = fixture();
    manifest.inject.push(Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "own".into(),
        at: "game::run".into(),
        kind: Kind::Replace,
        with: "listen".into(),
        access: Access::Internals,
        priority: 0,
        hash: Some("signature".into()),
        call: None,
        nth: None,
        ctx: None,
    });
    check(&catalog, &manifest).unwrap();
    let refused = |catalog: &Catalog, manifest: &Manifest, reason: NoSlotReason| match check(
        catalog, manifest,
    ) {
        Err(error @ InjectError::NoSlot { reason: actual, .. }) if actual == reason => {
            error.to_string()
        }
        other => panic!("{other:?}"),
    };
    // A `Result<(), E>` raw cancel answers `Ok(())`; a borrowed return has no raw `result`.
    for (outcome, ty) in [
        ("cancel", "Result < (), StoreError >"),
        ("none", "&mut usize"),
    ] {
        let raw = catalog.points[0].raw_shape.as_mut().unwrap();
        raw.outcome = outcome.into();
        raw.return_ty = ty.into();
        let text = refused(&catalog, &manifest, NoSlotReason::ReplaceWithoutValue);
        assert!(
            text.contains("example:own") && text.contains(&format!("`{ty}`")),
            "{text}"
        );
    }
    // An unlisted function takes no internals replace on the source route: the refusal names the
    // slot row that lists it.
    catalog.points.clear();
    let text = refused(&catalog, &manifest, NoSlotReason::NotListed);
    assert!(text.contains("slots/example.txt: game::run"), "{text}");
}

#[test]
fn unified_route_checks_freshness_and_selects_native_function_source() {
    let (mut manifest, mut catalog) = fixture();
    manifest.inject.push(Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "test".into(),
        at: "game::run".into(),
        kind: Kind::WrapIf,
        with: "listen".into(),
        access: Access::Internals,
        priority: 0,
        hash: Some("signature".into()),
        call: Some("game::step".into()),
        nth: Some(0),
        ctx: Some("context".into()),
    });
    catalog.sites[0].output = "u32".into();
    assert!(matches!(
        check(&catalog, &manifest),
        Err(InjectError::Invalid { field: "kind", .. })
    ));
    manifest.inject[0].hash = Some("stale".into());
    assert!(matches!(
        check(&catalog, &manifest),
        Err(InjectError::Changed { .. })
    ));
    manifest.inject[0].hash = Some("signature".into());
    manifest.inject[0].call = None;
    manifest.inject[0].kind = Kind::Head;
    catalog.points.clear();
    check(&catalog, &manifest).unwrap();
    assert!(report(&catalog, &manifest)
        .next()
        .unwrap()
        .contains("source (private slot"));
    manifest.inject[0].kind = Kind::Expression;
    assert_eq!(
        routes(&catalog, &manifest).unwrap(),
        vec![Route::Source(NoSlotReason::ExpressionSourceOnly)]
    );
    manifest.inject[0].kind = Kind::Head;
    catalog.internals.clear();
    assert!(matches!(
        check(&catalog, &manifest),
        Err(InjectError::Missing { .. })
    ));
}

#[test]
fn unstaged_source_value_before_listed_wrap_if_does_not_panic() {
    let (mut manifest, catalog) = fixture();
    let mut entry = Inject {
        local: None,
        local_type: None,
        position: None,
        expr: None,
        id: "value".into(),
        at: "game::run".into(),
        kind: Kind::ModifyValue,
        with: "listen".into(),
        access: Access::Internals,
        priority: 7,
        hash: Some("signature".into()),
        call: Some("game::other".into()),
        nth: Some(0),
        ctx: Some("other-statement".into()),
    };
    manifest.inject.push(entry.clone());
    entry.id = "condition".into();
    entry.kind = Kind::WrapIf;
    entry.call = Some("game::step".into());
    entry.ctx = Some("context".into());
    manifest.inject.push(entry);
    assert!(matches!(
        routes(&catalog, &manifest).unwrap().as_slice(),
        [Route::Source(_), Route::Slot(SlotKind::Site)]
    ));
}
