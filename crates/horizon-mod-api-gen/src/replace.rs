//! Function replacement: how a patched copy of a game function is installed in place of the
//! original, with the game prebuilt. The staging pass ([`stage`]) writes one check at the head of
//! every replaceable function of a staged source copy: a load of the function's slot in its
//! crate's `__REPLACE` array and, when a copy is installed there, a direct call of it. Beside the
//! function it writes the slot's typed reader (`__fnr_read_slot_<index>`) and installer
//! (`__fnr_<name>`), which a mixin build's patch crate calls with its copy
//! ([`crate::patch`]). A function's check is part of its body, so every inlined copy of the
//! function carries it.
//!
//! Replaceable: every free function and inherent method of the game module that [`eligible`]
//! admits, and each function `replaceable.txt` lists in the crates below it. The source tree
//! carries none of this: only a staged copy does, and everything the pass writes stays on the
//! line it annotates, so the staged source's line numbers are the pinned source's.
use crate::source::{self, GameFile};
use quote::quote;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{FnArg, ImplItem, Item, Pat, Type};

/// The list of replaceable functions below the game crate, one path per line.
pub(crate) const LIST: &str = "crates/horizon-mod-api-gen/replaceable.txt";

/// Whether a function with this signature and these attributes can take a replacement: its
/// slot holds one function pointer, so one compiled copy must stand for every call.
pub fn eligible(attrs: &[syn::Attribute], sig: &syn::Signature) -> bool {
    struct Opaque(bool);
    impl<'ast> syn::visit::Visit<'ast> for Opaque {
        fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
            self.0 = true;
        }
    }
    let mut opaque = Opaque(false);
    syn::visit::Visit::visit_signature(&mut opaque, sig);
    !opaque.0
        && sig.constness.is_none()
        && sig.asyncness.is_none()
        && sig.unsafety.is_none()
        && sig.abi.is_none()
        && sig.variadic.is_none()
        && lifetimes_only(&sig.generics)
        && !matches!(&sig.output, syn::ReturnType::Type(_, ty) if matches!(**ty, Type::Never(_)))
        && sig.inputs.iter().all(|input| match input {
            // `&self` and `&mut self` only: a by-value or typed receiver is not forwarded.
            FnArg::Receiver(receiver) => {
                receiver.reference.is_some()
                    && receiver.colon_token.is_none()
                    && receiver.attrs.is_empty()
            }
            FnArg::Typed(typed) => typed.attrs.is_empty(),
        })
        && !attrs.iter().any(|attr| {
            let path = attr.path();
            path.is_ident("cfg")
                || path.is_ident("cfg_attr")
                || path.is_ident("test")
                || path.is_ident("track_caller")
                || (path.is_ident("inline")
                    && attr
                        .parse_args::<syn::Ident>()
                        .is_ok_and(|arg| arg == "always"))
        })
}

/// Whether `generics` declares lifetimes only: a type or const parameter means one function per
/// instance, which one slot can't replace.
pub fn lifetimes_only(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .all(|param| matches!(param, syn::GenericParam::Lifetime(_)))
}

/// The installer of the function `name`: `__fnr_<name>`, in a free function's module or on an
/// inherent method's type; `__fnr_as_<Trait>_<name>` on the type for a method of its
/// `impl Trait`.
pub fn installer(trait_name: Option<&syn::Ident>, name: &syn::Ident) -> syn::Ident {
    match trait_name {
        Some(trait_name) => {
            quote::format_ident!("__fnr_as_{trait_name}_{}", source::unraw(name))
        }
        None => quote::format_ident!("__fnr_{}", source::unraw(name)),
    }
}

/// The trait of `block` when its methods can be replaced: a trait one of the game's own crates
/// declares (`traits`, by name), written without generic arguments, implemented safely for a
/// type its own crate declares: the installer is an inherent function of the type. The type is
/// written as one name, which the crate declares (`types`) and the file does not import from
/// another crate (`foreign`). Another crate's trait (`Ord`, `Iterator`, `Drop`) is the caller's
/// contract with that crate, and often a hot path.
pub fn replaceable_trait<'a>(
    block: &'a syn::ItemImpl,
    traits: &BTreeSet<String>,
    types: &BTreeSet<String>,
    foreign: &BTreeSet<String>,
) -> Option<&'a syn::Ident> {
    let (negative, path, _) = block.trait_.as_ref()?;
    let last = path.segments.last()?;
    let Type::Path(owner) = &*block.self_ty else {
        return None;
    };
    let [owner] = owner.path.segments.iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    let owner = owner.ident.to_string();
    (negative.is_none()
        && block.unsafety.is_none()
        && last.arguments.is_none()
        && traits.contains(&last.ident.to_string())
        && types.contains(&owner)
        && !foreign.contains(&owner))
    .then_some(&last.ident)
}

