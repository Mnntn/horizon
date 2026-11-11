use super::super::{Access, Kind};
use super::*;

fn catalog() -> Catalog {
    let point = |name: &str, target: &str, refusal: Option<&str>| Point {
        name: name.into(),
        target: target.into(),
        hash: format!("{name:0>16}").chars().take(16).collect(),
        signature: format!("{target}: fn f()"),
        raw: true,
        at_return: true,
        api_refusal: refusal.map(str::to_owned),
        returns: "cancel".into(),
        host: "none".into(),
        return_ty: "()".into(),
        listener: Some(format!("crate::__{name}Listener")),
        return_listener: Some(format!("crate::__{name}ReturnListener")),
    };
    Catalog {
        krate: "chemlib".into(),
        table: "__horizon_points".into(),
        opted_out: vec!["chemlib::secret".into()],
        points: vec![
            point("transfer", "chemlib::transfer", None),
            point(
                "opaque",
                "chemlib::opaque",
                Some("argument 1 `value` (`Opaque`) has no API type yet"),
            ),
        ],
    }
}

fn inject(at: &str, kind: Kind, access: Access, hash: &str) -> Inject {
    Inject {
        id: "listen".into(),
        at: at.into(),
        kind,
        with: "listener".into(),
        access,
        priority: 0,
        hash: Some(hash.into()),
        call: None,
        nth: None,
        ctx: None,
    }
}

fn library(catalog: &Catalog) -> Library<'_> {
    Library {
        id: "chemlib",
        version: "1.2.0",
        catalog,
    }
}

#[test]
fn an_injection_routes_to_its_place_by_access_and_kind() {
    let catalog = catalog();
    let same = library(&catalog);
    let hash = &catalog.points[0].hash;
    for (kind, access, place) in [
        (Kind::Head, Access::Api, "transfer"),
        (Kind::Replace, Access::Api, "transfer"),
        (Kind::Return, Access::Api, "transfer_return"),
        (Kind::Head, Access::Internals, "transfer"),
        (Kind::Return, Access::Internals, "transfer_return"),
    ] {
        let routed = route(
            "addon",
            &inject("chemlib::transfer", kind, access, hash),
            &same,
        )
        .unwrap();
        assert_eq!((routed.place.as_str(), routed.access), (place, access));
    }
    assert!(catalog.owns("chemlib::tanks::fill"));
    assert!(!catalog.owns("chemlibx::fill"));
}

#[test]
fn each_refusal_names_what_changed_or_why() {
    let catalog = catalog();
    let same = library(&catalog);
    let hash = catalog.points[0].hash.clone();
    let opaque = catalog.points[1].hash.clone();
    let refused = |inject: Inject, library: &Library<'_>| {
        route("addon", &inject, library).unwrap_err().to_string()
    };
    assert_eq!(
        refused(
            inject(
                "chemlib::transfer",
                Kind::Head,
                Access::Api,
                "0000000000000000"
            ),
            &same
        ),
        format!(
            "E_INJECT_CHANGED: addon:listen: chemlib 1.2.0 changed chemlib::transfer: pinned \
             0000000000000000, installed {hash} (chemlib::transfer: fn f()); update the \
             declaration with `cargo horizon-mod inject update` and rebuild against it"
        )
    );
    assert_eq!(
        refused(
            inject("chemlib::secret", Kind::Head, Access::Api, &hash),
            &same
        ),
        "E_INJECT_MISSING: addon:listen: chemlib::secret has no point (opted out with #[mod_api::no_point])"
    );
    assert!(
        refused(
            inject("chemlib::gone", Kind::Head, Access::Api, &hash),
            &same
        )
        .starts_with("E_INJECT_MISSING")
    );
    assert!(
        refused(
            inject("chemlib::opaque", Kind::Head, Access::Api, &opaque),
            &same
        )
        .contains("argument 1 `value` (`Opaque`) has no API type yet")
    );
    // The opaque point keeps its internals form on the same target.
    assert!(
        route(
            "addon",
            &inject("chemlib::opaque", Kind::Head, Access::Internals, &opaque),
            &same
        )
        .is_ok()
    );
    let mut call = inject("chemlib::transfer", Kind::BeforeCall, Access::Api, &hash);
    call.call = Some("chemlib::helper".into());
    assert!(refused(call, &same).starts_with("E_INJECT_NO_SLOT"));
    // Nothing records another mod's hash, so the declaration must carry it.
    let mut unpinned = inject("chemlib::transfer", Kind::Head, Access::Api, &hash);
    unpinned.hash = None;
    let unpinned = refused(unpinned, &same);
    assert!(
        unpinned.contains("addon:listen")
            && unpinned.contains("hash")
            && unpinned.contains("inject add"),
        "{unpinned}"
    );
    // Routed as a head, an internals replace would run beside the body instead of replacing it.
    let replace = refused(
        inject("chemlib::transfer", Kind::Replace, Access::Internals, &hash),
        &same,
    );
    assert!(
        replace.starts_with("E_INJECT_NO_SLOT")
            && replace.contains("addon:listen")
            && replace.contains("api replace only"),
        "{replace}"
    );
}

