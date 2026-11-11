use super::identity::{CallIdentity, Refusal, Selection, select};
use super::resolve::{Resolved, resolve};
use super::*;
use crate::{crossing::Rule, internals::Internal, points::file_functions};
use quote::{ToTokens, format_ident, quote};
use sha2::{Digest, Sha256};
use syn::{FnArg, Type, spanned::Spanned};

fn digest(text: String) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..16].to_owned()
}

fn covers(span: proc_macro2::Span, expr: &Expr) -> bool {
    let (start, end) = (expr.span().start(), expr.span().end());
    let (from, to) = (span.start(), span.end());
    (from.line < start.line || (from.line == start.line && from.column <= start.column))
        && (to.line > end.line || (to.line == end.line && to.column >= end.column))
}

/// Statement tokens plus the if/match/block path. Token text ignores source formatting.
fn call_context(body: &syn::Block, expr: &Expr) -> String {
    fn block(body: &syn::Block, expr: &Expr, path: &str) -> Option<String> {
        for stmt in &body.stmts {
            if !covers(stmt.span(), expr) {
                continue;
            }
            let inner = match stmt {
                syn::Stmt::Expr(inner, _) => Some(inner),
                syn::Stmt::Local(local) => local.init.as_ref().map(|init| &*init.expr),
                _ => None,
            };
            if let Some(inner) = inner
                && let Some(found) = nested(inner, expr, path)
            {
                return Some(found);
            }
            return Some(format!("{path}|{}", stmt.to_token_stream()));
        }
        None
    }
    fn nested(expr: &Expr, target: &Expr, path: &str) -> Option<String> {
        match expr {
            Expr::If(item) => {
                block(&item.then_branch, target, &format!("{path}/t")).or_else(|| {
                    item.else_branch
                        .as_ref()
                        .and_then(|(_, branch)| match branch.as_ref() {
                            Expr::Block(item) => block(&item.block, target, &format!("{path}/e")),
                            other => nested(other, target, &format!("{path}/e")),
                        })
                })
            }
            Expr::Match(item) => item
                .arms
                .iter()
                .enumerate()
                .filter(|(_, arm)| covers(arm.body.span(), target))
                .find_map(|(index, arm)| match arm.body.as_ref() {
                    Expr::Block(item) => block(&item.block, target, &format!("{path}/a{index}")),
                    other => nested(other, target, &format!("{path}/a{index}"))
                        .or_else(|| Some(format!("{path}/a{index}|{}", other.to_token_stream()))),
                }),
            Expr::Block(item) => block(&item.block, target, &format!("{path}/b")),
            Expr::Unsafe(item) => block(&item.block, target, &format!("{path}/b")),
            Expr::Loop(item) => block(&item.body, target, &format!("{path}/b")),
            Expr::While(item) => block(&item.body, target, &format!("{path}/b")),
            Expr::ForLoop(item) => block(&item.body, target, &format!("{path}/b")),
            _ => None,
        }
    }
    block(body, expr, "").unwrap_or_default()
}

pub(crate) fn name(declaration: &Declaration, ordinal: usize) -> String {
    format!(
        "S{}",
        digest(format!(
            "{}|{}|{ordinal}",
            declaration.target, declaration.selector
        ))
    )
}

