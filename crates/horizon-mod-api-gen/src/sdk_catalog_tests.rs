use super::*;

#[test]
fn coverage_document_is_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source =
        std::fs::read_to_string(root.join("crates/horizon-mod-package/src/manifest.rs")).unwrap();
    let manifest = syn::parse_file(&source).unwrap();
    let api = manifest
        .items
        .into_iter()
        .find_map(|item| {
            let syn::Item::Const(item) = item else {
                return None;
            };
            if item.ident != "API" {
                return None;
            }
            let syn::Expr::Lit(expr) = *item.expr else {
                return None;
            };
            let syn::Lit::Str(value) = expr.lit else {
                return None;
            };
            Some(value.value())
        })
        .expect("manifest API constant");
    let document = sdk_catalog(&root, &api).unwrap();
    let value: Value = serde_json::from_str(&document).unwrap();
    for hook in value["hooks"].as_array().unwrap() {
        assert!(["server", "client", "both"].contains(&hook["side"].as_str().unwrap()));
        assert!(
            ["listen", "cancel", "collect", "replace"]
                .contains(&hook["contract"].as_str().unwrap())
        );
        if !hook["scope"].is_null() {
            assert!(
                ["block", "blockentity", "item", "menu", "fluid"]
                    .contains(&hook["scope"]["kind"].as_str().unwrap())
            );
        }
    }
    assert!(!value["hooks"].as_array().unwrap().is_empty());
    // Regenerate from the same source instead of maintaining a shared fingerprint.
    assert_eq!(document, sdk_catalog(&root, &api).unwrap());
}

#[test]
fn schema_order_and_drift() {
    let hooks = vec![
        json!({"id":"z", "arguments":[{"name":"second","type":"u32"},{"name":"first","type":"bool"}],"output":{"type":"u64"}}),
        json!({"id":"a"}),
    ];
    let original = serialize("7", hooks.clone()).unwrap();
    assert_eq!(original, serialize("7", hooks.clone()).unwrap());
    let document: Value = serde_json::from_str(&original).unwrap();
    assert_eq!(document["schema_version"], 2);
    assert_eq!(document["api_version"], "7");
    assert_eq!(document["hooks"].as_array().unwrap().len(), 2);
    assert_eq!(document["hooks"][0]["id"], "a");
    assert_eq!(document["hooks"][1]["arguments"][0]["name"], "second");
    assert_eq!(document["hooks"][1]["arguments"][1]["type"], "bool");
    assert_eq!(document["hooks"][1]["output"]["type"], "u64");
    let mut changed = hooks;
    changed.push(json!({"id":"new"}));
    let changed = serialize("7", changed).unwrap();
    assert_ne!(original, changed);
    assert_eq!(
        serde_json::from_str::<Value>(&changed).unwrap()["hooks"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
