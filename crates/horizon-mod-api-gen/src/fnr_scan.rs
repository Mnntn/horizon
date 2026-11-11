//! Spike only (fn-replace-spike): which of the game's points could move into a replacement crate.
//! Run: `cargo test -p horizon-mod-api-gen --lib fnr_scan -- --ignored --nocapture`.
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::Visit;

/// Macros that mean the same thing in any crate.
const PORTABLE: &[&str] = &[
    "format",
    "vec",
    "matches",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "concat",
    "stringify",
    "format_args",
    "dbg",
    "quote",
];
/// Macros whose expansion depends on the crate or file they sit in.
const PLACE: &[&str] = &[
    "include_str",
    "include_bytes",
    "include",
    "file",
    "line",
    "column",
    "module_path",
    "env",
    "option_env",
    "cfg",
    "thread_local",
];
const LOGGING: &[&str] = &["trace", "debug", "info", "warn", "error", "log"];

#[derive(Default)]
struct Body {
    macros: BTreeSet<String>,
    nested_impl: bool,
    nested_static: bool,
    inner_cfg: bool,
    relative_paths: usize,
}

impl<'ast> Visit<'ast> for Body {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            let first = mac.path.segments.first().unwrap().ident.to_string();
            let name = if mac.path.segments.len() > 1 && first != "std" && first != "core" {
                format!("{first}::{}", last.ident)
            } else {
                last.ident.to_string()
            };
            self.macros.insert(name);
        }
        syn::visit::visit_macro(self, mac);
    }
    fn visit_item(&mut self, item: &'ast syn::Item) {
        match item {
            syn::Item::Impl(_) => self.nested_impl = true,
            syn::Item::Static(_) => self.nested_static = true,
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
            self.inner_cfg = true;
        }
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first() {
            let first = first.ident.to_string();
            if first == "super" || first == "crate" || first == "self" {
                self.relative_paths += 1;
            }
        }
        syn::visit::visit_path(self, path);
    }
}

#[test]
#[ignore = "spike measurement, not a test"]
fn fnr_scan() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = root.join("crates/horizon-client/src");
    let files = crate::points::game_files(&src, false).unwrap();
    // Crate-local macro_rules and whether each is exported.
    let mut local: BTreeMap<String, bool> = BTreeMap::new();
    for krate in [
        "crates/horizon-client/src",
        "crates/horizon-client-render/src",
        "crates/horizon-client-core/src",
        "crates/horizon-world/src",
        "crates/horizon-core/src",
    ] {
        for entry in walk(&root.join(krate)) {
            let text = std::fs::read_to_string(&entry).unwrap();
            for (i, _) in text.match_indices("macro_rules!") {
                let name: String = text[i + 12..]
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let exported = text[..i].trim_end().ends_with("#[macro_export]");
                *local.entry(name).or_default() |= exported;
            }
        }
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0;
    let mut movable = 0;
    for file in files {
        let text = std::fs::read_to_string(&file.path).unwrap();
        let parsed = syn::parse_file(&text).unwrap();
        let mut found = Vec::new();
        crate::points::file_functions(
            &file.module,
            &parsed.items,
            &|_| true,
            &|attrs, sig| point_shape::eligible(attrs, sig),
            &mut found,
        );
        for function in found {
            total += 1;
            let mut body = Body::default();
            body.visit_block(function.body);
            let mut blockers = Vec::new();
            let generic = function
                .sig
                .generics
                .params
                .iter()
                .any(|p| !matches!(p, syn::GenericParam::Lifetime(_)));
            if generic {
                blockers.push("generic");
            }
            if body.nested_impl {
                blockers.push("impl item in body");
            }
            if body.nested_static {
                blockers.push("static in body");
            }
            if body.inner_cfg {
                blockers.push("cfg inside body");
            }
            for name in &body.macros {
                let plain = name.rsplit("::").next().unwrap();
                if PORTABLE.contains(&plain) {
                    continue;
                }
                if PLACE.contains(&plain) {
                    blockers.push("place macro (file!/include_str!/module_path!/env!/cfg!)");
                } else if LOGGING.contains(&plain) {
                    *counts
                        .entry("logging macro (target = module_path changes)")
                        .or_default() += 1;
                } else if let Some(exported) = local.get(plain) {
                    if *exported {
                        *counts
                            .entry("exported crate macro (ok via $crate)")
                            .or_default() += 1;
                    } else {
                        blockers.push("crate-local macro_rules (not exported)");
                    }
                } else {
                    *unknown.entry(name.clone()).or_default() += 1;
                }
            }
            if body.relative_paths > 0 {
                *counts
                    .entry("uses super::/crate::/self:: paths (rewritable)")
                    .or_default() += 1;
            }
            blockers.sort();
            blockers.dedup();
            for blocker in &blockers {
                *counts.entry(blocker).or_default() += 1;
            }
            if blockers.is_empty() {
                movable += 1;
            }
        }
    }
    println!("eligible point functions scanned: {total}");
    println!(
        "movable by body rules: {movable} ({:.1}%)",
        100.0 * movable as f64 / total as f64
    );
    for (what, n) in counts {
        println!("  {what}: {n} ({:.1}%)", 100.0 * n as f64 / total as f64);
    }
    let mut unknown: Vec<_> = unknown.into_iter().collect();
    unknown.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!(
        "other macros (dependency crates, counted as portable): {:?}",
        &unknown[..unknown.len().min(25)]
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
