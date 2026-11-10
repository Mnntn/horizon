//! Per-area API surfaces, assembled at their original lexical scopes.
use crate::catalog::Catalog;
use quote::ToTokens;
use std::{collections::BTreeMap, fs, path::Path};
use syn::Item;

pub(crate) const DIRECTORY: &str = "crates/horizon-mod-vocab/src/exports";
const PREFIX: &str = "mod_api_exports_";
pub(crate) const API_DIRECTORY: &str = "crates/horizon-mod-api/src/exports";
const API_PREFIX: &str = "mod_api_facade_exports_";

pub(crate) fn sources(root: &Path) -> Result<BTreeMap<String, String>, String> {
    sources_in(root, DIRECTORY, PREFIX)
}

pub(crate) fn api_sources(root: &Path) -> Result<BTreeMap<String, String>, String> {
    sources_in(root, API_DIRECTORY, API_PREFIX)
}

fn sources_in(
    root: &Path,
    directory: &str,
    prefix: &str,
) -> Result<BTreeMap<String, String>, String> {
    fn collect(
        root: &Path,
        scope: &str,
        items: Vec<Item>,
        out: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        for mut item in items {
            if let Item::Mod(module) = &mut item {
                if let Some((_, items)) = module.content.take() {
                    let next = match (scope, module.ident.to_string().as_str()) {
                        ("root", "__private") => "private".to_owned(),
                        ("private", name @ ("built" | "game" | "reload")) => {
                            format!("private_{name}")
                        }
                        _ => {
                            return Err(format!(
                                "unsupported export scope {scope}::{}",
                                module.ident
                            ));
                        }
                    };
                    if !module.attrs.is_empty() {
                        return Err("export scope containers must not carry attributes".into());
                    }
                    collect(root, &next, items, out)?;
                    continue;
                }
                if scope != "root" {
                    return Err("out-of-line export modules belong at the API root".into());
                }
                let path = root.join(format!("{}.rs", module.ident));
                let path = if path.is_file() {
                    path
                } else {
                    path.with_extension("").join("mod.rs")
                };
                let path = path.to_string_lossy();
                module.attrs.push(syn::parse_quote!(#[path = #path]));
            } else if !matches!(item, Item::Use(_)) {
                return Err("export fragments contain only uses and module declarations".into());
            }
            let code = out.entry(scope.to_owned()).or_default();
            code.push_str(&item.to_token_stream().to_string());
            code.push('\n');
        }
        Ok(())
    }
    let dir = root.join(directory);
    let mut out = [
        "root",
        "private",
        "private_built",
        "private_game",
        "private_reload",
    ]
    .map(|scope| (scope.to_owned(), String::new()))
    .into_iter()
    .collect();
    let mut has_parts = false;
    let mut has_host = false;
    for area in Catalog::modules(&dir).map_err(|e| e.to_string())? {
        let path = dir.join(format!("{area}.rs"));
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        has_host |= file.items.iter().any(|item| matches!(item, Item::Mod(module) if module.ident == "host" && module.content.is_none()));
        has_parts |= file.items.iter().any(|item| matches!(item, Item::Mod(module) if module.ident == "parts" && module.content.is_none()));
        collect(dir.parent().unwrap(), "root", file.items, &mut out)?;
    }
    if has_host {
        // Re-export the generated macro aliases, not the include!-expanded
        // macro_export implementations, at their existing crate-root paths.
        for item in syn::parse_file(&super::host::areas_source(root)?)
            .map_err(|e| e.to_string())?
            .items
        {
            if let Item::Use(item) = item
                && let syn::UseTree::Rename(rename) = item.tree
            {
                let name = rename.rename;
                out.get_mut("root")
                    .unwrap()
                    .push_str(&quote::quote!(#[doc(hidden)] pub use host::#name;).to_string());
            }
        }
    }
    if has_parts {
        out.get_mut("root")
            .unwrap()
            .push_str(&super::parts::trait_exports(root)?);
    }
    Ok(out
        .into_iter()
        .map(|(scope, code)| (format!("{prefix}{scope}.rs"), code))
        .collect())
}

/// Expand the same generated surfaces for source consumers (Host validation and wasm resolution).
pub(crate) fn expand(root: &Path, source: &str) -> Result<String, String> {
    fn items(input: Vec<Item>, sources: &BTreeMap<String, String>) -> Result<Vec<Item>, String> {
        let mut out = Vec::new();
        for mut item in input {
            if let Item::Macro(mac) = &item
                && mac.mac.path.is_ident("include")
                && let Some((_, code)) = sources
                    .iter()
                    .find(|(name, _)| mac.mac.tokens.to_string().contains(name.as_str()))
            {
                out.extend(syn::parse_file(code).map_err(|e| e.to_string())?.items);
                continue;
            }
            if let Item::Mod(module) = &mut item
                && let Some((_, content)) = &mut module.content
            {
                *content = items(std::mem::take(content), sources)?;
            }
            out.push(item);
        }
        Ok(out)
    }
    let parts = source.contains(super::parts::OUTPUT);
    if !source.contains(PREFIX) && !source.contains(API_PREFIX) && !parts {
        return Ok(source.to_owned());
    }
    let mut file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut generated = sources(root)?;
    if source.contains(API_PREFIX) {
        generated.extend(api_sources(root)?);
    }
    if parts {
        generated.insert(super::parts::OUTPUT.to_owned(), super::parts::source(root)?);
    }
    file.items = items(file.items, &generated)?;
    Ok(file.to_token_stream().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_modules_support_file_and_directory_roots() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/export-module-roots-{}",
            std::process::id()
        ));
        let api = root.join("crates/horizon-mod-vocab/src");
        fs::create_dir_all(api.join("exports")).unwrap();
        fs::create_dir_all(api.join("parts")).unwrap();
        fs::write(
            api.join("parts/unions.rs.in"),
            "pub trait Registrar {} pub trait CapabilityProviders {}\n",
        )
        .unwrap();
        fs::write(api.join("exports/parts.rs"), "mod parts;").unwrap();
        for expected in [api.join("parts/mod.rs"), api.join("parts.rs")] {
            fs::write(&expected, "").unwrap();
            let generated = sources(&root).unwrap();
            let file = syn::parse_file(&generated["mod_api_exports_root.rs"]).unwrap();
            let Item::Mod(module) = &file.items[0] else {
                panic!("export lost its module")
            };
            let syn::Meta::NameValue(path) = &module.attrs[0].meta else {
                panic!("module has no path")
            };
            let syn::Expr::Lit(value) = &path.value else {
                panic!("module path is not a literal")
            };
            let syn::Lit::Str(value) = &value.lit else {
                panic!("module path is not a string")
            };
            assert_eq!(Path::new(&value.value()), expected);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_new_area_is_discovered_without_a_shared_list() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/export-discovery-{}",
            std::process::id()
        ));
        let dir = root.join(DIRECTORY);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("zeta.rs"), "pub use crate::zeta::Last;").unwrap();
        let first = sources(&root).unwrap();
        assert!(first["mod_api_exports_root.rs"].contains("Last"));
        fs::write(dir.join("alpha.rs"), "pub use crate::alpha::First; mod __private { mod built { pub use crate::alpha::Copy; } }").unwrap();
        let added = sources(&root).unwrap();
        assert!(
            added["mod_api_exports_root.rs"].find("First")
                < added["mod_api_exports_root.rs"].find("Last")
        );
        assert!(added["mod_api_exports_private_built.rs"].contains("Copy"));
        let expanded = expand(&root, "pub mod built { include!(concat!(\n env!(\"OUT_DIR\"),\n \"/mod_api_exports_private_built.rs\"\n)); }").unwrap();
        let parsed = syn::parse_file(&expanded).unwrap();
        let Item::Mod(module) = &parsed.items[0] else {
            panic!("built scope lost")
        };
        assert!(matches!(
            &module.content.as_ref().unwrap().1[0],
            Item::Use(_)
        ));
        fs::write(dir.join("invalid.rs"), "pub struct Leaked;").unwrap();
        assert!(sources(&root).unwrap_err().contains("only uses"));
        fs::remove_dir_all(root).unwrap();
    }
}
