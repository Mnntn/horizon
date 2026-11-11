//! Native hook targets come from definitions, not declaration file names.
use crate::catalog::Catalog;
use quote::ToTokens;
use std::{collections::BTreeMap, fs, path::Path};

fn definitions(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut targets = BTreeMap::new();
    for path in crate::native_hook_files(root)? {
        let area = path.file_stem().unwrap().to_str().unwrap();
        let file = syn::parse_file(&fs::read_to_string(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for item in file.items {
            let syn::Item::Impl(item) = item else {
                continue;
            };
            if item.trait_.is_some() {
                continue;
            }
            let ty = item.self_ty.to_token_stream().to_string().replace(' ', "");

            for member in item.items {
                let syn::ImplItem::Fn(method) = member else {
                    continue;
                };
                let name = method.sig.ident.to_string();
                if !name.starts_with("native_") {
                    continue;
                }
                let target = format!("horizon_client::game::native_hooks::{area}::{ty}::{name}");
                if targets.insert(format!("{ty}::{name}"), target).is_some() {
                    return Err(format!("duplicate native hook definition: {name}"));
                }
            }
        }
    }
    Ok(targets)
}

pub(crate) fn resolve(root: &Path, catalog: &mut Catalog) -> Result<(), String> {
    let targets = definitions(root)?;
    for hook in &mut catalog.hooks {
        if hook
            .target
            .starts_with("horizon_client::game::native_hooks::")
        {
            let tail = hook
                .target
                .strip_prefix("horizon_client::game::native_hooks::")
                .unwrap();
            let name = tail.split("::").skip(1).collect::<Vec<_>>().join("::");
            hook.target = targets
                .get(&name)
                .ok_or_else(|| format!("{}: no native definition", hook.name))?
                .clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_target_resolves_and_missing_function_is_rejected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut catalog = Catalog::load(&root).unwrap();

        let declared = Catalog::load_declarations(&root).unwrap();
        let wrong = declared
            .hooks
            .iter()
            .zip(&catalog.hooks)
            .filter(|(old, new)| old.target != new.target)
            .map(|(old, _)| old.name.as_str())
            .collect::<Vec<_>>();
        println!("{} corrected targets: {}", wrong.len(), wrong.join(", "));
        // Dispatch resolves named game methods and free render functions through the
        // same source parsers used for generation (including crate aliases).
        crate::sources(&root, crate::FillMode::Require).unwrap();
        crate::dispatch::render(&root, &catalog).unwrap();
        let hook = catalog
            .hooks
            .iter_mut()
            .find(|h| h.name == "anvil_craft_post")
            .unwrap();
        assert!(hook.target.contains("::base::"));
        hook.target.push_str("_nonexistent");
        assert!(
            resolve(&root, &mut catalog)
                .unwrap_err()
                .contains("no native definition")
        );
    }
}
