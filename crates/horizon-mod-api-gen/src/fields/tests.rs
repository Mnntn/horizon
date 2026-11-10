use super::*;
fn declaration(mod_id: &str) -> Declaration {
    Declaration {
        id: "count-slot".into(),
        target: "fixture::S".into(),
        mod_id: mod_id.into(),
        name: "count".into(),
        ty: "u32".into(),
    }
}
fn execute(source: &str) {
    execute_with_dependencies(source, &[]);
}
fn execute_with_dependencies(source: &str, dependencies: &[&str]) {
    let dir = std::env::temp_dir().join(format!(
        "horizon-field-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    fs::write(&file, source).unwrap();
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    // Shared lanes retain several feature sets. The two serde crates must use the same
    // serde_core artifact; modification times do not identify compatible externs.
    let mut combinations = vec![Vec::new()];
    for dependency in dependencies {
        let prefix = format!("lib{dependency}-");
        let mut libraries = fs::read_dir(&deps)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&prefix)
                    && p.extension().is_some_and(|e| e == "rlib")
            })
            .collect::<Vec<_>>();
        libraries.sort_by_key(|p| std::cmp::Reverse(p.metadata().unwrap().modified().unwrap()));
        assert!(!libraries.is_empty(), "test dependency {dependency}");
        combinations = combinations
            .into_iter()
            .flat_map(|prefix| {
                libraries.iter().map(move |library| {
                    let mut entries = prefix.clone();
                    entries.push(format!("{dependency}={}", library.display()));
                    entries
                })
            })
            .collect();
    }
    let mut compiled = false;
    let mut errors = String::new();
    for externs in combinations {
        let mut command = std::process::Command::new("rustc");
        command
            .args(["--edition=2024", "--crate-name=field_fixture"])
            .arg(&file)
            .arg("-o")
            .arg(dir.join("run"))
            .arg("-L")
            .arg(format!("dependency={}", deps.display()));
        for external in externs {
            command.arg("--extern").arg(external);
        }
        let output = command.output().unwrap();
        if output.status.success() {
            compiled = true;
            break;
        }
        errors.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    assert!(compiled, "{errors}\n{source}");
    assert!(
        std::process::Command::new(dir.join("run"))
            .status()
            .unwrap()
            .success()
    );
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn field_derived_construction_and_clone_execute() {
    let source = "#[derive(Clone, Default)] struct S { a: i32 } fn main() { let mut s = S { a: 1 }; assert_eq!(s.m__count, 0); s.m__count=11; let c=s.clone(); s.m__count=17; assert_eq!(s.m__count,17); assert_eq!(c.m__count,11); assert_eq!(S::default().m__count,0); }";
    let (rewritten, found) = rewrite(source, "fixture", &[declaration("m")]).unwrap();
    assert_eq!(found, BTreeSet::from(["fixture::S".into()]));
    assert!(rewritten.contains("struct S { a: i32 , pub m__count: u32,}"));
    assert!(rewritten.contains("S { a: 1, m__count: Default::default() }"));
    execute(&rewritten);
    assert_eq!(rewrite(source, "fixture", &[]).unwrap().0, source);
}
#[test]
fn field_manual_default_clone_and_two_mods_execute() {
    let source = "use std::clone::Clone as Duplicate; struct S { a: i32 } impl Default for S { fn default()->Self { Self { a: 0 } } } impl Duplicate for S { fn clone(&self)->Self { return Self { a:self.a }; } fn clone_from(&mut self, src:&Self) { self.a=src.a; } } fn main() { let mut s=S { a:1 }; assert_eq!(S::default().m1__count,0); s.m1__count=11; s.m2__count=23; let mut c=s.clone(); s.m1__count=17; assert_eq!(c.m1__count,11); assert_eq!(c.m2__count,23); c.clone_from(&s); assert_eq!(c.m1__count,17); assert_eq!(c.m2__count,23); }";
    execute(
        &rewrite(source, "fixture", &[declaration("m1"), declaration("m2")])
            .unwrap()
            .0,
    );
}
#[test]
fn field_refusals_name_target_field_and_type() {
    let tests = "#[cfg(test)] mod tests { fn f() { make!(S); } }";
    let source = format!("struct S {{ a:i32 }} {tests}");
    assert!(
        rewrite(&source, "fixture", &[declaration("m")])
            .unwrap()
            .0
            .ends_with(tests)
    );

    let mut d = declaration("m");
    d.name = "a".into();
    assert_eq!(
        rewrite("struct S { a:i32 }", "fixture", &[d]).unwrap_err(),
        "E_INJECT_FIELD: m:count-slot: fixture::S: field a collides with an existing field"
    );
    let mut d = declaration("m");
    d.ty = "m::Counter".into();
    assert_eq!(
        rewrite("struct S { a:i32 }", "fixture", &[d]).unwrap_err(),
        "E_INJECT_FIELD: m:count-slot: fixture::S: field type m::Counter is not reachable from the target engine crate; mod-defined types require attachments"
    );
    assert!(
        rewrite(
            "struct S { a:i32 } fn f(){ log!(\"S\"); }",
            "fixture",
            &[declaration("m")]
        )
        .is_ok()
    );
    for source in [
        "#[derive(Wire)] struct S { a:i32 }",
        "struct S { a:i32 } fn f(s:S) { let S { a }=s; }",
        "struct S { a:i32 } fn f()->S { make!(S) }",
    ] {
        assert!(rewrite(source, "fixture", &[declaration("m")]).is_err());
    }
}
#[test]
fn field_imports_self_and_update_syntax_execute() {
    let source = "const LABEL: &str = \"λ\"; mod inner { #[derive(Clone,Default)] pub struct S { pub a:i32 } impl S { pub fn new()->Self { Self { a:1 } } } } fn main(){ use inner::S as Alias; let fresh=Alias { a:3 }; assert_eq!(fresh.m__count,0); let mut s=Alias::new(); s.m__count=11; let t=Alias { a:2, ..s }; assert_eq!(t.m__count,11); }";
    let mut d = declaration("m");
    d.target = "fixture::inner::S".into();
    execute(&rewrite(source, "fixture", &[d]).unwrap().0);
}

#[test]
fn field_stage_unknown_target_is_atomic_and_names_type() {
    let root = std::env::temp_dir().join(format!("horizon-field-stage-{}", std::process::id()));
    let dir = root.join("crates/fixture");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    let file = dir.join("src/lib.rs");
    let source = "#[derive(Clone,Default)] pub struct S { pub a:i32 }";
    fs::write(&file, source).unwrap();
    let mut unknown = declaration("m");
    unknown.target = "fixture::T".into();
    assert_eq!(
        stage(&root, &[declaration("m"), unknown]).unwrap_err(),
        "E_INJECT_FIELD: m:count-slot: fixture::T: unknown target struct fixture::T"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
    stage(&root, &[declaration("m")]).unwrap();
    assert!(
        fs::read_to_string(file)
            .unwrap()
            .contains("pub m__count: u32")
    );
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn field_serde_skip_and_wire_refusal() {
    let source = "#[derive(serde::Serialize, serde::Deserialize)] struct S { a:i32 }";
    let out = rewrite(source, "fixture", &[declaration("m")]).unwrap().0;
    assert!(out.contains("#[serde(skip)] pub m__count: u32,"));
    assert_eq!(
        rewrite(
            "#[derive(Wire)] struct S { a:i32 }",
            "fixture",
            &[declaration("m")]
        )
        .unwrap_err(),
        "E_INJECT_FIELD: m:count-slot: fixture::S: derive Wire cannot guarantee the field is never saved or synced"
    );
}

#[test]
fn field_default_and_clone_bounds_are_checked_even_without_constructors() {
    for (source, ty, expected) in [
        (
            "struct NoDefault; struct S { a:i32 } fn main() {}",
            "crate::NoDefault",
            "Default",
        ),
        (
            "#[derive(Default)] struct NoClone; #[derive(Clone)] struct S { a:i32 } fn main() {}",
            "crate::NoClone",
            "Clone",
        ),
    ] {
        let mut d = declaration("m");
        d.ty = ty.into();
        let out = rewrite(source, "fixture", &[d]).unwrap().0;
        let dir = std::env::temp_dir().join(format!(
            "horizon-field-bounds-{}-{expected}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        fs::write(&file, &out).unwrap();
        let output = std::process::Command::new("rustc")
            .args(["--edition=2024", "--crate-name=field_bounds"])
            .arg(&file)
            .arg("-o")
            .arg(dir.join("run"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(expected)
                && error.contains(if expected == "Default" {
                    "NoDefault"
                } else {
                    "NoClone"
                }),
            "{error}"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn field_is_never_serialized_and_deserialization_defaults_it() {
    let source = r##"#[derive(serde::Serialize, serde::Deserialize)] struct S { a:i32 } fn main() { let mut s=S { a:1 }; s.m__count=11; assert_eq!(serde_json::to_string(&s).unwrap(), r#"{"a":1}"#); let loaded:S=serde_json::from_str(r#"{"a":1}"#).unwrap(); assert_eq!(loaded.m__count,0); }"##;
    execute_with_dependencies(
        &rewrite(source, "fixture", &[declaration("m")]).unwrap().0,
        &["serde", "serde_json"],
    );
}

#[test]
fn field_engine_type_uses_the_target_crates_namespace() {
    let source = "#[derive(Default,Clone)] struct Counter(u32); #[derive(Default,Clone)] struct S { a:i32 } fn main(){ let s=S { a:1 }; assert_eq!(s.m__count.0,0); }";
    let mut d = declaration("m");
    d.ty = "fixture::Counter".into();
    execute(&rewrite(source, "fixture", &[d]).unwrap().0);
}

#[test]
fn emitted_field_collision_names_both_declaration_owners() {
    let mut a = declaration("m");
    a.id = "first-slot".into();
    a.name = "_count".into();
    let mut b = declaration("m_");
    b.id = "second-slot".into();
    let error = stage(Path::new("nonexistent-no-io"), &[a, b]).unwrap_err();
    assert!(error.contains("m___count"), "{error}");
    assert!(error.contains("m:first-slot"), "{error}");
    assert!(error.contains("m_:second-slot"), "{error}");
}

#[test]
fn comparison_derives_refuse_mod_state_by_declaration_name() {
    for derive in ["PartialEq", "Eq", "PartialOrd", "Ord", "Hash"] {
        let source = format!("#[derive({derive})] struct S {{ value: u32 }}");
        let error = rewrite(&source, "fixture", &[declaration("example")]).unwrap_err();
        assert!(
            error.contains("E_INJECT_FIELD: example:count-slot"),
            "{error}"
        );
        assert!(error.contains(&format!("derive {derive}")), "{error}");
    }
}
