//! Injection points (#589): every function of the game module that [`point_shape::eligible`]
//! admits is a point, and so is each function `slots/*.txt` lists in the crates below it. The staging
//! pass ([`stage`]) puts `#[point]` on each in a staged copy of the game's source, the copy the
//! prebuilt game builds compile, with each raw form's types beside it;
//! the generator lists the same points from any copy, staged or not, as `mod_points` (each point's
//! record, typed function slots, `PointId`) and as `points.json`, the compiled slot catalog. All of them are build outputs, never committed.

use quote::{ToTokens, quote};
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{ImplItem, Item, Type};

pub mod mod_crate;

#[cfg(test)]
mod target_tests;

/// One point, as `points.json` lists it.
#[derive(Debug, serde::Serialize)]
pub struct GamePoint {
    /// Why its API listeners are refused at registration, native and wasm alike: the first
    /// parameter with no API type, naming it. Absent when the point has an API record.
    pub api_refusal: Option<String>,
    pub name: String,
    /// The function, as an injection's `target` names it.
    pub target: String,
    /// The side of a `Game` method's point, which picks its API listeners' host; `none` off `Game`.
    pub side: &'static str,
    /// The API listener's host: `host` (`&self`), the side's writing host (`&mut self`), or `none`.
    pub host: &'static str,
    /// The record its API listeners get, in `mod_points`.
    pub record: String,
    /// The parameters passed to API listeners, with their types, in order.
    pub args: Vec<(String, String)>,
    /// What its listeners may do with the return: `cancel`, `result` (an API-typed value they
    /// may set at the head or change at the return) or `none`.
    pub returns: &'static str,
    /// The function's return type as written (`()` when it has none). A `cancel` point that
    /// returns `Result<(), E>` has no `replace` listener, which would have to produce it.
    pub return_ty: String,
    /// Whether its API listeners run at the return: not when the value returned borrows.
    pub at_return: bool,
    /// Whether it has a raw form: native listeners with the receiver and every parameter by its
    /// own type (`access = "internals"`).
    pub raw: bool,
    /// Full raw listener contract, including engine-typed parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_shape: Option<point_shape::RawShape>,
    /// Its raw listeners' function type, by path: a mod's raw listener has this type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listener: Option<String>,
    /// Its raw return listeners' function type, by path: they get the value returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_listener: Option<String>,
    /// A mod point's signature hash (#2339): what an addon's declaration pins, checked at load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// A mod point's signature as hashed: its path, receiver, parameters and return type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip)]
    record_item: String,
    /// Its typed function constructors, place ids and dispatch functions
    /// ([`point_shape::Shape::api_items`]).
    #[serde(skip)]
    api: [String; 3],
}

impl GamePoint {
    /// The return type a `replace` at this point's head can't produce: none when it returns `()`
    /// (the cancel is forced) or an API value (the listener's). The rule `horizon-mod-point-emit`
    /// applies when a mod builds, kept for the loader of a module built elsewhere.
    fn replace_refusal(&self) -> Option<&str> {
        match self.returns {
            "result" => None,
            "cancel" if self.return_ty == "()" => None,
            _ => Some(&self.return_ty),
        }
    }
}

impl GamePoint {
    /// The point of `found`'s function, whose shape is `shape`, as `points.json` lists it.
    pub(crate) fn new(shape: point_shape::Shape, found: &Found<'_>, target: String) -> Self {
        let api = shape.api_items(&target);
        GamePoint {
            record_item: if shape.api() {
                shape.record_item(&target).to_string()
            } else {
                String::new()
            },
            api: [
                api.listeners.to_string(),
                api.variants.to_string(),
                api.dispatch.to_string(),
            ],
            at_return: shape.api() && shape.has_return(),
            raw: shape.raw,
            api_refusal: shape.api_refusal.clone(),
            raw_shape: shape
                .raw
                .then(|| point_shape::raw_shape(&shape, found.sig, found.owner.map(|(ty, _)| ty))),
            listener: shape
                .raw
                .then(|| format!("{}::{}", found.module, shape.raw_listener())),
            return_listener: shape
                .raw
                .then(|| format!("{}::{}", found.module, shape.raw_return_listener())),
            target,
            side: shape.side.map_or("none", point_shape::Side::name),
            host: match (shape.receiver, shape.side) {
                (_, None) => "none",
                (point_shape::Receiver::Mut, Some(side)) => side.name(),
                (_, Some(_)) => "host",
            },
            args: shape
                .args
                .iter()
                .map(|arg| {
                    (
                        arg.name.to_string(),
                        arg.ty.to_token_stream().to_string().replace("& ", "&"),
                    )
                })
                .collect(),
            record: shape.record().to_string(),
            returns: match (&shape.returns, shape.api()) {
                (_, false) => "none",
                (point_shape::Returns::Unit | point_shape::Returns::UnitResult, true) => "cancel",
                (point_shape::Returns::Value(_), true) => "result",
                (point_shape::Returns::Opaque, true) => "none",
            },
            return_ty: match &found.sig.output {
                syn::ReturnType::Default => "()".to_owned(),
                syn::ReturnType::Type(_, ty) => point_shape::spelling(ty),
            },
            name: shape.name,
            hash: None,
            signature: None,
        }
    }
}

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
    let files = module_files(
        "horizon_client::game".to_owned(),
        src.join("game.rs"),
        src.join("game"),
        every_build,
    )?;
    #[cfg(feature = "listener-fixture")]
    let files = {
        let mut files = files;
        let path = src.join("../../horizon-mod-wasm/tests/fixture/src/points.rs");
        if path.is_file() {
            files.push(GameFile {
                module: "listener_fixture".into(),
                path,
                cfg: Vec::new(),
            });
        }
        files
    };
    Ok(files)
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
fn crate_files(root: &Path, krate: &str) -> Result<Vec<GameFile>, String> {
    let (_, lib) = crate_root(root, krate)?;
    let dir = lib.parent().expect("a source file has a folder").to_owned();
    module_files(krate.to_owned(), lib, dir, true)
}

/// The crates the points' own crate is built on (`horizon-mod-points` and everything under it), by
/// library name: their functions can't be points, since their slots would have to reach up to it.
pub(crate) fn below_points(root: &Path) -> Result<std::collections::BTreeSet<String>, String> {
    let mut seen = std::collections::BTreeSet::from(["horizon_mod_points".to_owned()]);
    let mut pending = vec![root.join("crates/horizon-mod-points")];
    while let Some(package) = pending.pop() {
        let manifest = package.join("Cargo.toml");
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let manifest: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
        // Build dependencies too: `horizon-mod-points` is generated by `horizon-mod-api-gen`.
        for path in ["dependencies", "build-dependencies"]
            .into_iter()
            .filter_map(|kind| manifest.get(kind).and_then(toml::Value::as_table))
            .flatten()
            .filter_map(|(_, dep)| dep.get("path").and_then(toml::Value::as_str))
        {
            let dependency = package.join(path);
            let name = dependency
                .file_name()
                .map(|name| name.to_string_lossy().replace('-', "_"))
                .unwrap_or_default();
            if seen.insert(name) {
                pending.push(dependency);
            }
        }
    }
    Ok(seen)
}

/// Listed lower-crate functions and their diagnostic locations.
fn listed(root: &Path) -> Result<Vec<(String, String)>, String> {
    Ok(crate::sites::slots(&root.join(crate::sites::DIRECTORY))
        .map_err(|e| e.to_string())?
        .functions
        .into_iter()
        .map(|(path, origin)| (origin, path))
        .collect())
}

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

