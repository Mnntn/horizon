use super::*;
mod listed_bench;
mod owned;
use crate::crossing::tests::internal;

fn rewrite(
    source: &str,
    module: &str,
    entries: &[Internal],
    declarations: &[Declaration],
) -> Result<(String, Vec<Site>), Error> {
    super::rewrite(source, module, entries, declarations)
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
        names.push(crate::sites::facade_name(
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
        crate::sites::facade_name(
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
fn borrowed_calls_have_call_scoped_records_for_both_ordinals() {
    let source = "fn run(wood: &str) { editor_sprite(\"hanging_signs\", wood); editor_sprite(\"signs\", wood); }";
    let entries = vec![
        internal("horizon_client::game::run", "fn", "fn run(wood: &str)"),
        internal(
            "horizon_client::game::editor_sprite",
            "fn",
            "fn editor_sprite(folder: &str, wood: &str) -> String",
        ),
    ];
    for ordinal in [0, 1] {
        let mut request = declaration("horizon_client::game::editor_sprite", Some(ordinal));
        request.target = "horizon_client::game::run".into();
        let (rewritten, sites) =
            super::rewrite(source, "horizon_client::game", &entries, &[request]).unwrap();
        syn::parse_file(&rewritten).unwrap();
        assert_eq!(sites[0].ordinal, ordinal);
        assert_eq!(sites[0].output, ":: std :: string :: String");
        assert!(rewritten.contains("Args < 'a >"), "{rewritten}");
        assert!(rewritten.contains("for < 'g , 'a >"));
        assert!(rewritten.contains("folder : & 'a str"));
    }
}

#[test]
fn engine_borrow_and_copy_array_are_call_scoped() {
    let source = "fn sense_secondary(w: &MobWorld) { farmland(w, [1, 2, 3]); }";
    let module = "horizon_client_lib::mobs::villager_brain::secondary";
    let entries = vec![
        internal(
            "horizon_client_lib::mobs::MobWorld",
            "struct",
            "pub struct MobWorld<'a> { marker: &'a u32 }",
        ),
        internal(
            &format!("{module}::sense_secondary"),
            "fn",
            "fn sense_secondary(w: &MobWorld)",
        ),
        internal(
            &format!("{module}::farmland"),
            "fn",
            "fn farmland(w: &MobWorld, p: [i32; 3]) -> bool",
        ),
    ];
    let mut entries = entries;
    // The real catalog resolves this imported engine type before lowering.
    let world_path = entries[0].path.clone();
    entries[2].refs.insert("MobWorld".into(), world_path);
    let mut request = declaration(&entries[2].path, Some(0));
    request.target = entries[1].path.clone();
    let (rewritten, sites) = super::rewrite(source, module, &entries, &[request.clone()]).unwrap();
    syn::parse_file(&rewritten).unwrap();
    assert!(
        rewritten.contains("& 'a crate :: mobs :: MobWorld < 'g >"),
        "{rewritten}"
    );
    assert!(rewritten.contains("p : [i32 ; 3]"));
    assert_eq!(sites[0].output, "bool");
    entries[2].signature = "fn farmland(w: &mut MobWorld, p: [i32; 3]) -> bool".into();
    assert!(
        super::rewrite(source, module, &entries, &[request])
            .unwrap_err()
            .to_string()
            .contains("borrowing or non-Copy")
    );
}
