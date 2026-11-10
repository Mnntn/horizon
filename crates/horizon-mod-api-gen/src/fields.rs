//! Transient real fields in native source copies. No declarations means no source I/O.
use crate::points;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::{
    Item, Type,
    spanned::Spanned,
    visit::{self, Visit},
};

#[derive(Clone, Debug)]
pub struct Declaration {
    pub id: String,
    pub target: String,
    pub mod_id: String,
    pub name: String,
    pub ty: String,
}
impl Declaration {
    fn field(&self) -> String {
        let prefix = if self.mod_id.starts_with(|c: char| c.is_ascii_digit()) {
            "_"
        } else {
            ""
        };
        format!("{prefix}{}__{}", self.mod_id, self.name)
    }
    fn local_type(&self) -> String {
        self.ty.replace(
            &format!("{}::", self.target.split("::").next().unwrap()),
            "crate::",
        )
    }
    fn error(&self, why: impl std::fmt::Display) -> String {
        format!(
            "E_INJECT_FIELD: {}:{}: {}: {why}",
            self.mod_id, self.id, self.target
        )
    }
}

/// Validate type origins before rustc checks visibility, Default and Clone bounds.
fn field_type(d: &Declaration) -> Result<Type, String> {
    let ty = syn::parse_str::<Type>(&d.ty).map_err(|e| d.error(e))?;
    struct Origins<'a> {
        d: &'a Declaration,
        invalid: bool,
    }
    impl<'ast> Visit<'ast> for Origins<'_> {
        fn visit_type_path(&mut self, t: &'ast syn::TypePath) {
            let first = t.path.segments.first().unwrap().ident.to_string();
            let primitive = matches!(
                first.as_str(),
                "bool"
                    | "char"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "f32"
                    | "f64"
                    | "String"
                    | "Vec"
                    | "Option"
                    | "Box"
                    | "Result"
            );
            let engine = self.d.target.split("::").next().unwrap();
            if t.qself.is_some()
                || !(primitive
                    || matches!(
                        first.as_str(),
                        "std" | "core" | "alloc" | "crate" | "self" | "super"
                    )
                    || first == engine
                    || first.starts_with("horizon_"))
            {
                self.invalid = true;
            }
            visit::visit_type_path(self, t);
        }
    }
    let mut origins = Origins { d, invalid: false };
    origins.visit_type(&ty);
    if origins.invalid {
        return Err(d.error(format!("field type {} is not reachable from the target engine crate; mod-defined types require attachments", d.ty)));
    }
    Ok(ty)
}

fn check_emitted_names(declarations: &[Declaration]) -> Result<(), String> {
    let mut owners = BTreeMap::new();
    for d in declarations {
        let key = (&d.target, d.field());
        if let Some(previous) = owners.insert(key, d) {
            return Err(d.error(format!(
                "emitted member {} collides between {}:{} and {}:{}",
                d.field(),
                previous.mod_id,
                previous.id,
                d.mod_id,
                d.id
            )));
        }
    }
    Ok(())
}

