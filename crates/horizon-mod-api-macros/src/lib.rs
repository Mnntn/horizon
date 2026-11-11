//! Derive hook subscriptions from a trait implementation.
mod payload;
mod point;
mod wire;

/// Derive a typed payload's value codec from its fields.
///
/// `#[payload(crate = ::mod_vocab)]` selects the contract crate for
/// lower-layer crates. The default is `::mod_api`.
#[proc_macro_derive(PayloadCodec, attributes(payload))]
pub fn derive_payload_codec(input: TokenStream) -> TokenStream {
    payload::expand(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the shared native/wasm owned-value encoding.
///
/// `#[wire(crate = ::mod_vocab::wire)]` selects the contract module for
/// lower-layer crates. The default is `::mod_api::wire`.
#[proc_macro_derive(Wire, attributes(wire))]
pub fn derive_wire(input: TokenStream) -> TokenStream {
    wire::expand(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    ImplItem, ImplItemFn, ItemImpl, Meta, Token, parse::Parser, parse_macro_input,
    punctuated::Punctuated,
};

/// The staged call site's typed boundary. Only generated source invokes this macro.
/// A borrowing record is passed as `Args::<'a>`; the generated dispatch function
/// binds that call lifetime and preserves it through every continuation.
#[proc_macro]
pub fn site(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input with Punctuated::<syn::Expr, Token![,]>::parse_terminated);
    let args: Vec<_> = args.iter().collect();
    let [
        wrap,
        place,
        listener,
        recv_ty,
        record,
        output,
        receiver,
        args,
        original,
    ] = args.as_slice()
    else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected fresh wrap name, site types, receiver, args and original",
        )
        .to_compile_error()
        .into();
    };
    let borrow_lifetime = match record {
        syn::Expr::Path(path) if path.path.segments.iter().any(|s| !s.arguments.is_empty()) => {
            quote!(, 'a)
        }
        _ => quote!(),
    };
    quote! {{
        fn #wrap<'g #borrow_lifetime>(__at: usize, __receiver: #recv_ty<'g, '_>, __args: &mut #record,
            __original: &mut dyn FnMut(#recv_ty<'g, '_>, &mut #record) -> #output) -> #output {
            ::mod_points::__private::wrap(::mod_points::SiteId::#place, __at, |__entry| {
                match __entry {
                    Some((__pointer, __next)) => {
                        // SAFETY: registration checks this exact site's listener type.
                        let __listener: #listener = unsafe { ::std::mem::transmute(__pointer) };
                        __listener(__receiver, __args, &mut |__recv, __args| #wrap(__next, __recv, __args, __original))
                    }
                    None => __original(__receiver, __args),
                }
            })
        }
        #wrap(0, #receiver, &mut #args, &mut #original)
    }}.into()
}

/// The consuming boundary for a native source call with owned arguments.
/// Only generated source invokes it. Each listener gets a private, stack-borrowed
/// forwarding capability whose `call(self, receiver, args)` consumes both values.
#[proc_macro]
pub fn site_once(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input with Punctuated::<syn::Expr, Token![,]>::parse_terminated);
    let args: Vec<_> = args.iter().collect();
    let [
        wrap,
        place,
        listener,
        recv_ty,
        record,
        output,
        capability,
        receiver,
        args,
        original,
    ] = args.as_slice()
    else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected consuming site types, capability, receiver, args and original",
        )
        .to_compile_error()
        .into();
    };
    quote! {{
        fn #wrap<'g, 'a>(__at: usize, __receiver: #recv_ty<'g, '_>, __args: #record,
            __original: &mut dyn FnMut(#recv_ty<'g, '_>, #record) -> #output) -> #output {
            ::mod_points::__private::wrap(::mod_points::SiteId::#place, __at, |__entry| {
                match __entry {
                    Some((__pointer, __next)) => {
                        // SAFETY: registration checks this exact site's consuming listener type.
                        let __listener: #listener = unsafe { ::std::mem::transmute(__pointer) };
                        let mut __forward = |__recv: #recv_ty<'g, '_>, __args: #record| #wrap(__next, __recv, __args, __original);
                        __listener(__receiver, __args, #capability::new(&mut __forward))
                    }
                    None => __original(__receiver, __args),
                }
            })
        }
        #wrap(0, #receiver, #args, &mut #original)
    }}.into()
}

