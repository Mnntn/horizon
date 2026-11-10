//! `expression` sites (mixin `@ModifyConstant`, `@ModifyExpressionValue` on a non-call): a literal,
//! constant path or field read, selected by its tokens among equal expressions of one function,
//! and lowered as a value wrap whose listener takes the value and returns the one used.
use super::{Cause, Error, Expression};
use quote::{ToTokens, format_ident, quote};
use syn::{Expr, Item, spanned::Spanned, visit::Visit};

/// The value types an expression site carries: copied in and out of its record, so a listener
/// can't move out of the function, and spelled exactly as the manifest's `type` names them.
const TYPES: &[&str] = &[
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "f32",
    "f64",
    "bool",
    "char",
    "&'static str",
];

/// The selector as tokens, so spacing never decides a match; refused unless it is a literal, a
/// constant's path or a field read.
pub fn spelling(text: &str) -> Result<String, Error> {
    let expr = syn::parse_str::<Expr>(text)
        .map_err(|_| Cause::Format("expr must be one Rust expression"))?;
    if !selectable(&expr) {
        return Err(Cause::Format(
            "expr must be a literal, a constant's path or a field read such as self.speed",
        )
        .into());
    }
    Ok(tokens(&expr))
}

/// The expression's tokens with a space only between two words (`self.speed`, `-1`, `"a b"`):
/// spacing between tokens never decides a match, and a literal keeps every character.
fn tokens(expr: &Expr) -> String {
    fn push(stream: proc_macro2::TokenStream, out: &mut String, word: &mut bool) {
        use proc_macro2::{Delimiter, TokenTree};
        for tree in stream {
            match tree {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::None => ("", ""),
                    };
                    out.push_str(open);
                    *word = false;
                    push(group.stream(), out, word);
                    out.push_str(close);
                    *word = false;
                }
                TokenTree::Punct(punct) => {
                    out.push(punct.as_char());
                    *word = false;
                }
                TokenTree::Ident(_) | TokenTree::Literal(_) => {
                    if *word {
                        out.push(' ');
                    }
                    out.push_str(&tree.to_string());
                    *word = true;
                }
            }
        }
    }
    let mut out = String::new();
    push(expr.to_token_stream(), &mut out, &mut false);
    out
}

fn selectable(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) | Expr::Path(_) | Expr::Field(_) => true,
        Expr::Unary(unary) => {
            matches!(unary.op, syn::UnOp::Neg(_)) && matches!(&*unary.expr, Expr::Lit(_))
        }
        _ => false,
    }
}

