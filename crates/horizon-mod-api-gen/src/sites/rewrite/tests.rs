use super::*;
use crate::crossing::tests::internal;

fn rewrite(
    source: &str,
    module: &str,
    entries: &[Internal],
    declarations: &[Declaration],
) -> Result<(String, Vec<Site>), Error> {
    super::rewrite(source, module, entries, declarations, false)
}

fn entries() -> Vec<Internal> {
    vec![
        internal(
            "horizon_client::game::Game",
            "struct",
            "struct Game { value: u32 }",
        ),
        internal(
            "horizon_client::game::Game::run",
            "method",
            "impl Game { fn run(&mut self) -> Result<u32, ()> }",
        ),
        internal(
            "horizon_client::game::Game::step",
            "method",
            "impl Game { fn step(&mut self, value: u32) -> u32 }",
        ),
        internal(
            "horizon_client::game::add",
            "fn",
            "fn add(value: u32) -> u32",
        ),
    ]
}
fn declaration(selector: &str, ordinal: Option<usize>) -> Declaration {
    Declaration {
        expression: None,
        local: None,
        target: "horizon_client::game::Game::run".into(),
        selector: selector.into(),
        ordinal,
        ctx: String::new(),
        origin: "slots/test.txt:2".into(),
    }
}

#[test]
fn site_rewrite_preserves_method_path_closure_and_question_mark_results() {
    let source = "pub struct Game { pub value: u32 } impl Game { fn step(&mut self, value: u32) -> u32 { self.value += value; self.value } pub fn run(&mut self) -> Result<u32, ()> { let r#__recv = 5; let __args = 7; let mut f = || self.step(2); let n = Ok::<_, ()>(f())?; let _: [u32; add(1) as usize] = [0, 0]; let _ = [0u32; add(1) as usize]; let _ = holder::<{ add(1) as usize }>(); const LOCAL: u32 = add(1); static STATIC: u32 = add(1); enum E { A = add(1) as isize } let fixed = const { add(3) }; Ok(n + fixed + add(__recv) + __wrap(__args)) } } const fn add(value: u32) -> u32 { value + 1 } fn holder<const N: usize>() -> usize { N } fn __wrap(value: u32) -> u32 { value + 2 }";
    let mut catalog = entries();
    catalog.push(internal(
        "horizon_client::game::__wrap",
        "fn",
        "fn __wrap(value: u32) -> u32",
    ));
    let (rewritten, mut sites) = rewrite(
        source,
        "horizon_client::game",
        &catalog,
        &[
            declaration("self.step", Some(0)),
            declaration("add", Some(0)),
            declaration("__wrap", Some(0)),
        ],
    )
    .unwrap();
    let lower_entries = [
        internal("lower::Made", "struct", "struct Made(u32);"),
        internal("lower::lower_run", "fn", "fn lower_run() -> Made"),
        internal("lower::make", "fn", "fn make() -> Made"),
    ];
    let request = Declaration {
        expression: None,
        local: None,
        target: "lower::lower_run".into(),
        selector: "make".into(),
        ordinal: Some(0),
        ctx: String::new(),
        origin: "slots/lower.txt:1".into(),
    };
    let (lower, found) = rewrite(
        "pub struct Made(pub u32); fn make() -> Made { Made(7) } fn lower_run() -> Made { make() }",
        "lower",
        &lower_entries,
        &[request],
    )
    .unwrap();
    sites.extend(found);
    let variants = sites
        .iter()
        .map(|site| format!("{},", site.name))
        .collect::<String>();
    let count = sites.len();
    let fixture = format!(
        r#"
        #![forbid(unsafe_code)]
        extern crate self as mod_points;
        extern crate self as horizon_mod_api_macros;
        pub enum SiteId {{ {variants} }}
        pub mod __private {{ pub static SITES: [std::sync::atomic::AtomicBool; {count}] = [const {{ std::sync::atomic::AtomicBool::new(false) }}; {count}]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }} }}
        #[macro_export] macro_rules! site {{ ($wrap:ident, $place:ident, $listener:ident, $recv_ty:ident, $record:ident, $output:ident, $receiver:expr, $args:expr, $original:expr) => {{ ($original)($receiver, &mut $args) }}; }}
        mod game {{ {rewritten} }}
        {lower}
        fn main() {{
            for enabled in [false, true] {{
                for flag in &__private::SITES {{ flag.store(enabled, std::sync::atomic::Ordering::Relaxed); }}
                let mut game = game::Game {{ value: 0 }};
                assert_eq!(game.run(), Ok(21));
                assert_eq!(game.value, 2);
                assert_eq!(lower_run().0, 7);
            }}
        }}
    "#
    );
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("site-rewrite-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("fixture.rs");
    let binary = directory.join("fixture");
    fs::write(&file, fixture).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024", "-Awarnings"])
        .arg(&file)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(
        std::process::Command::new(binary)
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn site_fingerprint_ignores_formatting_but_duplicate_contexts_are_lost() {
    let hash = |source: &str| {
        rewrite(
            source,
            "horizon_client::game",
            &entries(),
            &[declaration(".step", Some(0))],
        )
        .unwrap()
        .1
        .remove(0)
        .hash
    };
    let original = hash("impl Game { fn run(&mut self) { self.step(1); } }");
    let error = rewrite(
        "impl Game { fn run(&mut self) { self.step(1); self.step(1); } }",
        "horizon_client::game",
        &entries(),
        &[declaration(".step", Some(0))],
    )
    .unwrap_err();
    assert!(error.to_string().contains("E_INJECT_LOST"));
    assert_eq!(
        original,
        hash("impl Game { fn run(&mut self) { let ready = true; self.step(1); } }")
    );
    assert_eq!(
        original,
        hash("impl Game { fn run(&mut self) { self . step ( 1 ) ; } }")
    );
}

#[test]
fn site_rewrite_ordinals_hashes_and_refusals() {
    let run = |source: &str, declarations: &[Declaration]| {
        rewrite(source, "horizon_client::game", &entries(), declarations)
    };
    let source = "impl Game { fn run(&mut self) { self.step(1); self.step(2); } }";
    let declarations = [declaration(".step", Some(1))];
    let (_, sites) = run(source, &declarations).unwrap();
    let (_, changed) = run(&source.replace("step(2)", "step(3)"), &declarations).unwrap();
    assert_eq!(sites[0].ordinal, 1);
    assert_eq!(sites[0].name, changed[0].name);
    assert_ne!(sites[0].hash, changed[0].hash);
    for (selector, ordinal, reason) in [
        (".step", None, "ambiguous"),
        (".step", Some(2), "ordinal"),
        (".missing", Some(0), "matched no call"),
    ] {
        let error = run(source, &[declaration(selector, ordinal)]).unwrap_err();
        assert!(error.origin.as_ref().unwrap().contains("slots/test.txt:2"));
        assert!(error.to_string().contains(reason));
    }
    let mut conflict = declaration("self.step", Some(1));
    conflict.origin = "slots/other.txt:4".into();
    let error = run(source, &[declarations[0].clone(), conflict]).unwrap_err();
    assert!(
        error.to_string().contains("slots/test.txt:2")
            && error.to_string().contains("slots/other.txt:4")
    );
    let nested = "impl Game { fn run(&mut self) { add(add(1)); } }";
    assert!(
        run(
            nested,
            &[declaration("add", Some(0)), declaration("add", Some(1))]
        )
        .unwrap_err()
        .to_string()
        .contains("E_INJECT_LOST")
    );
    let bad = "impl Game { fn run(&mut self) { self.step(self.value); } }";
    assert!(
        run(bad, &[declaration(".step", None)])
            .unwrap_err()
            .to_string()
            .contains("two-phase")
    );
}

#[test]
fn trait_impl_callers_are_left_for_the_coordinators_missing_target_refusal() {
    let source = "trait Ext { fn run(&mut self); fn make() -> u32; } impl Ext for Game { fn run(&mut self) { Self::make(); } fn make() -> u32 { 7 } }";
    let (actual, sites) = rewrite(
        source,
        "horizon_client::game",
        &entries(),
        &[declaration("Self::make", Some(0))],
    )
    .unwrap();
    assert_eq!(actual, source);
    assert!(sites.is_empty());
}

#[test]
fn site_rewrite_places_records_after_eof_without_a_final_newline() {
    let mut catalog = entries();
    catalog.push(internal(
        "horizon_client::game::run",
        "fn",
        "fn run() -> u32",
    ));
    for (target, source, selector) in [
        (
            "Game::run",
            "impl Game {\n fn run(&mut self) { self.step(1); }\n}",
            ".step",
        ),
        (
            "Game::run",
            "impl Game {\n fn run(&mut self) { let café = 1; self.step(café); }\n}",
            ".step",
        ),
        ("run", "fn run() -> u32 { add(1) }", "add"),
    ] {
        let mut request = declaration(selector, Some(0));
        request.target = format!("horizon_client::game::{target}");
        let (actual, sites) =
            rewrite(source, "horizon_client::game", &catalog, &[request]).unwrap();
        assert_eq!(sites.len(), 1);
        let parsed = syn::parse_file(&actual).unwrap();
        assert!(matches!(parsed.items.last(), Some(syn::Item::Type(_))));
    }
}

#[test]
fn site_identity_uses_resolved_callees_for_aliases_and_non_record_safe_calls() {
    let mut catalog = entries();
    catalog[1]
        .refs
        .insert("alias".into(), "horizon_client::game::add".into());
    let source = "impl Game { fn run(&mut self) { alias(1); add(2); } }";
    for selector in ["alias", "add", "horizon_client::game::add"] {
        let (_, sites) = rewrite(
            source,
            "horizon_client::game",
            &catalog,
            &[declaration(selector, Some(1))],
        )
        .unwrap();
        assert_eq!(sites[0].ordinal, 1);
        assert_eq!(sites[0].call, "horizon_client::game::add");
        let json = serde_json::to_value(&sites[0]).unwrap();
        assert_eq!(json["ctx"], sites[0].ctx);
        assert_eq!(json["hash"], sites[0].hash);
    }
    // The first call cannot be lowered, but still counts toward the same callee's nth.
    let (_, sites) = rewrite(
        "impl Game { fn run(&mut self) { self.step(self.value); self.step(2); } }",
        "horizon_client::game",
        &catalog,
        &[declaration(".step", Some(1))],
    )
    .unwrap();
    assert_eq!(sites[0].ordinal, 1);
}

#[test]
fn pinned_site_context_refusal_table() {
    let source = "impl Game { fn run(&mut self) { self.step(2); } }";
    let request = declaration(".step", Some(0));
    let (_, mut sites) = rewrite(
        source,
        "horizon_client::game",
        &entries(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    let mut request = request;
    request.ctx = sites.remove(0).ctx;
    for (body, reason) in [
        ("self . step (2);", None),
        ("add(9); self.step(2);", None),
        (
            "self.step(1); self.step(2);",
            Some("E_INJECT_MOVED: now at nth=1"),
        ),
        ("self.step(3);", Some("E_INJECT_LOST")),
        ("add(2);", Some("E_INJECT_LOST")),
        ("self.step(2); self.step(2);", Some("E_INJECT_LOST")),
        ("if true { self.step(2); }", Some("E_INJECT_LOST")),
    ] {
        let source = format!("impl Game {{ fn run(&mut self) {{ {body} }} }}");
        let actual = rewrite(
            &source,
            "horizon_client::game",
            &entries(),
            std::slice::from_ref(&request),
        );
        match reason {
            None => {
                actual.unwrap();
            }
            Some(reason) => assert!(actual.unwrap_err().to_string().contains(reason), "{body}"),
        }
    }
    // The same statement in different branches/arms must have distinct nesting paths.
    for body in [
        "if true { self.step(2); } else { self.step(2); }",
        "let value = match 1 { 0 => self.step(2), _ => self.step(2) };",
    ] {
        let source = format!("impl Game {{ fn run(&mut self) {{ {body} }} }}");
        let (_, sites) = rewrite(
            &source,
            "horizon_client::game",
            &entries(),
            &[declaration(".step", Some(0)), declaration(".step", Some(1))],
        )
        .unwrap();
        assert_ne!(sites[0].ctx, sites[1].ctx);
    }
}

#[test]
fn explicit_receivers_preserve_canonical_identity_and_execute() {
    let mut names = Vec::new();
    for (selector, call) in [
        ("self.step", "self.step(2)"),
        ("Self::step", "Self::step(self, 2)"),
    ] {
        let source = format!(
            "pub struct Game {{ pub value: u32 }} impl Game {{ fn step(&mut self, value: u32) -> u32 {{ self.value += value; self.value }} pub fn run(&mut self) -> u32 {{ self.step(self.value); {call} }} }}"
        );
        let (rewritten, sites) = rewrite(
            &source,
            "horizon_client::game",
            &entries(),
            &[declaration(selector, Some(1))],
        )
        .unwrap();
        assert_eq!(sites[0].call, "horizon_client::game::Game::step");
        assert_eq!(sites[0].ordinal, 1);
        names.push(crate::sites::source::facade_name(
            &sites[0].target,
            &sites[0].call,
            sites[0].ordinal,
        ));
        let variant = &sites[0].name;
        let code = format!(
            r#"
            extern crate self as mod_points;
            extern crate self as horizon_mod_api_macros;
            pub enum SiteId {{ {variant} }}
            pub mod __private {{ pub static SITES: [std::sync::atomic::AtomicBool; 1] = [std::sync::atomic::AtomicBool::new(true)]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }} }}
            #[macro_export] macro_rules! site {{ ($wrap:ident, $place:ident, $listener:ident, $recv_ty:ident, $record:ident, $output:ident, $receiver:expr, $args:expr, $original:expr) => {{ ($original)($receiver, &mut $args) }}; }}
            mod game {{ {rewritten} }}
            fn main() {{ let mut game = game::Game {{ value: 0 }}; assert_eq!(game.run(), 2); assert_eq!(game.value, 2); }}
        "#
        );
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/explicit-receiver-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let input = root.join("fixture.rs");
        let binary = root.join("fixture");
        fs::write(&input, code).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(&input)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert!(
            std::process::Command::new(binary)
                .status()
                .unwrap()
                .success()
        );
        fs::remove_dir_all(root).unwrap();
    }
    assert_eq!(names[0], names[1]);
    assert_ne!(
        names[0],
        crate::sites::source::facade_name(
            "horizon_client::game::Game::run",
            "horizon_client::game::other",
            1
        )
    );
}

#[test]
fn changing_an_import_to_a_same_signature_callee_changes_the_fingerprint() {
    let mut catalog = entries();
    catalog.push(internal(
        "horizon_client::other::add",
        "fn",
        "fn add(value: u32) -> u32",
    ));
    let mut sites = Vec::new();
    for path in ["horizon_client::game::add", "horizon_client::other::add"] {
        catalog[1].refs.insert("alias".into(), path.into());
        sites.push(
            rewrite(
                "impl Game { fn run(&mut self) { alias(2); } }",
                "horizon_client::game",
                &catalog,
                &[declaration("alias", Some(0))],
            )
            .unwrap()
            .1
            .remove(0),
        );
    }
    assert_eq!(sites[0].ctx, sites[1].ctx);
    assert_ne!(sites[0].hash, sites[1].hash);
}

#[test]
fn modify_local_executes_real_binding_at_call_boundary() {
    let catalog = [
        internal("fixture::f", "fn", "fn f() -> i32"),
        internal("fixture::sink", "fn", "fn sink(n: i32)"),
    ];
    for source in [
        "fn f() -> i32 { let mut n: i32 = 3; sink(n); n * 2 }",
        "fn f() -> i32 { let mut n = 3i32; sink(n); n * 2 }",
    ] {
        let (untouched, sites) = super::rewrite(source, "fixture", &catalog, &[], true).unwrap();
        assert_eq!(untouched, source);
        assert!(sites.is_empty());
        for (after, listeners, expected_sink, expected_result) in [
            (false, "", 3, 6),
            (false, "*receiver = 7;", 7, 14),
            (true, "*receiver = 7;", 3, 14),
            (
                false,
                "let listeners: [fn(&mut i32); 2] = [|n| *n += 2, |n| *n *= 3]; for listener in listeners { listener(receiver); }",
                15,
                30,
            ),
        ] {
            let request = Declaration {
                expression: None,
                target: "fixture::f".into(),
                selector: "fixture::sink".into(),
                ordinal: Some(0),
                ctx: String::new(),
                origin: "local fixture".into(),
                local: Some(super::super::Local {
                    binding: "n".into(),
                    ty: "i32".into(),
                    after,
                }),
            };
            let (rewritten, sites) =
                super::rewrite(source, "fixture", &catalog, &[request], true).unwrap();
            let variant = &sites[0].name;
            let enabled = !listeners.is_empty();
            let fixture = format!(
                r#"
            extern crate self as mod_points;
            extern crate self as horizon_mod_api_macros;
            pub enum SiteId {{ {variant} }}
            pub mod __private {{ pub static SITES: [std::sync::atomic::AtomicBool; 1] = [std::sync::atomic::AtomicBool::new({enabled})]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }} }}
            #[macro_export] macro_rules! site {{ ($wrap:ident, $place:ident, $listener:ident, $recv_ty:ident, $record:ident, $output:ident, $receiver:expr, $args:expr, $original:expr) => {{{{ let receiver = $receiver; {listeners} }}}}; }}
            static SINK: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
            fn sink(n: i32) {{ SINK.store(n, std::sync::atomic::Ordering::Relaxed); }}
            {rewritten}
            fn main() {{ assert_eq!(f(), {expected_result}); assert_eq!(SINK.load(std::sync::atomic::Ordering::Relaxed), {expected_sink}); }}
        "#
            );
            let dir = std::env::temp_dir().join(format!(
                "horizon-local-{}-{after}-{expected_result}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("main.rs"), &fixture).unwrap();
            let result = std::process::Command::new("rustc")
                .args(["--edition", "2024"])
                .arg(dir.join("main.rs"))
                .arg("-o")
                .arg(dir.join("run"))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}\n{fixture}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                std::process::Command::new(dir.join("run"))
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn modify_local_refuses_uncertain_bindings_by_name() {
    let catalog = [
        internal("fixture::f", "fn", "fn f() -> i32"),
        internal("fixture::sink", "fn", "fn sink(n: i32)"),
    ];
    for (source, binding, ty, words) in [
        (
            "fn f() -> i32 { let mut n: i32 = 3; let n = n + 1; sink(n); n * 2 }",
            "n",
            "i32",
            vec!["n", "shadowed"],
        ),
        (
            "fn f() -> i32 { let n: i32 = 3; sink(n); n * 2 }",
            "n",
            "i32",
            vec!["n", "mut"],
        ),
        (
            "fn f() -> i32 { let mut n: i32 = 3; sink(n); n * 2 }",
            "n",
            "i64",
            vec!["n", "i32", "i64"],
        ),
        (
            "fn f() -> i32 { let mut n: i32 = 3; sink(n); n * 2 }",
            "m",
            "i32",
            vec!["m", "not found"],
        ),
        (
            "fn f() -> i32 { let mut n: i32 = 3; let r = &n; sink(n); n * 2 }",
            "n",
            "i32",
            vec!["n", "borrowed"],
        ),
        (
            "fn f() -> i32 { let mut n: Thing = make(); consume(n); sink(3); 6 }",
            "n",
            "Thing",
            vec!["n", "moved"],
        ),
    ] {
        let request = Declaration {
            expression: None,
            target: "fixture::f".into(),
            selector: "fixture::sink".into(),
            ordinal: Some(0),
            ctx: String::new(),
            origin: "local fixture".into(),
            local: Some(super::super::Local {
                binding: binding.into(),
                ty: ty.into(),
                after: false,
            }),
        };
        let error = super::rewrite(source, "fixture", &catalog, &[request], true)
            .unwrap_err()
            .to_string();
        for word in words {
            assert!(error.contains(word), "{error}");
        }
    }
}

#[test]
fn modify_local_refuses_pattern_borrows_uninitialized_and_inferred_bindings() {
    let catalog = [
        internal("fixture::f", "fn", "fn f() -> i32"),
        internal("fixture::sink", "fn", "fn sink(n: i32)"),
    ];
    for (body, reason) in [
        (
            "let mut n: i32 = 3; let ref r = n; sink(0); *r",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; let ref mut r = n; sink(0); *r",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; let (ref r, _) = (n, 0); sink(0); *r",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; match n { ref r => { retain(r); } } sink(0); n",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; match Some(n) { Some(ref mut r) => { retain(r); }, _ => {} } sink(0); n",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; if let Some(ref r) = Some(n) { retain(r); } sink(0); n",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32 = 3; while let Some(ref mut r) = Some(n) { retain(r); break; } sink(0); n",
            "binding is borrowed across the position",
        ),
        (
            "let mut n: i32; sink(0); n = 1; n",
            "binding is not initialized at its declaration",
        ),
        (
            "let mut n = 3; sink(0); n",
            "cannot prove declared type `i32`",
        ),
    ] {
        for after in [false, true] {
            let request = Declaration {
                expression: None,
                target: "fixture::f".into(),
                selector: "fixture::sink".into(),
                ordinal: Some(0),
                ctx: String::new(),
                origin: "local fixture".into(),
                local: Some(super::super::Local {
                    binding: "n".into(),
                    ty: "i32".into(),
                    after,
                }),
            };
            let error = super::rewrite(
                &format!("fn f() -> i32 {{ {body} }}"),
                "fixture",
                &catalog,
                &[request],
                true,
            )
            .unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("local fixture: fixture::f: local `n`: {reason}")
            );
        }
    }
    // An after position also scans the selected call's argument expressions.
    let request = Declaration {
        expression: None,
        target: "fixture::f".into(),
        selector: "fixture::sink".into(),
        ordinal: Some(0),
        ctx: String::new(),
        origin: "local fixture".into(),
        local: Some(super::super::Local {
            binding: "n".into(),
            ty: "i32".into(),
            after: true,
        }),
    };
    let error = super::rewrite(
        "fn f() -> i32 { let mut n: i32 = 3; sink(match n { ref r => *r }); n }",
        "fixture",
        &catalog,
        &[request],
        true,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "local fixture: fixture::f: local `n`: binding is borrowed across the position"
    );
}

fn expression(expr: &str, ty: &str, nth: usize) -> Declaration {
    Declaration {
        expression: Some(Expression {
            ty: ty.into(),
            origin: "example:reach".into(),
        }),
        local: None,
        target: "horizon_client::game::Game::run".into(),
        selector: expr.into(),
        ordinal: Some(nth),
        ctx: String::new(),
        origin: "example:reach".into(),
    }
}

#[test]
fn an_expression_site_wraps_one_read_and_keeps_its_value_when_nothing_listens() {
    // Two `4` reads (one unsuffixed, one in a borrow), an assignment's place and a receiver,
    // which a value can't stand in for, and a field read.
    let source = "pub struct Game { pub value: u32 } impl Game { fn step(&mut self, value: u32) -> u32 { value } pub fn run(&mut self) -> Result<u32, ()> { let far = 4; let near = &4; self.value = 4; let n = 4u32.max(1); let k = 2u32; Ok(far + *near + n + self.value + 4 + k) } } const fn add(value: u32) -> u32 { value }";
    let staged = |declarations: &[Declaration]| {
        super::rewrite(
            source,
            "horizon_client::game",
            &entries(),
            declarations,
            true,
        )
    };
    // The reads are `far = 4`, the value assigned to `self.value` and the last `4`; `&4`, the
    // assignment's place and `4u32` (another literal) are not.
    let (_, first) = staged(&[expression("4", "u32", 0)]).unwrap();
    let (rewritten, last) = staged(&[expression(" 4 ", "u32", 2)]).unwrap();
    assert_ne!(first[0].ctx, last[0].ctx);
    assert_eq!(
        (
            last[0].call.as_str(),
            last[0].output.as_str(),
            last[0].receiver.as_str()
        ),
        ("4", "u32", "()")
    );
    let marker = super::super::expression::marker(&last[0].name);
    assert!(
        rewritten.contains(&format!("let {marker} : u32 = 4 ;")),
        "{rewritten}"
    );
    let lost = staged(&[expression("4", "u32", 3)])
        .unwrap_err()
        .to_string();
    assert!(lost.contains("expr matched no read at that nth"), "{lost}");
    let mut moved = expression("4", "u32", 0);
    moved.ctx = last[0].ctx.clone();
    assert!(matches!(
        staged(&[moved]).unwrap_err().cause,
        Cause::Identity(Refusal::Moved(2))
    ));
    let mut gone = expression("4", "u32", 0);
    gone.ctx = "changed".into();
    let error = staged(&[gone]).unwrap_err().to_string();
    assert!(
        error.contains(&format!("nth = 2, ctx = \"{}\"", last[0].ctx)),
        "{error}"
    );
    // A field read, and the refusals that need no compiler.
    let (_, field) = staged(&[expression("self.value", "u32", 0)]).unwrap();
    assert_eq!(field[0].call, "self.value");
    for (expr, ty, why) in [
        ("2u32", "u64", "is `u32`, but its declared type is `u64`"),
        ("4", "String", "carries a copied scalar"),
        (
            "self.step(1)",
            "u32",
            "a literal, a constant's path or a field read",
        ),
    ] {
        let error = staged(&[expression(expr, ty, 0)]).unwrap_err().to_string();
        assert!(error.contains(why), "{expr}: {error}");
    }
    assert!(
        rewrite(
            source,
            "horizon_client::game",
            &entries(),
            &[expression("4", "u32", 0)]
        )
        .unwrap_err()
        .to_string()
        .contains("expression requires native source rebuilding")
    );
    // The lowered site compiles and keeps the read's value with the slot off and on (a stub
    // `site!` calls the original).
    let fixture = format!(
        r#"
        extern crate self as mod_points;
        extern crate self as horizon_mod_api_macros;
        pub enum SiteId {{ {name} }}
        pub mod __private {{ pub static SITES: [std::sync::atomic::AtomicBool; 1] = [const {{ std::sync::atomic::AtomicBool::new(false) }}; 1]; pub struct Table; pub static TABLE: Table = Table; impl Table {{ pub fn active_site(&self, index: usize) -> bool {{ SITES[index].load(std::sync::atomic::Ordering::Relaxed) }} }} }}
        #[macro_export] macro_rules! site {{ ($wrap:ident, $place:ident, $listener:ident, $recv_ty:ident, $record:ident, $output:ident, $receiver:expr, $args:expr, $original:expr) => {{ ($original)($receiver, &mut $args) }}; }}
        mod game {{ {rewritten} }}
        fn main() {{
            for enabled in [false, true] {{
                __private::SITES[0].store(enabled, std::sync::atomic::Ordering::Relaxed);
                let mut game = game::Game {{ value: 0 }};
                assert_eq!(game.run(), Ok(22));
            }}
        }}
    "#,
        name = last[0].name
    );
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("expression-rewrite-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let file = directory.join("fixture.rs");
    let binary = directory.join("fixture");
    fs::write(&file, fixture).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024", "-Awarnings"])
        .arg(&file)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(
        std::process::Command::new(binary)
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn string_literals_differing_only_in_spaces_are_separate_reads() {
    let source = "pub struct Game { pub value: u32 } impl Game { pub fn run(&mut self) -> Result<u32, ()> { let a = \"a b\"; let b = \"ab\"; let c = \" \"; let d = \"\"; Ok((a.len() + b.len() + c.len() + d.len()) as u32) } }";
    let staged = |expr: &str, nth: usize| {
        super::rewrite(
            source,
            "horizon_client::game",
            &entries(),
            &[expression(expr, "&'static str", nth)],
            true,
        )
    };
    let mut contexts = Vec::new();
    for literal in ["\"a b\"", "\"ab\"", "\" \"", "\"\""] {
        let (rewritten, sites) = staged(literal, 0).unwrap();
        assert_eq!(sites[0].call, literal);
        let marker = super::super::expression::marker(&sites[0].name);
        assert!(
            rewritten.contains(&format!("{marker} : & 'static str = {literal} ;")),
            "{literal}: {rewritten}"
        );
        // Each literal is one read: no second one at nth 1.
        assert!(staged(literal, 1).is_err(), "{literal}");
        contexts.push(sites[0].ctx.clone());
    }
    contexts.sort();
    contexts.dedup();
    assert_eq!(contexts.len(), 4);
    // Spacing between tokens still never decides a match.
    assert_eq!(
        super::super::expression::spelling("self . value").unwrap(),
        "self.value"
    );
    assert_eq!(super::super::expression::spelling("- 1").unwrap(), "-1");
}
