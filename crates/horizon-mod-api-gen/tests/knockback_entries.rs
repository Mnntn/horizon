//! Execute the real mob entry bodies with native point instrumentation, isolating world state.
use quote::{ToTokens, quote};
use std::{fs, path::Path, process::Command};

#[test]
fn delegated_knockback_entries_apply_both_heads_but_only_one_push() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut modules = proc_macro2::TokenStream::new();
    for (module, path) in [
        ("entries", "mobs/knockback.rs"),
        ("player_entries", "game/mobs/creatures/knockback.rs"),
    ] {
        let source = fs::read_to_string(root.join("crates/horizon-client/src").join(path)).unwrap();
        let mut file = syn::parse_file(&source).unwrap();
        let mut records = proc_macro2::TokenStream::new();
        for item in &mut file.items {
            if let syn::Item::Impl(owner) = item {
                for item in &mut owner.items {
                    if let syn::ImplItem::Fn(function) = item {
                        function.vis = syn::parse_quote!(pub(crate));
                        let name = function.sig.ident.to_string();
                        if !matches!(
                            name.as_str(),
                            "knockback"
                                | "knockback_full"
                                | "knock_back_player"
                                | "knock_back_player_full"
                        ) {
                            continue;
                        }
                        let name = if module == "entries" {
                            format!("client_lib_mobs_knockback_mob_{name}")
                        } else {
                            name
                        };
                        let attribute = quote!(name = #name, raw);
                        let shape = point_shape::shape(attribute.clone(), &function.sig).unwrap();
                        records.extend(shape.raw_items(
                            &function.sig,
                            Some(&owner.self_ty),
                            Some(&owner.generics),
                        ));
                        function
                            .attrs
                            .push(syn::parse_quote!(#[horizon_mod_api_macros::point(#attribute)]));
                    }
                }
            }
        }
        let module = syn::Ident::new(module, proc_macro2::Span::call_site());
        modules.extend(quote!(mod #module { #file #records }));
    }
    let directory = root.join(format!(
        "target/knockback-entry-fixture-{}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(directory.join("Cargo.toml"), format!(
        "[package]\nname = \"knockback-entry-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nmod_api = {{ package = \"horizon-mod-api\", path = {:?} }}\nmod_points = {{ package = \"horizon-mod-points\", path = {:?} }}\nhorizon-mod-api-macros = {{ path = {:?} }}\n",
        root.join("crates/horizon-mod-api"), root.join("crates/horizon-mod-points"), root.join("crates/horizon-mod-api-macros")
    )).unwrap();
    fs::write(
        directory.join("src/main.rs"),
        format!(
            "{}\n{}\n{}",
            include_str!("knockback_fixture/preamble.rs.txt"),
            modules.into_token_stream(),
            include_str!("knockback_fixture/main.rs.txt")
        ),
    )
    .unwrap();
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--offline", "--manifest-path"])
        .arg(directory.join("Cargo.toml"))
        .env(
            "CARGO_TARGET_DIR",
            root.join("target/knockback-fixture-target"),
        )
        .env("CARGO_INCREMENTAL", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(directory).unwrap();
}