struct Rewriter<'a> {
    source: &'a str,
    lines: Vec<usize>,
    module: String,
    d: &'a Declaration,
    edits: BTreeMap<usize, String>,
    found: usize,
    error: Option<String>,
    aliases: BTreeMap<String, String>,
    owner: Option<String>,
}
impl Rewriter<'_> {
    fn offset(&self, at: proc_macro2::LineColumn) -> usize {
        points::offset(self.source, &self.lines, at)
    }
    fn insert(&mut self, at: proc_macro2::LineColumn, text: String) {
        let offset = self.offset(at);
        self.edits.entry(offset).or_default().push_str(&text);
    }
    fn refuse(&mut self, why: impl std::fmt::Display) {
        self.error = Some(self.d.error(why));
    }
    fn path(&self, path: &syn::Path) -> String {
        let parts = path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>();
        let first = parts.first().map(String::as_str).unwrap_or("");
        let rest = parts.iter().skip(1).cloned().collect::<Vec<_>>().join("::");
        let base = match first {
            "Self" => self.owner.clone().unwrap_or_default(),
            "crate" => self.module.split("::").next().unwrap().to_owned(),
            "self" => self.module.clone(),
            "super" => self
                .module
                .rsplit_once("::")
                .map(|(p, _)| p)
                .unwrap_or("")
                .to_owned(),
            name => self.aliases.get(name).cloned().unwrap_or_else(|| {
                if name.starts_with("horizon_") || name == self.d.target.split("::").next().unwrap()
                {
                    name.to_owned()
                } else {
                    format!("{}::{name}", self.module)
                }
            }),
        };
        if rest.is_empty() {
            base
        } else {
            format!("{base}::{rest}")
        }
    }
    fn selected(&self, path: &syn::Path) -> bool {
        self.path(path) == self.d.target
    }
    fn imports(&mut self, tree: &syn::UseTree, prefix: String) {
        match tree {
            syn::UseTree::Path(p) => self.imports(&p.tree, format!("{prefix}{}::", p.ident)),
            syn::UseTree::Group(g) => {
                for t in &g.items {
                    self.imports(t, prefix.clone());
                }
            }
            syn::UseTree::Name(n) => {
                let text = format!("{prefix}{}", n.ident);
                if let Ok(path) = syn::parse_str(&text) {
                    let resolved = self.path(&path);
                    self.aliases.insert(n.ident.to_string(), resolved);
                }
            }
            syn::UseTree::Rename(n) => {
                let text = format!("{prefix}{}", n.ident);
                if let Ok(path) = syn::parse_str(&text) {
                    let resolved = self.path(&path);
                    self.aliases.insert(n.rename.to_string(), resolved);
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
}
impl<'ast> Visit<'ast> for Rewriter<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let attrs = match item {
            Item::Const(i) => &i.attrs,
            Item::Enum(i) => &i.attrs,
            Item::ExternCrate(i) => &i.attrs,
            Item::Fn(i) => &i.attrs,
            Item::ForeignMod(i) => &i.attrs,
            Item::Impl(i) => &i.attrs,
            Item::Macro(i) => &i.attrs,
            Item::Mod(i) => &i.attrs,
            Item::Static(i) => &i.attrs,
            Item::Struct(i) => &i.attrs,
            Item::Trait(i) => &i.attrs,
            Item::TraitAlias(i) => &i.attrs,
            Item::Type(i) => &i.attrs,
            Item::Union(i) => &i.attrs,
            Item::Use(i) => &i.attrs,
            _ => return,
        };
        if !points::test_only(attrs) {
            visit::visit_item(self, item);
        }
    }
    fn visit_file(&mut self, file: &'ast syn::File) {
        for item in &file.items {
            if let Item::Use(u) = item {
                self.imports(&u.tree, String::new());
            }
        }
        visit::visit_file(self, file);
    }
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let module = self.module.clone();
        let aliases = std::mem::take(&mut self.aliases);
        self.module = format!("{module}::{}", item.ident);
        if let Some((_, items)) = &item.content {
            for item in items {
                if let Item::Use(u) = item {
                    self.imports(&u.tree, String::new());
                }
            }
            for item in items {
                self.visit_item(item);
            }
        }
        self.module = module;
        self.aliases = aliases;
    }
    fn visit_block(&mut self, block: &'ast syn::Block) {
        let aliases = self.aliases.clone();
        for statement in &block.stmts {
            if let syn::Stmt::Item(Item::Use(item)) = statement {
                self.imports(&item.tree, String::new());
            }
        }
        visit::visit_block(self, block);
        self.aliases = aliases;
    }
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        if format!("{}::{}", self.module, item.ident) != self.d.target {
            return;
        }
        self.found += 1;
        let syn::Fields::Named(fields) = &item.fields else {
            self.refuse("target requires named fields");
            return;
        };
        if fields.named.iter().any(|f| {
            f.ident
                .as_ref()
                .is_some_and(|n| n == &self.d.name || n == &self.d.field())
        }) {
            self.refuse(format!(
                "field {} collides with an existing field",
                self.d.name
            ));
            return;
        }
        let mut serde = false;
        for attr in &item.attrs {
            if ![
                "derive",
                "doc",
                "allow",
                "warn",
                "deny",
                "expect",
                "forbid",
                "serde",
                "cfg",
                "must_use",
                "non_exhaustive",
                "deprecated",
                "repr",
            ]
            .iter()
            .any(|name| attr.path().is_ident(name))
            {
                self.refuse("attribute cannot guarantee the field is never saved or synced");
                return;
            }
            if attr.path().is_ident("derive") {
                use syn::parse::Parser;
                if let Ok(derives) =
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated
                        .parse2(attr.meta.require_list().unwrap().tokens.clone())
                {
                    for derive in derives {
                        let name = derive.segments.last().unwrap().ident.to_string();
                        match name.as_str() {
                            "Serialize" | "Deserialize" => serde = true,
                            "PartialEq" | "Eq" | "PartialOrd" | "Ord" | "Hash" => {
                                self.refuse(format!(
                                    "derive {name} would include mod state in engine comparison"
                                ));
                                return;
                            }
                            "Clone" | "Default" | "Debug" | "Copy" => {}
                            _ => {
                                self.refuse(format!("derive {name} cannot guarantee the field is never saved or synced"));
                                return;
                            }
                        }
                    }
                }
            }
            if attr.path().is_ident("repr") {
                self.refuse("repr layout cannot be extended safely");
                return;
            }
        }
        let comma = if fields.named.empty_or_trailing() {
            ""
        } else {
            ","
        };
        let skip = if serde { "#[serde(skip)] " } else { "" };
        self.insert(
            fields.brace_token.span.close().start(),
            format!(
                "{comma} {skip}pub {}: {},",
                self.d.field(),
                self.d.local_type()
            ),
        );
        self.insert(item.semi_token.map(|s|s.span.end()).unwrap_or_else(|| fields.brace_token.span.close().end()),
            format!(" const _: fn() = || {{ fn require<T: ::core::default::Default>() {{}} require::<{}>(); }};", self.d.local_type()));
    }
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let owner = self.owner.clone();
        self.owner = if let Type::Path(p) = &*item.self_ty {
            Some(self.path(&p.path))
        } else {
            None
        };
        let target = self.owner.as_deref() == Some(self.d.target.as_str());
        visit::visit_item_impl(self, item);
        if target
            && item
                .trait_
                .as_ref()
                .is_some_and(|(_, p, _)| self.path(p).rsplit("::").next() == Some("Clone"))
        {
            for member in &item.items {
                if let syn::ImplItem::Fn(method) = member {
                    let field = self.d.field();
                    if method.sig.ident == "clone" {
                        self.insert(
                            method.block.brace_token.span.open().end(),
                            " let mut __horizon_field_clone = (|| {".into(),
                        );
                        self.insert(method.block.brace_token.span.close().start(),format!(" }})(); __horizon_field_clone.{field} = ::core::clone::Clone::clone(&self.{field}); __horizon_field_clone "));
                    } else if method.sig.ident == "clone_from" {
                        let Some(syn::FnArg::Typed(arg)) = method.sig.inputs.iter().nth(1) else {
                            self.refuse("unsupported Clone::clone_from");
                            continue;
                        };
                        let syn::Pat::Ident(arg) = &*arg.pat else {
                            self.refuse("unsupported Clone::clone_from argument");
                            continue;
                        };
                        self.insert(method.block.brace_token.span.open().end(), " (|| {".into());
                        self.insert(
                            method.block.brace_token.span.close().start(),
                            format!(" }})(); ::core::clone::Clone::clone_from(&mut self.{field}, &{}.{field}); ", arg.ident),
                        );
                    }
                }
            }
        }
        self.owner = owner;
    }
    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        visit::visit_expr_struct(self, expr);
        if !self.selected(&expr.path) || expr.rest.is_some() {
            return;
        }
        let comma = if expr.fields.empty_or_trailing() {
            ""
        } else {
            ","
        };
        let at = if comma.is_empty() {
            expr.brace_token.span.close().start()
        } else {
            expr.fields.last().expect("nonempty literal").span().end()
        };
        self.insert(
            at,
            format!("{comma} {}: Default::default()", self.d.field()),
        );
    }
    fn visit_expr_unsafe(&mut self, expr: &'ast syn::ExprUnsafe) {
        if self.owner.as_deref() == Some(&self.d.target) {
            self.refuse("unsafe construction cannot be rewritten safely");
            return;
        }
        visit::visit_expr_unsafe(self, expr);
    }
    fn visit_pat_struct(&mut self, pat: &'ast syn::PatStruct) {
        if self.selected(&pat.path) && pat.rest.is_none() {
            self.refuse("exhaustive struct pattern cannot be extended safely");
        }
        visit::visit_pat_struct(self, pat);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        use syn::parse::Parser;
        if [
            "assert",
            "assert_eq",
            "assert_ne",
            "debug_assert",
            "debug_assert_eq",
            "debug_assert_ne",
        ]
        .iter()
        .any(|name| mac.path.is_ident(name))
            && let Ok(args) =
                syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(mac.tokens.clone())
        {
            for expr in &args {
                self.visit_expr(expr);
            }
            return;
        }
        let name = self.d.target.rsplit("::").next().unwrap();
        fn mentions(tokens: proc_macro2::TokenStream, name: &str, this: bool) -> bool {
            tokens.into_iter().any(|token| match token {
                proc_macro2::TokenTree::Ident(ident) => ident == name || (this && ident == "Self"),
                proc_macro2::TokenTree::Group(group) => mentions(group.stream(), name, this),
                _ => false,
            })
        }
        if mentions(
            mac.tokens.clone(),
            name,
            self.owner.as_deref() == Some(&self.d.target),
        ) {
            self.refuse("macro construction cannot be rewritten safely");
        }
    }
}

