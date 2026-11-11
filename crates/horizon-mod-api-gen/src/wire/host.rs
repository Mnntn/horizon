//! Host code. Runtime contract (on WasmMod and WasmHandler):
//! `fn call<R>(&self, class: CallClass, frame: Frame, id: u32, handler: Option<Id>,
//! encode: impl FnOnce(&mut Writer), decode: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>) -> Option<R>`.
//! `None` means refused/trapped; the runtime poisons the group before returning it.
use super::{Method, calls::*, ty::Ty};
use heck::ToSnakeCase;
use std::fmt::Write;

fn snake(owner: &str) -> String {
    owner.rsplit("::").next().unwrap().to_snake_case()
}

fn frame(surface: &mut super::Surface<'_>) -> Result<String, String> {
    use crate::index::Found;
    let types = &mut surface.types;
    let api = types.index.module("mod_api", &[])?;
    let mut supers = Vec::new();
    for owner in super::IMPORTS {
        let path = owner.split("::").map(str::to_string).collect::<Vec<_>>();
        let Found::Item(id) = types.index.resolve_path(api, &path)? else {
            return Err(format!("{owner} is not a trait"));
        };
        let syn::Item::Trait(item) = types.index.item(id) else {
            return Err(format!("{owner} is not a trait"));
        };
        let parents = item
            .supertraits
            .iter()
            .filter_map(|b| match b {
                syn::TypeParamBound::Trait(t) => Some(t.path.segments.last()?.ident.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();
        supers.push((*owner, parents));
    }
    let mut fields = String::new();
    let mut methods = String::new();
    for owner in super::IMPORTS {
        let name = snake(owner);
        let path = trait_path(owner);
        if let Some((_, from, expr)) = super::REACHED.iter().find(|(o, ..)| o == owner) {
            let mutable = from.ends_with("_mut")
                || (from.is_empty()
                    && surface
                        .methods
                        .iter()
                        .any(|m| m.owner == *owner && m.receiver == Some(true)));
            let body = if from.is_empty() {
                let mut body = expr.to_string();
                for accessor in ["level_host", "host", "level_random_host_mut", "host_mut"] {
                    body = body.replace(&format!("{accessor}()"), &format!("self.{accessor}()"));
                }
                body.replace("level_self.host", "level_host")
                    .replace("level_random_self.host_mut", "level_random_host_mut")
            } else {
                format!("let h = self.{from}()?; {expr}")
            };
            let body = body.replace(
                "h.backend().map(crate::BackendRef::open)",
                "crate::backend_of(h)",
            );
            writeln!(methods, "pub unsafe fn {name}{suffix}(&self) -> Result<&'static {m}dyn {path}, ServeError> {{ {body} }}", suffix=if mutable {"_mut"} else {""}, m=if mutable {"mut "} else {""}).unwrap();
            continue;
        }
        writeln!(
            fields,
            "{name}: Option<(*mut (dyn {path} + 'static), bool)>,"
        )
        .unwrap();
        writeln!(methods, "pub fn set_{name}(&mut self, v: &(dyn {path} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute(v as *const (dyn {path} + '_)) }}, false)); }} pub fn set_{name}_mut(&mut self, v: &mut (dyn {path} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute(v as *mut (dyn {path} + '_)) }}, true)); }}").unwrap();
        for mutable in [false, true] {
            let mut choices = Vec::new();
            for sub in super::IMPORTS {
                if super::REACHED.iter().any(|(o, ..)| o == sub) {
                    continue;
                }
                let mut stack = vec![sub.rsplit("::").next().unwrap().to_string()];
                let mut serves = false;
                while let Some(t) = stack.pop() {
                    if t == owner.rsplit("::").next().unwrap() {
                        serves = true;
                        break;
                    }
                    if let Some((_, parents)) = supers
                        .iter()
                        .find(|(o, _)| o.rsplit("::").next() == Some(t.as_str()))
                    {
                        stack.extend(parents.iter().cloned());
                    }
                }
                if serves {
                    choices.push(format!(
                        "self.{}{filter}.map(|(p, _)| unsafe {{ &{m}*p as &{m}dyn {path} }})",
                        snake(sub),
                        filter = if mutable { ".filter(|(_, m)| *m)" } else { "" },
                        m = if mutable { "mut " } else { "" }
                    ));
                }
            }
            // A zero-argument accessor lends another frame resource. Resolve directly
            // from a stored source pointer, not recursively through getters: cyclic
            // accessor graphs cannot recurse when no source is present.
            for method in &surface.methods {
                let Ty::Dyn(target, ret_mut) = &method.ret else {
                    continue;
                };
                if target != owner.rsplit("::").next().unwrap()
                    || (mutable && !ret_mut)
                    || !method.params.is_empty()
                    || !super::IMPORTS.contains(&method.owner)
                    || method.owner == *owner
                    || super::REACHED.iter().any(|(o, ..)| *o == method.owner)
                {
                    continue;
                }
                let source = snake(method.owner);
                let source_mut = method.receiver == Some(true);
                choices.push(format!("self.{source}{filter}.map(|(p,_)| unsafe {{ (&{s}*p).{}() as &{m}dyn {path} }})",method.name,filter=if source_mut {".filter(|(_,m)|*m)"} else {""},s=if source_mut {"mut "} else {""},m=if mutable {"mut "} else {""}));
            }
            let expr = choices.join(".or_else(|| ");
            let expr = format!("{expr}{}", ")".repeat(choices.len().saturating_sub(1)));
            writeln!(methods, "pub unsafe fn {name}{suffix}(&self) -> Result<&'static {m}dyn {path}, ServeError> {{ {expr}.ok_or(ServeError::Missing(\"{name}\")) }}", suffix=if mutable {"_mut"} else {""}, m=if mutable {"mut "} else {""}).unwrap();
        }
    }
    Ok(format!(
        "#[derive(Default)] pub struct Frame {{ {fields} }}\n// SAFETY: pointers are used only by unsafe serve during the synchronous call that lent them.\nunsafe impl Send for Frame {{}}\nimpl Frame {{ {methods} }}"
    ))
}

fn resource(ty: &Ty) -> bool {
    match ty {
        Ty::Handler(..) => true,
        Ty::Option(t) => resource(t),
        _ => false,
    }
}
fn supported_resource(ty: &Ty) -> bool {
    match ty {
        Ty::Handler(name, _) => name == super::ERASED || super::exports().contains(&name.as_str()),
        Ty::Option(t) => supported_resource(t),
        _ => false,
    }
}
pub(super) fn put(ty: &Ty, expr: &str) -> String {
    match ty {
        Ty::Ref(..) => format!("w.put(&*{expr});"),
        Ty::Str(super::ty::Str::Borrowed | super::ty::Str::Static)
        | Ty::List(_, super::ty::List::Slice) => format!("w.put({expr});"),
        Ty::Option(inner)
            if matches!(
                **inner,
                Ty::Ref(..)
                    | Ty::Str(super::ty::Str::Borrowed)
                    | Ty::List(_, super::ty::List::Slice)
            ) =>
        {
            // Already a borrow: `Option<&T>` is passed as it is.
            format!("w.put_opt({expr});")
        }
        _ => format!("w.put(&{expr});"),
    }
}

fn event_input(
    types: &super::ty::Types<'_>,
    def: usize,
    expr: &str,
    setup: &mut String,
    put: &mut String,
) {
    if types.defs[def].append.is_some() || types.defs[def].projection.is_some() {
        write!(
            put,
            "{{ use mod_api::wire::Put as _; ({expr}).put_to(w); }}"
        )
        .unwrap();
        return;
    }
    if let Some(cases) = frame_cases(types, def) {
        let tag = format!("tag_{}", expr.replace('.', "_"));
        write!(setup, "let {tag}=match &mut {expr} {{").unwrap();
        for (i, case) in cases.iter().enumerate() {
            let v = &case.payload;
            let Ty::Dyn(name, m) = &v[0] else {
                unreachable!()
            };
            write!(
                setup,
                "{}::{}(h) => {{ frame.set_{}{}(&{}**h); {i}u32 }},",
                types.defs[def].rust,
                case.rust,
                snake(name),
                if *m { "_mut" } else { "" },
                if *m { "mut " } else { "" }
            )
            .unwrap();
        }
        setup.push_str("};");
        write!(put, "w.put(&{tag});").unwrap();
        return;
    }
    let Some(fields) = types.defs[def].borrowed.fields() else {
        return;
    };
    for f in fields.iter() {
        let path = format!("{expr}.{}", f.rust);
        match &f.ty {
            Ty::Dyn(name, m) => {
                write!(
                    setup,
                    "frame.set_{}{}(&{}*{path});",
                    snake(name),
                    if *m { "_mut" } else { "" },
                    if *m { "mut " } else { "" }
                )
                .unwrap();
            }
            ty => {
                let inner = match ty {
                    Ty::Ref(t, _) => &**t,
                    t => t,
                };
                if let Ty::Named(d) = inner
                    && types.defs[*d].lifetimes != 0
                    && types.defs[*d].view.is_none()
                {
                    event_input(types, *d, &path, setup, put);
                } else if lifetime_def(types, ty)
                    && !matches!(inner, Ty::Named(d) if types.defs[*d].view.is_some())
                {
                    put.push_str(&super::calls::snapshot(types, ty, &path).unwrap());
                } else {
                    put.push_str(&self::put(ty, &path));
                }
            }
        }
    }
}
fn fallback(types: &super::ty::Types<'_>, m: &Method) -> &'static str {
    match &m.ret {
        Ty::Result(ok, _) if **ok == Ty::Unit => "Ok(())",
        Ty::Result(..) => "Err(call_refused())",
        Ty::Named(d) if types.defs[*d].rust.ends_with("::Flow") => "mod_api::Flow::Continue",
        _ => "NoAnswer::no_answer()",
    }
}

fn resources() -> String {
    let mut store = String::from(
        r#"if value.is::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value=*value.downcast::<std::sync::Arc<dyn std::any::Any + Send + Sync>>().map_err(|_|WireError::Invalid("wrong erased handler"))?;
        if let Some(proxy)=value.downcast_ref::<crate::WasmHandler>() {
            let module=unsafe {(&*cx).module.as_ref()}.ok_or(WireError::Invalid("no calling module"))?;
            if proxy.belongs_to(module) {return Ok(proxy.index)}
        }
        return Ok(mod_api::wire::Unsupported::ID);
    }"#,
    );
    let mut load = String::from(
        r#"if view==std::any::TypeId::of::<std::sync::Arc<dyn std::any::Any + Send + Sync>>() {
        let value:std::sync::Arc<dyn std::any::Any + Send + Sync>=if id==mod_api::wire::Unsupported::ID {mod_api::wire::Unsupported::shared()} else {std::sync::Arc::new(crate::WasmHandler::new(&module,id))};
        return Ok(Box::new(value));
    }"#,
    );
    for owner in super::exports()
        .iter()
        .filter(|owner| super::mod_receiver(owner).is_none())
    {
        let path = trait_path(owner);
        writeln!(store,"if value.is::<std::sync::Arc<dyn {path}>>() {{ let value = *value.downcast::<std::sync::Arc<dyn {path}>>().map_err(|_| WireError::Invalid(\"wrong handler\"))?; return Ok(unsafe {{ (&mut *cx).held.insert_long(Box::new(OwnerThread::new(value))) }}); }}").unwrap();
        for container in ["Box", "std::sync::Arc"] {
            writeln!(load,"if view == std::any::TypeId::of::<{container}<dyn {path}>>() {{ return Ok(Box::new({container}::new(crate::WasmHandler::new(&module,id)) as {container}<dyn {path}>)); }}").unwrap();
        }
    }
    format!(
        r#"struct HostCodec;
thread_local! {{
    static CURRENT_CX:std::cell::Cell<*mut Cx> = const {{std::cell::Cell::new(std::ptr::null_mut())}};
    static HOST_CODEC:std::rc::Rc<dyn mod_api::wire::ResourceCodec> = std::rc::Rc::new(HostCodec);
}}
fn current_cx()->Result<*mut Cx,WireError> {{
    let cx=CURRENT_CX.with(|current|current.get());
    if cx.is_null() {{Err(WireError::Invalid("no resource context"))}} else {{Ok(cx)}}
}}
impl mod_api::wire::ResourceCodec for HostCodec {{
    fn store(&self,value:Box<dyn std::any::Any>)->Result<mod_api::wire::Id,WireError> {{
        let cx=current_cx()?;
        {store}
        Err(WireError::Invalid("unknown handler type"))
    }}
    fn load(&self,id:mod_api::wire::Id,view:std::any::TypeId)->Result<Box<dyn std::any::Any>,WireError> {{
        let cx=current_cx()?;
        let module=unsafe {{(&*cx).module.as_ref()}}.ok_or(WireError::Invalid("no calling module"))?;
        {load}
        Err(WireError::Invalid("unknown handler view"))
    }}
    fn refused(&self,handler:&str) {{
        let Ok(cx)=current_cx() else {{return}};
        let Some(module)=(unsafe {{(&*cx).module.as_ref()}}) else {{return}};
        let seen=unsafe {{&mut (*cx).refused}};
        match seen.get_mut(module.mod_id()) {{
            Some(names) if names.contains(handler)=>return,
            Some(names)=>{{names.insert(handler.to_owned());}},
            None=>{{seen.insert(module.mod_id().to_owned(),std::collections::HashSet::from([handler.to_owned()]));}},
        }}
        log::debug!("wasm mod {{}}: {{handler:?}} unsupported: cross-group capability",module.mod_id());
    }}
}}
struct HostContext(*mut Cx);
impl Drop for HostContext {{fn drop(&mut self) {{CURRENT_CX.with(|current|current.set(self.0));}}}}
// Field order removes the codec scope before restoring the outer context.
pub struct HostResources {{_scope:mod_api::wire::ResourceScope,_context:HostContext}}
/// # Safety
/// cx and its frame pointees stay live until this guard drops. These private runtime
/// guards must drop in reverse construction order, before the frame or store is released.
pub unsafe fn resource_scope(cx:*mut Cx,native_root:bool)->HostResources {{
    let context=HostContext(CURRENT_CX.with(|current|current.replace(cx)));
    let codec=HOST_CODEC.with(std::rc::Rc::clone);
    let scope=if native_root {{unsafe {{mod_api::wire::ResourceScope::native_root(codec)}}}} else {{mod_api::wire::ResourceScope::new(codec)}};
    HostResources {{_scope:scope,_context:context}}
}}
"#
    )
}
pub(super) fn generate(surface: &mut super::Surface<'_>) -> Result<String, String> {
    let mut code = String::from(
        "// @generated by horizon-mod-api-gen; do not edit.\nuse mod_api::wire::{Reader, Writer, WireError, Slots};\n#[derive(Debug)] pub enum ServeError { Decode(WireError), Missing(&'static str), Stale, WrongThread, Unsupported(u32) } impl From<WireError> for ServeError { fn from(e: WireError) -> Self { Self::Decode(e) } } impl std::fmt::Display for ServeError { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, \"{self:?}\") } } impl std::error::Error for ServeError {} fn missing(s: &'static str) -> ServeError { ServeError::Missing(s) }\n#[derive(Default)] pub struct Cx { pub frames: Vec<Frame>, pub held: Slots<Box<dyn std::any::Any + Send>>, pub module: *const crate::WasmMod, pub refused: std::collections::HashMap<String,std::collections::HashSet<String>>, static_strings: std::collections::HashMap<&'static str,u32> } impl Cx { pub fn frame(&self) -> Result<&Frame, ServeError> { self.frames.last().ok_or(ServeError::Missing(\"frame\")) } }\n",
    );
    code.push_str("use crate::{NoAnswer, call_refused};\n");
    code.push_str(&frame(surface)?);
    code.push_str(include_str!("owner_thread.rs"));
    code.push_str(&resources());
    let types = &surface.types;
    code.push_str(
        "pub fn export_id(owner: &str, name: &str) -> Option<u32> { match (owner,name) {\n",
    );
    for (id, m) in surface
        .methods
        .iter()
        .filter(|m| super::exports().contains(&m.owner))
        .enumerate()
    {
        writeln!(code, "({:?},{:?}) => Some({id}),", m.owner, m.name).unwrap();
    }
    code.push_str("_ => None } }\n");
    code.push_str(
        "pub fn import_id(owner: &str, name: &str) -> Option<u32> { match (owner,name) {\n",
    );
    for (id, m) in imports(&surface.methods)
        .iter()
        .copied()
        .chain(held_methods(&surface.methods))
        .enumerate()
    {
        writeln!(code, "({:?},{:?}) => Some({id}),", m.owner, m.name).unwrap();
    }
    code.push_str("_ => None } }\n");
    code.push_str(
        r#"
impl Cx {
    fn check_static_ack(&self, ack: u32) -> Result<(), ServeError> {
        if ack != u32::MAX && ack as usize >= self.static_strings.len() {
            return Err(WireError::Invalid("static string acknowledgement outside table").into());
        }
        Ok(())
    }
    fn put_static(&mut self, out: &mut Writer, text: &'static str, ack: u32) -> Result<(), ServeError> {
        if text.len() > mod_api::wire::MAX_STATIC_STRING_BYTES {
            return Err(WireError::Invalid("static string too long").into());
        }
        let id = if let Some(&id) = self.static_strings.get(text) { id } else {
            if self.static_strings.len() >= mod_api::wire::MAX_STATIC_STRINGS {
                // Admission is an optimization: a full group table cannot impose a
                // stricter limit than the existing per-member static string storage.
                out.put(&2u8);
                out.put(&u32::MAX);
                out.put(text);
                return Ok(());
            }
            let id = self.static_strings.len() as u32;
            self.static_strings.insert(text, id);
            id
        };
        let definition = id != ack;
        out.put(&u8::from(definition));
        out.put(&id);
        if definition { out.put(text); }
        Ok(())
    }
}
"#,
    );
    code.push_str("pub unsafe fn serve(id: u32, cx: *mut Cx, bytes: &[u8], out: &mut Writer) -> Result<(), ServeError> { let mut input = Reader::new(bytes); match id {\n");
    let imports = imports(&surface.methods);
    let mut import_gaps = Vec::new();
    for (id, m) in imports
        .iter()
        .copied()
        .chain(held_methods(&surface.methods))
        .enumerate()
    {
        let is_held = id >= imports.len();
        if (resource(&m.ret) && !supported_resource(&m.ret))
            || (has_frame(&m.ret) && !resource(&m.ret))
            || m.params
                .iter()
                .any(|(_, t)| !matches!(t, Ty::Dyn(..)) && has_frame(t) && !supported_resource(t))
            || m.params
                .iter()
                .any(|(_, t)| matches!(t, Ty::Ref(_, true)) && !mutable_scalar(t))
            || m.params.iter().any(|(_, t)| {
                (lifetime_def(types, t)
                    && (super::calls::snapshot(types, t, "p").is_none()
                        || lend(types, t, "p").is_none()))
                    || (has_ref_in_data(t) && lend(types, t, "p").is_none())
                    || (!matches!(t, Ty::Dyn(..)) && !resource(t) && held(types, t).is_none())
            })
        {
            import_gaps.push(format!(
                "{}{}::{}",
                if is_held { "held-" } else { "" },
                m.owner,
                m.name
            ));
            continue;
        }
        let mut body = if is_held {
            "let held_id: mod_api::wire::Id = input.take()?;".to_string()
        } else {
            String::new()
        };
        if m.ret == Ty::Str(super::ty::Str::Static) {
            body.push_str("let static_ack: u32 = input.take()?; unsafe { (&*cx).check_static_ack(static_ack)?; }");
        }
        let mut args = Vec::new();
        for (i, (_, ty)) in m.params.iter().enumerate() {
            if let Ty::Dyn(name, mutable) = ty {
                args.push(format!(
                    "unsafe {{ (&*cx).frame()?.{}{}()? }}",
                    snake(name),
                    if *mutable { "_mut" } else { "" }
                ));
                continue;
            }
            writeln!(
                body,
                "let {mutable}p{i}: {} = input.take()?;",
                if resource(ty) {
                    rust_ty(types, ty, false)
                } else {
                    held(types, ty).unwrap()
                },
                mutable = if mutable_scalar(ty) { "mut " } else { "" }
            )
            .unwrap();
            let arg = match ty {
                Ty::List(inner, super::ty::List::Slice)
                    if matches!(**inner, Ty::Str(super::ty::Str::Borrowed)) =>
                {
                    format!("&p{i}.iter().map(String::as_str).collect::<Vec<_>>()")
                }
                _ if resource(ty) => format!("p{i}"),
                _ => lend(types, ty, &format!("p{i}")).unwrap().expr,
            };
            args.push(arg);
        }
        let mut answer: String = if m.ret == Ty::Str(super::ty::Str::Static) {
            "unsafe { (&mut *cx).put_static(out,answer,static_ack)?; }".into()
        } else if resource(&m.ret) {
            "out.put_owned(answer);".into()
        } else if matches!(&m.ret,Ty::Option(t) if matches!(**t,Ty::Str(super::ty::Str::Borrowed | super::ty::Str::Static) | Ty::Ref(..) | Ty::List(_,super::ty::List::Slice)))
        {
            "out.put_opt(answer);".into()
        } else {
            "out.put_owned(answer);".into()
        };
        for (i, (_, ty)) in m.params.iter().enumerate() {
            if mutable_scalar(ty) {
                write!(answer, "out.put(&p{i});").unwrap();
            }
        }
        let receiver = if is_held {
            format!(
                "unsafe {{ (&*cx).held.get(held_id).map_err(|_| ServeError::Stale)?.downcast_ref::<OwnerThread<std::sync::Arc<dyn {}>>>().ok_or(ServeError::Stale)?.get()?.clone() }}",
                trait_path(m.owner)
            )
        } else {
            format!(
                "unsafe {{ (&*cx).frame()?.{}{}()? }}",
                snake(m.owner),
                if m.receiver == Some(true) { "_mut" } else { "" }
            )
        };
        // Keep decoded values on the selected method's stack, not in the dispatcher's
        // shared frame: a native import may synchronously re-enter wasm.
        writeln!(code, "{id} => {{ #[inline(never)] unsafe fn invoke(cx: *mut Cx, input: &mut Reader<'_>, out: &mut Writer) -> Result<(), ServeError> {{ {body} if !input.rest().is_empty() {{ return Err(ServeError::Decode(WireError::Tag(u32::MAX))); }} let receiver = {receiver}; let answer = receiver.{}({}); {answer} Ok(()) }} unsafe {{ invoke(cx, &mut input, out)?; }} }},",m.name,args.join(",")).unwrap();
    }
    code.push_str("_ => return Err(ServeError::Unsupported(id)) } Ok(()) }\n");
    writeln!(
        code,
        "pub const UNSUPPORTED_IMPORTS: &[&str] = &{import_gaps:?};"
    )
    .unwrap();
    let mut gaps = Vec::new();
    for owner in super::exports() {
        let receiver = if super::mod_receiver(owner).is_some() {
            "WasmMod"
        } else {
            "WasmHandler"
        };
        let mut groups = vec![owner.to_string()];
        for m in surface
            .methods
            .iter()
            .chain(&surface.dropped)
            .filter(|m| m.owner == *owner)
        {
            if let Some(via) = &m.via
                && !groups.contains(via)
            {
                groups.push(via.clone());
            }
        }
        for group in groups {
            writeln!(code, "impl {} for crate::{receiver} {{", trait_path(&group)).unwrap();
            if receiver == "WasmMod" && group == "Hooks" {
                code.push_str(
                    "fn listeners(&self) -> &'static [mod_api::Listener] { self.listeners }\n",
                );
            }
            for (id, m) in surface
                .methods
                .iter()
                .filter(|m| super::exports().contains(&m.owner))
                .chain(
                    surface
                        .dropped
                        .iter()
                        .filter(|m| super::exports().contains(&m.owner)),
                )
                .enumerate()
                .filter(|(_, m)| m.owner == *owner && m.via.as_deref().unwrap_or(m.owner) == group)
            {
                let sig = rust_sig(types, m);
                if super::SUBSCRIBED
                    .iter()
                    .any(|(o, n)| *o == m.owner && *n == m.name)
                {
                    let body = if receiver == "WasmMod" {
                        "self.subscribed".to_string()
                    } else {
                        format!("self.subscriptions({owner:?}, {id})")
                    };
                    writeln!(
                        code,
                        "fn {}(&self) -> &'static [Option<mod_api::HookId>] {{ {body} }}",
                        m.name
                    )
                    .unwrap();
                    continue;
                }
                if surface.dropped.iter().any(|d| std::ptr::eq(d, m)) || !dispatchable(types, m) {
                    gaps.push(format!("{}::{}", m.owner, m.name));
                    writeln!(code, "{sig} {{ {} }}", fallback(types, m)).unwrap();
                    continue;
                }
                let (mut setup, mut put, mut take, mut apply) =
                    (String::new(), String::new(), String::new(), String::new());
                let mut backs = vec!["answer".to_string()];
                for (i, (_, ty)) in m.params.iter().enumerate() {
                    match ty {
                        Ty::Dyn(name, mutable) => {
                            write!(
                                setup,
                                "frame.set_{}{}(p{i});",
                                snake(name),
                                if *mutable { "_mut" } else { "" }
                            )
                            .unwrap();
                        }
                        Ty::Named(d) if types.defs[*d].lifetimes != 0 => {
                            write!(setup, "let mut p{i}=p{i};").unwrap();
                            event_input(types, *d, &format!("p{i}"), &mut setup, &mut put);
                        }
                        Ty::Ref(inner, mutable) if matches!(&**inner, Ty::Named(d) if types.defs[*d].lifetimes != 0) =>
                        {
                            let Ty::Named(d) = &**inner else {
                                unreachable!()
                            };
                            event_input(types, *d, &format!("p{i}"), &mut setup, &mut put);
                            for leaf in
                                writeback_fields(types, *d, &format!("p{i}"), *mutable).unwrap()
                            {
                                let local = format!("b_{}", leaf.local);
                                backs.push(local.clone());
                                write!(take, "let {local}: {} = r.take()?;", leaf.owned).unwrap();
                                let path = leaf.path;
                                if leaf.append {
                                    let owner = path.rsplit_once('.').unwrap().0;
                                    write!(take, "let {local} = {owner}.__wire_checked({local})?;")
                                        .unwrap();
                                    write!(apply, "{owner}.__wire_apply({local});").unwrap();
                                } else if leaf.view {
                                    write!(apply, "{path}.__wire_apply({local});").unwrap();
                                } else if leaf.optional {
                                    write!(take,"if {path}.is_some() != {local}.is_some() {{ return Err(WireError::Tag(u32::MAX)); }}").unwrap();
                                    write!(apply,"if let (Some(target),Some(value)) = ({path}.as_deref_mut(),{local}) {{ *target=value; }}").unwrap();
                                } else {
                                    write!(
                                        apply,
                                        "{}{path} = {local};",
                                        if leaf.reference { "*" } else { "" }
                                    )
                                    .unwrap();
                                }
                            }
                        }
                        _ => {
                            put.push_str(&if matches!(
                                ty,
                                Ty::Ref(..)
                                    | Ty::Str(super::ty::Str::Borrowed)
                                    | Ty::List(_, super::ty::List::Slice)
                            ) {
                                self::put(ty, &format!("p{i}"))
                            } else {
                                format!("w.put_owned(p{i});")
                            });
                            if matches!(ty, Ty::Ref(_, true)) {
                                backs.push(format!("b{i}"));
                                write!(take, "let b{i}: {} = r.take()?;", held(types, ty).unwrap())
                                    .unwrap();
                                write!(apply, "*p{i} = b{i};").unwrap();
                            }
                        }
                    }
                }
                let class =
                    super::call_class(types, m).replace("super::Call::", "crate::CallClass::");
                let tuple = format!("({},)", backs.join(","));
                writeln!(code,"{sig} {{ let mut frame = Frame::default(); {setup} match self{module}.call({class}, frame, {id}, {}, |w| {{ {put} }}, |r| {{ let answer: {} = r.take()?; {take} if !r.rest().is_empty() {{ return Err(WireError::Tag(u32::MAX)); }} Ok({tuple}) }}) {{ Some({tuple}) => {{ {apply} answer }}, None => {} }} }}",if receiver=="WasmMod" {"None"} else {"Some(self.index)"},rust_ty(types,&m.ret,true),fallback(types,m),module=if receiver=="WasmMod" {""} else {".module"}).unwrap();
            }
            code.push_str("}\n");
        }
    }
    writeln!(code, "pub const UNSUPPORTED_EXPORTS: &[&str] = &{gaps:?};").unwrap();
    // The API is qualified as crate in the shared signature writer, but lives in mod_api here.
    Ok(code
        .replace("crate ::", "mod_api ::")
        .replace("crate::", "mod_api::")
        .replace("mod_api::Wasm", "crate::Wasm")
        .replace("mod_api::CallClass", "crate::CallClass")
        .replace("mod_api::refused", "crate::refused")
        .replace("mod_api::NoAnswer", "crate::NoAnswer")
        .replace(
            "use mod_api::{NoAnswer, call_refused}",
            "use crate::{NoAnswer, call_refused}",
        ))
}

/// Generate the host boundary from the same source surface as the guest.
pub fn generate_host(root: &std::path::Path) -> Result<String, String> {
    host_and_coverage(root).map(|(code, _)| code)
}

/// The book's wasm boundary coverage page, generated from the same surface and host gap tables
/// as the host boundary: the book's preprocessor renders it at build time.
pub fn generate_coverage(root: &std::path::Path) -> Result<String, String> {
    host_and_coverage(root).map(|(_, report)| report)
}

fn host_and_coverage(root: &std::path::Path) -> Result<(String, String), String> {
    let generated = crate::sources(root, crate::FillMode::Require)?;
    let providers = crate::capability_providers::generate(root, &[])?.0;
    let included = [
        ("mod_api_generated.rs", generated.hooks.as_str()),
        ("mod_api_generated_host.rs", generated.host.as_str()),
        ("mod_api_host_areas.rs", generated.areas.as_str()),
        ("capability_providers_generated.rs", providers.as_str()),
    ];
    let mut surface = super::surface(root, &included)?;
    let count = surface
        .methods
        .iter()
        .filter(|m| super::exports().contains(&m.owner))
        .position(|m| m.owner == "Hooks" && m.name == "subscribed")
        .ok_or("Hooks::subscribed absent")?;
    let mut code = generate(&mut surface)?;
    let imports = code
        .lines()
        .find(|l| l.starts_with("pub const UNSUPPORTED_IMPORTS:"))
        .unwrap_or("");
    let exports = code
        .lines()
        .find(|l| l.starts_with("pub const UNSUPPORTED_EXPORTS:"))
        .unwrap_or("");
    let mut report = String::from(
        "# Wire boundary coverage\n\nSurface methods excluding unbound and Not on wasm. Generated from both sides' shared surface and current host gap tables.\n\n| Method | Status | Reason |\n|---|---|---|\n",
    );
    for m in surface.methods.iter().chain(&surface.dropped) {
        if surface
            .excused
            .iter()
            .any(|(o, n)| *o == m.owner && *n == m.name)
        {
            continue;
        }
        let name = format!("{}::{}", m.owner, m.name);
        let dropped = surface.dropped.iter().any(|d| std::ptr::eq(d, m));
        let gap = dropped
            || if super::exports().contains(&m.owner) {
                exports.contains(&format!("{name:?}"))
            } else {
                imports.contains(&format!("{name:?}")) && !super::frame_accessor(m)
            };
        writeln!(
            report,
            "| `{name}` | {} | {} |",
            if gap { "gap" } else { "crosses" },
            if dropped {
                "mentions skipped type"
            } else if gap {
                "borrow/resource shape not yet supported"
            } else if super::frame_accessor(m) {
                "frame accessor; no bytes"
            } else {
                "Wire call"
            }
        )
        .unwrap();
    }
    report.push_str("\n| Library mod plain `pub fn` / `pub static` called by a dependent | crosses | shared dependency rlib, stable crate identity, and retained public exports; production-loader library fixture covers separate library/observer/dependent builds |\n");
    let gaps = report
        .lines()
        .filter(|line| line.contains("| gap |") || line.contains("| OPEN |"))
        .count();
    writeln!(report, "\nRequired gap total: {gaps}.").unwrap();
    report.push_str(include_str!("../../coverage-notes.md"));
    writeln!(code, "pub const SUBSCRIBED: u32 = {count};").unwrap();
    code.push_str("pub fn hook_id(name: &str) -> Option<mod_api::HookId> { match name {\n");
    let ast = syn::parse_file(&generated.hooks).map_err(|e| e.to_string())?;
    let ids = ast
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == "HookId" => Some(
                e.variants
                    .iter()
                    .map(|v| v.ident.to_string())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .ok_or("HookId absent")?;
    for hook in crate::Catalog::load(root)?.hooks {
        if !ids.contains(&crate::hook_id(&hook.name)) {
            continue;
        }
        writeln!(
            code,
            "{:?} => Some(mod_api::HookId::{}),",
            hook.name,
            crate::hook_id(&hook.name)
        )
        .unwrap();
    }
    code.push_str("_ => None } } pub fn subscriptions_of(owner: &str, names: Vec<String>) -> &'static [Option<mod_api::HookId>] { names.into_iter().filter_map(|name| { let id = hook_id(&name); if id.is_none() { log::warn!(\"wasm {owner}: unknown hook {name}\"); } id.map(Some) }).collect::<Vec<_>>().leak() }\n");
    Ok((code, report))
}
