use super::*;
use crate::{crossing::tests::internal, sites::analysis::Scope};

fn catalog(parameter: &str) -> Vec<Internal> {
    let entry = |path, kind, sig| internal(&format!("horizon_client::game::{path}"), kind, sig);
    vec![
        entry("Game", "struct", "struct Game { child: Child }"),
        entry("Game::child", "field", "Child"),
        entry("Child", "struct", "struct Child {}"),
        entry("Game::tick", "method", "impl Game { fn tick(&mut self) }"),
        entry(
            "Child::step",
            "method",
            &format!("impl Child {{ fn step(&mut self, value: {parameter}) -> u32 }}"),
        ),
        entry("add", "fn", "fn add(value: u32) -> u32"),
        entry("View", "struct", "struct View<'a>(&'a u32);"),
        entry(
            "Child::add",
            "method",
            "impl Child { fn add(value: u32) -> bool }",
        ),
    ]
}

fn refuse(entries: &[Internal], sig: &syn::Signature, call: &str) -> Error {
    let expr = syn::parse_str(call).unwrap();
    let rule = Rule::new(entries);
    let lexical = Scope::scan(sig, None);
    resolve(&expr, &entries[3], sig, &lexical, entries, &rule)
        .err()
        .expect("call must be refused")
}

#[test]
fn site_resolution_walks_fields_and_annotated_parameters_and_path_calls() {
    let entries = catalog("u32");
    let rule = Rule::new(&entries);
    let sig: syn::Signature = syn::parse_quote!(fn tick(&mut self, child: &mut Child));
    let lexical = Scope::scan(&sig, None);
    for call in [
        "self.child.step(7)",
        "child.step(7)",
        "add(7)",
        "horizon_client::game::add(7)",
        "crate::game::add(7)",
        "self::add(7)",
        "super::game::add(7)",
    ] {
        let expr = syn::parse_str(call).unwrap();
        let resolved = resolve(&expr, &entries[3], &sig, &lexical, &entries, &rule).unwrap();
        assert_eq!(resolved.fields[0], "value");
        assert_eq!(resolved.types[0].to_token_stream().to_string(), "u32");
        assert_eq!(resolved.output.to_string(), "u32");
    }
}

#[test]
fn site_resolution_refuses_incompatible_callee_signatures() {
    for (signature, reason) in [
        (
            "impl<T> Child { fn step(&mut self, value: u32) }",
            "generic or bounded",
        ),
        (
            "impl Child { fn step<T>(&mut self, value: u32) }",
            "generic, unsafe",
        ),
        (
            "impl Child { unsafe fn step(&mut self, value: u32) }",
            "generic, unsafe",
        ),
        (
            "impl Child { fn step(self, value: u32) }",
            "by-value receiver",
        ),
        (
            "impl Child { fn step(&mut self, (value,): (u32,)) }",
            "destructured",
        ),
        (
            "impl Child { fn step(&mut self, value: u32) -> &u32 }",
            "borrowed or unresolved",
        ),
        (
            "impl Child { fn step(&mut self, value: u32) -> Option<&u32> }",
            "borrowed or unresolved",
        ),
        (
            "impl Child { fn step(&mut self, value: u32) -> View<'_> }",
            "borrowed or unresolved",
        ),
        (
            "impl Child { fn step(&mut self, value: u32) -> View }",
            "borrowed or unresolved",
        ),
    ] {
        let mut entries = catalog("u32");
        entries[4].signature = signature.into();
        let error = refuse(
            &entries,
            &syn::parse_quote!(fn tick(&mut self)),
            "self.child.step(1)",
        );
        assert!(error.to_string().contains(reason), "{signature}: {error}");
    }
}

#[test]
fn site_resolution_refuses_unresolved_receivers_and_non_record_shapes() {
    for (parameter, call, reason) in [
        (
            "u32",
            "local.step(1)",
            "local has no parameter type annotation",
        ),
        ("u32", "self.missing.step(1)", "unresolved receiver field"),
        ("u32", "self.make().step(1)", "chained calls"),
        ("u32", "self.child.step::<u8>(1)", "generic callee"),
        ("T", "self.child.step(1)", "borrowing or non-Copy"),
        ("&mut u32", "self.child.step(1)", "borrowing or non-Copy"),
        ("u32", "self.child.step(self.x)", "two-phase borrowing"),
        ("u32", "child.step(child.x)", "two-phase borrowing"),
        ("u32", "self.child.step(alias.x)", "two-phase borrowing"),
        ("u32", "self.child.step(opaque!())", "macro-dependent"),
        ("u32", "self.child.step()", "argument count"),
        ("u32", "unknown(1)", "unresolved callee path"),
        ("u32", "<Child>::add(1)", "qualified-self"),
    ] {
        let entries = catalog(parameter);
        let sig = syn::parse_quote!(fn tick(&mut self, child: &mut Child));
        let error = refuse(&entries, &sig, call);
        assert!(error.to_string().contains(reason), "{call}: {error}");
    }
    let sig = syn::parse_quote!(fn tick(&mut self, add: fn(u32) -> bool));
    assert!(
        refuse(&catalog("u32"), &sig, "add(1)")
            .to_string()
            .contains("shadowed")
    );
}