#[test]
fn across_targets_internals_and_unwired_records_are_refused_by_reason() {
    let catalog = catalog();
    let (hash, opaque) = (&catalog.points[0].hash, &catalog.points[1].hash);
    let refused = |at: &str, access: Access, hash: &str| {
        cross_target(
            "addon",
            &inject(at, Kind::Head, access, hash),
            "chemlib",
            &catalog,
        )
        .map(|refusal| refusal.to_string())
    };
    // Internals reach the library's own types: refused across targets, whatever the record.
    assert_eq!(
        refused("chemlib::transfer", Access::Internals, hash).as_deref(),
        Some(
            "addon:listen: internals access to chemlib needs the same target: the same binary \
             on native, the same dependency group on wasm"
        )
    );
    assert_eq!(
        refused("chemlib::opaque", Access::Api, opaque).as_deref(),
        Some(
            "addon:listen: an api listener at chemlib on the other target crosses through the \
             game, and argument 1 `value` (`Opaque`) has no API type yet: add \
             #[derive(mod_api::Wire)] to it"
        )
    );
    // A record that could cross, or no such point: the general mixed-target refusal applies.
    assert_eq!(refused("chemlib::transfer", Access::Api, hash), None);
    assert_eq!(refused("chemlib::gone", Access::Internals, hash), None);
}

#[test]
fn a_package_catalog_is_read_bounded_and_strict() {
    let json = serde_json::to_vec(&catalog()).unwrap();
    assert_eq!(Catalog::read(json.as_slice()).unwrap(), catalog());
    assert!(
        Catalog::read(b"{".as_slice())
            .unwrap_err()
            .starts_with("mod-points.json")
    );
    assert!(
        Catalog::read(br#"{"crate":"c","table":"t","points":[],"extra":1}"#.as_slice())
            .unwrap_err()
            .contains("extra")
    );
    let huge = vec![b' '; (8 << 20) + 1];
    assert!(read_bytes(huge.as_slice()).unwrap_err().contains("exceeds"));
}

#[test]
fn a_library_whose_crate_is_not_named_after_its_id_is_refused() {
    let mut catalog = catalog();
    catalog.krate = "chem_core".into();
    for point in &mut catalog.points {
        point.target = point.target.replace("chemlib::", "chem_core::");
    }
    let hash = catalog.points[0].hash.clone();
    for access in [Access::Api, Access::Internals] {
        let injection = inject("chem_core::transfer", Kind::Head, access, &hash);
        assert_eq!(
            route("addon", &injection, &library(&catalog))
                .unwrap_err()
                .to_string(),
            "addon:listen: mod point library chemlib's crate is chem_core; a mod point library's \
             crate is named after its mod id: chemlib"
        );
    }
}
