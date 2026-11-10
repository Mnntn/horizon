//! Mod points (#2339): every function of a mod crate that [`point_shape::eligible`] admits gets a
//! point, public and private, by the engine's own rules: the same walk ([`super::module_files`],
//! [`super::file_functions`]), the same `#[point]`, records and raw forms, in a slot table of the
//! mod's own. `#[mod_api::no_point]` opts a function out. [`stage_mod`] writes them into a staged
//! copy of the crate, the copy `cargo horizon-mod build` and the launcher compile, and lists them
//! as the mod's `mod-points.json`, each with the signature hash an addon's declaration pins.
//!
//! A parameter crosses an `api` record when its type has a `Wire` form: scalars, text and std
//! containers, `mod_api`'s types, and the mod's own publicly reachable types that carry
//! `#[derive(Wire)]` (or a hand-written `impl Wire`), as [`crate::wire_types`] finds them. Nothing adds the derive: any other parameter makes the point's API
//! listeners refuse, naming it, and leaves its `internals` form.
use super::{Found, GamePoint, WireTypes, file_functions, imports, module_files, unraw};
use point_shape::catalog::Catalog;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use syn::Type;

/// The module a mod crate's slot table is generated into, at its crate root.
pub const TABLE: &str = "__horizon_points";

/// The mod crate at `dir`: its library's name and root file.
fn library(dir: &Path) -> Result<(String, PathBuf), String> {
    let manifest = dir.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let cargo: toml::Value =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let lib = cargo.get("lib");
    let name = lib
        .and_then(|lib| lib.get("name"))
        .or_else(|| cargo.get("package").and_then(|p| p.get("name")))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{}: no package name", manifest.display()))?
        .replace('-', "_");
    let root = lib
        .and_then(|lib| lib.get("path"))
        .and_then(toml::Value::as_str)
        .unwrap_or("src/lib.rs");
    Ok((name, dir.join(root)))
}

/// One parsed file of the crate.
struct File {
    path: PathBuf,
    module: String,
    source: String,
    parsed: syn::File,
}

fn files(dir: &Path) -> Result<(String, PathBuf, Vec<File>), String> {
    let (krate, root) = library(dir)?;
    let folder = root
        .parent()
        .ok_or("a library root has a folder")?
        .to_owned();
    let mut out = Vec::new();
    for file in module_files(krate.clone(), root.clone(), folder, true)? {
        let source = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
        let parsed = crate::game_modules::parse(&file.path, &source)?;
        out.push(File {
            path: file.path,
            module: file.module,
            source,
            parsed,
        });
    }
    Ok((krate, root, out))
}

/// The `api(…)` keys of `found`'s signature: each leaf naming a type with a `Wire` form, the
/// written spelling with the path its record names it by.
fn api_types(
    found: &Found<'_>,
    krate: &str,
    uses: &[(String, String)],
    own: &BTreeSet<String>,
    wire: &WireTypes,
) -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = Vec::new();
    for leaf in point_shape::leaves(found.sig) {
        let written = point_shape::spelling(&leaf);
        if keys.iter().any(|(seen, _)| *seen == written) {
            continue;
        }
        let full = match leaf.get_ident() {
            Some(ident) => match uses.iter().find(|(name, _)| ident == name) {
                Some((_, path)) => path.clone(),
                None => format!("{}::{ident}", found.module),
            },
            None => {
                let path = written.trim_start_matches("::");
                match path.strip_prefix("crate::") {
                    Some(rest) => format!("{krate}::{rest}"),
                    None => path.to_owned(),
                }
            }
        };
        // A path relative to the module (`use tanks::Tank` in the crate root) names a child.
        let relative = format!("{}::{full}", found.module);
        let full = if own.contains(&relative) {
            relative
        } else {
            full
        };
        if own.contains(&full) {
            let local = full.strip_prefix(krate).unwrap_or(&full);
            keys.push((written, format!("crate{local}")));
        } else if let Some(api) = super::wire_type(wire, &full) {
            keys.push((written, format!("::{api}")));
        }
    }
    keys
}

