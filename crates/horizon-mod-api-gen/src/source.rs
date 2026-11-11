//! The game's source tree as the generator walks it: each crate's files with their module paths
//! and the `cfg`s they are compiled under, following `mod` declarations, `#[path]`s and inline
//! modules.

use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{Item, Type};

/// A source file of the game's crates, with its module path and the `cfg`s it is compiled under.
pub(crate) struct GameFile {
    pub(crate) module: String,
    pub(crate) path: PathBuf,
    /// The conditions of the `mod` declarations that lead to it, outermost first.
    pub(crate) cfg: Vec<String>,
}

/// The `cfg` conditions in `attrs`, as written, except `cfg(test)`.
pub(crate) fn conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") && !test_only(std::slice::from_ref(attr)))
        .filter_map(|attr| attr.parse_args::<syn::Meta>().ok())
        .map(|meta| meta.to_token_stream().to_string())
        .collect()
}

/// Whether an item is compiled only into tests, explicitly or by `#[test]`/`#[bench]`.
pub(crate) fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || attr.path().is_ident("bench")
            || (attr.path().is_ident("cfg")
                && matches!(attr.parse_args::<syn::Meta>(), Ok(syn::Meta::Path(path)) if path.is_ident("test")))
    })
}

/// A module file still to read: its module path, file, children's folder and conditions.
type Pending = (String, PathBuf, PathBuf, Vec<String>);

/// The files of the module `module`, read from `path` (its children in `dir`), following `mod`
/// declarations (and `#[path]`s, and inline modules that declare files). With `every_build`, a
/// module behind a `cfg` is left out with everything under it (the points: functions every build
/// compiles); without, it is kept with its conditions (the internals, listed with the builds they
/// exist in). Test-only modules are always left out, including files gated by inner attributes.
pub(crate) fn module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
) -> Result<Vec<GameFile>, String> {
    let mut files = Vec::new();
    let mut pending = vec![(module, path, dir, Vec::new())];
    while let Some((module, path, dir, cfg)) = pending.pop() {
        // A declared module with no file is one another target builds (its file named by a
        // `cfg_attr` path this walk already followed): nothing here compiles from it.
        if !path.is_file() {
            continue;
        }
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file = crate::game_modules::parse(&path, &source)?;
        if test_only(&file.attrs) || (every_build && gated(&file.attrs)) {
            continue;
        }
        let cfg = cfg
            .into_iter()
            .chain(conditions(&file.attrs))
            .collect::<Vec<_>>();
        let here = path
            .parent()
            .expect("a source file has a folder")
            .to_owned();
        children(
            &module,
            &file.items,
            &here,
            &dir,
            &cfg,
            every_build,
            &mut pending,
        );
        files.push(GameFile { module, path, cfg });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Queues the module files `items` declare, looking inside inline modules too: a file module
/// declared in `mod a { mod b; }` lives in `dir/a/b.rs`. A `#[path]` is relative to `here`.
fn children(
    module: &str,
    items: &[Item],
    here: &Path,
    dir: &Path,
    cfg: &[String],
    every_build: bool,
    pending: &mut Vec<Pending>,
) {
    for item in items {
        if let Item::Macro(included) = item
            && included.mac.path.is_ident("include")
            && !test_only(&included.attrs)
            && !(every_build && gated(&included.attrs))
            && let Ok(relative) = syn::parse2::<syn::LitStr>(included.mac.tokens.clone())
        {
            pending.push((
                module.to_owned(),
                here.join(relative.value()),
                dir.to_owned(),
                cfg.to_vec(),
            ));
        }
        let Item::Mod(declared) = item else { continue };
        if test_only(&declared.attrs) || (every_build && gated(&declared.attrs)) {
            continue;
        }
        let name = declared.ident.to_string();
        let child = format!("{module}::{name}");
        let cfg: Vec<String> = cfg
            .iter()
            .cloned()
            .chain(conditions(&declared.attrs))
            .collect();
        let explicit = declared.attrs.iter().find_map(|attr| match &attr.meta {
            syn::Meta::NameValue(value) if attr.path().is_ident("path") => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }) => Some(path.value()),
                _ => None,
            },
            _ => None,
        });
        // `#[cfg_attr(unix, path = "a/unix.rs")]`: each target's file, under its condition.
        let per_target: Vec<(String, String)> = declared
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg_attr"))
            .filter_map(|attr| {
                let args = attr
                    .parse_args_with(
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                    )
                    .ok()?;
                let mut args = args.into_iter();
                let condition = args.next()?;
                let path = args.find_map(|meta| match meta {
                    syn::Meta::NameValue(value) if value.path.is_ident("path") => {
                        match value.value {
                            syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(path),
                                ..
                            }) => Some(path.value()),
                            _ => None,
                        }
                    }
                    _ => None,
                })?;
                Some((condition.to_token_stream().to_string(), path))
            })
            .collect();
        if declared.content.is_none() && explicit.is_none() && !per_target.is_empty() {
            for (condition, relative) in per_target {
                let file = here.join(relative);
                let dir = file
                    .parent()
                    .expect("a module file has a folder")
                    .to_owned();
                let cfg = cfg.iter().cloned().chain([condition]).collect();
                pending.push((child.clone(), file, dir, cfg));
            }
            continue;
        }
        if let Some((_, inner)) = &declared.content {
            // An inline module's children live in its folder, named by its own `#[path]` if any.
            let inner_dir = dir.join(explicit.as_deref().unwrap_or(&name));
            children(
                &child,
                inner,
                &inner_dir,
                &inner_dir,
                &cfg,
                every_build,
                pending,
            );
            continue;
        }
        let (file, child_dir) = match explicit {
            Some(relative) => {
                let file = here.join(relative);
                let dir = file
                    .parent()
                    .expect("a module file has a folder")
                    .to_owned();
                (file, dir)
            }
            None if dir.join(format!("{name}.rs")).is_file() => {
                (dir.join(format!("{name}.rs")), dir.join(&name))
            }
            None => (dir.join(&name).join("mod.rs"), dir.join(&name)),
        };
        pending.push((child, file, child_dir, cfg));
    }
}