/// The names `items`' `use`s bring in from other crates: paths that don't start at `crate`,
/// `self` or `super`.
pub fn foreign_imports(items: &[Item]) -> BTreeSet<String> {
    fn leaves(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(path) => leaves(&path.tree, out),
            syn::UseTree::Name(name) => drop(out.insert(name.ident.to_string())),
            syn::UseTree::Rename(rename) => drop(out.insert(rename.rename.to_string())),
            syn::UseTree::Group(group) => group.items.iter().for_each(|tree| leaves(tree, out)),
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut out = BTreeSet::new();
    for item in items {
        if let Item::Use(item) = item
            && !matches!(&item.tree, syn::UseTree::Path(path) if path.ident == "crate" || path.ident == "self" || path.ident == "super")
        {
            leaves(&item.tree, &mut out);
        }
    }
    out
}

/// The names of the types `items` declare, inline modules included.
pub fn declared_types(items: &[Item], out: &mut BTreeSet<String>) {
    for item in items {
        match item {
            Item::Struct(item) => drop(out.insert(item.ident.to_string())),
            Item::Enum(item) => drop(out.insert(item.ident.to_string())),
            Item::Union(item) => drop(out.insert(item.ident.to_string())),
            Item::Mod(item) => {
                if let Some((_, inner)) = &item.content {
                    declared_types(inner, out);
                }
            }
            _ => {}
        }
    }
}

/// Whether `sig` names something through `Self::` (an associated type or constant of the trait):
/// its pointer type is written outside the trait's `impl`, where that path doesn't resolve.
fn names_associated(sig: &syn::Signature) -> bool {
    struct Find(bool);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.0 |= path.segments.len() > 1 && path.segments[0].ident == "Self";
            syn::visit::visit_path(self, path);
        }
    }
    let mut find = Find(false);
    syn::visit::Visit::visit_signature(&mut find, sig);
    find.0
}

/// The names of the traits the game's own crates declare: every `trait` item under `crates/`.
pub fn declared_traits(root: &Path) -> Result<BTreeSet<String>, String> {
    fn walk(dir: &Path, out: &mut BTreeSet<String>) -> Result<(), String> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("{}: {error}", dir.display())),
        };
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let text =
                    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                for (at, _) in text.match_indices("trait ") {
                    // An item keyword, not the end of a longer word or a doc sentence's.
                    let before = text[..at].lines().next_back().unwrap_or_default().trim();
                    if !matches!(
                        before,
                        "" | "pub" | "unsafe" | "pub unsafe" | "pub(crate)" | "pub(super)"
                    ) {
                        continue;
                    }
                    let name: String = text[at + "trait ".len()..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        out.insert(name);
                    }
                }
            }
        }
        Ok(())
    }
    let mut traits = BTreeSet::new();
    let crates = root.join("crates");
    for entry in fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let package = entry.map_err(|e| e.to_string())?.path();
        walk(&package.join("src"), &mut traits)?;
    }
    Ok(traits)
}

/// Elided lifetimes of a method's return borrow from its receiver: named.
struct ElideTo(syn::Lifetime);

impl syn::visit_mut::VisitMut for ElideTo {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(self.0.clone());
        }
        syn::visit_mut::visit_type_reference_mut(self, reference);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = self.0.clone();
        }
    }
    // A function pointer or trait object in the return elides on its own.
    fn visit_type_bare_fn_mut(&mut self, _: &mut syn::TypeBareFn) {}
    fn visit_type_trait_object_mut(&mut self, _: &mut syn::TypeTraitObject) {}
}