/// The point's name: its path from the crate root, the `impl`'s type snake-cased, then the
/// function (`tanks_tank_fill` for `chemlib::tanks::Tank::fill`).
fn name(found: &Found<'_>, krate: &str) -> String {
    let mut parts: Vec<String> = found
        .module
        .strip_prefix(krate)
        .unwrap_or(&found.module)
        .split("::")
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    if let Some((Type::Path(path), _)) = found.owner
        && let Some(last) = path.path.segments.last()
    {
        parts.push(super::snake(&unraw(&last.ident)));
    }
    parts.push(unraw(&found.sig.ident));
    parts.join("_")
}

/// What an addon's declaration pins: the function's canonical path and its signature as written
/// (receiver, parameter names and types, return type), and the `api` record's field types.
fn signature(target: &str, sig: &syn::Signature, shape: &point_shape::Shape) -> String {
    let mut sig = sig.clone();
    sig.ident = syn::Ident::new("f", proc_macro2::Span::call_site());
    let fields: Vec<String> = shape
        .args
        .iter()
        .map(|arg| point_shape::spelling(&arg.field))
        .collect();
    format!(
        "{target}: {} [{}]",
        point_shape::spelling(&sig),
        fields.join(", ")
    )
}

/// One point found in the crate, with the file it is in and its `#[point]` arguments.
struct Listed<'a> {
    file: usize,
    found: Found<'a>,
    attribute: String,
    point: GamePoint,
}

fn listed<'a>(files: &'a [File], krate: &str, wire: &WireTypes) -> Result<Vec<Listed<'a>>, String> {
    let own: BTreeSet<String> = crate::wire_types::reachable(
        krate,
        files
            .iter()
            .map(|file| (file.module.as_str(), file.parsed.items.as_slice())),
    )?
    .into_values()
    .collect();
    let mut out = Vec::new();
    let mut names = std::collections::BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        let mut found = Vec::new();
        file_functions(
            &file.module,
            &file.parsed.items,
            &|_| true,
            &point_shape::eligible,
            &mut found,
        );
        let uses = imports(&file.parsed.items, krate);
        for found in found {
            if found.attrs.iter().any(|attr| {
                attr.path()
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "point" || segment.ident == "__point")
            }) {
                return Err(format!(
                    "{}: {} already has a #[point]: list and stage the mod's own source, not a staged copy",
                    file.path.display(),
                    found.target()
                ));
            }
            let name = name(&found, krate);
            let mut attribute = vec![
                format!("name = {name:?}"),
                format!("table = \"crate::{TABLE}\""),
            ];
            if point_shape::raw_able(found.sig, found.owner.map(|(_, generics)| generics)) {
                attribute.push("raw".into());
            }
            if found.owner.is_none() {
                attribute.push("free".into());
            }
            let keys = api_types(&found, krate, &uses, &own, wire);
            if !keys.is_empty() {
                let keys: Vec<_> = keys.iter().map(|(k, v)| format!("{k} = {v:?}")).collect();
                attribute.push(format!("api({})", keys.join(", ")));
            }
            let attribute = attribute.join(", ");
            let shape = point_shape::shape(attribute.parse().expect("the attribute"), found.sig)
                .map_err(|e| format!("{}: {e}", found.target()))?;
            for variant in [shape.variant(), shape.return_variant()] {
                if let Some(other) = names.insert(variant.to_string(), found.target()) {
                    return Err(format!(
                        "the points of {other} and {} have one name, {name}: rename one, or opt \
                         one out with #[mod_api::no_point]",
                        found.target()
                    ));
                }
            }
            let target = found.target();
            let signature = signature(&target, found.sig, &shape);
            let mut point = GamePoint::new(shape, &found, target);
            // Raw forms are named from the crate root, where the table module is.
            let local =
                |path: String| format!("crate{}", path.strip_prefix(krate).unwrap_or(&path));
            point.listener = point.listener.take().map(local);
            point.return_listener = point.return_listener.take().map(local);
            point.hash = Some(crate::internals::signature_hash(&signature));
            point.signature = Some(signature);
            out.push(Listed {
                file: index,
                found,
                attribute,
                point,
            });
        }
    }
    Ok(out)
}

