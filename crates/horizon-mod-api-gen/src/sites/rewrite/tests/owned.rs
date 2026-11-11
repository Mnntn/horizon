use super::*;

/// Compile the actual generated record, capability and dispatch macro together.
/// The payload deliberately has no Clone implementation.
#[test]
fn owned_records_forward_cancel_unwind_and_reject_escapes() {
    let source = r#"
        pub struct Payload(pub std::rc::Rc<std::cell::Cell<usize>>);
        impl Drop for Payload { fn drop(&mut self) { self.0.set(self.0.get() + 1); } }
        type Stack = Payload;
        type Pos = (i32, i32, i32);
        pub struct Game<'g>(pub &'g mut usize);
        impl Game<'_> {
            fn take(&mut self, label: &str, pos: Pos, value: Stack) -> usize { *self.0 += 1; assert_eq!(pos, (1, 2, 3)); drop(value); label.len() }
            pub fn run(&mut self, label: &str, value: Payload) -> usize { let pos = (1, 2, 3); self.take(label, pos, value) }
        }
    "#;
    let catalog = vec![
        internal("horizon_client::game::Payload", "struct", "struct Payload;"),
        internal(
            "horizon_client::game::Game",
            "struct",
            "struct Game<'g>(&'g mut usize);",
        ),
        internal(
            "horizon_client::game::Game::take",
            "method",
            "impl Game < '_ > { fn take(&mut self, label: &str, pos: Pos, value: Stack) -> usize }",
        ),
        internal(
            "horizon_client::game::Game::run",
            "method",
            "impl Game < '_ > { fn run(&mut self, label: &str, value: Payload) -> usize }",
        ),
    ];
    let mut catalog = catalog;
    catalog.push(internal(
        "horizon_client::game::Stack",
        "type",
        "type Stack = Payload;",
    ));
    catalog.push(internal(
        "horizon_client::game::Pos",
        "type",
        "type Pos = (i32, i32, i32);",
    ));
    let request = Declaration {
        target: "horizon_client::game::Game::run".into(),
        selector: "horizon_client::game::Game::take".into(),
        ..declaration("", Some(0))
    };
    let (rewritten, sites) = crate::sites::rewrite::rewrite(
        source,
        "horizon_client::game",
        &catalog,
        std::slice::from_ref(&request),
    )
    .unwrap();
    let site = &sites[0];
    assert!(site.once);
    assert!(site.args[2].1.contains("crate :: game :: Payload"));
    // Listed and source builds expose exactly the same record and listener.
    let (listed, listed_sites) =
        rewrite(source, "horizon_client::game", &catalog, &[request]).unwrap();
    assert_eq!(listed, rewritten);
    assert_eq!(listed_sites[0].hash, site.hash);
    let name = &site.name;
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("owned-contract-{}", std::process::id()));
    fs::create_dir_all(directory.join("src")).unwrap();
    let macros = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../horizon-mod-api-macros");
    fs::write(directory.join("Cargo.toml"), format!("[package]\nname='owned-contract-fixture'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nhorizon-mod-api-macros={{path={:?}}}\n", macros)).unwrap();
    let fixture = |body: &str| {
        format!(
            r#"
        extern crate self as mod_points;
        pub enum SiteId {{ {name} }}
        pub mod __private {{
            pub static SITES: [std::sync::atomic::AtomicBool; 1] = [std::sync::atomic::AtomicBool::new(true)]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }}
            pub fn wrap<O>(_: super::SiteId, at: usize, f: impl FnOnce(Option<(*const (), usize)>) -> O) -> O {{
                f(match at {{ 0 => Some((super::game::listener as *const (), 1)), 1 => Some((super::game::after as *const (), 2)), _ => None }})
            }}
        }}
        static MODE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        pub mod game {{
            {rewritten}
            pub fn listener<'g, 'a, 'next>(receiver: &mut Game<'g>, mut args: __{name}Args<'a>, mut original: __{name}Original<'g, 'a, 'next>) -> usize {{ {body} }}
            pub fn after<'g, 'a, 'next>(receiver: &mut Game<'g>, args: __{name}Args<'a>, mut original: __{name}Original<'g, 'a, 'next>) -> usize {{ original.call(receiver, args) + 1 }}
        }}
        fn main() {{
            use std::sync::atomic::Ordering::Relaxed;
            for enabled in [false, true] {{
                __private::SITES[0].store(enabled, Relaxed);
                for mode in 0..4 {{
                    MODE.store(mode, Relaxed);
                    let drops = std::rc::Rc::new(std::cell::Cell::new(0));
                    let label = String::from("oak");
                    let mut calls = 0;
                    let mut game = game::Game(&mut calls);
                    let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| game.run(&label, game::Payload(drops.clone()))));
                    assert_eq!(calls, usize::from(!enabled || mode == 0 || mode == 3));
                    assert_eq!(drops.get(), 1, "enabled={{enabled}} mode={{mode}}");
                    match (enabled, mode) {{
                        (false, _) => assert_eq!(value.unwrap(), 3),
                        (true, 0) => assert_eq!(value.unwrap(), 8),
                        (true, 1) => assert_eq!(value.unwrap(), 99),
                        (true, _) => assert!(value.is_err()),
                    }}
                    assert_eq!(label, "oak"); // dropping Args never destroys the referent
                }}
            }}
        }}
    "#
        )
    };
    let valid = r#"
        match crate::MODE.load(std::sync::atomic::Ordering::Relaxed) {
            0 => { args.label = "changed"; original.call(receiver, args) }
            1 => 99, // cancellation/non-forwarding wrap drops the owned record
            2 => panic!("listener before forwarding"),
            _ => { let _ = original.call(receiver, args); panic!("listener after forwarding") }
        }
    "#;
    let invoke = |command: &str| {
        std::process::Command::new("cargo")
            .args([command, "--offline", "--quiet", "--manifest-path"])
            .arg(directory.join("Cargo.toml"))
            .env("CARGO_TARGET_DIR", directory.join("target"))
            .output()
            .unwrap()
    };
    fs::write(directory.join("src/main.rs"), fixture(valid)).unwrap();
    let result = invoke("run");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    for (body, diagnostic) in [
        (
            "let out = original.call(receiver, args); drop(args); out",
            "use of moved value: `args`",
        ),
        (
            "let _ = original.call(receiver, __PLACEHOLDERArgs { label: \"x\", pos: (1,2,3), value: Payload(std::rc::Rc::new(std::cell::Cell::new(0))) }); original.call(receiver, args)",
            "use of moved value: `original`",
        ),
        (
            "static mut SAVED: Option<&'static str> = None; unsafe { SAVED = Some(args.label); } 0",
            "lifetime may not live long enough",
        ),
        (
            "static mut SAVED: Option<__PLACEHOLDEROriginal<'static, 'static, 'static>> = None; unsafe { SAVED = Some(original); } 0",
            "lifetime may not live long enough",
        ),
    ] {
        let body = body.replace("__PLACEHOLDER", &format!("__{name}"));
        fs::write(directory.join("src/main.rs"), fixture(&body)).unwrap();
        let result = invoke("check");
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success() && stderr.contains(diagnostic),
            "expected {diagnostic}: {stderr}"
        );
    }
    fs::remove_dir_all(directory).unwrap();
}
