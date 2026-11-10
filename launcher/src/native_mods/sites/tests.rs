use super::*;
use horizon_mod_package::Manifest;
use std::{fs, path::Path, process::Command};

#[path = "slot_lists.rs"]
mod slot_lists;

struct Case<'a> {
    kind: &'a str,
    body: &'a str,
    output: &'a str,
    initial: u32,
    expected: &'a str,
    state: u32,
}

fn exercise(dir: &Path, registration: &str, constructor: &str, case: &Case<'_>) {
    let Case {
        body,
        output,
        initial,
        expected,
        state,
        ..
    } = *case;
    let original = if output == "()" {
        "*receiver += 1; args.value += 1;"
    } else {
        "*receiver += 1; args.value * 10"
    };
    let program = format!(
        r#"
        struct Args {{ value: u32 }}
        type Output = {output};
        type Listener = fn(&mut u32, &mut Args, &mut dyn FnMut(&mut u32, &mut Args) -> Output) -> Output;
        mod mod_0 {{ use super::Args; {body} }}
        mod mod_0_internals {{ pub mod raw {{ pub fn {constructor}(_: &'static str, _: i32, _: &'static str, f: super::super::Listener) -> super::super::Listener {{ f }} }} }}
        fn main() {{
            let listeners = [{registration}];
            let mut state = 0; let mut args = Args {{ value: {initial} }};
            let value = listeners[0](&mut state, &mut args, &mut |receiver, args| {{ {original} }});
            assert_eq!(value, {expected}); assert_eq!(state, {state});
        }}
    "#
    );
    let source = dir.join("adapter.rs");
    let binary = dir.join(format!("adapter{}", std::env::consts::EXE_SUFFIX));
    fs::write(&source, program).unwrap();
    let checked = Command::new("rustc")
        .args(["--edition=2024", "-Awarnings"])
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(Command::new(binary).status().unwrap().success());
}

