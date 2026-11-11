//! Function replacement for `[[mixin]]` (fn-replace): the patch crate a mixin build compiles
//! against the prebuilt game instead of rebuilding it. Each target function is copied, unchanged,
//! out of the pinned source into a module that mirrors its own (`patch::game::autosave` for
//! `horizon_client::game::autosave`), where every name the body uses resolves as it did in the
//! game: the mirror re-exports its module's items from the prebuilt game, which makes them all
//! public, and repeats the module's own `use` items, so `self`, `super::` and `crate::` mean what
//! they meant. A method moves into a one-method trait implemented for its own type, so `self`,
//! `Self` and the helpers' bindings stay as written. horizon-mixins then patches the copies, and
//! the game installs each through its function's replacement slot (`__fnr_<name>`,
//! [`crate::replace`]).
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{ImplItem, Item};

/// One target function, copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    /// The target as the `[[mixin]]` row writes it.
    pub target: String,
    /// The copy's id for horizon-mixins, in the patch crate.
    pub id: String,
    /// The expression that installs the copy and says whether it did, from the patch crate's
    /// root.
    pub install: String,
}

/// The patch crate's library source and the copies in it.
#[derive(Debug)]
pub struct PatchCrate {
    pub lib: String,
    pub patched: Vec<Patched>,
}

/// A `[[native.module]]`: Rust items emitted as a module of the mirror of the module `file`
/// holds, where the copies reach them by the paths they would have had in the game.
#[derive(Debug, Clone)]
pub struct SupportModule {
    pub id: String,
    /// The game source file, relative to the source root, as the row names it.
    pub file: String,
    pub name: String,
    /// The payload's text.
    pub text: String,
    pub public: bool,
}

/// A `[[native.bridge]]`: horizon-mixins' accessor or invoker, emitted as an extension trait the
/// mirror modules import, so the helpers' method calls resolve as they did on the game's type.
#[derive(Debug, Clone)]
pub struct Bridge {
    pub id: String,
    /// The generated method's name, `__horizon_mixins_<mod>_<id>`.
    pub method: String,
    /// `GET`, `GET_MUT`, `SET` or `INVOKE`.
    pub kind: String,
    pub target: String,
    pub member: Option<String>,
}

/// What a mod set adds to the patch crate beside its copies.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub modules: Vec<SupportModule>,
    pub bridges: Vec<Bridge>,
}

/// Why a target can't be copied out: a named refusal, never a game rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub target: String,
    pub code: &'static str,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {}", self.code, self.target, self.why)
    }
}

/// Macros whose expansion depends on the file or crate they are written in.
const PLACE_MACROS: &[&str] = &[
    "file",
    "line",
    "column",
    "module_path",
    "include",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
];

/// One source module: its items, from its file or its inline `mod` block.
struct Module {
    items: Vec<Item>,
}

/// The game crate's modules by path, parsed lazily from the pinned source.
struct Source {
    files: BTreeMap<String, PathBuf>,
    parsed: BTreeMap<String, Module>,
    /// `macro_rules!` names the crate defines without `#[macro_export]`: a body using one can't
    /// leave the crate.
    local_macros: BTreeSet<String>,
}

impl Source {
    fn new(root: &Path) -> Result<Self, String> {
        let src = root.join("crates/horizon-client/src");
        let mut files = BTreeMap::new();
        files.insert("horizon_client".to_owned(), src.join("lib.rs"));
        for file in crate::source::game_files(&src, false)? {
            files.insert(file.module, file.path);
        }
        let mut local_macros = BTreeSet::new();
        let mut exported = BTreeSet::new();
        for path in walk(&src) {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            for (at, _) in text.match_indices("macro_rules!") {
                let name: String = text[at + "macro_rules!".len()..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if text[..at].trim_end().ends_with("#[macro_export]") {
                    exported.insert(name);
                } else {
                    local_macros.insert(name);
                }
            }
        }
        local_macros.retain(|name| !exported.contains(name));
        Ok(Self {
            files,
            parsed: BTreeMap::new(),
            local_macros,
        })
    }

    /// Module `path`'s items: its file's, or an inline module's inside its parent's.
    fn module(&mut self, path: &str) -> Result<&Module, String> {
        if !self.parsed.contains_key(path) {
            let items = if let Some(file) = self.files.get(path) {
                let text = std::fs::read_to_string(file)
                    .map_err(|e| format!("{}: {e}", file.display()))?;
                syn::parse_file(&text)
                    .map_err(|e| format!("{}: {e}", file.display()))?
                    .items
            } else {
                let (parent, name) = path
                    .rsplit_once("::")
                    .ok_or_else(|| format!("no module {path}"))?;
                let parent = self.module(parent)?;
                parent
                    .items
                    .iter()
                    .find_map(|item| match item {
                        Item::Mod(module) if module.ident == name => {
                            module.content.as_ref().map(|(_, items)| items.clone())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| format!("no module {path}"))?
            };
            self.parsed.insert(path.to_owned(), Module { items });
        }
        Ok(&self.parsed[path])
    }

    /// The `macro_rules!` items, not exported, that the modules on `path` define, outermost
    /// first: the macros a function in `path` may use by their bare names.
    fn path_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let segments: Vec<&str> = path.split("::").collect();
        let mut out = Vec::new();
        for end in 1..=segments.len() {
            out.extend(self.scope_macros(&segments[..end].join("::")));
        }
        out
    }

    /// The macros module `path` itself defines and those its `#[macro_use] mod` children do,
    /// which textual scope carries on to the rest of `path`.
    fn scope_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let Ok(module) = self.module(path) else {
            return Vec::new();
        };
        let items = module.items.clone();
        let mut out = module_macros(&items);
        for item in &items {
            if let Item::Mod(child) = item
                && child.attrs.iter().any(|a| a.path().is_ident("macro_use"))
            {
                out.extend(self.scope_macros(&format!("{path}::{}", child.ident)));
            }
        }
        out
    }

    /// Whether `path` names a module of the game crate.
    fn is_module(&mut self, path: &str) -> bool {
        path == "horizon_client" || self.files.contains_key(path) || self.module(path).is_ok()
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

/// A type or path without its generic arguments or spaces: `Game<'_>` is `Game`.
fn plain(text: &str) -> String {
    let mut depth = 0usize;
    text.chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            c => depth == 0 && !c.is_whitespace(),
        })
        .collect()
}

/// What a copied body may not contain.
#[derive(Default)]
struct BodyCheck<'a> {
    local_macros: Option<&'a BTreeSet<String>>,
    refusal: Option<String>,
}