/// Matching reads in source order. Left out, besides constant contexts and nested items as for
/// calls: an assignment's place, a borrow's operand and a method's receiver, which a value can't
/// stand in for.
pub(super) fn candidates<'a>(body: &'a syn::Block, selector: &str) -> Vec<&'a Expr> {
    struct Reads<'a, 's> {
        selector: &'s str,
        found: Vec<&'a Expr>,
    }
    impl<'a> Visit<'a> for Reads<'a, '_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            match expr {
                Expr::Assign(assign) => return self.visit_expr(&assign.right),
                Expr::Binary(binary) if is_compound(&binary.op) => {
                    return self.visit_expr(&binary.right);
                }
                Expr::Reference(_) => return,
                Expr::MethodCall(call) => {
                    for arg in &call.args {
                        self.visit_expr(arg);
                    }
                    return;
                }
                _ => {}
            }
            if selectable(expr) && tokens(expr) == self.selector {
                self.found.push(expr);
                return;
            }
            syn::visit::visit_expr(self, expr);
        }
        fn visit_item(&mut self, _: &'a Item) {}
        fn visit_type(&mut self, _: &'a syn::Type) {}
        fn visit_generic_argument(&mut self, _: &'a syn::GenericArgument) {}
        fn visit_expr_const(&mut self, _: &'a syn::ExprConst) {}
        fn visit_expr_repeat(&mut self, repeat: &'a syn::ExprRepeat) {
            self.visit_expr(&repeat.expr);
        }
    }
    let mut reads = Reads {
        selector,
        found: Vec::new(),
    };
    reads.visit_block(body);
    reads.found.sort_by_key(|expr| expr.span().start());
    reads.found
}

fn is_compound(op: &syn::BinOp) -> bool {
    use syn::BinOp::*;
    matches!(
        op,
        AddAssign(_)
            | SubAssign(_)
            | MulAssign(_)
            | DivAssign(_)
            | RemAssign(_)
            | BitXorAssign(_)
            | BitAndAssign(_)
            | BitOrAssign(_)
            | ShlAssign(_)
            | ShrAssign(_)
    )
}

/// The declared type, refused unless it is one the site carries; a literal whose own type is
/// written (`64.0f32`, `true`, `'x'`, `"name"`) must agree with it. An unsuffixed number takes its
/// type from where it is used, which only the compiler knows: the lowered site binds it as the
/// declared type, and a wrong one fails the build at the site, attributed to the injection.
pub(super) fn checked_type(expr: &Expr, expression: &Expression) -> Result<syn::Type, Error> {
    let ty = syn::parse_str::<syn::Type>(&expression.ty)
        .map_err(|_| Cause::Format("type must be a Rust type"))?;
    let spelled = ty.to_token_stream().to_string().replace(' ', "");
    if !TYPES
        .iter()
        .any(|allowed| allowed.replace(' ', "") == spelled)
    {
        return Err(Cause::Source(format!(
            "expression type `{spelled}`: an expression site carries a copied scalar or \
             &'static str, one of {}",
            TYPES.join(", ")
        ))
        .into());
    }
    let literal = match expr {
        Expr::Lit(lit) => Some(&lit.lit),
        Expr::Unary(unary) => match &*unary.expr {
            Expr::Lit(lit) => Some(&lit.lit),
            _ => None,
        },
        _ => None,
    };
    let written = literal.and_then(|lit| match lit {
        syn::Lit::Int(n) if !n.suffix().is_empty() => Some(n.suffix().to_owned()),
        syn::Lit::Float(n) if !n.suffix().is_empty() => Some(n.suffix().to_owned()),
        syn::Lit::Bool(_) => Some("bool".into()),
        syn::Lit::Char(_) => Some("char".into()),
        syn::Lit::Str(_) => Some("&'staticstr".into()),
        _ => None,
    });
    if let Some(written) = written.filter(|written| *written != spelled) {
        return Err(Cause::Source(format!(
            "expression `{}` is `{written}`, but its declared type is `{spelled}`",
            expr.to_token_stream()
        ))
        .into());
    }
    Ok(ty)
}

/// The marker binding the lowered site declares: diagnostics find a failing site by it.
pub fn marker(name: &str) -> String {
    format!("__horizon_expression_{name}")
}

/// The items declared beside the function and the replacement for `expr`, site `name`.
pub(super) fn lower(
    name: &str,
    expr: &Expr,
    ty: &syn::Type,
    lexical: &mut super::analysis::Scope,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let variant = format_ident!("{name}");
    let record = format_ident!("__{name}Args");
    let listener = format_ident!("__{name}Listener");
    let receiver = format_ident!("__{name}Receiver");
    let output = format_ident!("__{name}Output");
    let value = format_ident!("{}", marker(name));
    let args = lexical.fresh("__expression_args");
    let wrap = lexical.fresh("__expression_wrap");
    let raw = quote! {
        pub struct #record { pub value: #ty }
        pub type #receiver<'g, 'r> = ();
        pub type #output = #ty;
        pub type #listener = fn((), &mut #record, &mut dyn FnMut((), &mut #record) -> #ty) -> #ty;
    };
    let replacement = quote! {{
        let #value: #ty = #expr;
        if ::mod_points::__private::SITES[::mod_points::SiteId::#variant as usize].load(::std::sync::atomic::Ordering::Relaxed) {
            let mut #args = #record { value: #value };
            ::horizon_mod_api_macros::site!(#wrap, #variant, #listener, #receiver, #record, #output, (), #args, |_: (), __args: &mut #record| __args.value)
        } else { #value }
    }};
    (raw, replacement)
}