#[test]
fn unchanged_callback_compiles_against_both_canonical_facades() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../target/canonical-facades-{}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let canonical = horizon_platform::mod_build::api_gen::sites::source::facade_name(
        "horizon_client::game::Game::run",
        "horizon_client::game::Game::step",
        0,
    );
    let callback = format!("use super::raw::{{__{canonical}Receiver as Receiver, __{canonical}Output as Output, __{canonical}Args as Args}}; pub fn with(receiver: Receiver<'_, '_>, args: &mut Args) {{ *receiver += 10; let increment: Output = 1; args.value += increment; }}");
    for (physical, selector, private) in [
        ("Slisted", "self.step", false),
        ("Ssource", "horizon_client::game::Game::step", true),
    ] {
        let site: Site = serde_json::from_value(serde_json::json!({
            "name":physical, "target":"horizon_client::game::Game::run", "selector":selector, "ordinal":0,
            "call":"horizon_client::game::Game::step", "ctx":"statement", "hash":"place", "private":private,
            "listener":format!("horizon_client::game::__{physical}Listener"), "record":format!("horizon_client::game::__{physical}Args"), "output":"u32"
        })).unwrap();
        let (mut exports, mut constructors, mut crates) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        site.facade(&mut exports, &mut constructors, &mut crates);
        assert!(!exports.contains(&format!("    pub use {};\n", site.listener)));
        assert!(!exports.contains(&format!("    pub use {};\n", site.record)));
        let exports = exports.into_iter().collect::<String>();
        let constructors = constructors.into_iter().collect::<String>();
        let registration = adapter(
            horizon_mod_package::manifest::inject::Kind::BeforeCall,
            "callback::with",
        );
        let constructor = site.constructor();
        let program = format!(
            r#"
            extern crate self as horizon_client;
            extern crate self as mod_points;
            #[derive(Debug, PartialEq)] pub enum SiteId {{ {physical} }}
            impl SiteId {{ pub const fn place(self) -> Self {{ self }} }}
            pub mod __private {{ pub struct SlotListener {{ pub place: super::SiteId, pub f: *const () }} impl SlotListener {{ pub const unsafe fn site(_: &'static str, place: super::SiteId, _: i32, _: &'static str, f: *const ()) -> Self {{ Self {{ place, f }} }} }} }}
            pub mod game {{
                pub struct __{physical}Args {{ pub value: u32 }}
                pub type __{physical}Receiver<'g, 'r> = &'r mut u32;
                pub type __{physical}Output = u32;
                pub type __{physical}Listener = fn(&mut u32, &mut __{physical}Args, &mut dyn FnMut(&mut u32, &mut __{physical}Args) -> u32) -> u32;
            }}
            mod raw {{ {exports} {constructors} }}
            mod callback {{ {callback} }}
            fn main() {{
                let slot = raw::{constructor}("fixture", 17, "value", {registration});
                assert_eq!(slot.place, SiteId::{physical});
                let f: game::__{physical}Listener = unsafe {{ std::mem::transmute(slot.f) }};
                let mut state = 0; let mut args = game::__{physical}Args {{ value: 2 }};
                assert_eq!(f(&mut state, &mut args, &mut |state, args| {{ *state += 1; args.value * 10 }}), 30);
                assert_eq!(state, 11);
            }}
        "#
        );
        let file = dir.join("facade.rs");
        let binary = dir.join(format!("facade{}", std::env::consts::EXE_SUFFIX));
        fs::write(&file, program).unwrap();
        let compiled = Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(file)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert!(Command::new(binary).status().unwrap().success());
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn site_routes_validate_freshness_and_execute_every_kind_adapter() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../target/site-adapters-{}", std::process::id()));
    crate::native_mods::tests::write_bundle(&dir, "example", "");
    let base = fs::read_to_string(dir.join(horizon_mod_package::manifest::MANIFEST)).unwrap();
    let mut site: Site = serde_json::from_value(serde_json::json!({
        "name": "Sone", "target": "horizon_client::game::Game::run", "selector": "self.step", "call": "horizon_client::game::Game::step", "ctx": "statement", "ordinal": 0,
        "hash": "0123456789abcdef", "listener": "horizon_client::game::__SoneListener", "record": "horizon_client::game::__SoneArgs", "output": "u32"
    })).unwrap();
    let before =
        "pub fn hook(receiver: &mut u32, args: &mut Args) { *receiver += 10; args.value += 1; }";
    let condition =
        "pub fn hook(receiver: &mut u32, args: &Args) -> bool { *receiver += 10; args.value == 0 }";
    for case in [
        Case { kind: "before_call", body: before, output: "u32", initial: 2, expected: "30", state: 11 },
        Case { kind: "after_call", body: "pub fn hook(receiver: &mut u32, value: &u32) { assert_eq!(*value, 20); *receiver += 10; }", output: "u32", initial: 2, expected: "20", state: 11 },
        Case { kind: "modify_value", body: "pub fn hook(receiver: &mut u32, value: u32) -> u32 { *receiver += 10; value + 7 }", output: "u32", initial: 2, expected: "27", state: 11 },
        Case { kind: "wrap_call", body: "pub fn hook(receiver: &mut u32, args: &mut Args, original: &mut dyn FnMut(&mut u32, &mut Args) -> u32) -> u32 { let first = original(receiver, args); args.value += 1; first + original(receiver, args) }", output: "u32", initial: 2, expected: "50", state: 2 },
        Case { kind: "wrap_if", body: condition, output: "()", initial: 2, expected: "()", state: 10 },
        Case { kind: "wrap_if", body: condition, output: "()", initial: 0, expected: "()", state: 11 },
    ] {
        let kind = case.kind;
        let text = format!("{base}[[inject]]\nid='test'\nat=\"horizon_client::game::Game<'_>::run\"\nkind='{kind}'\nwith='hook'\naccess='internals'\nhash='signature'\ncall='horizon_client::game::Game::step'\nnth=0\nctx='statement'\n");
        let mut manifest = Manifest::parse(&text).unwrap();
        site.output = case.output.into();
        let catalog = super::super::internals::Catalog {
            discovery: false,
            points: vec![],
            sites: vec![site.clone()],
            wire: Default::default(),
            mods: Vec::new(),
            internals: serde_json::from_value(serde_json::json!([{
                "path":"horizon_client::game::Game::run", "kind":"method", "signature":"fn run(&mut self)", "hash":"signature"
            }])).unwrap(),
        };
        use horizon_mod_package::manifest::inject::InjectError;
        super::super::slots::check(&catalog, &manifest).unwrap();
        let registration = super::super::api::main_rs("/* MOD_SLOTS */", &[&manifest], &super::super::internals::Catalog { sites: catalog.sites.clone(), ..Default::default() });
        exercise(&dir, &registration, &site.constructor(), &case);
        manifest.inject[0].hash = Some("changed".into());
        assert!(matches!(super::super::slots::check(&catalog, &manifest), Err(InjectError::Changed { .. })));
        manifest.inject[0].hash = Some("signature".into());
        manifest.inject[0].ctx = Some("changed".into());
        assert!(matches!(super::super::slots::check(&catalog, &manifest), Err(InjectError::Lost { .. })));
        manifest.inject[0].ctx = Some("statement".into());
        manifest.inject[0].nth = Some(9);
        assert!(matches!(super::super::slots::check(&catalog, &manifest), Err(InjectError::Moved { .. })));
        if kind == "wrap_if" {
            manifest.inject[0].nth = Some(0);
            let mut nonunit = catalog;
            nonunit.sites[0].output = "u32".into();
            assert!(matches!(super::super::slots::check(&nonunit, &manifest), Err(InjectError::Invalid { field: "kind", .. })));
        }
        assert!(Manifest::parse(&text.replace(&format!("kind='{kind}'"), "kind='head'")).is_err());
        assert!(Manifest::parse(&(text + "cancellable=true\n")).is_err());
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn source_call_identity_does_not_require_a_legacy_selector() {
    use super::super::{internals::Catalog, slots};
    use horizon_mod_package::manifest::inject::{InjectError, Route};
    let manifest = Manifest::parse("id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='0123456789012345678901234567890123456789'\nmixins='0123456789012345678901234567890123456789'\n[[inject]]\nid='condition'\nat='game::run'\nkind='wrap_if'\nwith='hook'\naccess='internals'\nhash='signature'\ncall='game::step'\nnth=0\nctx='statement'\n").unwrap();
    // Staged source sites use resolved call identity, not legacy self.step selectors.
    let site: Site = serde_json::from_value(serde_json::json!({
        "name":"Sprivate", "target":"game::run", "ordinal":0,
        "call":"game::step", "ctx":"statement", "hash":"place", "private":true,
        "listener":"game::__SprivateListener", "record":"game::__SprivateArgs", "output":"()"
    }))
    .unwrap();
    let mut catalog = Catalog {
        internals: serde_json::from_value(serde_json::json!([{
            "path":"game::run", "kind":"fn", "signature":"fn run()", "hash":"signature"
        }]))
        .unwrap(),
        sites: vec![site],
        ..Catalog::default()
    };
    assert!(matches!(
        slots::routes(&catalog, &manifest).unwrap().as_slice(),
        [Route::Source(_)]
    ));
    catalog.sites[0].output = "u32".into();
    assert!(matches!(
        slots::check(&catalog, &manifest),
        Err(InjectError::Invalid { field: "kind", .. })
    ));
}

#[test]
fn private_wrap_adapters_repeat_suppress_and_condition_the_original() {
    use super::super::{api, internals::Catalog, slots};
    use horizon_mod_package::manifest::inject::{Access, Inject, Kind, Route};
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../target/private-wraps-{}", std::process::id()));
    crate::native_mods::tests::write_bundle(&dir, "example", "");
    let base = fs::read_to_string(dir.join(horizon_mod_package::manifest::MANIFEST)).unwrap();
    let site: Site = serde_json::from_value(serde_json::json!({
        "name":"Sprivate", "target":"game::run", "selector":"game::step", "ordinal":0,
        "call":"game::step", "ctx":"statement", "hash":"place", "private":true,
        "listener":"game::__SprivateListener", "record":"game::__SprivateArgs", "output":"u32"
    }))
    .unwrap();
    let mut catalog = Catalog {
        internals: serde_json::from_value(serde_json::json!([{
            "path":"game::run", "kind":"fn", "signature":"fn run()", "hash":"signature", "cfg":"", "shape_only":false
        }])).unwrap(),
        points: vec![], sites: vec![], discovery: false,
        wire: Default::default(), mods: Vec::new(),
    };
    for (kind, case) in [
        (Kind::WrapCall, Case { kind: "wrap_call", body: "pub fn hook(receiver: &mut u32, args: &mut Args, original: &mut dyn FnMut(&mut u32, &mut Args) -> u32) -> u32 { let first = original(receiver, args); args.value += 1; first + original(receiver, args) }", output: "u32", initial: 2, expected: "50", state: 2 }),
        (Kind::WrapCall, Case { kind: "wrap_call", body: "pub fn hook(receiver: &mut u32, _: &mut Args, _: &mut dyn FnMut(&mut u32, &mut Args) -> u32) -> u32 { *receiver += 10; 99 }", output: "u32", initial: 2, expected: "99", state: 10 }),
        (Kind::WrapIf, Case { kind: "wrap_if", body: "pub fn hook(receiver: &mut u32, args: &Args) -> bool { *receiver += 10; args.value == 0 }", output: "()", initial: 2, expected: "()", state: 10 }),
        (Kind::WrapIf, Case { kind: "wrap_if", body: "pub fn hook(receiver: &mut u32, args: &Args) -> bool { *receiver += 10; args.value == 0 }", output: "()", initial: 0, expected: "()", state: 11 }),
    ] {
        let mut manifest = Manifest::parse(&base).unwrap();
        manifest.inject.push(Inject {
            local: None, local_type: None, position: None, expr: None,
            id: "wrap".into(), at: "game::run".into(), kind, with: "hook".into(),
            access: Access::Internals, priority: 7, hash: Some("signature".into()),
            call: Some("game::step".into()), nth: Some(0), ctx: Some("statement".into()),
        });
        catalog.sites.clear();
        assert!(matches!(slots::routes(&catalog, &manifest).unwrap().as_slice(), [Route::Source(_)]));
        let mut private = site.clone();
        private.output = case.output.into();
        catalog.sites.push(private);
        assert!(matches!(slots::routes(&catalog, &manifest).unwrap().as_slice(), [Route::Source(_)]));
        let registration = api::main_rs("/* MOD_SLOTS */", &[&manifest], &catalog);
        exercise(&dir, &registration, &site.constructor(), &case);
        if kind == Kind::WrapIf {
            catalog.sites[0].output = "u32".into();
            assert!(matches!(slots::check(&catalog, &manifest), Err(horizon_mod_package::manifest::inject::InjectError::Invalid { field: "kind", .. })));
        }
    }
    fs::remove_dir_all(dir).unwrap();
}
