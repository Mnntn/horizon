//! Conservative syntax-aware test fixture opt-in. Run from the repository root.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{
    spanned::Spanned,
    visit::{self, Visit},
};
const STANDARD: &str = "crate::game::native_entity_refs::tests::with_game";
const CLIENT: &str = "crate::game::native_entity_refs::tests::with_client_game";
fn path(p: &syn::Path) -> String {
    p.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}
fn imports(tree: &syn::UseTree, prefix: String, out: &mut BTreeMap<String, String>) {
    match tree {
        syn::UseTree::Path(p) => imports(&p.tree, format!("{prefix}{}::", p.ident), out),
        syn::UseTree::Name(n) => {
            bind(out, n.ident.to_string(), format!("{prefix}{}", n.ident));
        }
        syn::UseTree::Rename(n) => {
            bind(out, n.rename.to_string(), format!("{prefix}{}", n.ident));
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                imports(t, prefix.clone(), out);
            }
        }
        syn::UseTree::Glob(_) => {
            out.insert("*".into(), prefix);
        }
    }
}
fn bind(out: &mut BTreeMap<String, String>, name: String, value: String) {
    if out.insert(name.clone(), value).is_some() {
        out.insert(name, "<ambiguous>".into());
    }
}
#[derive(Default)]
struct Shadows {
    names: BTreeSet<String>,
}
impl<'a> Visit<'a> for Shadows {
    fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
        self.names.insert(p.ident.to_string());
        visit::visit_pat_ident(self, p);
    }
    fn visit_item_use(&mut self, u: &'a syn::ItemUse) {
        let mut b = BTreeMap::new();
        imports(&u.tree, String::new(), &mut b);
        self.names.extend(b.into_keys());
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.names.insert(f.sig.ident.to_string());
        visit::visit_item_fn(self, f);
    }
}
struct Calls {
    names: BTreeSet<String>,
}
impl<'a> Visit<'a> for Calls {
    fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            self.names.insert(path(&p.path));
        }
        visit::visit_expr_call(self, c);
    }
}
struct Scan<'a> {
    file: &'a str,
    source: &'a str,
    bindings: BTreeMap<String, String>,
    locals: BTreeSet<String>,
    risks: Vec<String>,
    function: String,
    records: &'a mut Vec<serde_json::Value>,
    edits: &'a mut Vec<(usize, usize, String)>,
}
impl<'a> Visit<'a> for Scan<'_> {
    fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*c.func {
            let name = path(&p.path);
            let bound = self.bindings.get(&name).unwrap_or(&name);
            let last = p.path.segments.last().unwrap().ident.to_string();
            if [
                "with_game",
                "with_client_game",
                "with_game_at",
                "with_frozen_game",
                "with_frozen_client_game",
            ]
            .contains(&last.as_str())
                || [STANDARD, CLIENT].contains(&bound.as_str())
            {
                let mut reasons = self.risks.clone();
                if bound != STANDARD && bound != CLIENT {
                    reasons.push("unresolved, custom, relative or local binding".into());
                }
                let mut calls = Calls {
                    names: BTreeSet::new(),
                };
                calls.visit_expr_call(c);
                if calls
                    .names
                    .iter()
                    .any(|n| n != &name && !n.contains("::") && self.locals.contains(n))
                {
                    reasons.push("indirect local helper".into());
                }
                if self.locals.contains(&name) {
                    reasons.push("local function shadow".into());
                }
                let span = p.span();
                let start = offset(self.source, span.start());
                let end = offset(self.source, span.end());
                let eligible = reasons.is_empty();
                self.records.push(serde_json::json!({"path":self.file,"line":span.start().line,"function":self.function,"binding":bound,"eligible":eligible,"skip_reasons":reasons}));
                if eligible {
                    self.edits.push((
                        start,
                        end,
                        format!(
                            "crate::game::native_entity_refs::tests::{}",
                            if bound == CLIENT {
                                "with_frozen_client_game"
                            } else {
                                "with_frozen_game"
                            }
                        ),
                    ));
                }
            }
        }
        visit::visit_expr_call(self, c);
    }
}
fn offset(s: &str, p: proc_macro2::LineColumn) -> usize {
    s.split_inclusive('\n')
        .take(p.line - 1)
        .map(str::len)
        .sum::<usize>()
        + p.column
}
fn walk(
    items: &[syn::Item],
    bindings: &BTreeMap<String, String>,
    source: &str,
    file: &str,
    records: &mut Vec<serde_json::Value>,
    edits: &mut Vec<(usize, usize, String)>,
    file_risks: &[String],
) {
    let mut bindings = bindings.clone();
    let mut locals = BTreeSet::new();
    for item in items {
        match item {
            syn::Item::Use(u) => imports(&u.tree, String::new(), &mut bindings),
            syn::Item::Fn(f) => {
                locals.insert(f.sig.ident.to_string());
            }
            _ => {}
        }
    }
    for item in items {
        match item {
            syn::Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    walk(items, &bindings, source, file, records, edits, file_risks)
                }
            }
            syn::Item::Fn(f) => {
                let mut shadows = Shadows::default();
                shadows.visit_block(&f.block);
                let mut scoped_locals = locals.clone();
                scoped_locals.extend(shadows.names);
                let mut scan = Scan {
                    file,
                    source,
                    bindings: bindings.clone(),
                    locals: scoped_locals,
                    risks: file_risks.to_vec(),
                    function: f.sig.ident.to_string(),
                    records,
                    edits,
                };
                scan.visit_block(&f.block);
            }
            _ => {}
        }
    }
}
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            files(&p, out)
        } else if p.extension().is_some_and(|s| s == "rs") {
            out.push(p)
        }
    }
}
fn main() {
    let apply = std::env::args().any(|a| a == "--apply");
    let mut paths = Vec::new();
    files(Path::new("crates/horizon-client/src/game"), &mut paths);
    paths.sort();
    let mut records = Vec::new();
    for p in paths {
        let source = fs::read_to_string(&p).unwrap();
        if !source.contains("with_game") && !source.contains("with_client_game") {
            continue;
        }
        let ast = syn::parse_file(&source).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        // File-wide exclusion deliberately includes helpers outside the callback.
        // False positives stay on the original uncached fixture, never an allowlist.
        let risks = [
            "options.resources",
            "reload",
            "ResourceRoot",
            "ResourceSource",
            "std::fs",
            "fs::write",
            "fs::remove",
            "copy_tree",
            "include!",
            "macro_rules!",
        ]
        .into_iter()
        .filter(|w| source.contains(w))
        .map(|w| format!("file signal: {w}"))
        .collect::<Vec<_>>();
        let mut edits = Vec::new();
        walk(
            &ast.items,
            &BTreeMap::new(),
            &source,
            &p.to_string_lossy(),
            &mut records,
            &mut edits,
            &risks,
        );
        if apply && !edits.is_empty() {
            let mut result = source;
            edits.sort_by_key(|e| e.0);
            for (start, end, text) in edits.into_iter().rev() {
                result.replace_range(start..end, &text)
            }
            fs::write(p, result).unwrap();
        }
    }
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"schema":1,"scope":"direct syntax-resolved absolute imports and calls; conservative file exclusions","calls":records})).unwrap());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn discover(source: &str) -> (Vec<serde_json::Value>, usize) {
        let ast = syn::parse_file(source).unwrap();
        let mut records = vec![];
        let mut edits = vec![];
        walk(
            &ast.items,
            &BTreeMap::new(),
            source,
            "fixture.rs",
            &mut records,
            &mut edits,
            &[],
        );
        (records, edits.len())
    }
    #[test]
    fn resolves_absolute_aliases_and_groups() {
        let (r, n) = discover(
            "use crate::game::native_entity_refs::tests::{with_game as fixture, with_client_game}; fn test(){fixture(|g|g.tick());with_client_game(|g|g.tick());}",
        );
        assert_eq!(n, 2);
        assert_eq!(r.len(), 2);
    }
    #[test]
    fn refuses_local_shadows_globs_custom_and_indirect() {
        for source in [
            "fn test(){with_game_at(root,|g|g.tick());}",
            "use crate::game::native_entity_refs::tests::with_game; fn test(){let with_game=other;with_game(|g|g.tick());}",
            "use crate::game::native_entity_refs::tests::*; fn test(){with_game(|g|g.tick());}",
            "use crate::game::native_entity_refs::tests::with_game;fn side(){} fn test(){with_game(|g|side());}",
            "fn with_game(){} fn test(){with_game();}",
        ] {
            assert_eq!(discover(source).1, 0, "{source}");
        }
    }
}