/// Rewrite a file, retaining all bytes outside inserted fields and constructor members.
pub fn rewrite(
    source: &str,
    module: &str,
    declarations: &[Declaration],
) -> Result<(String, BTreeSet<String>), String> {
    check_emitted_names(declarations)?;
    let mut out = source.to_owned();
    let mut found = BTreeSet::new();
    for d in declarations {
        field_type(d)?;
        syn::parse_str::<syn::Ident>(&d.field()).map_err(|e| d.error(e))?;
        let file = syn::parse_file(&out).map_err(|e| d.error(e))?;
        let mut r = Rewriter {
            source: &out,
            lines: std::iter::once(0)
                .chain(out.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            module: module.into(),
            d,
            edits: BTreeMap::new(),
            found: 0,
            error: None,
            aliases: BTreeMap::new(),
            owner: None,
        };
        r.visit_file(&file);
        if let Some(error) = r.error {
            return Err(error);
        }
        if r.found > 1 {
            return Err(d.error("ambiguous target struct"));
        }
        if r.found == 1 {
            found.insert(d.target.clone());
        }
        let edits = r.edits;
        for (offset, text) in edits.into_iter().rev() {
            out.insert_str(offset, &text);
        }
    }
    Ok((out, found))
}

/// Validate every source edit before writing. rustc enforces field Default/Clone and visibility.
pub fn stage(root: &Path, declarations: &[Declaration]) -> Result<(), String> {
    if declarations.is_empty() {
        return Ok(());
    }
    check_emitted_names(declarations)?;
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root.join("crates")).map_err(|e| e.to_string())? {
        let dir = entry.map_err(|e| e.to_string())?.path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(manifest).map_err(|e| e.to_string())?;
        let value: toml::Value = toml::from_str(&text).map_err(|e| e.to_string())?;
        let Some(name) = value
            .get("package")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let lib = dir.join("src/lib.rs");
        for file in points::module_files(name.replace('-', "_"), lib, dir.join("src"), false)? {
            files.insert(file.path, file.module);
        }
    }
    let game = root.join("crates/horizon-client/src/game.rs");
    if game.exists() {
        for file in points::game_files(&root.join("crates/horizon-client/src"), false)? {
            files.insert(file.path, file.module);
        }
    }
    let mut changes = Vec::new();
    let mut found = BTreeSet::new();
    for (path, module) in files {
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let (rewritten, matched) = rewrite(&source, &module, declarations)?;
        found.extend(matched);
        if rewritten != source {
            changes.push((path, rewritten));
        }
    }
    for d in declarations {
        if !found.contains(&d.target) {
            return Err(d.error(format!("unknown target struct {}", d.target)));
        }
    }
    for (path, source) in changes {
        fs::write(path, source).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
