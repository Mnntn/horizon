//! `#[point]`: an injection point's slot check, record copy-in and copy-out, and raw listeners.
use point_shape::{Arg, Pass, RawOutcome, Receiver, Returns, Side};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ImplItemFn;

/// The parameter local the outer function binds parameter `input` (receiver included) to.
fn local(input: usize) -> syn::Ident {
    format_ident!("__p{input}")
}

/// The local holding a listener's copy of a `&T` parameter, and the reference the body gets.
fn shared(input: usize) -> (syn::Ident, syn::Ident) {
    (format_ident!("__s{input}"), format_ident!("__r{input}"))
}

/// The API record's field value of `arg`, bound at `input`, before the body (`fresh` after it,
/// where a `&T` is what the function read). Mirrors take the converted value `__m<input>`.
fn copy_in(arg: &Arg, input: usize, after: bool) -> TokenStream {
    let p = local(input);
    let (_, r) = shared(input);
    let m = format_ident!("__m{input}");
    let field = &arg.field;
    if arg.mirror {
        return quote!(#m);
    }
    match arg.pass {
        Pass::Value if arg.wide.is_some() => quote!(#p as #field),
        Pass::Value if after => quote!(#m),
        Pass::Value => quote!(#p),
        Pass::Shared if arg.text && after => quote!(::std::borrow::Cow::Borrowed(#r)),
        Pass::Shared if arg.text => quote!(::std::borrow::Cow::Borrowed(#p)),
        Pass::Shared => {
            let source = if after { quote!(#r) } else { quote!(#p) };
            if arg.wide.is_some() {
                quote!(*#source as #field)
            } else if matches!(&arg.ty, syn::Type::Reference(r) if matches!(*r.elem, syn::Type::Slice(_)))
            {
                quote!(#source.to_vec())
            } else {
                quote!(::std::clone::Clone::clone(#source))
            }
        }
        Pass::Mut if arg.wide.is_some() => quote!(*#p as #field),
        Pass::Mut => quote!(::std::clone::Clone::clone(&*#p)),
    }
}

/// The mirror conversion of `arg` (only a mirror has one), from what the record copies.
fn mirror_in(arg: &Arg, input: usize, after: bool) -> Option<(syn::Ident, TokenStream)> {
    arg.mirror.then(|| {
        let p = local(input);
        let (_, r) = shared(input);
        let source = match (arg.pass, after) {
            (Pass::Value, _) => quote!(&#p),
            (Pass::Shared, true) => quote!(#r),
            (Pass::Shared, false) => quote!(#p),
            (Pass::Mut, _) => quote!(&*#p),
        };
        (
            format_ident!("__m{input}"),
            quote!(crate::game::points::mirror_in(#source)),
        )
    })
}

/// Writes a listener's change of `arg` back where the function reads it. `point` names the
/// point in the log line of a value the engine refuses.
fn copy_out(arg: &Arg, input: usize, point: &str) -> TokenStream {
    let p = local(input);
    let (s, _) = shared(input);
    let name = &arg.name;
    let value = quote!(__args.#name);
    let ty = match (&arg.wide, &arg.ty) {
        (Some(wide), _) => quote!(#wide),
        _ => quote!(_),
    };
    let target = match arg.pass {
        Pass::Value => quote!(#p),
        Pass::Mut => quote!(*#p),
        Pass::Shared => quote!(#s),
    };
    let wrap = |value: TokenStream| match arg.pass {
        Pass::Shared => quote!(::std::option::Option::Some(#value)),
        _ => value,
    };
    if arg.mirror {
        let converted = wrap(quote!(__value));
        quote! {
            if let ::std::option::Option::Some(__value) = crate::game::points::mirror_out(&#value, #point) {
                #target = #converted;
            }
        }
    } else if arg.text {
        quote! {
            if let ::std::borrow::Cow::Owned(__value) = #value {
                #target = ::std::option::Option::Some(__value);
            }
        }
    } else if arg.wide.is_some() {
        let converted = wrap(quote!(#value as #ty));
        quote!(#target = #converted;)
    } else {
        let converted = wrap(value);
        quote!(#target = #converted;)
    }
}

/// `#[point]` on a function: see the attribute's documentation in the crate root.
pub fn expand(attribute: TokenStream, mut function: ImplItemFn) -> TokenStream {
    if let Some(inline) = function.attrs.iter().find(|attr| {
        attr.path().is_ident("inline")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|arg| arg == "always")
    }) {
        return syn::Error::new_spanned(
            inline,
            "a point keeps its function out of line: drop #[inline(always)]",
        )
        .to_compile_error();
    }
    let shape = match point_shape::shape(attribute, &function.sig) {
        Ok(shape) => shape,
        Err(e) => return e.to_compile_error(),
    };
    let name = shape.name.clone();
    // The body moves to an inlined inner function, so the return listeners run however it
    // returns; the outer function takes each parameter by a plain, mutable name.
    let inner = syn::Ident::new(
        &format!(
            "__point_{}",
            function.sig.ident.to_string().trim_start_matches("r#")
        ),
        function.sig.ident.span(),
    );
    let mut outer = function.clone();
    // Each typed parameter's input index, in order: the API record's arguments line up with them.
    let mut inputs = Vec::new();
    for (i, input) in outer.sig.inputs.iter_mut().enumerate() {
        if let syn::FnArg::Typed(typed) = input {
            let plain = local(i);
            *typed.pat = syn::parse_quote!(mut #plain);
            inputs.push(i);
        }
    }
    let api = shape.api();
    let args: Vec<(&Arg, usize)> = if api {
        shape.args.iter().zip(inputs.iter().copied()).collect()
    } else {
        Vec::new()
    };
    // What the body gets: a `&T` the listeners may have replaced, a `&mut` reborrowed so the
    // return listeners can still copy it.
    let call: Vec<TokenStream> = inputs
        .iter()
        .map(|&input| {
            let p = local(input);
            match args
                .iter()
                .find(|(_, at)| *at == input)
                .map(|(arg, _)| arg.pass)
            {
                Some(Pass::Shared) => {
                    let (_, r) = shared(input);
                    quote!(#r)
                }
                Some(Pass::Mut) => quote!(&mut *#p),
                _ => quote!(#p),
            }
        })
        .collect();
    let generics: Vec<_> = function
        .sig
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(&ty.ident),
            syn::GenericParam::Const(c) => Some(&c.ident),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect();
    let turbofish = if generics.is_empty() {
        quote!()
    } else {
        quote!(::<#(#generics),*>)
    };
    let (receiver, callee) = match (shape.receiver, shape.free) {
        (Receiver::None, true) => (quote!(), quote!(#inner)),
        (Receiver::None, false) => (quote!(), quote!(Self::#inner)),
        (Receiver::Shared, _) => (quote!(self,), quote!(self.#inner)),
        (Receiver::Mut, _) => (quote!(&mut *self,), quote!(self.#inner)),
    };
    let erased_receiver = match shape.receiver {
        Receiver::None => quote!(::std::ptr::null_mut()),
        Receiver::Shared => quote!(self as *const Self as *mut ()),
        Receiver::Mut => quote!(&mut *self as *mut Self as *mut ()),
    };
    // What the API listeners get before their record: the shape's host, if it has one.
    let host = match (shape.receiver, shape.side) {
        (_, None) => quote!(),
        (Receiver::Mut, Some(Side::Both)) => {
            quote!(crate::game::points::host_side(&mut *self),)
        }
        (Receiver::Mut, Some(_)) => quote!(&mut *self,),
        (_, Some(_)) => quote!(self,),
    };
    let record = shape.record();
    let outcome = shape.raw_outcome(&function.sig);
    let api_value = match &shape.returns {
        Returns::Value(value) if api => Some(value),
        _ => None,
    };
    let cancels =
        matches!(shape.returns, Returns::Unit | Returns::UnitResult) && (api || shape.raw);
    let results = (shape.raw && matches!(outcome, RawOutcome::Result(_))) || api_value.is_some();
    let listened = |variant: &syn::Ident| quote!(mod_points::__private::POINTS[mod_points::PointId::#variant as usize].load(::std::sync::atomic::Ordering::Acquire));

    // The head: copy in, the API listeners, copy out; then the raw listeners on the same places.
    let shared_locals =
        args.iter()
            .filter(|(arg, _)| arg.pass == Pass::Shared)
            .map(|(_, input)| {
                let (s, _) = shared(*input);
                quote!(let mut #s = ::std::option::Option::None;)
            });
    let shared_refs = args
        .iter()
        .filter(|(arg, _)| arg.pass == Pass::Shared)
        .map(|(arg, input)| {
            let (s, r) = shared(*input);
            let p = local(*input);
            let syn::Type::Reference(reference) = &arg.ty else {
                unreachable!("a shared argument is a reference")
            };
            let pointee = &reference.elem;
            quote!(let #r: &#pointee = match &#s { ::std::option::Option::Some(__value) => __value, ::std::option::Option::None => #p };)
        });
    let record_call = |after: bool, mirrored: &[(syn::Ident, TokenStream)]| {
        let fields = args.iter().map(|(arg, input)| copy_in(arg, *input, after));
        let made = quote!(mod_points::#record::__new(#(#fields),*));
        let (names, values): (Vec<_>, Vec<_>) = mirrored.iter().cloned().unzip();
        (made, names, values)
    };
    let head_api = api.then(|| {
        let mirrored: Vec<_> = args
            .iter()
            .filter_map(|(arg, input)| mirror_in(arg, *input, false))
            .collect();
        let (made, names, values) = record_call(false, &mirrored);
        let outs = args.iter().map(|(arg, input)| copy_out(arg, *input, &name));
        let outcome = match (&shape.returns, api_value) {
            (Returns::Unit | Returns::UnitResult, _) => quote!(__cancel = __args.cancel;),
            (_, Some(value)) => {
                let out = result_out(value, &name);
                quote! {
                    if let ::std::option::Option::Some(__value) = __args.result {
                        if let ::std::option::Option::Some(__value) = #out { __result_slot = ::std::option::Option::Some(__value); }
                    }
                }
            }
            _ => quote!(),
        };
        let method = syn::Ident::new(&name, proc_macro2::Span::call_site());
        let body = quote! {
            #[allow(unused_mut)]
            let mut __args = #made;
            mod_points::dispatch::#method(#host &mut __args);
            #(#outs)*
            #outcome
        };
        guarded(&names, &values, body, &name)
    });
    let raw_places = |at_return: bool| {
        let fields = point_shape::raw_fields(&function.sig)
            .into_iter()
            .map(|field| {
                let (name, place) = (field.name, local(field.input));
                quote!(#name: &mut #place)
            });
        let outcome = match (&outcome, at_return) {
            (RawOutcome::Cancel, _) => quote!(cancel: &mut __cancel,),
            (RawOutcome::Result(_), _) => quote!(result: &mut __result_slot,),
            (RawOutcome::None, _) => quote!(),
        };
        let ty = shape.raw_record();
        quote!(#ty { #(#fields,)* #outcome })
    };
    let head_raw = shape.raw.then(|| {
        let listener = shape.raw_listener();
        let variant = shape.variant();
        let raw_record = raw_places(false);
        let fire = quote! {
            mod_points::__private::fire_raw(
                mod_points::PointId::#variant,
                #name,
                &mut |__kind| match __kind {
                    mod_points::__private::RawKind::Typed(__function) => {
                        // SAFETY: registration guarantees this point's listener type.
                        unsafe {
                            ::std::mem::transmute::<*const (), #listener>(__function)(
                                #receiver &mut #raw_record,
                            )
                        }
                    }
                    mod_points::__private::RawKind::Erased(__function) => {
                        // SAFETY: this slot supplies its exact receiver and raw record types.
                        unsafe {
                            __function(
                                mod_points::PointId::#variant as usize,
                                #erased_receiver,
                                &mut #raw_record as *mut _ as *mut (),
                            )
                        }
                    }
                },
            );
        };
        // A `replace` listener (the last API one) owns the outcome: the internals listeners
        // after it run, but its cancel and value stand whatever they leave.
        if !api || !(cancels || results) {
            return fire;
        }
        let keep = results.then(|| {
            quote!(let __owned = if __replaced { __result_slot.take() } else { ::std::option::Option::None };)
        });
        let force = cancels.then(|| quote!(if __replaced { __cancel = true; }));
        let restore = results.then(|| {
            quote!(if let ::std::option::Option::Some(__value) = __owned { __result_slot = ::std::option::Option::Some(__value); })
        });
        quote! {
            let __replaced = mod_points::__private::replaced(mod_points::PointId::#variant);
            #keep
            #fire
            #force
            #restore
        }
    });
    let stop = {
        let cancel = match (&shape.returns, cancels) {
            (Returns::Unit, true) => quote!(if __cancel {
                return;
            }),
            (Returns::UnitResult, true) => quote!(if __cancel {
                return Ok(());
            }),
            _ => quote!(),
        };
        let result = results.then(|| {
            quote!(
                if let ::std::option::Option::Some(__value) = __result_slot.take() {
                    return __value;
                }
            )
        });
        quote!(#cancel #result)
    };
    let head_variant = shape.variant();
    let head_listened = listened(&head_variant);

    // The return: the API listeners get the value returned and copies of the arguments (written
    // back only for `&mut` ones); then the raw listeners get the value returned itself.
    let has_api_return = api && shape.has_return();
    let back = has_api_return.then(|| {
        let variant = shape.return_variant();
        let return_listened = listened(&variant);
        // A by-value argument is spent by the body: it is copied (or converted) before.
        let before: Vec<_> = args
            .iter()
            .filter(|(arg, _)| arg.pass == Pass::Value && arg.wide.is_none())
            .map(|(arg, input)| {
                let m = format_ident!("__m{input}");
                let p = local(*input);
                let value = if arg.mirror {
                    quote!(crate::game::points::mirror_in(&#p))
                } else {
                    quote!(::std::clone::Clone::clone(&#p))
                };
                (m, value)
            })
            .collect();
        let (names, values): (Vec<_>, Vec<_>) = before.into_iter().unzip();
        let before = quote! {
            let __back = if #return_listened {
                ::std::option::Option::Some((#(#values,)*))
            } else {
                ::std::option::Option::None
            };
        };
        (before, names)
    });
    let returned = {
        let api_part = has_api_return.then(|| {
            let (_, before_names) = back.as_ref().unwrap();
            // By-value mirrors were converted before the body; the rest convert now.
            let mirrored: Vec<_> = args
                .iter()
                .filter(|(arg, _)| arg.pass != Pass::Value)
                .filter_map(|(arg, input)| mirror_in(arg, *input, true))
                .collect();
            let by_value_mirrors: Vec<syn::Ident> = args
                .iter()
                .filter(|(arg, _)| arg.pass == Pass::Value && arg.mirror)
                .map(|(_, input)| format_ident!("__m{input}"))
                .collect();
            let (made, mut names, mut values) = record_call(true, &mirrored);
            // A by-value mirror is an `Option` already: unwrap it with the others.
            for name in &by_value_mirrors {
                names.push(name.clone());
                values.push(quote!(#name));
            }
            let (result_in, result_out) = match api_value {
                Some(value) => {
                    let into = if value.mirror {
                        names.push(format_ident!("__mres"));
                        values.push(quote!(crate::game::points::mirror_in(&__result)));
                        quote!(::std::option::Option::Some(__mres))
                    } else {
                        result_in(value)
                    };
                    let out = result_out(value, &name);
                    (
                        quote!(__args.result = #into;),
                        quote! {
                            if let ::std::option::Option::Some(__value) = __args.result {
                                if let ::std::option::Option::Some(__value) = #out { __result = __value; }
                            }
                        },
                    )
                }
                None => (quote!(), quote!()),
            };
            let outs = args
                .iter()
                .filter(|(arg, _)| arg.pass == Pass::Mut)
                .map(|(arg, input)| copy_out(arg, *input, &name));
            let method = format_ident!("{}_return", name);
            let body = quote! {
                #[allow(unused_mut)]
                let mut __args = #made;
                #result_in
                mod_points::dispatch::#method(#host &mut __args);
                #result_out
                #(#outs)*
            };
            let body = guarded(&names, &values, body, &name);
            quote! {
                if let ::std::option::Option::Some((#(#before_names,)*)) = __back {
                    #body
                }
            }
        });
        let raw_part = shape.raw.then(|| {
            let variant = shape.return_variant();
            let return_listened = listened(&variant);
            let listener = shape.raw_return_listener();
            let (receiver, erased_receiver) = if shape.borrowed_return {
                (quote!(), quote!(::std::ptr::null_mut()))
            } else {
                (receiver.clone(), erased_receiver.clone())
            };
            let value = match &function.sig.output {
                syn::ReturnType::Type(..) => quote!(&mut __result),
                syn::ReturnType::Default => quote!(),
            };
            let return_name = format!("{name}_return");
            quote! {
                if #return_listened {
                    mod_points::__private::fire_raw(
                        mod_points::PointId::#variant,
                        #return_name,
                        &mut |__kind| match __kind {
                            mod_points::__private::RawKind::Typed(__function) => {
                                // SAFETY: registration guarantees this place's return listener type.
                                unsafe {
                                    ::std::mem::transmute::<*const (), #listener>(__function)(
                                        #receiver #value
                                    )
                                }
                            }
                            mod_points::__private::RawKind::Erased(__function) => {
                                // SAFETY: this slot supplies its exact receiver and return types.
                                unsafe {
                                    __function(
                                        mod_points::PointId::#variant as usize,
                                        #erased_receiver,
                                        &mut __result as *mut _ as *mut (),
                                    )
                                }
                            }
                        },
                    );
                }
            }
        });
        quote!(#api_part #raw_part)
    };
    let back = back.map(|(before, _)| before);
    let cancel_local = cancels.then(|| quote!(let mut __cancel = false;));
    let result_local =
        results.then(|| quote!(let mut __result_slot = ::std::option::Option::None;));
    outer.block = syn::parse_quote!({
        #cancel_local
        #result_local
        #(#shared_locals)*
        if #head_listened {
            #head_api
            #head_raw
            #stop
        }
        #(#shared_refs)*
        #back
        #[allow(unused_mut)]
        let mut __result = #callee #turbofish(#(#call),*);
        #returned
        __result
    });
    // The outer function keeps the function's own `#[inline]`: a small accessor in a lower crate
    // (`Chunk::get`) stays inlinable into its callers, slot check and all.
    function.sig.ident = inner;
    function.vis = syn::Visibility::Inherited;
    function
        .attrs
        .retain(|attr| !attr.path().is_ident("doc") && !attr.path().is_ident("inline"));
    function.attrs.push(syn::parse_quote!(#[inline(always)]));
    quote!(#outer #function)
}

/// `body`, run only when every mirror converted; a value the API can't carry skips the listeners
/// for this call, logged, rather than giving them a wrong value.
fn guarded(
    names: &[syn::Ident],
    values: &[TokenStream],
    body: TokenStream,
    point: &str,
) -> TokenStream {
    if names.is_empty() {
        return quote!({ #body });
    }
    quote! {
        if let (#(::std::option::Option::Some(#names),)*) = (#(#values,)*) {
            #body
        } else {
            crate::game::points::unconverted(#point);
        }
    }
}

/// The record's `result` from the value returned, `__result` (a mirror converts in the guard).
fn result_in(value: &point_shape::Value) -> TokenStream {
    let field = &value.field;
    if point_shape::wide(&value.ty).is_some() {
        quote!(::std::option::Option::Some(__result as #field))
    } else {
        quote!(::std::option::Option::Some(::std::clone::Clone::clone(
            &__result
        )))
    }
}

/// The engine value of a listener's `result`, `__value`: none when the engine refuses it.
fn result_out(value: &point_shape::Value, point: &str) -> TokenStream {
    let ty = &value.ty;
    if value.mirror {
        quote!(crate::game::points::mirror_out(&__value, #point))
    } else if point_shape::wide(ty).is_some() {
        quote!(::std::option::Option::Some(__value as #ty))
    } else {
        quote!(::std::option::Option::Some(__value))
    }
}