/// The function-pointer type of `sig`, as written inside its `impl`: the receiver as the first
/// parameter (`&Self` or `&mut Self`), the function's own lifetimes bound for any call, and a
/// return that borrows from the receiver as the method's elision says.
pub fn pointer_type(sig: &syn::Signature) -> proc_macro2::TokenStream {
    // The function's own lifetimes are renamed: a `for<>` binder may not shadow them where
    // they are in scope.
    struct Rename(Vec<(syn::Ident, syn::Lifetime)>);
    impl syn::visit_mut::VisitMut for Rename {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            if let Some((_, fresh)) = self.0.iter().find(|(name, _)| *name == lifetime.ident) {
                *lifetime = fresh.clone();
            }
        }
    }
    let mut rename = Rename(
        sig.generics
            .lifetimes()
            .enumerate()
            .map(|(index, param)| {
                (
                    param.lifetime.ident.clone(),
                    syn::Lifetime::new(&format!("'__fnr_l{index}"), proc_macro2::Span::call_site()),
                )
            })
            .collect(),
    );
    let mut lifetimes: Vec<syn::Lifetime> =
        rename.0.iter().map(|(_, fresh)| fresh.clone()).collect();
    let mut output = sig.output.clone();
    syn::visit_mut::VisitMut::visit_return_type_mut(&mut rename, &mut output);
    let first = sig.receiver().map(|written| {
        let named = written.reference.as_ref().and_then(|(_, lifetime)| {
            lifetime.clone().map(|mut lifetime| {
                syn::visit_mut::VisitMut::visit_lifetime_mut(&mut rename, &mut lifetime);
                lifetime
            })
        });
        let lifetime = named.unwrap_or_else(|| {
            let fresh = syn::Lifetime::new("'__fnr_s", proc_macro2::Span::call_site());
            lifetimes.push(fresh.clone());
            fresh
        });
        syn::visit_mut::VisitMut::visit_return_type_mut(
            &mut ElideTo(lifetime.clone()),
            &mut output,
        );
        let mutability = &written.mutability;
        quote!(&#lifetime #mutability Self,)
    });
    let types = sig.inputs.iter().filter_map(|input| match input {
        FnArg::Typed(typed) => {
            let mut ty = (*typed.ty).clone();
            syn::visit_mut::VisitMut::visit_type_mut(&mut rename, &mut ty);
            Some(ty)
        }
        FnArg::Receiver(_) => None,
    });
    let bound = if lifetimes.is_empty() {
        quote!()
    } else {
        quote!(for<#(#lifetimes),*>)
    };
    quote!(#bound fn(#first #(#types),*) #output)
}

/// A replaceable function, as found in a parsed file.
struct Found<'a> {
    module: String,
    sig: &'a syn::Signature,
    body: &'a syn::Block,
    /// The `impl` it is a method of.
    owner: Option<&'a syn::ItemImpl>,
    /// That `impl`'s trait, when it is one's.
    trait_name: Option<&'a syn::Ident>,
    /// Where the item that holds it ends: the `impl` block, or the function.
    end: proc_macro2::LineColumn,
}

impl Found<'_> {
    /// Its path, as a mixin's `target` names it.
    fn target(&self) -> String {
        match (self.owner, self.trait_name) {
            (Some(block), Some(trait_name)) => format!(
                "{}::<{} as {trait_name}>::{}",
                self.module,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (Some(block), None) => format!(
                "{}::{}::{}",
                self.module,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (None, _) => format!("{}::{}", self.module, self.sig.ident),
        }
    }
}

/// The replaceable functions among `items`: free functions, inherent methods and methods of the
/// game's own traits (`traits`) that every build compiles, inline modules included.
fn functions<'a>(
    module: &str,
    items: &'a [Item],
    traits: &BTreeSet<String>,
    types: &BTreeSet<String>,
    out: &mut Vec<Found<'a>>,
) {
    let foreign = foreign_imports(items);
    for item in items {
        match item {
            Item::Mod(inline) if !source::gated(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content
                    && !source::test_only(&inline.attrs)
                {
                    functions(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        traits,
                        types,
                        out,
                    );
                }
            }
            Item::Fn(function)
                if !source::gated(&function.attrs) && eligible(&function.attrs, &function.sig) =>
            {
                out.push(Found {
                    module: module.to_owned(),
                    sig: &function.sig,
                    body: &function.block,
                    owner: None,
                    trait_name: None,
                    end: function.block.brace_token.span.close().end(),
                });
            }
            Item::Impl(block)
                if !source::gated(&block.attrs)
                    && lifetimes_only(&block.generics)
                    && matches!(&*block.self_ty, Type::Path(ty) if ty.qself.is_none()) =>
            {
                let trait_name = replaceable_trait(block, traits, types, &foreign);
                if block.trait_.is_some() && trait_name.is_none() {
                    continue;
                }
                for member in &block.items {
                    if let ImplItem::Fn(method) = member
                        && eligible(&method.attrs, &method.sig)
                        && !(trait_name.is_some() && names_associated(&method.sig))
                    {
                        out.push(Found {
                            module: module.to_owned(),
                            sig: &method.sig,
                            body: &method.block,
                            owner: Some(block),
                            trait_name,
                            end: block.brace_token.span.close().end(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// The listed lower-crate functions, each with its line in the list for diagnostics.
pub(crate) fn listed(root: &Path) -> Result<Vec<(String, String)>, String> {
    let file = root.join(LIST);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", file.display())),
    };
    Ok(text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let path = line.split('#').next().unwrap_or_default().trim();
            (!path.is_empty()).then(|| (format!("{LIST}:{}", index + 1), source::plain_path(path)))
        })
        .collect())
}

/// What [`stage`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Functions given a replacement slot.
    pub functions: usize,
    /// Files rewritten.
    pub files: usize,
    /// Crates given a slot array.
    pub crates: usize,
}

/// One crate's files in the staged copy.
struct Crate {
    /// Its library root, where its slot array goes.
    lib: PathBuf,
    files: Vec<GameFile>,
    /// Whether only its listed functions are replaceable.
    listed_only: bool,
}

/// The staging pass: in the staged copy at `root`, in place, gives every replaceable function its
/// head check, reader and installer, and each crate that has one its slot array. Run on a copy,
/// never on a working tree.
pub fn stage(root: &Path) -> Result<Staged, String> {
    let traits = declared_traits(root)?;
    let list = listed(root)?;
    let mut used = vec![false; list.len()];
    let client = root.join("crates/horizon-client/src");
    let mut crates = vec![Crate {
        lib: client.join("lib.rs"),
        files: source::game_files(&client, true)?,
        listed_only: false,
    }];
    let lower: BTreeSet<&str> = list
        .iter()
        .map(|(_, path)| path.split("::").next().unwrap_or_default())
        .collect();
    for name in lower {
        if name == "horizon_client" {
            return Err(format!(
                "{LIST}: {name}: every function of the game crate's game module is replaceable already"
            ));
        }
        let (_, lib) = source::crate_root(root, name)?;
        crates.push(Crate {
            lib,
            files: source::crate_files(root, name)?,
            listed_only: true,
        });
    }
    let mut staged = Staged::default();
    for krate in &crates {
        let mut count = 0;
        // Each file's text as the pass leaves it, the crate root's included.
        let mut texts: BTreeMap<&Path, String> = BTreeMap::new();
        let mut parsed = Vec::new();
        let mut types = BTreeSet::new();
        for file in &krate.files {
            let text = fs::read_to_string(&file.path)
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            let items = syn::parse_file(&text)
                .map_err(|e| format!("{}: {e}", file.path.display()))?
                .items;
            declared_types(&items, &mut types);
            parsed.push((file, text, items));
        }
        for (file, text, items) in &parsed {
            let mut found = Vec::new();
            functions(&file.module, items, &traits, &types, &mut found);
            if krate.listed_only {
                found.retain(|found| {
                    let target = source::plain_path(&found.target());
                    list.iter()
                        .position(|(_, path)| *path == target)
                        .inspect(|&index| used[index] = true)
                        .is_some()
                });
            }
            let Some(text) = rewrite(&file.path, text, &found, &mut count)? else {
                continue;
            };
            staged.files += 1;
            texts.insert(&file.path, text);
        }
        if count == 0 {
            continue;
        }
        let root_text = match texts.remove(krate.lib.as_path()) {
            Some(text) => text,
            None => fs::read_to_string(&krate.lib)
                .map_err(|e| format!("{}: {e}", krate.lib.display()))?,
        };
        // The generated readers cast a slot's pointer back to its function's type: the staged
        // crate allows that cast by name where the source forbids unsafe code outright.
        let mut root_text =
            root_text.replacen("#![forbid(unsafe_code)]", "#![deny(unsafe_code)]", 1);
        root_text.push_str(&format!(
            "\n/// The replacement of each replaceable function of this crate, or null: see `horizon_mod_api_gen::replace`.\n#[doc(hidden)]\npub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; {count}] = [const {{ ::std::sync::atomic::AtomicPtr::new(::std::ptr::null_mut()) }}; {count}];\n"
        ));
        texts.insert(&krate.lib, root_text);
        for (path, text) in texts {
            fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        staged.functions += count;
        staged.crates += 1;
    }
    if let Some(((origin, path), _)) = list.iter().zip(&used).find(|(_, used)| !**used) {
        return Err(format!(
            "{origin}: {path}: no free function or inherent method every build compiles that can be replaced"
        ));
    }
    Ok(staged)
}

/// `text` with the head check, reader and installer of each of `found`, numbering their slots
/// from `count`, and with each `replaced!(f)` of a function it stages answered from `f`'s slot.
/// None when the file has nothing to stage.
fn rewrite(
    path: &Path,
    text: &str,
    found: &[Found<'_>],
    count: &mut usize,
) -> Result<Option<String>, String> {
    if found.is_empty() {
        return Ok(None);
    }
    let lines: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let at = |position| source::offset(text, &lines, position);
    // Replacements of byte ranges (an insertion is an empty range), applied in order.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut names = BTreeSet::new();
    let mut free = BTreeMap::new();
    for function in found {
        let slot = *count;
        *count += 1;
        let name = source::unraw(&function.sig.ident);
        let installer = installer(function.trait_name, &function.sig.ident);
        let reader = quote::format_ident!("__fnr_read_slot_{slot}");
        if function.owner.is_none() {
            free.insert(name.clone(), reader.to_string());
        }
        let scope = match function.owner {
            Some(block) => format!("{}::{installer}", source::owner(&block.self_ty)),
            None => name.clone(),
        };
        if !names.insert((function.module.clone(), scope)) {
            return Err(format!(
                "{}: {} is declared twice in one module: one of them can't be replaced",
                path.display(),
                function.target()
            ));
        }
        // The copy is called with the function's own arguments: a parameter bound by a pattern
        // is taken by a plain name first, and bound as written after the check.
        let mut arguments = Vec::new();
        let mut rebind = String::new();
        for (index, input) in function.sig.inputs.iter().enumerate() {
            match input {
                FnArg::Receiver(_) => arguments.push("self".to_owned()),
                FnArg::Typed(typed) => match &*typed.pat {
                    Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => {
                        arguments.push(pat.ident.to_string());
                    }
                    pat => {
                        let plain = format!("__fnr_p{index}");
                        let span = syn::spanned::Spanned::span(pat);
                        let (start, end) = (at(span.start()), at(span.end()));
                        if !matches!(pat, Pat::Wild(_)) {
                            rebind.push_str(&format!(" let {} = {plain};", &text[start..end]));
                        }
                        edits.push((start, end, plain.clone()));
                        arguments.push(plain);
                    }
                },
            }
        }
        let call = if function.owner.is_some() {
            format!("Self::{reader}()")
        } else {
            format!("{reader}()")
        };
        let open = at(function.body.brace_token.span.open().end());
        edits.push((
            open,
            open,
            format!(
                " if let ::std::option::Option::Some(__fnr) = {call} {{ return __fnr({}); }}{rebind}",
                arguments.join(", ")
            ),
        ));
        let ty = pointer_type(function.sig);
        let items = quote! {
            #[doc(hidden)]
            #[inline(always)]
            #[allow(unsafe_code, dead_code, clippy::all)]
            pub fn #reader() -> ::std::option::Option<#ty> {
                let installed = crate::__REPLACE[#slot].load(::std::sync::atomic::Ordering::Relaxed);
                if installed.is_null() {
                    ::std::option::Option::None
                } else {
                    // SAFETY: only this function's installer stores here, a pointer of this type.
                    ::std::option::Option::Some(unsafe { ::std::mem::transmute::<*mut (), #ty>(installed) })
                }
            }
            #[doc(hidden)]
            #[inline]
            #[allow(dead_code, non_snake_case, clippy::all)]
            pub fn #installer(replacement: #ty) -> bool {
                crate::__REPLACE[#slot]
                    .compare_exchange(
                        ::std::ptr::null_mut(),
                        replacement as *mut (),
                        ::std::sync::atomic::Ordering::Release,
                        ::std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
            }
        };
        let items = match function.owner {
            Some(block) => {
                let (generics, _, clause) = block.generics.split_for_impl();
                let ty = &block.self_ty;
                quote!(impl #generics #ty #clause { #items })
            }
            None => items,
        };
        let end = at(function.end);
        edits.push((end, end, format!(" {items}")));
    }
    // The placeholder macro has no callers after its invocations are rewritten. Remove its
    // declaration too, keeping line endings so source locations still match the pinned tree.
    let parsed = syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display()))?;
    for item in parsed.items {
        if let Item::Macro(item) = item
            && item.mac.path.is_ident("macro_rules")
            && item.ident.as_ref().is_some_and(|name| name == "replaced")
        {
            let span = syn::spanned::Spanned::span(&item);
            let (start, end) = (at(span.start()), at(span.end()));
            edits.push((
                start,
                end,
                text[start..end].chars().filter(|c| *c == '\n').collect(),
            ));
        }
    }
    // `replaced!(f)`: whether a copy of the free function `f` of this module is installed.
    let mut from = 0;
    while let Some(start) = text[from..].find("replaced!(").map(|i| i + from) {
        let inner = start + "replaced!(".len();
        let Some(close) = text[inner..].find(')').map(|i| i + inner) else {
            break;
        };
        from = close;
        let name = text[inner..close].trim();
        let Some(reader) = free.get(name) else {
            return Err(format!(
                "{}: replaced!({name}): no replaceable free function of that name in this file",
                path.display()
            ));
        };
        edits.push((start, close + 1, format!("{reader}().is_some()")));
    }
    // At one offset, a replacement of a range before an insertion there, each in pushed order.
    edits.sort_by_key(|(start, end, _)| (*start, *end == *start));
    let mut out =
        String::with_capacity(text.len() + edits.iter().map(|e| e.2.len()).sum::<usize>());
    let mut from = 0;
    for (start, end, replacement) in edits {
        if start < from {
            return Err(format!("{}: overlapping staging edits", path.display()));
        }
        out.push_str(&text[from..start]);
        out.push_str(&replacement);
        from = end;
    }
    out.push_str(&text[from..]);
    debug_assert_eq!(out.matches('\n').count(), text.matches('\n').count());
    Ok(Some(out))
}

/// The names of a file's replaceable functions, as targets: what the patch generator and tests
/// compare against.
pub fn targets(
    module: &str,
    file: &syn::File,
    traits: &BTreeSet<String>,
    types: &BTreeSet<String>,
) -> Vec<String> {
    let mut found = Vec::new();
    functions(module, &file.items, traits, types, &mut found);
    found.iter().map(Found::target).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn staged(source: &str) -> String {
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &mut found,
        );
        let mut count = 0;
        rewrite(Path::new("k.rs"), source, &found, &mut count)
            .unwrap()
            .unwrap_or_else(|| source.to_owned())
    }

    fn eligible_fn(source: &str) -> bool {
        let function: syn::ItemFn = syn::parse_str(source).unwrap();
        eligible(&function.attrs, &function.sig)
    }

    #[test]
    fn hook_functions_are_replaceable_and_single_instance_rules_refuse() {
        assert!(eligible_fn("fn native_tick(&mut self) {}"));
        assert!(eligible_fn(
            "fn borrow<'a>(&'a self, key: &str) -> &'a str { key }"
        ));
        for refused in [
            "const fn f() {}",
            "async fn f() {}",
            "unsafe fn f() {}",
            "fn f<T>(value: T) {}",
            "fn f<const N: usize>() {}",
            "fn f(value: impl Clone) {}",
            "fn f() -> impl Clone {}",
            "fn f(self) {}",
            "fn f(self: Box<Self>) {}",
            "fn f() -> ! { loop {} }",
            "#[inline(always)] fn f() {}",
            "#[track_caller] fn f() {}",
            "#[cfg(unix)] fn f() {}",
        ] {
            assert!(!eligible_fn(refused), "{refused}");
        }
    }

    #[test]
    fn a_method_checks_its_slot_first_and_forwards_its_own_arguments() {
        let out = staged(
            "impl Game<'_> {\n    fn next(&mut self, (a, b): (u32, u32), mut c: u8, _: bool) -> u32 {\n        a + b\n    }\n}\n",
        );
        assert!(
            out.contains(
                "fn next(&mut self, __fnr_p1: (u32, u32), mut c: u8, __fnr_p3: bool) -> u32 { if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { return __fnr(self, __fnr_p1, c, __fnr_p3); } let (a, b) = __fnr_p1;\n"
            ),
            "{out}"
        );
        // The reader and installer sit in an `impl` of the same type, after the method's own.
        let file = syn::parse_file(&out).unwrap();
        let Item::Impl(added) = &file.items[1] else {
            panic!("{out}")
        };
        assert_eq!(source::owner(&added.self_ty), "Game<'_>");
        let names: Vec<_> = added
            .items
            .iter()
            .map(|item| match item {
                ImplItem::Fn(function) => function.sig.ident.to_string(),
                _ => panic!("{out}"),
            })
            .collect();
        assert_eq!(names, ["__fnr_read_slot_0", "__fnr_next"]);
        assert_eq!(out.lines().count(), 5);
    }

    #[test]
    fn the_pointer_type_binds_the_functions_lifetimes_and_the_receivers_borrow() {
        let method: syn::ImplItemFn =
            syn::parse_str("fn name<'a>(&self, key: &'a str, other: &Self) -> &str { key }")
                .unwrap();
        assert_eq!(
            pointer_type(&method.sig).to_string(),
            quote!(
                for<'__fnr_l0, '__fnr_s> fn(&'__fnr_s Self, &'__fnr_l0 str, &Self) -> &'__fnr_s str
            )
            .to_string()
        );
        let free: syn::ItemFn = syn::parse_str("fn interval(rate: f32) -> i64 { 0 }").unwrap();
        assert_eq!(
            pointer_type(&free.sig).to_string(),
            quote!(fn(f32) -> i64).to_string()
        );
    }

    #[test]
    fn trait_installers_compile_with_warnings_denied() {
        let source = "pub struct Example; pub trait GameTrait { fn value(&self) -> u32; } impl GameTrait for Example { fn value(&self) -> u32 { 7 } }";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::from(["GameTrait".to_owned()]),
            &BTreeSet::from(["Example".to_owned()]),
            &mut found,
        );
        let mut count = 0;
        let mut out = rewrite(Path::new("k.rs"), source, &found, &mut count)
            .unwrap()
            .unwrap();
        assert_eq!(count, 1);
        out.push_str(" pub static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 1] = [std::sync::atomic::AtomicPtr::new(std::ptr::null_mut())];");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("fnr-trait-lint-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("fixture.rs");
        fs::write(&input, out).unwrap();
        let result = std::process::Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=lib",
                "--emit=metadata",
                "-Dwarnings",
            ])
            .arg(&input)
            .arg("--out-dir")
            .arg(&dir)
            .output()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn reader_names_do_not_collide_with_get_function_installers() {
        let out = staged(
            "struct Game; impl Game { fn projectile(&self) {} fn get_projectile(&self) {} }",
        );
        let file = syn::parse_file(&out).unwrap();
        let mut names = BTreeSet::new();
        for item in file.items {
            if let Item::Impl(block) = item {
                for item in block.items {
                    if let ImplItem::Fn(method) = item {
                        assert!(
                            names.insert(method.sig.ident.to_string()),
                            "duplicate method: {}",
                            method.sig.ident
                        );
                    }
                }
            }
        }
        assert!(names.contains("__fnr_get_projectile"));
        assert!(names.contains("__fnr_read_slot_0"));
    }

    #[test]
    fn slots_are_numbered_per_crate_and_replaced_reads_the_slot() {
        let source = "macro_rules! replaced { ($name:ident) => { false }; }\npub fn world_drawn(view: &mut View) { let _ = view; }\npub fn hooked() -> bool { replaced!(world_drawn) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &mut found,
        );
        let mut count = 7;
        let out = rewrite(Path::new("k.rs"), source, &found, &mut count)
            .unwrap()
            .unwrap();
        assert_eq!(count, 9);
        assert!(!out.contains("macro_rules!"));
        assert_eq!(source.lines().count(), out.lines().count());
        assert!(out.contains("crate :: __REPLACE [7usize]"), "{out}");
        assert!(out.contains("crate :: __REPLACE [8usize]"), "{out}");
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = __fnr_read_slot_8() { return __fnr(); } __fnr_read_slot_7().is_some() }"),
            "{out}"
        );
        syn::parse_file(&out).unwrap();
    }

    #[test]
    fn replaced_of_a_function_the_file_does_not_stage_is_refused() {
        let source = "pub fn hooked() -> bool { replaced!(elsewhere) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &mut found,
        );
        let error = rewrite(Path::new("k.rs"), source, &found, &mut 0).unwrap_err();
        assert!(error.contains("replaced!(elsewhere)"), "{error}");
    }

    #[test]
    fn other_crates_traits_generic_impls_and_test_modules_get_no_slot() {
        let file = syn::parse_file(
            "impl Clone for A { fn clone(&self) -> Self { A } }\nimpl<T> B<T> { fn f(&self) {} }\n#[cfg(test)] mod tests { fn t() {} }\nmod inner { fn kept() {} }\nfn free() {}\n",
        )
        .unwrap();
        assert_eq!(
            targets(
                "k",
                &file,
                &BTreeSet::from(["Goal".to_owned()]),
                &BTreeSet::new()
            ),
            ["k::inner::kept", "k::free"]
        );
    }

    #[test]
    fn a_method_of_the_games_own_trait_gets_a_slot_on_its_type() {
        let source = "use engine::Style; struct Wander; struct Raw; mod other { struct Style; }\nimpl Goal for Style { fn start(&mut self) {} } impl Goal for engine::Raw { fn start(&mut self) {} } impl Goal for Wander { fn start(&mut self, mob: &Mob) -> bool { true } fn kind(&self) -> Self::Kind { Kind } }\nimpl Goal<u8> for Wander { fn start(&mut self) {} }\nunsafe impl Goal for Raw { fn start(&mut self) {} }\nimpl Goal for Vec3 { fn start(&mut self) {} }\n";
        let traits = BTreeSet::from(["Goal".to_owned()]);
        let file = syn::parse_file(source).unwrap();
        let mut types = BTreeSet::new();
        declared_types(&file.items, &mut types);
        // A signature naming the trait's associated type, a trait written with arguments, an
        // unsafe impl and another crate's type (imported under a name the crate also declares,
        // written by path, or not declared here at all) are left as the game wrote them.
        assert_eq!(
            targets("k", &file, &traits, &types),
            ["k::<Wander as Goal>::start"]
        );
        let mut found = Vec::new();
        functions("k", &file.items, &traits, &types, &mut found);
        let out = rewrite(Path::new("k.rs"), source, &found, &mut 0)
            .unwrap()
            .unwrap();
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { return __fnr(self, mob); } true }"),
            "{out}"
        );
        let staged = syn::parse_file(&out).unwrap();
        let Some(Item::Impl(added)) = staged.items.get(7) else {
            panic!("{out}")
        };
        assert!(added.trait_.is_none());
        assert!(out.contains("pub fn __fnr_as_Goal_start"), "{out}");
    }

    #[test]
    fn declared_traits_are_the_trait_items_of_the_crates() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("crates/horizon-low/src/deep");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("a.rs"),
            "/// Not a trait Item, only words.\npub trait Goal {}\n    pub(crate) trait Inner: Goal {}\nunsafe trait Raw {}\nfn f(x: &dyn Portrait) {}\n",
        )
        .unwrap();
        assert_eq!(
            declared_traits(root.path()).unwrap(),
            BTreeSet::from(["Goal".to_owned(), "Inner".to_owned(), "Raw".to_owned()])
        );
    }

    #[test]
    fn the_pass_gives_each_crate_one_array_and_refuses_an_unknown_listed_function() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/horizon-client/Cargo.toml",
            "[package]\nname = 'horizon-client'\n",
        );
        write(
            "crates/horizon-client/src/lib.rs",
            "#![forbid(unsafe_code)]\nmod game;\n",
        );
        write(
            "crates/horizon-client/src/game.rs",
            "impl Game<'_> { fn native_tick(&mut self) {} }\nfn free(a: u8) -> u8 { a }\n",
        );
        write(
            "crates/horizon-low/Cargo.toml",
            "[package]\nname = 'horizon-low'\n",
        );
        write(
            "crates/horizon-low/src/lib.rs",
            "pub fn drawn() {}\npub fn unlisted() {}\n",
        );
        write(LIST, "# listed\nhorizon_low::drawn\n");
        let staged = stage(root.path()).unwrap();
        assert_eq!(
            staged,
            Staged {
                functions: 3,
                files: 2,
                crates: 2
            }
        );
        let lib = fs::read_to_string(root.path().join("crates/horizon-client/src/lib.rs")).unwrap();
        assert!(lib.starts_with("#![deny(unsafe_code)]\n"), "{lib}");
        assert!(lib.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 2]"));
        let low = fs::read_to_string(root.path().join("crates/horizon-low/src/lib.rs")).unwrap();
        assert!(
            low.contains("__fnr_drawn") && !low.contains("__fnr_unlisted"),
            "{low}"
        );
        assert!(low.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 1]"));
        syn::parse_file(&low).unwrap();

        write(LIST, "horizon_low::missing\n");
        write("crates/horizon-low/src/lib.rs", "pub fn drawn() {}\n");
        let error = stage(root.path()).unwrap_err();
        assert!(
            error.contains("replaceable.txt:1: horizon_low::missing"),
            "{error}"
        );
    }
}