/// Keep only attributes that can gate a method, including nested `cfg_attr`s.
fn cfg_meta(meta: &Meta) -> Option<Meta> {
    if meta.path().is_ident("cfg") {
        return Some(meta.clone());
    }
    let Meta::List(list) = meta else { return None };
    if !list.path.is_ident("cfg_attr") {
        return None;
    }
    let args = Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()?;
    let mut args = args.iter();
    let condition = args.next()?;
    let gated: Vec<_> = args.filter_map(cfg_meta).collect();
    if gated.is_empty() {
        None
    } else {
        Some(syn::parse_quote!(cfg_attr(#condition, #(#gated),*)))
    }
}

fn cfg_attributes(method: &ImplItemFn) -> Vec<syn::Attribute> {
    method
        .attrs
        .iter()
        .filter_map(|attr| cfg_meta(&attr.meta))
        .map(|meta| syn::parse_quote!(#[#meta]))
        .collect()
}

/// A method's `#[listener(priority = …, receive_cancelled)]`, removed from the method.
struct ListenerOptions {
    priority: syn::Expr,
    /// The attribute's span, for the error when the hook has no cancel to receive.
    receive_cancelled: Option<proc_macro2::Span>,
}

/// Takes `method`'s `#[listener]`, if it has one.
fn take_listener(method: &mut ImplItemFn) -> syn::Result<Option<ListenerOptions>> {
    let Some(index) = method
        .attrs
        .iter()
        .position(|attr| attr.path().is_ident("listener"))
    else {
        return Ok(None);
    };
    let attr = method.attrs.remove(index);
    if let Some(again) = method
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("listener"))
    {
        return Err(syn::Error::new_spanned(
            again,
            "a hook method takes one #[listener]",
        ));
    }
    let mut options = ListenerOptions {
        priority: syn::parse_quote!(0),
        receive_cancelled: None,
    };
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("priority") {
            options.priority = meta.value()?.parse()?;
            Ok(())
        } else if meta.path.is_ident("receive_cancelled") {
            options.receive_cancelled = Some(meta.path.get_ident().unwrap().span());
            Ok(())
        } else {
            Err(meta.error("expected `priority = <i32>` or `receive_cancelled`"))
        }
    })?;
    Ok(Some(options))
}

fn uninspectable(item: &ImplItem) -> bool {
    matches!(item, ImplItem::Macro(_) | ImplItem::Verbatim(_))
}

fn expand(mut implementation: ItemImpl, export: bool) -> TokenStream {
    let Some((_, trait_path, _)) = &implementation.trait_ else {
        return syn::Error::new_spanned(
            &implementation,
            "expected an impl of Hooks or a Part trait",
        )
        .into_compile_error()
        .into();
    };
    let name = &trait_path.segments.last().unwrap().ident;
    let part = match name.to_string().as_str() {
        "Hooks" => None,
        "BlockPart" => Some("block_subscribed"),
        "BlockEntityPart" => Some("block_entity_subscribed"),
        "ItemPart" => Some("item_subscribed"),
        "MenuPart" => Some("menu_subscribed"),
        "FluidPart" => Some("fluid_subscribed"),
        // Entity parts carry no scoped hook yet (#1077): `register_goals` is
        // not one, so its subscriptions stay empty whatever it overrides.
        "EntityPart" => Some("entity_subscribed_empty"),
        _ => {
            return syn::Error::new_spanned(name, "expected a hook or part trait")
                .into_compile_error()
                .into();
        }
    };
    if part.is_some()
        && let Some(item) = implementation.items.iter().find(|item| uninspectable(item))
    {
        let ty = &implementation.self_ty;
        return syn::Error::new_spanned(
            item,
            format!("uninspectable items in impl {trait_path} for {ty} are not supported by horizon_part; write hook methods explicitly", trait_path = quote!(#trait_path), ty = quote!(#ty)),
        )
        .into_compile_error()
        .into();
    }
    let mut methods = Vec::new();
    let mut cfgs = Vec::new();
    let mut listeners = Vec::new();
    for item in &mut implementation.items {
        if let ImplItem::Fn(method) = item {
            match take_listener(method) {
                Ok(Some(_)) if part.is_some() => {
                    return syn::Error::new_spanned(
                        &method.sig.ident,
                        "#[listener] orders a trait mod's hooks; a part's are called for its own thing",
                    )
                    .into_compile_error()
                    .into();
                }
                Ok(Some(options)) => {
                    listeners.push((method.sig.ident.clone(), cfg_attributes(method), options))
                }
                Ok(None) => {}
                Err(error) => return error.into_compile_error().into(),
            }
            methods.push(method.sig.ident.clone());
            cfgs.push(cfg_attributes(method));
        }
    }
    let id = format_ident!("HookId");
    let ids = format_ident!("hook_ids");
    let subscribed_name = format_ident!("{}", part.unwrap_or("subscribed"));
    let subscribed = if subscribed_name == "entity_subscribed_empty" {
        // Entity parts (arismoko/horizon#1077): no scoped hook exists for them
        // yet, so `register_goals` cannot be mistaken for one.
        syn::parse_quote! {
            fn entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
                &[]
            }
        }
    } else {
        syn::parse_quote! {
            fn #subscribed_name(&self) -> &'static [Option<mod_api::#id>] {
                &[#(#(#cfgs)* mod_api::#ids::#methods),*]
            }
        }
    };
    implementation.items.push(subscribed);
    if !listeners.is_empty() {
        let entries = listeners.iter().map(|(method, cfgs, options)| {
            let priority = &options.priority;
            let receive_cancelled = options.receive_cancelled.is_some();
            quote! { #(#cfgs)* mod_api::Listener { hook: mod_api::#ids::#method, priority: #priority, receive_cancelled: #receive_cancelled } }
        });
        // Only a hook a mod may cancel has a constant here, so the option fails to compile on
        // any other.
        let checks = listeners.iter().filter_map(|(method, cfgs, options)| {
            let at_option = syn::Ident::new(&method.to_string(), options.receive_cancelled?);
            Some(quote! { #(#cfgs)* const _: () = mod_api::cancel_hooks::#at_option; })
        });
        implementation.items.push(syn::parse_quote! {
            fn listeners(&self) -> &'static [mod_api::Listener] {
                #(#checks)*
                &[#(#entries),*]
            }
        });
    }
    let export = if export && part.is_none() {
        let ty = &implementation.self_ty;
        let registrations: proc_macro2::TokenStream =
            match std::env::var("HORIZON_GUEST_REGISTRATIONS") {
                Ok(source) => match source.parse::<proc_macro2::TokenStream>() {
                    Ok(tokens) => tokens,
                    Err(error) => {
                        return syn::Error::new_spanned(
                            &implementation,
                            format!("guest registrations: {error}"),
                        )
                        .to_compile_error()
                        .into();
                    }
                },
                Err(_) => quote!(),
            };
        quote! {
            pub fn hooks() -> Box<dyn mod_api::Hooks> { Box::new(<#ty as Default>::default()) }
            /// The mod's hooks for the guest runtime in `mod_api::wire::guest`, when built to wasm.
            #[cfg(target_arch = "wasm32")]
            #[unsafe(no_mangle)]
            fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> {
                #registrations
                hooks()
            }
        }
    } else {
        quote!()
    };
    quote!(#implementation #export).into()
}

/// Generate the panicking defaults of a host area from each method's access declaration.
/// The same declarations drive game forwarders in the build-time API generator.
#[proc_macro_attribute]
pub fn host_area(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    let mut area = parse_macro_input!(input as syn::ItemTrait);
    let name = area.ident.to_string();
    let union = [
        "ClientBackendMut",
        "BackendMut",
        "ClientBackend",
        "ServerHost",
        "ClientHost",
        "Backend",
        "Host",
    ]
    .into_iter()
    .find(|suffix| name.ends_with(suffix))
    .unwrap_or("Host");
    let union = if union == "ClientBackend" {
        "Backend"
    } else {
        union
    };
    for member in &mut area.items {
        let syn::TraitItem::Fn(method) = member else {
            continue;
        };
        let Some(index) = method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("access"))
        else {
            return syn::Error::new_spanned(&method.sig, "host method needs #[access(Level)]")
                .to_compile_error()
                .into();
        };
        let attr = method.attrs.remove(index);
        let level = match attr.parse_args::<syn::Ident>() {
            Ok(level) => level,
            Err(error) => return error.to_compile_error().into(),
        };
        method.attrs.retain(|attr| {
            !attr.path().is_ident("static_call") && !attr.path().is_ident("service")
        });
        let operation = method.sig.ident.to_string();
        let label = format!("{union}::{operation}");
        method.default = Some(syn::parse_quote!({
            crate::world::phase::unprovided(#label, #operation, crate::world::phase::LoadAccess::#level)
        }));
        method.semi_token = None;
    }
    quote!(#area).into()
}

/// Mark a trait mod and derive its hook subscriptions.
///
/// A hook method may carry `#[listener(priority = <i32>, receive_cancelled)]`, both optional:
/// mods run by descending priority (0 when omitted), then in dependency order, and a mod that
/// receives cancelled events is still called after an earlier mod cancels, with
/// `event.cancelled` to read and clear. `receive_cancelled` compiles only on a hook a mod may
/// cancel.
#[proc_macro_attribute]
pub fn horizon_mod(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), true)
}

/// Mark a part implementation and derive the scoped hooks it overrides.
///
/// Hook methods must be written explicitly, not generated inside the impl:
///
/// ```compile_fail
/// use horizon_mod_api_macros::horizon_part;
/// trait BlockPart { fn tick(&self) {} }
/// struct Block;
/// macro_rules! hook { () => { fn tick(&self) {} }; }
/// #[horizon_part]
/// impl BlockPart for Block { hook!(); }
/// ```
#[proc_macro_attribute]
pub fn horizon_part(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `#[no_point]` (as `mod_api::no_point`) on a function of a mod crate: `cargo horizon-mod build`
/// gives it no injection point, so no addon can listen at it (#2339). It changes nothing else: the
/// function compiles as written. An addon declaring a listener there is refused at load, the
/// function named as opted out.
#[proc_macro_attribute]
pub fn no_point(attribute: TokenStream, input: TokenStream) -> TokenStream {
    if !attribute.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[no_point] takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    let function = parse_macro_input!(input as ImplItemFn);
    quote::quote!(#function).into()
}

/// `#[point(side = "server" | "client" | "both", name = "…", table = "…", raw, free, mirror(…))]` on a
/// function: an injection point (#589), a slot the launcher fills at load for the mods that listen
/// there. The staging pass that builds the prebuilt game (and the launcher's source builds) puts it
/// on every function [`point_shape::eligible`] admits; the source tree carries none. Its head checks
/// the slot (one atomic load and a branch while nothing listens). An API listener, native or wasm,
/// gets the side's host (on a `Game` method) and a record holding a copy of every parameter as an
/// API value (an engine type `mirror(…)` names as its `mod_api` mirror): the function runs with the
/// record's by-value and `&T` values, and a `&mut T` parameter's place gets the record's value back.
/// A raw listener (`raw`, native only) gets the receiver and a `&mut` to every parameter itself. Its
/// return does the same with the value returned; a value that borrows reaches only raw listeners,
/// as itself. `free` marks a free function, whose body moves to a sibling function. `table` names
/// the slot table the point is in: the game's `mod_points` by default, or a mod crate's own
/// generated table for a mod point (#2339). A `replace`
/// listener (`mod_points::replace!`) owns the head's outcome: it runs after the other API
/// listeners, and on a point with both forms the head keeps its value and cancel through the raw
/// listeners after it.
#[proc_macro_attribute]
pub fn point(attribute: TokenStream, input: TokenStream) -> TokenStream {
    point::expand(attribute.into(), parse_macro_input!(input as ImplItemFn)).into()
}

/// Derive subscriptions for internal test fixtures that construct their own mods.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn __subscriptions(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `if_installed!(host, other_mod, { other_mod::call(); })`: runs the block only where the optional
/// dependency `other_mod` is installed. A native mod is built with the Cargo feature `other_mod`
/// only when it is; a wasm mod is built with it and asks the game at the call, since an absent
/// library's functions trap. One guard for both builds.
#[proc_macro]
pub fn if_installed(input: TokenStream) -> TokenStream {
    let IfInstalled { host, id, block } = parse_macro_input!(input as IfInstalled);
    let name = id.to_string();
    quote! {
        {
            // A `let` takes the attribute where an expression statement can't.
            #[cfg(feature = #name)]
            let () = if ::mod_api::prelude::LevelsHost::is_loaded(&*#host, #name) #block;
            #[cfg(not(feature = #name))]
            let _ = &#host;
        }
    }
    .into()
}

struct IfInstalled {
    host: syn::Expr,
    id: syn::Ident,
    block: syn::Block,
}

impl syn::parse::Parse for IfInstalled {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let host = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let id = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        Ok(Self {
            host,
            id,
            block: input.parse()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_items_are_uninspectable() {
        let item = ImplItem::Verbatim(quote!(future syntax));
        assert!(uninspectable(&item));
    }
}
