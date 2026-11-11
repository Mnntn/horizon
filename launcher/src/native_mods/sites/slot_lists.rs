use horizon_mod_package::manifest::inject::{
    route, Access, CallPlace, Entry, Inject, InjectError, Kind, Place, Target,
};
use horizon_platform::mod_build::api_gen::points;
use std::{fs, path::Path};

#[test]
fn shipped_slot_list_is_accepted_by_the_reader() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let tracked = points::tracked(&root).unwrap();
    assert!(tracked.contains(&root.join("crates/horizon-mod-api-gen/slots/horizon.txt")));
}

#[test]
fn no_slot_suggestions_round_trip_through_the_slot_reader() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../target/slot-suggestions-{}", std::process::id()));
    let directory = root.join("crates/horizon-mod-api-gen/slots");
    let source = root.join("crates/test-slots/src/lib.rs");
    fs::create_dir_all(&directory).unwrap();
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(root.join("crates/horizon-client/src")).unwrap();
    fs::write(root.join("crates/horizon-client/src/game.rs"), "").unwrap();
    fs::write(root.join("crates/test-slots/Cargo.toml"), "").unwrap();
    fs::write(&source, "").unwrap();
    let mut declarations = String::new();
    for kind in [Kind::Head, Kind::BeforeCall] {
        let is_call = kind == Kind::BeforeCall;
        let inject = Inject {
            id: "test".into(),
            at: "test_slots::run".into(),
            kind,
            with: "on_slot".into(),
            access: Access::Api,
            priority: 0,
            hash: Some("signature".into()),
            call: is_call.then(|| "test_slots::callee".into()),
            nth: is_call.then_some(2),
            ctx: is_call.then(|| "context-identity".into()),
        };
        let place = Place {
            at: inject.at.clone(),
            hash: inject.hash.clone().unwrap(),
            slots: vec![],
            native_only: false,
            calls: vec![CallPlace {
                call: "test_slots::callee".into(),
                nth: 2,
                ctx: "context-identity".into(),
                slots: vec![],
            }],
        };
        let error = route(
            Entry {
                inject: &inject,
                target: Target::Wasm,
                mod_id: "test_mod",
                resolved_call: inject.call.as_deref(),
            },
            &[place],
        )
        .unwrap_err();
        assert!(error.to_string().starts_with("E_INJECT_NO_SLOT:"));
        let InjectError::NoSlot { declaration, .. } = error else {
            panic!("{error}")
        };
        declarations.push_str(declaration.strip_prefix("slots/test_mod.txt: ").unwrap());
        declarations.push('\n');
    }
    let list = directory.join("test_mod.txt");
    fs::write(&list, declarations).unwrap();
    // This public entry point invokes the shared slot parser for every declaration form.
    let tracked = points::tracked(&root).unwrap();
    assert!(tracked.contains(&list));
    assert!(tracked.contains(&source));
    fs::remove_dir_all(root).unwrap();
}