/// Rewrite one source file. Missing targets are left for the coordinator to check across files;
/// every matched target must resolve its call, and overlapping declarations are refused.
///
/// An unsubscribed call keeps the existing listed-site cost: one relaxed atomic load and
/// a branch, with no argument record or allocation. An optimized i9-13900H author run
/// measured 1.44 ns plain versus 1.93 ns listed (+0.49 ns; six alternating 50-million-call
/// samples with an opaque subscription flag). This is not a zero-overhead plain call.
pub fn rewrite(
    source: &str,
    module: &str,
    entries: &[Internal],
    declarations: &[Declaration],
) -> Result<(String, Vec<Site>), Error> {
    let parsed = syn::parse_file(source)?;
    let mut functions = Vec::new();
    file_functions(
        module,
        &parsed.items,
        &|_| true,
        &point_shape::eligible,
        &mut functions,
    );
    let rule = Rule::new(entries);
    let mut edits: BTreeMap<(usize, usize), String> = BTreeMap::new();
    let mut claimed = BTreeMap::new();
    let mut sites = Vec::new();
    let lines: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let offset = |at| crate::points::offset(source, &lines, at);
    let declarations = declarations.iter().collect::<Vec<_>>();
    for declaration in declarations {
        let enclosing = match super::identity::enclosing(&declaration.target, entries, &rule) {
            Ok(entry) => entry,
            Err(Error {
                cause: Cause::Missing(_),
                ..
            }) => continue,
            Err(error) => return Err(error.at(declaration.origin.clone())),
        };
        for found in functions
            .iter()
            .filter(|f| plain_path(&f.target()) == plain_path(&enclosing.path))
        {
            let build = || -> Result<_, Error> {
                let mut lexical = super::analysis::Scope::scan(found.sig, Some(found.body));

                let written = super::analysis::calls(found.body, &declaration.selector);
                let paths = written
                    .iter()
                    .map(|expr| {
                        super::resolve::callee(expr, enclosing, found.sig, &lexical, entries, &rule)
                            .and_then(|c| {
                                super::identity::nominal(c.callee, &rule).ok_or_else(|| {
                                    Cause::Missing("unresolved nominal callee owner").into()
                                })
                            })
                    })
                    .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
                if paths.len() > 1 {
                    return Err(Cause::Ambiguous("selector resolves to several callees").into());
                }
                let path = paths
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| declaration.selector.clone());
                let calls = super::analysis::calls(found.body, "")
                    .into_iter()
                    .filter(|expr| {
                        super::resolve::callee(expr, enclosing, found.sig, &lexical, entries, &rule)
                            .is_ok_and(|c| {
                                super::identity::nominal(c.callee, &rule).as_deref()
                                    == Some(path.as_str())
                            })
                    })
                    .collect::<Vec<_>>();
                if calls.is_empty() && declaration.ctx.is_empty() {
                    return Err(Cause::Missing("selector matched no call").into());
                }
                if calls.len() > 1 && declaration.ordinal.is_none() {
                    return Err(Cause::Ambiguous("ambiguous selector: supply an ordinal").into());
                }
                let ordinal = declaration.ordinal.unwrap_or(0);
                let contexts = calls
                    .iter()
                    .map(|expr| digest(call_context(found.body, expr)))
                    .collect::<Vec<_>>();
                // Empty contexts are only for tool discovery; committed seeds are pinned below.
                let ctx = if declaration.ctx.is_empty() {
                    contexts
                        .get(ordinal)
                        .ok_or(Cause::Missing("ordinal matched no call"))?
                } else {
                    &declaration.ctx
                };
                let identities = contexts
                    .iter()
                    .enumerate()
                    .map(|(nth, ctx)| CallIdentity {
                        call: &path,
                        nth,
                        ctx,
                    })
                    .collect::<Vec<_>>();
                let index = match select(&identities, &path, ordinal, ctx) {
                    Selection::Selected(index) => index,
                    Selection::Moved(nth) => {
                        return Err(Cause::Identity(Refusal::Moved(nth)).into());
                    }
                    Selection::Lost | Selection::Ambiguous => {
                        return Err(Cause::Identity(Refusal::Lost).into());
                    }
                };
                let expr = calls[index];

                let Resolved {
                    callee,
                    signature,
                    receiver,
                    receiver_type,
                    arguments,
                    fields,
                    types,
                    output,
                    once,
                } = resolve(expr, enclosing, found.sig, &lexical, entries, &rule)?;
                let name = name(declaration, ordinal);
                let record = format_ident!("__{name}Args");
                let listener = format_ident!("__{name}Listener");
                let variant = format_ident!("{name}");
                let receiver_alias = format_ident!("__{name}Receiver");
                let output_alias = format_ident!("__{name}Output");
                let recv = lexical.fresh("__recv");
                let args = lexical.fresh("__args");
                let wrap = lexical.fresh("__wrap");
                let mut original = expr.clone();
                let arg_fields = fields
                    .iter()
                    .map(|f| syn::parse_quote!(#args.#f))
                    .collect::<Vec<Expr>>();
                match &mut original {
                    Expr::MethodCall(call) => {
                        *call.receiver = syn::parse_quote!(#recv);
                        call.args = arg_fields.into_iter().collect();
                    }
                    Expr::Call(call) => {
                        call.args = receiver
                            .map(|_| syn::parse_quote!(#recv))
                            .into_iter()
                            .chain(arg_fields)
                            .collect();
                    }
                    _ => unreachable!(),
                }
                let binding = match (receiver, signature.receiver()) {
                    (Some(expr), Some(r)) => {
                        let referenced = matches!(expr, Expr::Path(path) if path.path.is_ident("self")) || found.sig.inputs.iter().any(|arg| matches!(arg, FnArg::Typed(arg) if matches!(&*arg.ty, Type::Reference(_)) && arg.pat.to_token_stream().to_string() == expr.to_token_stream().to_string()));
                        let place = if referenced {
                            quote!(*#expr)
                        } else {
                            quote!(#expr)
                        };
                        if r.mutability.is_some() {
                            quote!(let #recv = &mut #place;)
                        } else {
                            quote!(let #recv = &#place;)
                        }
                    }
                    _ => quote!(let #recv = ();),
                };
                let owning = module.split("::").next().expect("a module has a crate");
                let local =
                    |ty: proc_macro2::TokenStream| -> Result<proc_macro2::TokenStream, Error> {
                        ty.to_string()
                            .replace(&format!(":: {owning} ::"), "crate ::")
                            .parse()
                            .map_err(|_| Cause::Signature("invalid native type spelling").into())
                    };
                let types = types
                    .iter()
                    .map(|ty| {
                        local(ty.to_token_stream())
                            .and_then(|tokens| syn::parse2::<Type>(tokens).map_err(Into::into))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                let receiver_type = local(receiver_type)?;
                let output = local(output)?;
                let mut alias_type: Type = syn::parse2(receiver_type.clone())?;
                if let Type::Reference(reference) = &mut alias_type {
                    reference.lifetime = Some(syn::parse_quote!('r));
                }
                let borrows = types.iter().any(|ty| matches!(ty, Type::Reference(_)));
                let inner_borrow = types
                    .iter()
                    .any(|ty| ty.to_token_stream().to_string().contains("'g"));
                let lifetime = if inner_borrow {
                    Some(quote!(<'g, 'a>))
                } else {
                    borrows.then(|| quote!(<'a>))
                };
                let record_type = quote!(#record #lifetime);
                let macro_record = if inner_borrow {
                    quote!(#record::<'g, 'a>)
                } else if borrows {
                    quote!(#record::<'a>)
                } else {
                    quote!(#record)
                };
                let inferred_record = if inner_borrow {
                    quote!(#record<'_, '_>)
                } else if borrows {
                    quote!(#record<'_>)
                } else {
                    quote!(#record)
                };
                let listener_lifetimes = if borrows {
                    quote!(for<'g, 'a>)
                } else {
                    quote!(for<'g>)
                };
                let original_type = format_ident!("__{name}Original");
                let raw = if once {
                    quote! {
                        pub struct #record #lifetime { #(pub #fields: #types,)* }
                        pub type #receiver_alias<'g, 'r> = #alias_type;
                        pub type #output_alias = #output;
                        /// A call-scoped, single-use forwarding capability for this native call.
                        pub struct #original_type<'g, 'a, 'next> {
                            next: &'next mut dyn FnMut(#receiver_type, #record_type) -> #output,
                            scope: ::std::marker::PhantomData<(&'g (), &'a ())>,
                        }
                        impl<'g, 'a, 'next> #original_type<'g, 'a, 'next> {
                            fn new(next: &'next mut dyn FnMut(#receiver_type, #record_type) -> #output) -> Self {
                                Self { next, scope: ::std::marker::PhantomData }
                            }
                            /// Consume this capability and the argument record, forwarding exactly once.
                            pub fn call(self, receiver: #receiver_type, args: #record_type) -> #output {
                                (self.next)(receiver, args)
                            }
                        }
                        pub type #listener = for<'g, 'a, 'next> fn(#receiver_type, #record_type, #original_type<'g, 'a, 'next>) -> #output;
                    }
                } else {
                    quote! {
                        pub struct #record #lifetime { #(pub #fields: #types,)* }
                        pub type #receiver_alias<'g, 'r> = #alias_type;
                        pub type #output_alias = #output;
                        pub type #listener = #listener_lifetimes fn(#receiver_type, &mut #record_type, &mut dyn FnMut(#receiver_type, &mut #record_type) -> #output) -> #output;
                    }
                };
                let dispatch = if once {
                    quote!(
                        ::horizon_mod_api_macros::site_once!(#wrap, #variant, #listener, #receiver_alias, #macro_record, #output_alias, #original_type, #recv, #args, |#recv: #receiver_alias<'_, '_>, #args: #inferred_record| #original)
                    )
                } else {
                    quote!(
                        ::horizon_mod_api_macros::site!(#wrap, #variant, #listener, #receiver_alias, #macro_record, #output_alias, #recv, #args, |#recv: #receiver_alias<'_, '_>, #args: &mut #inferred_record| #original)
                    )
                };
                let mutable = (!once).then(|| quote!(mut));
                let replacement = quote! {
                    if ::mod_points::__private::TABLE.active_site(::mod_points::SiteId::#variant as usize) {
                        #binding
                        let #mutable #args = #record { #(#fields: #arguments,)* };
                        #dispatch
                    } else { #expr }
                };
                let nth = ordinal;
                let same = calls.len();
                let site = Site {
                    name,
                    target: declaration.target.clone(),
                    selector: declaration.selector.clone(),
                    ordinal,
                    hash: digest(format!(
                        "{}|{}|{}|{}|{nth}|{same}|{}",
                        enclosing.signature,
                        path,
                        callee.signature,
                        expr.to_token_stream(),
                        call_context(found.body, expr)
                    )),
                    call: path,
                    ctx: contexts[index].clone(),
                    listener: format!("{}::{listener}", found.module),
                    once,
                    record: format!("{}::{record}", found.module),
                    receiver: receiver_type.to_string(),
                    args: fields
                        .iter()
                        .zip(&types)
                        .map(|(f, t)| (f.to_string(), t.to_token_stream().to_string()))
                        .collect(),
                    output: output.to_string(),
                };
                Ok((expr.span(), raw.to_string(), replacement.to_string(), site))
            };
            let (span, raw, replacement, site) = build()
                .map_err(|e| e.at(format!("{}: {}", declaration.origin, declaration.target)))?;
            let start = offset(span.start());
            let end = offset(span.end());
            if let Some(other) = claimed.insert((start, end), declaration.origin.clone()) {
                return Err(Error::from(Cause::Conflict(other)).at(declaration.origin.clone()));
            }
            edits.insert((start, end), replacement);
            edits
                .entry((offset(found.end), offset(found.end)))
                .or_default()
                .push_str(&raw);
            sites.push(site);
        }
    }
    let mut out = source.to_owned();
    let mut boundary = source.len();
    for ((start, end), text) in edits.into_iter().rev() {
        if end > boundary {
            return Err(Cause::Record("nested selected call sites overlap").into());
        }
        out.replace_range(start..end, &text);
        boundary = start;
    }
    Ok((out, sites))
}

#[cfg(test)]
mod tests;
