use crate::{internals, sites};
use std::path::Path;

/// Production catalog proof; deliberately opt-in because catalog traversal is expensive.
#[test]
#[ignore]
fn production_listed_records() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entries = internals::catalog(&root).unwrap();
    let slots = sites::slots(&root.join(sites::DIRECTORY)).unwrap();
    let mut proof = Vec::new();
    for (file, module, suffix, ordinal, once) in [
        (
            "crates/horizon-client/src/mobs/villager_secondary.rs",
            "horizon_client_lib::mobs::villager_brain::secondary",
            "sense_secondary",
            0,
            false,
        ),
        (
            "crates/horizon-client/src/game/interface/sign_edit.rs",
            "horizon_client::game::sign_edit",
            "draw_sign_editor",
            0,
            false,
        ),
        (
            "crates/horizon-client/src/game/interface/sign_edit.rs",
            "horizon_client::game::sign_edit",
            "draw_sign_editor",
            1,
            false,
        ),
        (
            "crates/horizon-client/src/game/blocks/machines/mod.rs",
            "horizon_client::game::machines",
            "dispense_from",
            0,
            true,
        ),
    ] {
        let declaration = slots
            .calls
            .iter()
            .find(|d| d.target.ends_with(suffix) && d.ordinal == Some(ordinal))
            .unwrap();
        let source = std::fs::read_to_string(root.join(file)).unwrap();
        let (listed, metadata) =
            sites::rewrite::rewrite(&source, module, &entries, std::slice::from_ref(declaration))
                .unwrap();
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].once, once);
        assert_eq!(metadata[0].ctx, declaration.ctx);
        println!("{}", serde_json::to_string_pretty(&metadata).unwrap());
        use quote::ToTokens;
        let name = &metadata[0].name;
        let raw = syn::parse_file(&listed)
            .unwrap()
            .items
            .into_iter()
            .filter(|item| match item {
                syn::Item::Struct(item) => item.ident.to_string().starts_with(&format!("__{name}")),
                syn::Item::Type(item) => item.ident.to_string().starts_with(&format!("__{name}")),
                syn::Item::Impl(item) => item
                    .self_ty
                    .to_token_stream()
                    .to_string()
                    .starts_with(&format!("__{name}")),
                _ => false,
            })
            .map(|item| item.to_token_stream().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!raw.is_empty());
        let enclosing = sites::identity::enclosing(
            &declaration.target,
            &entries,
            &crate::crossing::Rule::new(&entries),
        )
        .unwrap();
        struct Caller<'a> {
            name: &'a str,
            expression: Option<String>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Caller<'_> {
            fn visit_expr_if(&mut self, node: &'ast syn::ExprIf) {
                if matches!(&*node.cond, syn::Expr::MethodCall(load) if load.method == "active_site")
                    && node.cond.to_token_stream().to_string().contains(self.name)
                {
                    assert!(self.expression.is_none());
                    self.expression = Some(node.to_token_stream().to_string());
                } else {
                    syn::visit::visit_expr_if(self, node);
                }
            }
        }
        let mut caller = Caller {
            name,
            expression: None,
        };
        syn::visit::Visit::visit_file(&mut caller, &syn::parse_file(&listed).unwrap());
        proof.push(serde_json::json!({"site": metadata[0], "raw": raw, "enclosing": enclosing, "caller": caller.expression.unwrap()}));
    }
    // The launcher layer consumes these production-generated records, never hand-kept metadata.
    if let Some(path) = std::env::var_os("HORIZON_LISTED_PROOF_OUT") {
        std::fs::write(path, serde_json::to_vec_pretty(&proof).unwrap()).unwrap();
    }
}