/// The functions `#[mod_api::no_point]` opted out, by path: eligible but for the attribute.
fn opted_out(files: &[File]) -> Vec<String> {
    let no_point = |attr: &syn::Attribute| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "no_point")
    };
    let mut found = Vec::new();
    for file in files {
        file_functions(
            &file.module,
            &file.parsed.items,
            &|_| true,
            &|attrs, sig| {
                let rest: Vec<_> = attrs.iter().filter(|a| !no_point(a)).cloned().collect();
                rest.len() < attrs.len() && point_shape::eligible(&rest, sig)
            },
            &mut found,
        );
    }
    let mut paths: Vec<_> = found.iter().map(Found::target).collect();
    paths.sort();
    paths
}

/// `points` as the mod's `mod-points.json` lists them.
fn catalog(krate: String, files: &[File], mut points: Vec<GamePoint>) -> Catalog {
    points.sort_by(|a, b| a.name.cmp(&b.name));
    Catalog {
        krate,
        table: TABLE.to_owned(),
        opted_out: opted_out(files),
        points: points
            .into_iter()
            .map(|point| point_shape::catalog::Point {
                hash: point.hash.unwrap_or_default(),
                signature: point.signature.unwrap_or_default(),
                raw: point.raw,
                at_return: point.at_return,
                api_refusal: point.api_refusal,
                returns: point.returns.to_owned(),
                host: point.host.to_owned(),
                return_ty: point.return_ty,
                listener: point.listener,
                return_listener: point.return_listener,
                name: point.name,
                target: point.target,
            })
            .collect(),
    }
}

/// The mod crate at `dir`'s points, as its `mod-points.json` lists them, sorted by name. `wire` is
/// the `mod_api` types with a `Wire` layout (the game's `points.json` lists them).
pub fn mod_points(dir: &Path, wire: &WireTypes) -> Result<Catalog, String> {
    let (krate, _, files) = files(dir)?;
    let points = listed(&files, &krate, wire)?
        .into_iter()
        .map(|listed| listed.point)
        .collect();
    Ok(catalog(krate, &files, points))
}

/// The staging pass for a mod crate: in the copy at `dir`, in place, puts `#[point]` on every
/// point function, declares each raw form's types after the item that holds it, and writes the
/// crate's slot table module at its root. Run on a copy, never on the author's source. Returns
/// the points, as [`mod_points`] lists them.
pub fn stage_mod(dir: &Path, wire: &WireTypes) -> Result<Catalog, String> {
    let (krate, root, files) = files(dir)?;
    let listed = listed(&files, &krate, wire)?;
    for (index, file) in files.iter().enumerate() {
        let here: Vec<_> = listed.iter().filter(|l| l.file == index).collect();
        if here.is_empty() {
            continue;
        }
        let source = &file.source;
        let lines: Vec<usize> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let mut edits: Vec<(usize, String)> = Vec::new();
        for listed in &here {
            let found = &listed.found;
            let start = match found.vis {
                syn::Visibility::Inherited => found.sig.fn_token.span.start(),
                vis => syn::spanned::Spanned::span(vis).start(),
            };
            edits.push((
                super::offset(source, &lines, start),
                format!("#[::mod_api::__point({})] ", listed.attribute),
            ));
            let shape =
                point_shape::shape(listed.attribute.parse().expect("the attribute"), found.sig)
                    .map_err(|e| format!("{}: {e}", found.target()))?;
            if shape.raw {
                let items = shape.raw_items(
                    found.sig,
                    found.owner.map(|(ty, _)| ty),
                    found.owner.map(|(_, generics)| generics),
                );
                edits.push((
                    super::offset(source, &lines, found.end),
                    format!("\n{items}\n"),
                ));
            }
        }
        edits.sort_by_key(|(at, _)| *at);
        let mut out = String::with_capacity(source.len());
        let mut from = 0;
        for (at, text) in edits {
            out.push_str(&source[from..at]);
            out.push_str(&text);
            from = at;
        }
        out.push_str(&source[from..]);
        fs::write(&file.path, out).map_err(|e| format!("{}: {e}", file.path.display()))?;
    }
    let mut points: Vec<GamePoint> = listed.into_iter().map(|listed| listed.point).collect();
    points.sort_by(|a, b| a.name.cmp(&b.name));
    let module = table(&points);
    let catalog = catalog(krate, &files, points);
    let folder = root.parent().ok_or("a library root has a folder")?;
    fs::write(folder.join(format!("{TABLE}.rs")), module).map_err(|e| e.to_string())?;
    let mut text = fs::read_to_string(&root).map_err(|e| e.to_string())?;
    text.push_str(&format!(
        "\n// hidden: the mod's generated injection point table, reached by addons' listeners\n#[doc(hidden)]\n#[path = \"{TABLE}.rs\"]\npub mod {TABLE};\n"
    ));
    fs::write(&root, text).map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(catalog)
}