/// The files of `horizon_client::game`; see [`module_files`] for `every_build`.
pub(crate) fn game_files(src: &Path, every_build: bool) -> Result<Vec<GameFile>, String> {
    module_files(
        "horizon_client::game".to_owned(),
        src.join("game.rs"),
        src.join("game"),
        every_build,
    )
}

/// The crate `krate`'s package folder and library root, from its manifest.
pub(crate) fn crate_root(root: &Path, krate: &str) -> Result<(PathBuf, PathBuf), String> {
    let package = root.join("crates").join(krate.replace('_', "-"));
    let manifest = package.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
    let lib = manifest
        .get("lib")
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("src/lib.rs");
    let lib = package.join(lib);
    Ok((package, lib))
}

/// The files of the crate `krate` that every build compiles.
pub(crate) fn crate_files(root: &Path, krate: &str) -> Result<Vec<GameFile>, String> {
    let (_, lib) = crate_root(root, krate)?;
    let dir = lib.parent().expect("a source file has a folder").to_owned();
    module_files(krate.to_owned(), lib, dir, true)
}

/// The crates the points' own crate is built on (`horizon-mod-points` and everything under it), by

/// A path without generic arguments or spaces, as listed paths and found ones are compared.
pub(crate) fn plain_path(path: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in path.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            c if depth == 0 && !c.is_whitespace() => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether `attrs` hold a `cfg` (or `cfg_attr`): the item is compiled only in some builds.
pub(crate) fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

/// `r#type` as `type`.
pub(crate) fn unraw(ident: &syn::Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

/// A type as a target names it: lifetimes as `'_`, no spaces.
pub(crate) fn owner(ty: &Type) -> String {
    struct EraseLifetimes;
    impl syn::visit_mut::VisitMut for EraseLifetimes {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            *lifetime = syn::Lifetime::new("'_", lifetime.span());
        }
    }
    let mut ty = ty.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut EraseLifetimes, &mut ty);
    ty.to_token_stream().to_string().replace(' ', "")
}

/// The point functions of one parsed file, with their module paths: in the game module, every
/// eligible function of its inherent `impl`s (a `Game` method only with a `&self`/`&mut self`

/// The byte offset of `at` in `source`, whose line starts are `lines`.
pub(crate) fn offset(source: &str, lines: &[usize], at: proc_macro2::LineColumn) -> usize {
    let line = lines[at.line - 1];
    source[line..]
        .char_indices()
        .nth(at.column)
        .map_or(source.len(), |(i, _)| line + i)
}

/// The staging pass: puts `#[point]` on every point function of the staged copy at `root`, in
/// place, declares each raw form's record and listener types after the item that holds the

#[cfg(test)]
mod tests {
    use super::*;

    /// A module whose file each target names with `cfg_attr(…, path = …)` is read from each
    /// target's file under its condition; it has no default file, and none is needed.
    #[test]
    fn per_target_module_paths_are_followed_and_need_no_default_file() {
        let dir = std::env::temp_dir().join(format!("points-cfg-attr-{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("control")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#[cfg_attr(unix, path = \"control/unix.rs\")]\n#[cfg_attr(windows, path = \"control/windows.rs\")]\nmod platform;\n",
        )
        .unwrap();
        std::fs::write(src.join("control/unix.rs"), "pub fn a() {}").unwrap();
        std::fs::write(src.join("control/windows.rs"), "pub fn b() {}").unwrap();
        let files = module_files("k".into(), src.join("lib.rs"), src.clone(), false).unwrap();
        let mut found: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.module.as_str(),
                    f.path.strip_prefix(&src).unwrap().to_owned(),
                    f.cfg.clone(),
                )
            })
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                ("k", "lib.rs".into(), vec![]),
                (
                    "k::platform",
                    "control/unix.rs".into(),
                    vec!["unix".to_owned()]
                ),
                (
                    "k::platform",
                    "control/windows.rs".into(),
                    vec!["windows".to_owned()]
                ),
            ]
        );
        // Every build compiles neither, so the point walk leaves the module out.
        let every = module_files("k".into(), src.join("lib.rs"), src, true).unwrap();
        assert_eq!(every.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