impl<'ast> Visit<'ast> for BodyCheck<'_> {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            let name = last.ident.to_string();
            if mac.path.segments.len() == 1 && PLACE_MACROS.contains(&name.as_str()) {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, which means something else outside the game's source"
                ));
            } else if mac.path.segments.len() == 1 && name == "thread_local" {
                self.refusal.get_or_insert(
                    "its body declares a thread-local: a copy would have a second one".into(),
                );
            } else if mac.path.segments.len() == 1
                && self.local_macros.is_some_and(|local| local.contains(&name))
            {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, a macro defined outside its module's path"
                ));
            }
        }
        syn::visit::visit_macro(self, mac);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        match item {
            Item::Static(_) => {
                self.refusal.get_or_insert(
                    "its body declares a `static`: a copy would have a second one".into(),
                );
            }
            Item::Impl(_) => {
                self.refusal.get_or_insert(
                    "its body implements a trait or type: a copy would implement it twice".into(),
                );
            }
            Item::Macro(mac) if mac.ident.is_some() => {
                self.refusal.get_or_insert(
                    "its body defines a `macro_rules!`, which moves with different hygiene".into(),
                );
            }
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }
}

/// The function a target names, found in its module.
struct Found {
    module: String,
    /// The `impl` it is in: none for a free function.
    owner: Option<syn::ItemImpl>,
    function: syn::ImplItemFn,
}