/// The slot table module: each point's record, `PointId`, the table, typed API listener
/// constructors and dispatch, and typed internals listener constructors.
pub(crate) fn table(points: &[GamePoint]) -> String {
    let parse = |code: &str| {
        code.parse::<proc_macro2::TokenStream>()
            .expect("generated points")
    };
    // A record crosses to a wasm guest with its `Wire` form. The game's records also derive it
    // under the game's `wasm-guest` feature; a mod crate has no such feature, so its records
    // derive it on wasm32 alone and the staged crate builds clean under `-D unexpected_cfgs`.
    let records = points.iter().map(|point| {
        let mut file: syn::File = syn::parse_str(&point.record_item).expect("point record");
        for item in &mut file.items {
            if let syn::Item::Struct(record) = item {
                for attr in &mut record.attrs {
                    if attr.path().is_ident("cfg_attr") {
                        *attr = syn::parse_quote!(
                            #[cfg_attr(target_arch = "wasm32", derive(::mod_api::Wire))]
                        );
                    }
                }
            }
        }
        file.into_token_stream()
    });
    let listeners = points.iter().map(|point| parse(&point.api[0]));
    let variants = points.iter().map(|point| parse(&point.api[1]));
    let dispatch = points.iter().map(|point| parse(&point.api[2]));
    let raw: Vec<_> = points.iter().filter(|point| point.raw).flat_map(|point| {
        let head = point.listener.clone().map(|listener| (point.name.clone(), listener));
        let back = point
            .return_listener
            .clone()
            .map(|listener| (format!("{}_return", point.name), listener));
        head.into_iter().chain(back).map(|(place, listener)| {
            let variant = quote::format_ident!("{}", point_shape::camel(&place));
            let method = quote::format_ident!("{place}");
            let listener: syn::Path = syn::parse_str(&listener).expect("a listener path");
            let doc = format!("Registers an internals listener at `{place}`: a function of its exact raw listener type.");
            quote! {
                #[doc = #doc]
                pub const fn #method(id: &'static str, listener: #listener) -> super::__private::SlotListener {
                    // SAFETY: `listener` has this place's raw listener type, checked by the signature.
                    unsafe { super::__private::SlotListener::new(id, PointId::#variant.place(), listener as *const ()) }
                }
            }
        })
    }).collect();
    // The wrapper executes in the library member, whose private mod_api owns its registry.
    // Only typed function pointers and scalar metadata cross the dynamic-link boundary.
    let mut registration = Vec::new();
    for (module, source) in [
        (
            "listeners",
            points.iter().map(|p| p.api[0].clone()).collect::<Vec<_>>(),
        ),
        ("raw", raw.iter().map(ToString::to_string).collect()),
    ] {
        for source in source {
            let file: syn::File = syn::parse_str(&source).expect("generated constructors");
            for item in file.items {
                let syn::Item::Fn(function) = item else {
                    continue;
                };
                let name = &function.sig.ident;
                let bridge = quote::format_ident!("register_{module}_{name}");
                let mut listener = function
                    .sig
                    .inputs
                    .last()
                    .expect("listener parameter")
                    .clone();
                if module == "listeners"
                    && let syn::FnArg::Typed(argument) = &mut listener
                {
                    let ty = &argument.ty;
                    *argument.ty = syn::parse_quote!(listeners::#ty);
                }
                let module = quote::format_ident!("{module}");
                let constructor = if module == "raw" {
                    quote!(#module::#name(id, listener).ordered(priority, injection))
                } else {
                    quote!(#module::#name(id, priority, injection, listener))
                };
                registration.push(quote! {
                    /// Registers in this library member's registry; false means registration was refused.
                    #[inline(never)]
                    pub fn #bridge(id: &'static str, priority: i32, injection: &'static str, replace: bool, #listener) -> bool {
                        let slot = #constructor;
                        let slot = if replace { slot.replacing() } else { slot };
                        ::mod_api::__private::points::register(&slot).is_ok()
                    }
                });
            }
        }
    }
    let doc =
        "A mod's injection points (#2339): a slot per place, set at load where an addon listens.";
    quote! {
        #![doc = #doc]
        #![allow(missing_docs, dead_code, clippy::all)]
        #(#records)*
        #(#registration)*
        /// Seals this library member's registry after all group members registered.
        #[inline(never)]
        pub fn seal() { ::mod_api::__private::points::seal(); }
        /// A point's place: its head and return.
        #[repr(usize)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum PointId { #(#variants)* __Count }
        /// The number of places, the length of the mod's slot table.
        pub const POINT_COUNT: usize = PointId::__Count as usize;
        impl PointId {
            /// This place in the mod's table, for a listener's constructor.
            pub const fn place(self) -> ::mod_api::__private::points::Place {
                ::mod_api::__private::points::Place::new(&__private::TABLE, self as usize)
            }
        }
        /// The table's side of the points.
        pub mod __private {
            use super::PointId;
            use ::std::sync::atomic::AtomicBool;
            pub use ::mod_api::__private::points::{ApiKind, PointHost, RawKind, SlotListener};
            /// Each place's slot: set once, at the seal, where an addon listens.
            pub static POINTS: [AtomicBool; super::POINT_COUNT] = [const { AtomicBool::new(false) }; super::POINT_COUNT];
            static SITES: [AtomicBool; 0] = [];
            /// The mod's table.
            pub static TABLE: ::mod_api::__private::points::Table = ::mod_api::__private::points::Table::new(&POINTS, &SITES);
            /// Whether an active `replace` listener owns `place`'s outcome.
            #[inline]
            pub fn replaced(place: PointId) -> bool { TABLE.replaced(place as usize) }
            /// Runs the API listeners at `place`.
            #[inline]
            pub fn fire_api(place: PointId, call: &mut dyn FnMut(ApiKind)) { TABLE.fire_api(place as usize, call) }
            /// Runs the internals listeners at `place`.
            #[inline]
            pub fn fire_raw(place: PointId, call: &mut dyn FnMut(RawKind)) { TABLE.fire_raw(place as usize, call) }
        }
        /// Typed API listener aliases and constructors.
        pub mod listeners { use super::*; #(#listeners)* }
        /// The points' typed dispatch.
        pub mod dispatch { use super::*; #(#dispatch)* }
        /// Typed internals listener constructors.
        pub mod raw { use super::*; #(#raw)* }
    }
    .to_string()
}

#[cfg(test)]
#[path = "mod_crate_tests.rs"]
mod tests;