/// The game module's files, slot lists and listed crates' files, for a build script to track.
pub fn tracked(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<_> = game_files(&root.join("crates/horizon-client/src"), true)?
        .into_iter()
        .map(|file| file.path)
        .collect();
    let directory = root.join(crate::sites::DIRECTORY);
    files.push(directory.clone());
    if directory.exists() {
        files.extend(
            fs::read_dir(directory)
                .map_err(|e| e.to_string())?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?,
        );
    }
    let crates: std::collections::BTreeSet<_> = listed(root)?
        .into_iter()
        .filter_map(|(_, path)| path.split("::").next().map(str::to_owned))
        .collect();
    for krate in crates {
        files.extend(crate_files(root, &krate)?.into_iter().map(|file| file.path));
    }
    Ok(files)
}

/// Whether `attrs` hold a `cfg` (or `cfg_attr`): the item is compiled only in some builds.
fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

/// A point's function, as found in a parsed file.
pub(crate) struct Found<'a> {
    pub(crate) module: String,
    attrs: &'a [syn::Attribute],
    vis: &'a syn::Visibility,
    pub(crate) sig: &'a syn::Signature,
    pub(crate) body: &'a syn::Block,
    /// The `impl`'s type and generics; none for a free function.
    owner: Option<(&'a Type, &'a syn::Generics)>,
    /// Where the item that holds it ends: the `impl` block, or the function. Its raw types go there.
    pub(crate) end: proc_macro2::LineColumn,
    /// The types its signature names that cross as `mod_api` types, as written, with the API
    /// type's path and whether it converts (an engine type's [`MIRRORS`] entry).
    mirrors: Vec<(String, String, bool)>,
}

impl Found<'_> {
    /// A method of `Game`, taking `self`: its point keeps the method's name and gets the game as
    /// its host. `Game`'s associated functions without `self` are named and hosted like any other
    /// type's.
    fn on_game(&self) -> bool {
        matches!(self.sig.inputs.first(), Some(syn::FnArg::Receiver(_)))
            && self.owner.is_some_and(|(ty, _)| matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 && path.path.segments[0].ident == "Game"))
    }

    /// Its path in its crate, as an injection's `target` names it.
    pub(crate) fn target(&self) -> String {
        match self.owner {
            Some((ty, _)) => format!("{}::{}::{}", self.module, owner(ty), self.sig.ident),
            None => format!("{}::{}", self.module, self.sig.ident),
        }
    }

    /// Its point's name: a `Game` method's own name; else its path from the game module (`game_…`),
    /// or in a lower crate from its crate's name less `horizon_`, snake-cased. The form is not
    /// injective (`a::b_c` and `a_b::c` collide): `all_points` refuses two points of one name.
    fn name(&self) -> String {
        if self.on_game() {
            return unraw(&self.sig.ident);
        }
        let module = match self.module.strip_prefix("horizon_client::") {
            Some(rest) => rest.to_owned(),
            None => self
                .module
                .strip_prefix("horizon_")
                .unwrap_or(&self.module)
                .to_owned(),
        };
        let mut parts: Vec<String> = module
            .split("::")
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        if let Some((Type::Path(path), _)) = self.owner
            && let Some(last) = path.path.segments.last()
        {
            parts.push(snake(&unraw(&last.ident)));
        }
        parts.push(unraw(&self.sig.ident));
        parts.join("_")
    }

    /// The attribute the pass writes on it.
    fn attribute(&self) -> String {
        let mut parts = Vec::new();
        if self.on_game() {
            parts.push("side = \"both\"".to_owned());
        } else {
            parts.push(format!("name = \"{}\"", self.name()));
        }
        if self.raw() {
            parts.push("raw".into());
        }
        if self.owner.is_none() {
            parts.push("free".into());
        }
        for (word, convert) in [("api", false), ("mirror", true)] {
            let named: Vec<_> = self
                .mirrors
                .iter()
                .filter(|(_, _, converts)| *converts == convert)
                .map(|(written, api, _)| format!("{written} = {api:?}"))
                .collect();
            if !named.is_empty() {
                parts.push(format!("{word}({})", named.join(", ")));
            }
        }
        parts.join(", ")
    }

    fn raw(&self) -> bool {
        point_shape::raw_able(self.sig, self.owner.map(|(_, generics)| generics))
    }

    fn shape(&self) -> Result<point_shape::Shape, String> {
        point_shape::shape(self.attribute().parse().expect("the attribute"), self.sig)
            .map_err(|e| format!("{}: {e}", self.target()))
    }
}