fn find(source: &mut Source, target: &str) -> Result<Found, (&'static str, String)> {
    let (front, name) = target
        .rsplit_once("::")
        .ok_or(("E_MIXIN_TARGET_MISSING", "not a function path".to_owned()))?;
    if front.split("::").next() != Some("horizon_client") {
        return Err((
            "E_MIXIN_NOT_REPLACEABLE",
            "only functions of the game crate (`horizon_client`) are replaced; a lower crate's \
             function has no copy route yet"
                .into(),
        ));
    }
    // The longest prefix that is a module; what follows it is the `impl`'s type, if anything.
    let segments: Vec<&str> = front.split("::").collect();
    let mut cut = segments.len();
    while cut > 0 && !source.is_module(&segments[..cut].join("::")) {
        cut -= 1;
    }
    let module = segments[..cut].join("::");
    let owner = plain(&segments[cut..].join("::"));
    let items = source
        .module(&module)
        .map_err(|why| ("E_MIXIN_TARGET_MISSING", why))?
        .items
        .clone();
    let mut found = Vec::new();
    for item in &items {
        match item {
            Item::Fn(function) if owner.is_empty() && function.sig.ident == name => {
                found.push(Found {
                    module: module.clone(),
                    owner: None,
                    function: syn::ImplItemFn {
                        attrs: function.attrs.clone(),
                        vis: function.vis.clone(),
                        defaultness: None,
                        sig: function.sig.clone(),
                        block: (*function.block).clone(),
                    },
                })
            }
            Item::Impl(block)
                if !owner.is_empty()
                    && block.trait_.is_none()
                    && plain(&block.self_ty.to_token_stream().to_string()) == owner =>
            {
                for member in &block.items {
                    if let ImplItem::Fn(function) = member
                        && function.sig.ident == name
                    {
                        found.push(Found {
                            module: module.clone(),
                            owner: Some(block.clone()),
                            function: function.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    match found.len() {
        0 => Err((
            "E_MIXIN_TARGET_MISSING",
            format!(
                "no function `{name}` in {module}{}",
                if owner.is_empty() {
                    String::new()
                } else {
                    format!(" on {owner}")
                }
            ),
        )),
        1 => Ok(found.pop().expect("one")),
        _ => Err((
            "E_MIXIN_TARGET_AMBIGUOUS",
            "several functions have this path (a cfg-gated pair, for one)".into(),
        )),
    }
}

/// Whether `function` can be copied and installed through its point's replacement slot.
fn replaceable(found: &Found, source: &mut Source) -> Result<(), String> {
    // A crate-local macro defined in a module on the function's path moves with the copy (the
    // mirror repeats it); one defined elsewhere would not resolve.
    let on_path: BTreeSet<String> = source
        .path_macros(&found.module)
        .iter()
        .filter_map(|mac| mac.ident.as_ref().map(ToString::to_string))
        .collect();
    let local_macros: BTreeSet<String> =
        source.local_macros.difference(&on_path).cloned().collect();
    let local_macros = &local_macros;
    let sig = &found.function.sig;
    if !crate::replace::eligible(&found.function.attrs, sig) {
        return Err(
            "it has no replacement slot: `const`, `async`, `unsafe`, a receiver other than \
             `&self` or `&mut self`, type or const generics, `impl Trait` or `!` in its \
             signature, `#[inline(always)]`, `#[track_caller]`, or compiled only under a `cfg`"
                .into(),
        );
    }
    if let Some(block) = &found.owner
        && !crate::replace::lifetimes_only(&block.generics)
    {
        return Err(
            "its `impl` is generic over types: one copy can't stand for every instance".into(),
        );
    }
    if let Some(block) = &found.owner
        && block.attrs.iter().any(|attr| attr.path().is_ident("cfg"))
    {
        return Err("its `impl` is compiled only under a `cfg`".into());
    }
    let mut check = BodyCheck {
        local_macros: Some(local_macros),
        refusal: None,
    };
    check.visit_block(&found.function.block);
    match check.refusal {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// The names a module's own items define, for explicit re-export (an explicit name outranks the
/// module's glob imports, as the item itself did).
fn defined(items: &[Item]) -> Vec<(syn::Ident, Vec<syn::Attribute>)> {
    let cfg = |attrs: &[syn::Attribute]| {
        attrs
            .iter()
            .filter(|attr| attr.path().is_ident("cfg"))
            .cloned()
            .collect::<Vec<_>>()
    };
    // Test-only items are not in the prebuilt game.
    let test_only = |attrs: &[syn::Attribute]| {
        attrs.iter().any(|attr| {
            attr.path().is_ident("test")
                || (attr.path().is_ident("cfg")
                    && attr.to_token_stream().to_string().contains("test"))
        })
    };
    items
        .iter()
        .filter(|item| {
            let attrs = match item {
                Item::Fn(item) => &item.attrs,
                Item::Mod(item) => &item.attrs,
                Item::Struct(item) => &item.attrs,
                Item::Enum(item) => &item.attrs,
                Item::Const(item) => &item.attrs,
                Item::Static(item) => &item.attrs,
                Item::Type(item) => &item.attrs,
                Item::Trait(item) => &item.attrs,
                Item::Union(item) => &item.attrs,
                _ => return true,
            };
            !test_only(attrs)
        })
        .filter_map(|item| match item {
            Item::Fn(item) => Some((item.sig.ident.clone(), cfg(&item.attrs))),
            Item::Struct(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Enum(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Union(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Const(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Static(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Type(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Trait(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            Item::Mod(item) => Some((item.ident.clone(), cfg(&item.attrs))),
            _ => None,
        })
        .filter(|(ident, attrs)| {
            ident != "tests"
                && !attrs
                    .iter()
                    .any(|a| a.to_token_stream().to_string().contains("test"))
        })
        .collect()
}

/// A module's own `macro_rules!` items that it does not export.
fn module_macros(items: &[Item]) -> Vec<syn::ItemMacro> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Macro(mac)
                if mac.ident.is_some()
                    && mac.mac.path.is_ident("macro_rules")
                    && !mac.attrs.iter().any(|a| a.path().is_ident("macro_export")) =>
            {
                Some(mac.clone())
            }
            _ => None,
        })
        .collect()
}

/// The module's `use` items, as written.
fn uses(items: &[Item]) -> Vec<&syn::ItemUse> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item) => Some(item),
            _ => None,
        })
        .collect()
}

/// A copied function's attributes: its docs, lint levels and cfg, nothing that changes codegen
/// or the point machinery.
fn kept(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs
        .iter()
        .filter(|attr| {
            ["doc", "allow", "cfg", "expect", "warn"]
                .iter()
                .any(|name| attr.path().is_ident(name))
        })
        .cloned()
        .collect()
}

/// A tree of mirror modules, keyed by segment.
#[derive(Default)]
struct Tree {
    children: BTreeMap<String, Tree>,
    copies: Vec<proc_macro2::TokenStream>,
    /// Names the copies define, which the mirror must not also re-export.
    own: BTreeSet<String>,
    /// At the root: whether there are bridges for every mirror module to import.
    bridges: bool,
}

/// The patch crate for `targets` (as `[[mixin]]` rows write them), from the pinned game source
/// at `root`. `krate` is the patch crate's name. Every target that can't be copied is refused by
/// name; the crate is returned only when none is.
pub fn generate(
    root: &Path,
    krate: &str,
    targets: &[String],
    extras: &Extras,
) -> Result<PatchCrate, Vec<Refusal>> {
    let refuse = |target: &str, code, why: String| Refusal {
        target: target.to_owned(),
        code,
        why,
    };
    let mut source = match Source::new(root) {
        Ok(source) => source,
        Err(why) => {
            return Err(targets
                .iter()
                .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                .collect());
        }
    };
    let mut refusals = Vec::new();
    let mut tree = Tree::default();
    let mut patched = Vec::new();
    let unique: BTreeSet<&String> = targets.iter().collect();
    for target in unique {
        let found = match find(&mut source, target) {
            Ok(found) => found,
            Err((code, why)) => {
                refusals.push(refuse(target, code, why));
                continue;
            }
        };
        if let Err(why) = replaceable(&found, &mut source) {
            refusals.push(refuse(target, "E_MIXIN_NOT_REPLACEABLE", why));
            continue;
        }
        let name = found.function.sig.ident.clone();
        let mut node = &mut tree;
        for segment in found.module.split("::").skip(1) {
            node = node.children.entry(segment.to_owned()).or_default();
        }
        let attrs = kept(&found.function.attrs);
        let mut sig = found.function.sig.clone();
        let block = &found.function.block;
        let install = format_ident!("__fnr_install_{}", name);
        let constructor = format_ident!("__fnr_{}", name);
        let module_path = found.module.split("::").skip(1).collect::<Vec<_>>();
        let (copy, id) = match &found.owner {
            None => {
                node.own.insert(name.to_string());
                node.own.insert(install.to_string());
                let original: syn::Path = syn::parse_str(&found.module).expect("a module path");
                (
                    quote! {
                        #(#attrs)*
                        pub #sig #block

                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            #original::#constructor(#name)
                        }
                    },
                    format!("{krate}::{}::{name}", module_path.join("::")),
                )
            }
            Some(owner) => {
                let self_ty = &owner.self_ty;
                let owner_name = plain(&self_ty.to_token_stream().to_string()).replace("::", "_");
                let trait_name = format_ident!("__Fnr_{}_{}", owner_name, name);
                node.own.insert(trait_name.to_string());
                node.own.insert(install.to_string());
                let generics = &owner.generics;
                let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
                // A trait declaration takes no patterns: its parameters are renamed.
                let mut declared = sig.clone();
                for (index, input) in declared.inputs.iter_mut().enumerate() {
                    if let syn::FnArg::Typed(typed) = input {
                        let fresh = format_ident!("__a{index}");
                        *typed.pat = syn::parse_quote!(#fresh);
                    }
                }
                sig.constness = None;
                // `Game<'_>` and `Game<'g>` alike are named with elided lifetimes in expressions.
                let mut elided = (**self_ty).clone();
                struct Elide;
                impl syn::visit_mut::VisitMut for Elide {
                    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
                        if lifetime.ident != "static" {
                            *lifetime = syn::parse_quote!('_);
                        }
                    }
                }
                syn::visit_mut::VisitMut::visit_type_mut(&mut Elide, &mut elided);
                let trait_args: Vec<_> = generics.lifetimes().map(|_| quote!('_)).collect();
                let trait_ref = if trait_args.is_empty() {
                    quote!(#trait_name)
                } else {
                    quote!(#trait_name<#(#trait_args),*>)
                };
                let id_type = plain_spaces(&elided.to_token_stream().to_string());
                let id_trait = plain_spaces(&{
                    let generics_args: Vec<_> =
                        generics.lifetimes().map(|l| l.lifetime.clone()).collect();
                    if generics_args.is_empty() {
                        quote!(#trait_name).to_string()
                    } else {
                        quote!(#trait_name<#(#generics_args),*>).to_string()
                    }
                });
                (
                    quote! {
                        #[doc(hidden)]
                        #[allow(non_camel_case_types)]
                        pub trait #trait_name #impl_generics #where_clause {
                            #declared;
                        }
                        impl #impl_generics #trait_name #type_generics for #self_ty #where_clause {
                            #(#attrs)*
                            #sig #block
                        }

                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            <#elided>::#constructor(<#elided as #trait_ref>::#name)
                        }
                    },
                    format!(
                        "{krate}::{}::<{id_type} as {id_trait}>::{name}",
                        module_path.join("::")
                    ),
                )
            }
        };
        node.copies.push(copy);
        patched.push(Patched {
            target: target.clone(),
            id,
            install: format!("{krate}::{}::{install}()", module_path.join("::")),
        });
    }
    for module in &extras.modules {
        match support_module(&source, module) {
            Ok((path, tokens)) => {
                let mut node = &mut tree;
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(module.name.clone());
                node.copies.push(tokens);
            }
            Err((code, why)) => refusals.push(refuse(&module.id, code, why)),
        }
    }
    let mut bridge_traits = Vec::new();
    for bridge in &extras.bridges {
        match bridge_trait(&mut source, bridge) {
            Ok((path, trait_name, tokens)) => {
                let mut node = &mut tree;
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(trait_name.to_string());
                node.copies.push(tokens);
                let mirror: syn::Path =
                    syn::parse_str(&path.replacen("horizon_client", "crate", 1))
                        .expect("a module path");
                bridge_traits.push(quote!(pub use #mirror::#trait_name;));
            }
            Err((code, why)) => refusals.push(refuse(&bridge.id, code, why)),
        }
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    tree.bridges = !bridge_traits.is_empty();
    let body = match mirror(&mut source, "horizon_client", &tree, tree.bridges) {
        Ok(body) => body,
        Err(why) => {
            return Err(targets
                .iter()
                .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                .collect());
        }
    };
    let lib = quote! {
        //! Generated by the launcher: game functions copied out of the pinned source for the
        //! installed mods' `[[mixin]]` rows, patched by horizon-mixins. Not for editing.
        #![allow(unused_imports, unused_macros, unused_mut, unused_variables, dead_code, unused_braces, unused_parens)]
        #![allow(clippy::all)]
        #body
        /// The bridges' traits, which every mirror module imports.
        #[doc(hidden)]
        pub mod __fnr_bridges {
            #(#bridge_traits)*
        }
    };
    Ok(PatchCrate {
        lib: prettyplease_or_tokens(lib),
        patched,
    })
}

/// A path's tokens without spaces, as horizon-mixins writes an id.
fn plain_spaces(text: &str) -> String {
    text.replace(' ', "")
}

fn prettyplease_or_tokens(tokens: proc_macro2::TokenStream) -> String {
    match syn::parse2::<syn::File>(tokens.clone()) {
        Ok(file) => prettyplease::unparse(&file),
        Err(_) => tokens.to_string(),
    }
}

/// The mirror of module `path` and its children in `tree`.
fn mirror(
    source: &mut Source,
    path: &str,
    tree: &Tree,
    bridges: bool,
) -> Result<proc_macro2::TokenStream, String> {
    let items = source.module(path)?.items.clone();
    let original: syn::Path = syn::parse_str(&format!("::{path}")).expect("a module path");
    let reserved: BTreeSet<String> = tree
        .children
        .keys()
        .cloned()
        .chain(tree.own.iter().cloned())
        .collect();
    // The crate root is not widened: only its public items are there to re-export.
    let public: BTreeSet<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.sig.ident.to_string())
            }
            Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Mod(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Union(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            _ => None,
        })
        .collect();
    let root = path == "horizon_client";
    let explicit = defined(&items)
        .into_iter()
        .filter(|(ident, _)| !reserved.contains(&ident.to_string()))
        .filter(|(ident, _)| !root || public.contains(&ident.to_string()))
        .map(|(ident, cfg)| quote!(#(#cfg)* pub use #original::#ident;));
    let uses = uses(&items).into_iter().filter(|item| {
        // A `use` naming a child mirror or a copy would collide with it.
        let mut names = Vec::new();
        collect_names(&item.tree, &mut names);
        !names.iter().any(|name| reserved.contains(name))
    });
    let mut children = Vec::new();
    for (name, child) in &tree.children {
        let ident = format_ident!("{name}");
        let inner = mirror(source, &format!("{path}::{name}"), child, bridges)?;
        children.push(quote!(pub mod #ident { #inner }));
    }
    // Repeated before the children, which see them by textual scope as the game's did.
    let macros = source.scope_macros(path);
    let copies = &tree.copies;
    let bridge_import = bridges.then(|| {
        quote!(
            use crate::__fnr_bridges::*;
        )
    });
    Ok(quote! {
        pub use #original::*;
        #bridge_import
        #(#macros)*
        #(#explicit)*
        #(#uses)*
        #(#children)*
        #(#copies)*
    })
}

/// The module path whose file is `file`, and the support module's tokens.
fn support_module(
    source: &Source,
    module: &SupportModule,
) -> Result<(String, proc_macro2::TokenStream), (&'static str, String)> {
    let path = source
        .files
        .iter()
        .find(|(_, path)| path.ends_with(&module.file))
        .map(|(name, _)| name.clone())
        .ok_or((
            "E_MIXIN_MODULE_DESTINATION",
            format!(
                "{} is not a file of the game crate (`horizon_client`); a support module goes \
                 into one of its files",
                module.file
            ),
        ))?;
    let items = syn::parse_file(&module.text)
        .map_err(|e| ("E_MIXIN_MODULE_SOURCE", e.to_string()))?
        .items;
    let name = format_ident!("{}", module.name);
    let vis = if module.public {
        quote!(pub)
    } else {
        quote!(pub(crate))
    };
    Ok((path, quote!(#vis mod #name { #(#items)* })))
}

/// The bridge's owner module, trait name and the trait with its impl, emitted in the mirror of
/// the owner's module so its types are named as the game names them.
fn bridge_trait(
    source: &mut Source,
    bridge: &Bridge,
) -> Result<(String, syn::Ident, proc_macro2::TokenStream), (&'static str, String)> {
    let method = format_ident!("{}", bridge.method);
    let trait_name = format_ident!("__Fnr_Bridge_{}", bridge.method);
    if bridge.kind == "INVOKE" {
        let found = find(source, &bridge.target)?;
        let owner = found.owner.as_ref().ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            "an invoker's target is a method or associated function of a type".to_owned(),
        ))?;
        if !owner.generics.params.is_empty() || !found.function.sig.generics.params.is_empty() {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                "an invoker's type and method are not generic".into(),
            ));
        }
        let self_ty = &owner.self_ty;
        let original = &found.function.sig.ident;
        let mut args = Vec::new();
        let mut sig = found.function.sig.clone();
        for (index, input) in sig.inputs.iter_mut().enumerate() {
            match input {
                syn::FnArg::Receiver(_) => args.push(quote!(self)),
                syn::FnArg::Typed(typed) => {
                    let name = format_ident!("__a{index}");
                    *typed.pat = syn::parse_quote!(#name);
                    args.push(quote!(#name));
                }
            }
        }
        sig.ident = method.clone();
        let call = match &owner.trait_ {
            Some((_, path, _)) => quote!(<#self_ty as #path>::#original(#(#args),*)),
            None => quote!(<#self_ty>::#original(#(#args),*)),
        };
        let path = found.module.clone();
        return Ok((
            path,
            trait_name.clone(),
            quote! {
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                pub trait #trait_name { #sig; }
                impl #trait_name for #self_ty { #sig { #call } }
            },
        ));
    }
    let (front, owner) = bridge.target.rsplit_once("::").ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge's target is a struct path".to_owned(),
    ))?;
    if !source.is_module(front) {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no module {front} in the game crate"),
        ));
    }
    let items = source
        .module(front)
        .map_err(|why| ("E_MIXIN_BRIDGE_TARGET", why))?
        .items
        .clone();
    let item = items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == owner => Some(item.clone()),
            _ => None,
        })
        .ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no struct {owner} in {front}"),
        ))?;
    if item.generics.type_params().next().is_some() || item.generics.const_params().next().is_some()
    {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("{owner} is generic over types"),
        ));
    }
    let member_name = bridge.member.as_deref().ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge names its `member`".to_owned(),
    ))?;
    let field = match &item.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .find(|f| f.ident.as_ref().is_some_and(|i| i == member_name)),
        syn::Fields::Unnamed(fields) => member_name
            .parse::<usize>()
            .ok()
            .and_then(|i| fields.unnamed.get(i)),
        syn::Fields::Unit => None,
    }
    .ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        format!("{owner} has no field {member_name}"),
    ))?;
    let ty = &field.ty;
    let member: syn::Member =
        syn::parse_str(member_name).map_err(|e| ("E_MIXIN_BRIDGE_TARGET", e.to_string()))?;
    let owner = &item.ident;
    // A type generic over lifetimes gets a trait over the same lifetimes.
    let (generics, args, _) = item.generics.split_for_impl();
    let (declared, defined) = match bridge.kind.as_str() {
        "GET" => (quote!(fn #method(&self) -> &#ty), quote!({ &self.#member })),
        "GET_MUT" => (
            quote!(fn #method(&mut self) -> &mut #ty),
            quote!({ &mut self.#member }),
        ),
        "SET" => (
            quote!(fn #method(&mut self, value: #ty)),
            quote!({ self.#member = value; }),
        ),
        other => {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                format!("unknown bridge kind {other}"),
            ));
        }
    };
    Ok((
        front.to_owned(),
        trait_name.clone(),
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub trait #trait_name #generics { #declared; }
            impl #generics #trait_name #args for #owner #args { #declared #defined }
        },
    ))
}

fn collect_names(tree: &syn::UseTree, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => collect_names(&path.tree, out),
        syn::UseTree::Name(name) => out.push(name.ident.to_string()),
        syn::UseTree::Rename(rename) => out.push(rename.rename.to_string()),
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(group) => group.items.iter().for_each(|tree| collect_names(tree, out)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn a_method_moves_into_a_trait_of_its_own_type() {
        let crate_ = generate(
            &root(),
            "horizon_patch",
            &["horizon_client::game::autosave::Game<'_>::next_autosave_interval".into()],
            &Extras::default(),
        )
        .unwrap();
        assert_eq!(
            crate_.patched[0].id,
            "horizon_patch::game::autosave::<Game<'_> as __Fnr_Game_next_autosave_interval>::next_autosave_interval"
        );
        assert!(crate_.lib.contains("pub mod autosave"));
        assert!(
            crate_
                .lib
                .contains("impl __Fnr_Game_next_autosave_interval for Game<'_>")
        );
        assert!(crate_.lib.contains("self.clock.sprinting()"));
    }

    #[test]
    fn refusals_are_named() {
        let error = generate(
            &root(),
            "horizon_patch",
            &[
                "horizon_client::game::autosave::Game<'_>::no_such_function".into(),
                "horizon_client_lib::clock::FixedStep::advance".into(),
            ],
            &Extras::default(),
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(codes, ["E_MIXIN_TARGET_MISSING", "E_MIXIN_NOT_REPLACEABLE"]);
    }

    #[test]
    fn support_modules_and_bridges_land_in_the_mirror_their_rows_name() {
        let extras = Extras {
            modules: vec![SupportModule {
                id: "support".into(),
                file: "crates/horizon-client/src/game/world/autosave.rs".into(),
                name: "fnr_support".into(),
                text: "pub fn twice(n: i64) -> i64 { n * 2 }".into(),
                public: false,
            }],
            bridges: vec![Bridge {
                id: "ticks".into(),
                method: "__horizon_mixins_m_ticks".into(),
                kind: "GET_MUT".into(),
                target: "horizon_client::game::Game".into(),
                member: Some("ticks_until_autosave".into()),
            }],
        };
        let crate_ = generate(
            &root(),
            "horizon_patch",
            &["horizon_client::game::autosave::Game<'_>::next_autosave_interval".into()],
            &extras,
        )
        .unwrap();
        let lib = crate_.lib.replace(char::is_whitespace, "");
        assert!(
            lib.contains("pub(crate)modfnr_support{pubfntwice"),
            "{}",
            crate_.lib
        );
        assert!(
            lib.contains("fn__horizon_mixins_m_ticks(&mutself)->&mut"),
            "{}",
            crate_.lib
        );
        assert!(lib.contains("{&mutself.ticks_until_autosave}"));
        assert!(lib.contains(
            "pubmod__fnr_bridges{pubusecrate::game::__Fnr_Bridge___horizon_mixins_m_ticks;}"
        ));
        assert!(lib.contains("usecrate::__fnr_bridges::*;"));
        let error = generate(
            &root(),
            "horizon_patch",
            &[],
            &Extras {
                modules: vec![SupportModule {
                    file: "crates/horizon-client-lib/src/clock.rs".into(),
                    ..extras.modules[0].clone()
                }],
                bridges: vec![Bridge {
                    member: Some("no_such_field".into()),
                    ..extras.bridges[0].clone()
                }],
            },
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(
            codes,
            ["E_MIXIN_MODULE_DESTINATION", "E_MIXIN_BRIDGE_TARGET"]
        );
    }
}

#[cfg(test)]
mod spike {
    /// Writes the patch crate for `FNR_TARGETS` (comma-separated) into `FNR_OUT`, for trying
    /// horizon-mixins on it by hand.
    #[test]
    #[ignore = "manual: writes a patch crate"]
    fn write_patch_crate() {
        let out = std::path::PathBuf::from(std::env::var("FNR_OUT").unwrap());
        let targets: Vec<String> = std::env::var("FNR_TARGETS")
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        match super::generate(&root, "horizon_patch", &targets, &super::Extras::default()) {
            Ok(crate_) => {
                std::fs::create_dir_all(out.join("horizon_patch/src")).unwrap();
                std::fs::write(out.join("horizon_patch/src/lib.rs"), &crate_.lib).unwrap();
                for patched in crate_.patched {
                    println!("{} => {} | {}", patched.target, patched.id, patched.install);
                }
            }
            Err(refusals) => refusals.iter().for_each(|r| println!("REFUSED {r}")),
        }
    }
}

#[cfg(test)]
mod census {
    use super::*;

    /// Every function of the game module, by whether the copy route takes it and, if not, the
    /// refusal it gets. `cargo test -p horizon-mod-api-gen --lib census -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement: prints the census"]
    fn refusal_census() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut source = Source::new(&root).unwrap();
        let modules: Vec<String> = source.files.keys().cloned().collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut total = 0;
        // Call sites in copyable bodies: all, beneath a borrow, borrowing a call themselves.
        let mut sites = [0usize; 3];
        let mut copyable: Vec<String> = Vec::new();
        let mut note = |why: String| *counts.entry(why).or_default() += 1;
        for module in modules
            .iter()
            .filter(|m| m.starts_with("horizon_client::game"))
        {
            let items = source.module(module).unwrap().items.clone();
            let mut functions: Vec<Found> = Vec::new();
            fn walk(items: &[Item], module: &str, out: &mut Vec<Found>, traits: &mut usize) {
                for item in items {
                    match item {
                        Item::Fn(f) => out.push(Found {
                            module: module.into(),
                            owner: None,
                            function: syn::ImplItemFn {
                                attrs: f.attrs.clone(),
                                vis: f.vis.clone(),
                                defaultness: None,
                                sig: f.sig.clone(),
                                block: (*f.block).clone(),
                            },
                        }),
                        Item::Impl(block) if block.trait_.is_some() => {
                            *traits += block
                                .items
                                .iter()
                                .filter(|i| matches!(i, ImplItem::Fn(_)))
                                .count();
                        }
                        Item::Impl(block) => {
                            for member in &block.items {
                                if let ImplItem::Fn(f) = member {
                                    out.push(Found {
                                        module: module.into(),
                                        owner: Some(block.clone()),
                                        function: f.clone(),
                                    });
                                }
                            }
                        }
                        Item::Mod(inline) if inline.content.is_some() => {
                            let test = inline
                                .attrs
                                .iter()
                                .any(|a| a.to_token_stream().to_string().contains("test"));
                            if !test {
                                walk(
                                    &inline.content.as_ref().unwrap().1,
                                    &format!("{module}::{}", inline.ident),
                                    out,
                                    traits,
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            let mut traits = 0;
            walk(&items, module, &mut functions, &mut traits);
            total += traits;
            for _ in 0..traits {
                note("refused: trait method".into());
            }
            for found in functions {
                let attrs = &found.function.attrs;
                if attrs.iter().any(|a| {
                    a.path().is_ident("test")
                        || (a.path().is_ident("cfg")
                            && a.to_token_stream().to_string().contains("test"))
                }) {
                    continue;
                }
                total += 1;
                let sig = &found.function.sig;
                let why = if attrs
                    .iter()
                    .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                    || found
                        .owner
                        .as_ref()
                        .is_some_and(|b| b.attrs.iter().any(|a| a.path().is_ident("cfg")))
                {
                    "refused: compiled only under a cfg".to_owned()
                } else if !crate::replace::lifetimes_only(&sig.generics)
                    || found
                        .owner
                        .as_ref()
                        .is_some_and(|b| !crate::replace::lifetimes_only(&b.generics))
                {
                    "refused: generic over types".to_owned()
                } else if !crate::replace::eligible(attrs, sig) {
                    "refused: const/async/unsafe/receiver other than &self or &mut self/impl Trait or !/inline(always)/track_caller"
                        .to_owned()
                } else {
                    if replaceable(&found, &mut source).is_ok() {
                        let owner = found.owner.as_ref().map(|b| {
                            format!("{}::", plain(&b.self_ty.to_token_stream().to_string()))
                        });
                        copyable.push(format!(
                            "{}::{}{}",
                            found.module,
                            owner.unwrap_or_default(),
                            found.function.sig.ident
                        ));
                        let mut s = CallSites::default();
                        s.visit_block(&found.function.block);
                        sites[0] += s.all;
                        sites[1] += s.beneath;
                        sites[2] += s.borrowing;
                    }
                    match replaceable(&found, &mut source) {
                        Ok(()) if referenced_call(&found.function.block) => {
                            "copied, but upstream refuses call-site kinds: `&` over a call"
                                .to_owned()
                        }
                        Ok(()) => "copied".to_owned(),
                        Err(why) => format!("refused: {}", why.split(':').next().unwrap_or(&why)),
                    }
                };
                note(why);
            }
        }
        println!("game-module functions: {total}");
        if let Ok(list) = std::env::var("FNR_CENSUS_LIST") {
            std::fs::write(list, copyable.join("\n")).unwrap();
        }
        println!(
            "call sites in copyable functions: {}; beneath `&` (value-wrapping kinds only with the \
             carried narrowing): {}; borrowing a call in their own expression (refused): {}",
            sites[0], sites[1], sites[2]
        );
        #[derive(Default)]
        struct CallSites {
            all: usize,
            beneath: usize,
            borrowing: usize,
            references: usize,
        }
        impl<'ast> Visit<'ast> for CallSites {
            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                    self.all += 1;
                    self.beneath += usize::from(self.references > 0);
                    self.borrowing += usize::from(referenced_call_in(e));
                }
                let reference = matches!(e, syn::Expr::Reference(_));
                self.references += usize::from(reference);
                syn::visit::visit_expr(self, e);
                self.references -= usize::from(reference);
            }
            fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
        }
        fn referenced_call_in(e: &syn::Expr) -> bool {
            let block = syn::Block {
                brace_token: Default::default(),
                stmts: vec![syn::Stmt::Expr(e.clone(), None)],
            };
            referenced_call(&block)
        }
        fn referenced_call(block: &syn::Block) -> bool {
            // horizon-mixins' rule: a call beneath `&` outside macro arguments refuses every
            // call-site kind on the function.
            struct Find(bool);
            impl<'ast> Visit<'ast> for Find {
                fn visit_expr(&mut self, e: &'ast syn::Expr) {
                    if let syn::Expr::Reference(r) = e {
                        struct Call(bool);
                        impl<'ast> Visit<'ast> for Call {
                            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                                    self.0 = true;
                                }
                                syn::visit::visit_expr(self, e);
                            }
                            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
                        }
                        let mut c = Call(false);
                        c.visit_expr(&r.expr);
                        self.0 |= c.0;
                    }
                    syn::visit::visit_expr(self, e);
                }
                fn visit_macro(&mut self, _: &'ast syn::Macro) {}
            }
            let mut f = Find(false);
            f.visit_block(block);
            f.0
        }
        for (why, n) in counts {
            println!("  {why}: {n} ({:.1}%)", 100.0 * n as f64 / total as f64);
        }
    }
}