/// `r#type` as `type`.
pub(crate) fn unraw(ident: &syn::Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

/// `MapData` as `map_data`.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A type as a target names it: lifetimes as `'_`, no spaces.
fn owner(ty: &Type) -> String {
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
/// receiver) and its free functions; elsewhere, those `keep` admits. Inline modules every build
/// compiles count too.
pub(crate) fn file_points<'a>(
    module: &str,
    items: &'a [Item],
    keep: &dyn Fn(&Found<'_>) -> bool,
    out: &mut Vec<Found<'a>>,
) {
    file_functions(module, items, keep, &point_shape::eligible, out);
}

pub(crate) fn file_functions<'a>(
    module: &str,
    items: &'a [Item],
    keep: &dyn Fn(&Found<'_>) -> bool,
    eligible: &dyn Fn(&[syn::Attribute], &syn::Signature) -> bool,
    out: &mut Vec<Found<'a>>,
) {
    for item in items {
        match item {
            Item::Mod(inline) if !gated(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content {
                    file_functions(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        keep,
                        eligible,
                        out,
                    );
                }
            }
            Item::Fn(function)
                if !gated(&function.attrs) && eligible(&function.attrs, &function.sig) =>
            {
                let found = Found {
                    module: module.to_owned(),
                    attrs: &function.attrs,
                    vis: &function.vis,
                    sig: &function.sig,
                    body: &function.block,
                    owner: None,
                    end: function.block.brace_token.span.close().end(),
                    mirrors: Vec::new(),
                };
                if keep(&found) {
                    out.push(found);
                }
            }
            Item::Impl(block) if block.trait_.is_none() && !gated(&block.attrs) => {
                let Type::Path(ty) = &*block.self_ty else {
                    continue;
                };
                if ty.qself.is_some() {
                    continue;
                }
                for member in &block.items {
                    if let ImplItem::Fn(method) = member
                        && eligible(&method.attrs, &method.sig)
                    {
                        let found = Found {
                            module: module.to_owned(),
                            attrs: &method.attrs,
                            vis: &method.vis,
                            sig: &method.sig,
                            body: &method.block,
                            owner: Some((&block.self_ty, &block.generics)),
                            end: block.brace_token.span.close().end(),
                            mirrors: Vec::new(),
                        };
                        if keep(&found) {
                            out.push(found);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// A parsed source file of the game's crates, with its points.
struct Scanned {
    path: PathBuf,
    module: String,
    source: String,
    parsed: syn::File,
    /// The crate's package, when it is a lower crate: the pass gives it the points' dependencies.
    package: Option<PathBuf>,
}

/// The `mod_api` types a record may hold ([`crate::build::wire_types`]), by path.
pub type WireTypes = std::collections::BTreeSet<String>;

/// Every file with points: the game module's, then each listed lower crate's. Fails on a listed
/// path that names no point function, or a crate the points' own crate is built on.
fn scan(root: &Path) -> Result<(Vec<Scanned>, Vec<(String, String)>), String> {
    let mut files = Vec::new();
    for file in game_files(&root.join("crates/horizon-client/src"), true)? {
        files.push((file, None));
    }
    let list = listed(root)?;
    let below = below_points(root)?;
    let mut crates = std::collections::BTreeSet::new();
    for (origin, path) in &list {
        let krate = path.split("::").next().unwrap_or_default();
        if below.contains(krate) || krate == "horizon_client" {
            return Err(format!(
                "{origin}: {path}: {krate} is built under the points' own crate (or is the game's): its functions can't be listed points"
            ));
        }
        crates.insert(krate.to_owned());
    }
    for krate in &crates {
        let (package, _) = crate_root(root, krate)?;
        for file in crate_files(root, krate)? {
            files.push((file, Some(package.clone())));
        }
    }
    let mut scanned = Vec::new();
    for (file, package) in files {
        let source = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let parsed = crate::interface_cache::parse(&file.path, &source)?;
        scanned.push(Scanned {
            path: file.path,
            module: file.module,
            source,
            parsed,
            package,
        });
    }
    Ok((scanned, list))
}

impl Scanned {
    /// This file's points: all of a game-module file's, or the listed ones of a lower crate's.
    fn points<'a>(
        &'a self,
        list: &[(String, String)],
        wire: &WireTypes,
        used: &mut Vec<bool>,
    ) -> Vec<Found<'a>> {
        let mut out = Vec::new();
        let used = std::cell::RefCell::new(used);
        let keep = |found: &Found<'_>| {
            if self.package.is_none() {
                return true;
            }
            let target = plain_path(&found.target());
            match list.iter().position(|(_, path)| *path == target) {
                Some(index) => {
                    used.borrow_mut()[index] = true;
                    true
                }
                None => false,
            }
        };
        file_points(&self.module, &self.parsed.items, &keep, &mut out);
        let uses = imports(&self.parsed.items, "horizon_client");
        for found in &mut out {
            // Only the game's own crate has the conversions a mirror goes through.
            found.mirrors = mirrors(found.sig, &uses, wire, self.package.is_none());
        }
        out
    }
}

/// The engine types a point carries as their `mod_api` mirrors, by full path, and the mirror. The
/// game converts each through its `TryIntoApi` and `TryIntoEngine` (`game::points`).
pub const MIRRORS: &[(&str, &str)] = &[
    ("horizon_client::text::Text", "::mod_api::Text"),
    ("horizon_client_core::text::Text", "::mod_api::Text"),
    (
        "horizon_world::registry::BlockStateId",
        "::mod_api::BlockState",
    ),
];

/// A file's top-level `use`s: each name it brings in, with the path it names (`crate::` as
/// `krate`). Globs and `self`/`super` paths are left out: a type they name has no mirror.
pub(crate) fn imports(items: &[Item], krate: &str) -> Vec<(String, String)> {
    fn walk(prefix: &str, tree: &syn::UseTree, out: &mut Vec<(String, String)>) {
        let join = |name: &str| {
            if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}::{name}")
            }
        };
        match tree {
            syn::UseTree::Path(path) => walk(&join(&path.ident.to_string()), &path.tree, out),
            syn::UseTree::Name(name) => {
                out.push((name.ident.to_string(), join(&name.ident.to_string())))
            }
            syn::UseTree::Rename(rename) => {
                out.push((rename.rename.to_string(), join(&rename.ident.to_string())))
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    walk(prefix, tree, out);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = Vec::new();
    for item in items {
        if let Item::Use(item) = item {
            walk("", &item.tree, &mut out);
        }
    }
    for (_, path) in &mut out {
        if let Some(rest) = path.strip_prefix("crate::") {
            *path = format!("{krate}::{rest}");
        }
    }
    out
}

/// The leaves of `sig` that name a `mod_api` type in `wire`, or (with `convert`) a [`MIRRORS`]
/// type, through the file's `imports` or in full.
fn mirrors(
    sig: &syn::Signature,
    imports: &[(String, String)],
    wire: &WireTypes,
    convert: bool,
) -> Vec<(String, String, bool)> {
    let mut found: Vec<(String, String, bool)> = Vec::new();
    for leaf in point_shape::leaves(sig) {
        let written = point_shape::spelling(&leaf);
        let full = match leaf.get_ident() {
            Some(ident) => match imports.iter().find(|(name, _)| ident == name) {
                Some((_, path)) => path.clone(),
                None => continue,
            },
            None => written
                .trim_start_matches("::")
                .replace("crate::", "horizon_client::"),
        };
        if found.iter().any(|(seen, _, _)| *seen == written) {
            continue;
        }
        if let Some(api) = wire_type(wire, &full) {
            found.push((written, format!("::{api}"), false));
        } else if let Some((_, api)) = MIRRORS.iter().find(|(path, _)| *path == full)
            && convert
        {
            found.push((written, (*api).to_owned(), true));
        }
    }
    found
}

/// The path `wire` lays out the `mod_api` type at `full` under: the same path, or the one type of
/// that name re-exported elsewhere in `mod_api`.
fn wire_type<'w>(wire: &'w WireTypes, full: &str) -> Option<&'w str> {
    if !full.starts_with("mod_api::") {
        return None;
    }
    if let Some(found) = wire.get(full) {
        return Some(found);
    }
    let name = full.rsplit("::").next()?;
    let mut named = wire
        .iter()
        .filter(|path| path.rsplit("::").next() == Some(name));
    match (named.next(), named.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Every point, scanned once: each with its file's index.
fn all_points<'a>(
    scanned: &'a [Scanned],
    list: &[(String, String)],
    wire: &WireTypes,
) -> Result<Vec<(usize, Found<'a>)>, String> {
    let mut used = vec![false; list.len()];
    let mut points = Vec::new();
    for (index, file) in scanned.iter().enumerate() {
        points.extend(
            file.points(list, wire, &mut used)
                .into_iter()
                .map(|found| (index, found)),
        );
    }
    if let Some(((origin, path), _)) = list.iter().zip(&used).find(|(_, used)| !**used) {
        return Err(format!(
            "{origin}: {path}: no function every build compiles that can be a point"
        ));
    }
    let mut names = std::collections::BTreeMap::new();
    for (_, found) in &points {
        let shape = found.shape()?;
        for variant in [shape.variant(), shape.return_variant()] {
            if let Some(other) = names.insert(variant.to_string(), found.target()) {
                return Err(format!(
                    "the points of {other} and {} have one name, {}: rename one",
                    found.target(),
                    shape.name
                ));
            }
        }
    }
    Ok(points)
}

/// The game's points, by name: every point function of the game module, and the listed ones of
/// the crates below it. Unlisted targets have no compiled slot.
pub fn game_points(root: &Path) -> Result<Vec<GamePoint>, String> {
    points_with(root, &crate::build::wire_types(root)?)
}

/// [`game_points`] with the `mod_api` types a record may hold.
pub(crate) fn points_with(root: &Path, wire: &WireTypes) -> Result<Vec<GamePoint>, String> {
    let (scanned, list) = scan(root)?;
    let mut points = Vec::new();
    for (_, found) in all_points(&scanned, &list, wire)? {
        let target = found.target();

        let shape = found.shape()?;
        points.push(GamePoint::new(shape, &found, target));
    }
    points.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(points)
}

/// What [`stage`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Points given a slot.
    pub points: usize,
    /// Of those, the ones whose API listeners get no return place (their value borrows).
    pub head_only: usize,
    /// Of those, the ones with a raw form.
    pub raw: usize,
    /// Files rewritten.
    pub files: usize,
}

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
/// function, and gives each lower crate with points the points' dependencies. Run on a copy, never
/// on a working tree: the source tree carries no `#[point]`.
pub fn stage(root: &Path) -> Result<Staged, String> {
    stage_with(root, &crate::build::wire_types(root)?)
}

/// [`stage`] with the `mod_api` types a record may hold.
pub(crate) fn stage_with(root: &Path, wire: &WireTypes) -> Result<Staged, String> {
    let (scanned, list) = scan(root)?;
    let points = all_points(&scanned, &list, wire)?;
    let mut staged = Staged::default();
    let mut packages = std::collections::BTreeSet::new();
    for (index, file) in scanned.iter().enumerate() {
        let here: Vec<_> = points
            .iter()
            .filter(|(i, _)| *i == index)
            .map(|(_, found)| found)
            .collect();
        if here.is_empty() {
            continue;
        }
        let source = &file.source;
        let lines: Vec<usize> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        // Insertions at byte offsets, in order; at one offset, in the order pushed.
        let mut edits: Vec<(usize, String)> = Vec::new();
        for found in &here {
            if let Some(attr) = found.attrs.iter().find(|attr| {
                attr.path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "point")
            }) {
                return Err(format!(
                    "{}: {} already has {}: the source carries no #[point]",
                    file.path.display(),
                    found.sig.ident,
                    attr.to_token_stream()
                ));
            }
            let start = match found.vis {
                syn::Visibility::Inherited => found
                    .sig
                    .constness
                    .map(|token| token.span)
                    .unwrap_or(found.sig.fn_token.span)
                    .start(),
                vis => syn::spanned::Spanned::span(vis).start(),
            };
            edits.push((
                offset(source, &lines, start),
                format!("#[horizon_mod_api_macros::point({})] ", found.attribute()),
            ));
            let shape = found.shape()?;
            staged.points += 1;
            staged.head_only += usize::from(!shape.has_return());
            if shape.raw {
                staged.raw += 1;
                let items = shape.raw_items(
                    found.sig,
                    found.owner.map(|(ty, _)| ty),
                    found.owner.map(|(_, generics)| generics),
                );
                edits.push((offset(source, &lines, found.end), format!("\n{items}\n")));
            }
        }
        edits.sort_by_key(|(at, _)| *at);
        let mut out = String::with_capacity(
            source.len() + edits.iter().map(|(_, text)| text.len()).sum::<usize>(),
        );
        let mut from = 0;
        for (at, text) in edits {
            out.push_str(&source[from..at]);
            out.push_str(&text);
            from = at;
        }
        out.push_str(&source[from..]);
        fs::write(&file.path, out).map_err(|e| format!("{}: {e}", file.path.display()))?;
        staged.files += 1;
        packages.insert(
            file.package
                .clone()
                .unwrap_or_else(|| root.join("crates/horizon-client")),
        );
    }
    dependencies(root, packages)?;
    Ok(staged)
}

/// Give listed lower-crate points and sites their staged dependencies, including the lock file.
pub(crate) fn dependencies(
    root: &Path,
    packages: std::collections::BTreeSet<PathBuf>,
) -> Result<(), String> {
    const ADDED: [&str; 3] = [
        "horizon-mod-points",
        "horizon-mod-api-macros",
        "horizon-mod-api",
    ];
    if packages.is_empty() {
        return Ok(());
    }
    let lock_path = root.join("Cargo.lock");
    // A Windows checkout has CRLF line endings; Cargo reads either, and the entries match on LF.
    let mut lock = fs::read_to_string(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?
        .replace("\r\n", "\n");
    for package in packages {
        let manifest = package.join("Cargo.toml");
        let text =
            fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
        let mut cargo: toml::Table = toml::from_str(&text).map_err(|e| e.to_string())?;
        let dependencies = cargo
            .entry("dependencies")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or("[dependencies] is not a table")?;
        // Every package with points sits in `crates/`, beside these two.
        for name in ADDED {
            dependencies.entry(name).or_insert_with(|| {
                toml::Value::Table(toml::Table::from_iter([(
                    "path".to_owned(),
                    format!("../{name}").into(),
                )]))
            });
        }
        let name = cargo
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{}: no package name", manifest.display()))?
            .to_owned();
        lock = locked_with(&lock, &name, &ADDED)
            .ok_or_else(|| format!("Cargo.lock has no entry for {name}"))?;
        fs::write(
            &manifest,
            toml::to_string(&cargo).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    }
    fs::write(&lock_path, lock).map_err(|e| format!("{}: {e}", lock_path.display()))?;
    Ok(())
}

/// `lock` with `added` in the dependency list of the package `name`, kept in the lock file's
/// sorted order; none when the lock file has no entry for it.
fn locked_with(lock: &str, name: &str, added: &[&str]) -> Option<String> {
    let header = format!("[[package]]\nname = \"{name}\"\n");
    let start = lock.find(&header)?;
    let end = lock[start + header.len()..]
        .find("\n\n")
        .map_or(lock.len(), |at| start + header.len() + at);
    let entry = &lock[start..end];
    let mut lines: Vec<String> = entry.lines().map(str::to_owned).collect();
    let mut dependencies: Vec<String> = Vec::new();
    let open = lines.iter().position(|line| line == "dependencies = [");
    if let Some(open) = open {
        let close = open + lines[open..].iter().position(|line| line == "]")?;
        dependencies = lines.drain(open + 1..close).collect();
    }
    for name in added {
        let quoted = format!(" \"{name}\",");
        if !dependencies.contains(&quoted) {
            dependencies.push(quoted);
        }
    }
    dependencies.sort();
    let open = match open {
        Some(open) => open,
        None => {
            // `dependencies` follows `version` and `source`, before `checksum`.
            let at = lines
                .iter()
                .position(|line| line.starts_with("checksum"))
                .unwrap_or(lines.len());
            lines.insert(at, "dependencies = [".to_owned());
            lines.insert(at + 1, "]".to_owned());
            at
        }
    };
    for (offset, dependency) in dependencies.into_iter().enumerate() {
        lines.insert(open + 1 + offset, dependency);
    }
    Some(format!(
        "{}{}{}",
        &lock[..start],
        lines.join("\n"),
        &lock[end..]
    ))
}

/// `mod_points`: records, typed function constructors, dispatchers and place ids.
#[cfg(test)]
pub(crate) fn records(points: &[GamePoint]) -> String {
    records_selected(points, None)
}

pub(crate) fn records_selected(
    points: &[GamePoint],
    selected: Option<&std::collections::BTreeSet<String>>,
) -> String {
    let enabled = |point: &GamePoint| {
        selected.is_none_or(|names| {
            names.contains(&point.name) || names.contains(&format!("{}_return", point.name))
        })
    };
    let parse = |code: &str| {
        code.parse::<proc_macro2::TokenStream>()
            .expect("generated points")
    };
    let records = points.iter().map(|point| {
        if enabled(point) {
            return parse(&point.record_item);
        }
        let mut file: syn::File = syn::parse_str(&point.record_item).expect("point record");
        // Keep every record's public shape, but only selected records need a guest codec.
        for item in &mut file.items {
            if let syn::Item::Struct(record) = item {
                record.attrs.retain(|attr| {
                    !(attr.path().is_ident("cfg_attr")
                        && attr
                            .to_token_stream()
                            .to_string()
                            .contains("derive (:: mod_api :: Wire)"))
                });
            }
        }
        quote!(#file)
    });
    let listeners = points.iter().map(|point| parse(&point.api[0]));
    let variants: Vec<_> = points.iter().map(|point| parse(&point.api[1])).collect();
    let dispatch = points.iter().map(|point| parse(&point.api[2]));
    let mut guest_functions = proc_macro2::TokenStream::new();
    let mut guest_entries = proc_macro2::TokenStream::new();
    let mut api_places = proc_macro2::TokenStream::new();
    let mut layout = 0xcbf29ce484222325u64;
    for point in points {
        for byte in format!(
            "{}:{}:{}:{}:{:?}",
            point.name, point.host, point.record_item, point.at_return, point.api_refusal
        )
        .bytes()
        {
            layout = (layout ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        if point.api_refusal.is_some() {
            continue;
        }
        let record = quote::format_ident!("{}", point.record);
        let passed = match point.host {
            "host" => quote!(&host,),
            "server" | "client" => quote!(&mut host,),
            "both" => quote!(if server {
                ::mod_api::HostSide::Server(&mut host)
            } else {
                ::mod_api::HostSide::Client(&mut host)
            },),
            _ => quote!(),
        };
        let function = quote::format_ident!("point_{}", point.name);
        let listener = quote::format_ident!("{}Listener", point_shape::camel(&point.name));
        let side = (point.host == "both").then(|| quote!(let server: bool = input.take()?;));
        if enabled(point) {
            guest_functions.extend(quote! {
            fn #function(place: PointId, input: &mut ::mod_api::wire::Reader<'_>, out: &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError> {
                let listener: u32 = input.take()?;
                #side
                let mut args: #record = input.take()?;
                let mut host = ::mod_api::wire::guest::Guest;
                crate::guest::fire(place, listener, |function| {
                    // SAFETY: head and return have the same checked host and record type.
                    let function = unsafe { ::std::mem::transmute::<*const (), listeners::#listener>(function) };
                    function(#passed &mut args);
                })?;
                out.put(&());
                out.put(&args);
                Ok(())
            }
        });
        }
        for name in std::iter::once(point.name.clone())
            .chain(point.at_return.then(|| format!("{}_return", point.name)))
        {
            let variant = quote::format_ident!("{}", point_shape::camel(&name));
            let target = &point.target;
            let at_return = name != point.name;
            let replace_refusal = match point.replace_refusal().filter(|_| !at_return) {
                Some(ty) => quote!(::std::option::Option::Some(#ty)),
                None => quote!(::std::option::Option::None),
            };
            api_places
                .extend(quote! { (PointId::#variant, #target, #at_return, #replace_refusal), });
            if selected.is_some_and(|names| !names.contains(&name)) {
                continue;
            }
            guest_entries.extend(quote! {
                table[PointId::#variant as usize] = Some((PointId::#variant, #function as Dispatch));
            });
        }
    }
    let ids: syn::ItemEnum = syn::parse2(quote! {
        enum PointId { #(#variants)* __Count }
    })
    .expect("point variants");
    let debug_names = ids.variants.iter().map(|variant| variant.ident.to_string());
    quote! {
        #(#records)*
        /// The generated point export table and record layout, checked separately from the API layout.
        #[doc(hidden)]
        pub const POINT_LAYOUT: u64 = #layout;
        /// API places as (place, target function, return phase, the return type a `replace` there
        /// can't produce), for manifest validation: a head whose return is neither `()` nor an
        /// API value refuses a replacement when a module loads.
        #[doc(hidden)]
        pub const API_PLACES: &[(PointId, &str, bool, Option<&str>)] = &[#api_places];
        #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
        #[doc(hidden)]
        pub fn guest_dispatch(id: u32, input: &mut ::mod_api::wire::Reader<'_>, out: &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError> {
            if id == 0xBFFF_FFFF { out.put(&crate::guest::subscriptions()); return Ok(()); }
            let index = (id & 0x3FFF_FFFF) as usize;
            let (place, function) = guest_exports::TABLE.get(index).copied().flatten().ok_or(::mod_api::wire::WireError::Tag(id))?;
            function(place, input, out)
        }
        #[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
        #[allow(unused_mut, unused_variables)]
        mod guest_exports {
            use super::*;
            type Dispatch = fn(PointId, &mut ::mod_api::wire::Reader<'_>, &mut ::mod_api::wire::Writer) -> Result<(), ::mod_api::wire::WireError>;
            pub(super) const TABLE: [Option<(PointId, Dispatch)>; POINT_COUNT] = {
                let mut table = [None; POINT_COUNT];
                #guest_entries
                table
            };
            #guest_functions
        }
        /// A point's place: its head and return in catalog order.
        #[doc(hidden)]
        #[repr(usize)]
        #[derive(Clone, Copy, PartialEq, Eq)]
        #[non_exhaustive]
        pub enum PointId { #(#variants)* __Count }
        // Deriving Debug creates an enormous match for this enum. Keep the exact
        // unit-variant names without its compiler-memory cost.
        impl ::std::fmt::Debug for PointId {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                static NAMES: [&str; PointId::__Count as usize + 1] = [#(#debug_names),*];
                formatter.write_str(NAMES[*self as usize])
            }
        }
        /// The number of places, the length of the game's slot table.
        #[doc(hidden)]
        pub const POINT_COUNT: usize = PointId::__Count as usize;
        /// Typed API listener aliases and constructors, generated from the point catalog.
        pub mod listeners { use super::*; #(#listeners)* }
        /// The game's typed dispatch, not part of the API a mod is written against.
        #[doc(hidden)]
        pub mod dispatch { use super::*; #(#dispatch)* }
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_guest_exports_keep_the_full_layout() {
        let make = |name: &str| {
            let sig: syn::Signature = syn::parse_str(&format!("fn {name}()")).unwrap();
            let shape = point_shape::shape(quote!(), &sig).unwrap();
            let api = shape.api_items(name);
            GamePoint {
                api_refusal: None,
                name: name.into(),
                target: name.into(),
                side: "none",
                host: "none",
                record: shape.record().to_string(),
                args: vec![],
                returns: "cancel",
                return_ty: "()".into(),
                at_return: true,
                raw: false,
                raw_shape: None,
                listener: None,
                return_listener: None,
                hash: None,
                signature: None,
                record_item: shape.record_item(name).to_string(),
                api: [
                    api.listeners.to_string(),
                    api.variants.to_string(),
                    api.dispatch.to_string(),
                ],
            }
        };
        let points = [make("first"), make("second")];
        let selected = std::collections::BTreeSet::from(["first_return".to_string()]);
        let all = records(&points);
        let selected = records_selected(&points, Some(&selected));
        let layout = |source: &str| {
            syn::parse_file(source)
                .unwrap()
                .items
                .into_iter()
                .find_map(|item| match item {
                    syn::Item::Const(c) if c.ident == "POINT_LAYOUT" => {
                        Some(c.expr.to_token_stream().to_string())
                    }
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(layout(&all), layout(&selected));
        let file = syn::parse_file(&selected).unwrap();
        let wires = file.items.iter().filter(|item| matches!(item, syn::Item::Struct(record) if record.attrs.iter().any(|attr| attr.path().is_ident("cfg_attr") && attr.to_token_stream().to_string().contains("derive (:: mod_api :: Wire)")))).count();
        assert_eq!(wires, 1);
        assert!(selected.contains("fn point_first"));
        assert!(!selected.contains("fn point_second"));
        assert!(selected.contains("table [PointId :: FirstReturn as usize]"));
        assert!(!selected.contains("table [PointId :: First as usize]"));
    }

    #[test]
    fn point_id_debug_preserves_every_declared_variant() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = records(&game_points(&root).unwrap());
        let file = syn::parse_file(&source).unwrap();
        let ids = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Enum(ids) if ids.ident == "PointId" => Some(ids),
                _ => None,
            })
            .unwrap();
        let debug = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Impl(item)
                    if item.trait_.as_ref().is_some_and(|(_, path, _)| {
                        path.segments.last().unwrap().ident == "Debug"
                    }) =>
                {
                    Some(item)
                }
                _ => None,
            })
            .unwrap();
        // Expected names and discriminant order come from the enum declarations,
        // independently of the implementation's name table.
        let variants: Vec<_> = ids.variants.iter().map(|variant| &variant.ident).collect();
        let names: Vec<_> = variants.iter().map(|variant| variant.to_string()).collect();
        let samples: Vec<_> = [
            0,
            variants.len() / 4,
            variants.len() / 2,
            variants.len() - 2,
            variants.len() - 1,
        ]
        .into_iter()
        .map(|index| variants[index])
        .collect();
        let program = quote! {
            #ids
            #debug
            #[derive(Debug)]
            enum Reference { #(#samples),* }
            fn main() {
                let cases = [#((PointId::#variants, #names)),*];
                assert_eq!(cases.len(), PointId::__Count as usize + 1);
                for (index, (place, expected)) in cases.into_iter().enumerate() {
                    assert_eq!(place as usize, index);
                    assert_eq!(format!("{place:?}"), expected);
                    assert_eq!(format!("{place:#?}"), expected);
                }
                #(assert_eq!(format!("{:?}", PointId::#samples), format!("{:?}", Reference::#samples));)*
            }
        };
        let directory = root
            .join("target")
            .join(format!("point-debug-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("main.rs");
        let binary = directory.join("point-debug");
        fs::write(&source, program.to_string()).unwrap();
        let output = std::process::Command::new("rustc")
            .args(["--edition=2024", "-o"])
            .arg(&binary)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            std::process::Command::new(&binary)
                .status()
                .unwrap()
                .success()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    /// The census: how many points have an API record, and why the others refuse.
    /// `POINT_INVENTORY=path` writes every point's name, target and refusal.
    #[test]
    fn point_inventory() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let points = game_points(&root).unwrap();
        if let Ok(path) = std::env::var("POINT_INVENTORY") {
            let inventory: Vec<_> = points
                .iter()
                .map(|point| {
                    serde_json::json!({
                        "name": point.name,
                        "target": point.target,
                        "api_refusal": point.api_refusal,
                        "raw": point.raw,
                    })
                })
                .collect();
            fs::write(path, serde_json::to_vec_pretty(&inventory).unwrap()).unwrap();
        }
        let api = points.iter().filter(|p| p.api_refusal.is_none()).count();
        println!("{} points; {api} with an API record", points.len());
        let mut refused = std::collections::BTreeMap::<String, usize>::new();
        for reason in points.iter().filter_map(|p| p.api_refusal.as_deref()) {
            // `argument 2 `x` (`T`) has no API type yet`: count by type.
            let key = reason
                .split_once("(`")
                .and_then(|(_, rest)| rest.split_once("`)"))
                .map_or(reason, |(ty, _)| ty);
            *refused.entry(key.to_owned()).or_default() += 1;
        }
        let mut refused: Vec<_> = refused.into_iter().collect();
        refused.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        for (reason, count) in refused.iter().take(60) {
            println!("refused {count}: {reason}");
        }
        assert!(api > 0);
    }

    #[test]
    fn no_wire_public_values_refuse_before_point_record_emission() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let wire = crate::build::wire_types(&root).unwrap();
        for name in [
            "EnchantmentEffect",
            "RegistrationData",
            "Parts",
            "Attachments",
            "AttachmentAccess",
        ] {
            let method: syn::ImplItemFn =
                syn::parse_str(&format!("fn patch(&mut self, value: &{name}) {{}}")).unwrap();
            let imports = vec![(name.to_owned(), format!("mod_api::{name}"))];
            assert!(
                super::mirrors(&method.sig, &imports, &wire, false).is_empty(),
                "{name} must not be admitted by Clone alone"
            );
            let shape = point_shape::shape(quote!(side = "server"), &method.sig).unwrap();
            assert!(shape.api_refusal.as_ref().unwrap().contains(name));
            assert!(shape.api_items("test").listeners.is_empty());
        }
        for name in [
            "mod_api::ConnectionKind",
            "mod_api::item::containers::ModRevision",
            "mod_api::client::camera::UseItem",
            "mod_api::wire::Id",
        ] {
            assert!(wire.contains(name), "supported public path {name} was lost");
        }
        assert!(!wire.contains("mod_api::wire::wire_slots::Id"));
    }

    fn point(attribute: &str, method: &str) -> Result<point_shape::Shape, String> {
        let method: syn::ImplItemFn = syn::parse_str(method).map_err(|e| e.to_string())?;
        point_shape::shape(attribute.parse().unwrap(), &method.sig).map_err(|e| e.to_string())
    }

    fn eligible(method: &str) -> bool {
        let method: syn::ImplItemFn = syn::parse_str(method).unwrap();
        point_shape::eligible(&method.attrs, &method.sig)
    }

    #[test]
    fn a_point_passes_every_parameter_or_refuses_naming_it() {
        let shape = point(
            r#"side = "client""#,
            "fn turn(&mut self, mut yaw: f64, text: &str, slots: &[u32], count: &mut usize, names: Vec<String>) {}",
        )
        .unwrap();
        let args: Vec<_> = shape
            .args
            .iter()
            .map(|arg| (arg.name.to_string(), arg.pass))
            .collect();
        use point_shape::Pass;
        assert_eq!(
            args,
            [
                ("yaw".into(), Pass::Value),
                ("text".into(), Pass::Shared),
                ("slots".into(), Pass::Shared),
                ("count".into(), Pass::Mut),
                ("names".into(), Pass::Value)
            ]
        );
        assert!(matches!(shape.returns, point_shape::Returns::Unit));
        let item = shape.record_item("t").to_string();
        assert!(
            item.contains("pub yaw : f64")
                && item.contains("pub text : :: std :: borrow :: Cow < 'a , str >")
                && item.contains("pub slots : :: std :: vec :: Vec < u32 >")
                && item.contains("pub count : u64")
                && item.contains("pub names : :: std :: vec :: Vec < :: std :: string :: String >")
                && item.contains("pub cancel : bool"),
            "{item}"
        );
        for (signature, reason) in [
            (
                "fn turn(&mut self, yaw: f64, batch: &mut HudBatch) {}",
                "argument 2 `batch` (`&mutHudBatch`) has no API type yet",
            ),
            (
                "fn turn(&mut self, event: &'static str) {}",
                "argument 1 `event` (`&'staticstr`) has no API type yet",
            ),
            (
                "fn turn(&mut self, (a, b): (u8, u8)) {}",
                "argument 1 is a pattern, not a name",
            ),
        ] {
            let shape = point(r#"side = "client""#, signature).unwrap();
            assert_eq!(shape.api_refusal.as_deref(), Some(reason), "{signature}");
            assert!(shape.api_items("t").listeners.is_empty());
        }
        let mirrored = point(
            r#"side = "both", mirror(Text = "::mod_api::Text")"#,
            "fn say(&mut self, line: &Text, lines: Option<Vec<Text>>) -> Text {}",
        )
        .unwrap();
        assert!(mirrored.api_refusal.is_none());
        assert!(mirrored.args.iter().all(|arg| arg.mirror));
        let item = mirrored.record_item("t").to_string();
        assert!(
            item.contains("pub line : :: mod_api :: Text")
                && item.contains(
                    "pub lines : :: std :: option :: Option < :: std :: vec :: Vec < :: mod_api :: Text > >"
                )
                && item.contains("pub result : Option < :: mod_api :: Text >"),
            "{item}"
        );
        // A receiver other than `Game` has no API handle yet.
        let other = point(r#"name = "boat_paddle""#, "fn paddle(&mut self) {}").unwrap();
        assert_eq!(
            other.api_refusal.as_deref(),
            Some("its `self` has no API handle yet")
        );
        let read = point(
            r#"side = "server", name = "reach_of""#,
            "fn reach(&self) -> f64 { 0.0 }",
        )
        .unwrap();
        assert_eq!(
            (read.name.as_str(), read.variant().to_string()),
            ("reach_of", "ReachOf".into())
        );
        assert!(
            read.record_item("t")
                .to_string()
                .contains("pub result : Option < f64 >")
        );
    }

    #[test]
    fn what_a_listener_may_do_follows_the_return_type() {
        let returns = |sig: &str| point(r#"side = "both""#, sig).map(|shape| shape.returns);
        assert!(matches!(
            returns("fn tick(&mut self) -> Result<()> {}"),
            Ok(point_shape::Returns::UnitResult)
        ));
        assert!(matches!(
            returns("fn hit(&self) -> bool {}"),
            Ok(point_shape::Returns::Value(_))
        ));
        assert!(matches!(
            returns("fn hit(&self) -> Option<(usize, f64)> {}"),
            Ok(point_shape::Returns::Value(_))
        ));
        assert!(matches!(
            returns("fn hit(&self) -> Option<HitResult> {}"),
            Ok(point_shape::Returns::Opaque)
        ));
        let at_return = |sig: &str| point(r#"side = "both""#, sig).unwrap().has_return();
        assert!(at_return("fn hit(&self) -> Option<(usize, f64)> {}"));
        assert!(!at_return(
            "fn slot(&mut self, i: u32) -> Option<&mut ItemStack> {}"
        ));
        assert!(!at_return("fn view(&self) -> View<'_> {}"));
        // A `'static` return borrows nothing from the receiver.
        assert!(at_return("fn jab(&self) -> Option<&'static str> {}"));
    }

    #[test]
    fn every_plain_game_method_is_a_point() {
        assert!(eligible("fn tick(&mut self) -> Result<()> {}"));
        assert!(eligible("pub(super) fn reach(&self) -> f64 { 0.0 }"));
        assert!(eligible("fn get<T: Copy>(&self) -> T { todo!() }"));
        // An associated or free function is a point too; off `Game`, with no host.
        assert!(eligible("fn new() -> Self {}"));
        for left_out in [
            "fn native_server_tick(&mut self) {}",
            "fn take(self) {}",
            "const fn size(&self) -> usize { 0 }",
            "async fn wait(&self) {}",
            "#[cfg(test)] fn probe(&self) {}",
            "#[inline(always)] fn hot(&self) {}",
            "fn each(&self, f: impl Fn()) {}",
            "fn iter(&self) -> impl Iterator<Item = u8> { todo!() }",
            "fn stop(&self) -> ! { loop {} }",
            "fn f(&mut self, cancel: bool) {}",
        ] {
            assert!(!eligible(left_out), "{left_out}");
        }
    }

    /// Two functions whose paths snake-case to one name are refused at build, naming both:
    /// `a::b_c` and `a_b::c` both give `game_a_b_c`.
    #[test]
    fn engine_points_with_one_snake_cased_name_are_refused_naming_both() {
        let root =
            std::env::temp_dir().join(format!("horizon-points-clash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put(
            "crates/horizon-client/Cargo.toml",
            "[package]
name = 'horizon-client'
",
        );
        put(
            "crates/horizon-client/src/lib.rs",
            "mod game;
",
        );
        put(
            "crates/horizon-client/src/game.rs",
            "mod a;
mod a_b;
",
        );
        put(
            "crates/horizon-client/src/game/a.rs",
            "fn b_c() {}
",
        );
        put(
            "crates/horizon-client/src/game/a_b.rs",
            "fn c() {}
",
        );
        put(
            "crates/horizon-mod-points/Cargo.toml",
            "[package]
name = 'horizon-mod-points'
[dependencies]
horizon-world = { path = '../horizon-world' }
[build-dependencies]
horizon-mod-api-gen = { path = '../horizon-mod-api-gen' }
",
        );
        put(
            "crates/horizon-world/Cargo.toml",
            "[package]
name = 'horizon-world'
",
        );
        put(
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]
name = 'horizon-mod-api-gen'
",
        );
        let error = points_with(&root, &WireTypes::new()).unwrap_err();
        assert!(
            error.contains("horizon_client::game::a::b_c")
                && error.contains("horizon_client::game::a_b::c")
                && error.contains("game_a_b_c"),
            "{error}"
        );
        // Without the second function the same tree is accepted.
        put("crates/horizon-client/src/game/a_b.rs", "");
        assert!(points_with(&root, &WireTypes::new()).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_pass_marks_the_functions_the_generator_lists() {
        let root = std::env::temp_dir().join(format!("horizon-points-{}", std::process::id()));
        let put = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        put("crates/horizon-mod-api-gen/internals.txt", "");
        put(
            "crates/horizon-client/Cargo.toml",
            "[package]\nname = 'horizon-client'\n",
        );
        put("crates/horizon-client/src/lib.rs", "mod game; mod entry;\n");
        put("crates/horizon-client/src/entry.rs", "");
        put(
            "crates/horizon-client/src/game.rs",
            "mod remote;\nmod nested { mod leaf; }\n#[path = \"elsewhere\"]\nmod moved { mod deep; }\n#[cfg(test)]\nmod probes;\nimpl Game<'_> {\n    fn tick(&mut self) {}\n    pub(super) fn slot(&mut self) -> &mut u8 { todo!() }\n    fn native_tick(&mut self) {}\n    fn new() -> Self { todo!() }\n}\n",
        );
        put(
            "crates/horizon-client/src/game/remote.rs",
            "mod lan;\nimpl Game<'_> { fn send(&self, id: u32) {} }\nimpl Other { fn send(&self) {} }\nfn decode<T>(bytes: &[u8]) -> T { todo!() }\n",
        );
        put(
            "crates/horizon-client/src/game/remote/lan.rs",
            "impl Game<'_> { fn join(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/elsewhere/deep.rs",
            "impl Game<'_> { fn dive(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/nested/leaf.rs",
            "impl Game<'_> { fn fall(&mut self) {} }",
        );
        put(
            "crates/horizon-client/src/game/probes.rs",
            "impl Game<'_> { fn probe(&self) {} }",
        );
        put(
            "crates/horizon-mod-points/Cargo.toml",
            "[package]\nname = 'horizon-mod-points'\n[dependencies]\nhorizon-world = { path = '../horizon-world' }\n[build-dependencies]\nhorizon-mod-api-gen = { path = '../horizon-mod-api-gen' }\n",
        );
        put(
            "crates/horizon-world/Cargo.toml",
            "[package]\nname = 'horizon-world'\n",
        );
        put(
            "crates/horizon-mod-api-gen/Cargo.toml",
            "[package]\nname = 'horizon-mod-api-gen'\n",
        );
        put(
            "crates/horizon-client-render/Cargo.toml",
            "[package]\nname = 'horizon-client-render'\n[dependencies]\nlog = '0.4'\n",
        );
        put(
            "Cargo.lock",
            // CRLF, as a Windows checkout writes it.
            &"version = 4\n\n[[package]]\nname = \"horizon-client\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"horizon-client-render\"\nversion = \"0.1.0\"\ndependencies = [\n \"log\",\n]\n\n[[package]]\nname = \"log\"\nversion = \"0.4.0\"\n"
                .replace('\n', "\r\n"),
        );
        put(
            "crates/horizon-client-render/src/lib.rs",
            "pub mod mesher;\n",
        );
        put(
            "crates/horizon-client-render/src/mesher.rs",
            "pub struct Mesher<'a>(&'a u8);\nimpl Mesher<'_> { pub fn mesh(&mut self, chunk: &Chunk) -> usize { 0 } fn face(&self) {} }\n",
        );
        put(
            "crates/horizon-mod-api-gen/slots/horizon.txt",
            "# lower points\nhorizon_client_render::mesher::Mesher<'_>::mesh\n",
        );
        let points = points_with(&root, &WireTypes::new()).unwrap();
        let named: Vec<_> = points
            .iter()
            .map(|p| (p.name.as_str(), p.target.as_str(), p.side, p.raw))
            .collect();
        assert_eq!(
            named,
            [
                (
                    "client_render_mesher_mesher_mesh",
                    "horizon_client_render::mesher::Mesher<'_>::mesh",
                    "none",
                    true
                ),
                (
                    "dive",
                    "horizon_client::game::moved::deep::Game<'_>::dive",
                    "both",
                    true
                ),
                (
                    "fall",
                    "horizon_client::game::nested::leaf::Game<'_>::fall",
                    "both",
                    true
                ),
                (
                    "game_game_new",
                    "horizon_client::game::Game<'_>::new",
                    "none",
                    true
                ),
                (
                    "game_remote_decode",
                    "horizon_client::game::remote::decode",
                    "none",
                    false
                ),
                (
                    "game_remote_other_send",
                    "horizon_client::game::remote::Other::send",
                    "none",
                    true
                ),
                (
                    "join",
                    "horizon_client::game::remote::lan::Game<'_>::join",
                    "both",
                    true
                ),
                (
                    "send",
                    "horizon_client::game::remote::Game<'_>::send",
                    "both",
                    true
                ),
                ("slot", "horizon_client::game::Game<'_>::slot", "both", true),
                ("tick", "horizon_client::game::Game<'_>::tick", "both", true),
            ]
        );
        let api = records(&points);
        assert!(
            api.contains("fn tick_return")
                // A borrowed return has a raw return place only.
                && !api.contains("fn slot_return")
                && api.contains("SlotReturn")
                // `Other` has no API handle: its point keeps only its raw places.
                && !api.contains("fn game_remote_other_send (")
                && api.contains("GameRemoteOtherSendReturn"),
            "{api}"
        );
        assert_eq!(
            stage_with(&root, &WireTypes::new()).unwrap(),
            Staged {
                points: 10,
                head_only: 1,
                raw: 9,
                files: 6
            }
        );
        let game = std::fs::read_to_string(root.join("crates/horizon-client/src/game.rs")).unwrap();
        let parsed = syn::parse_file(&game).unwrap();
        for name in ["tick", "slot"] {
            let method = parsed
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Impl(item) => Some(&item.items),
                    _ => None,
                })
                .flatten()
                .find_map(|item| match item {
                    syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
                    _ => None,
                })
                .unwrap();
            assert!(
                method
                    .attrs
                    .iter()
                    .any(|attr| attr.to_token_stream().to_string()
                        == "# [horizon_mod_api_macros :: point (side = \"both\" , raw)]")
            );
        }
        assert!(
            game.contains("fn tick(")
                && game.contains("pub(super) fn slot(")
                && !game.contains("raw)] fn native_tick")
                // `Game`'s associated functions are named and hosted like any other type's.
                && game.contains("point(name = \"game_game_new\"")
                && game.contains("pub type __TickListener")
                && game.contains("pub type __SlotReturnListener"),
            "{game}"
        );
        let remote =
            std::fs::read_to_string(root.join("crates/horizon-client/src/game/remote.rs")).unwrap();
        assert!(
            remote.contains("point(name = \"game_remote_decode\", free)] fn decode")
                && remote.contains("pub type __GameRemoteOtherSendListener"),
            "{remote}"
        );
        let render = root.join("crates/horizon-client-render");
        let mesher = std::fs::read_to_string(render.join("src/mesher.rs")).unwrap();
        assert!(
            mesher.contains("point(name = \"client_render_mesher_mesher_mesh\", raw)]")
                && mesher.contains("pub fn mesh")
                && !mesher.contains("client_render_mesher_mesher_face")
                && mesher.contains("& 'r mut Mesher < '__self0 >"),
            "{mesher}"
        );
        let manifest = std::fs::read_to_string(render.join("Cargo.toml")).unwrap();
        assert!(
            manifest.contains("horizon-mod-points") && manifest.contains("../horizon-mod-points"),
            "{manifest}"
        );
        // The lock file lists them too, in order, so a `--locked` build accepts the manifest.
        let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap();
        assert!(
            lock.contains("name = \"horizon-client-render\"\nversion = \"0.1.0\"\ndependencies = [\n \"horizon-mod-api\",\n \"horizon-mod-api-macros\",\n \"horizon-mod-points\",\n \"log\",\n]\n\n[[package]]\nname = \"log\""),
            "{lock}"
        );
        // A second pass finds the attribute and refuses: the source tree never carries one.
        assert!(
            stage_with(&root, &WireTypes::new())
                .unwrap_err()
                .contains("already has")
        );
        // A crate the points' own crate is built on can't be listed.
        put(
            "crates/horizon-mod-api-gen/slots/horizon.txt",
            "horizon_world::level::tick\n",
        );
        assert!(
            points_with(&root, &WireTypes::new())
                .unwrap_err()
                .contains("built under")
        );
        // Nor one it builds with (the generator), nor the points' own crate.
        for listed in [
            "horizon_mod_api_gen::points::stage\n",
            "horizon_mod_points::register\n",
        ] {
            put("crates/horizon-mod-api-gen/slots/horizon.txt", listed);
            assert!(
                points_with(&root, &WireTypes::new())
                    .unwrap_err()
                    .contains("built under"),
                "{listed}"
            );
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

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
