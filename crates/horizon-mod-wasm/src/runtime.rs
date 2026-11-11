//! Sandboxed core-wasm transport. One lock, memory and poison flag per dependency group.
//! Refusals leave the group running; only faults and native unwinds poison it.
//! The preallocated call path adds an Arc increment/decrement per call, without
//! allocating on success. Diagnostic keys are built and checked only on refusal.
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;
use crate::{
    Refusal,
    boundary::{Cx, Frame, serve},
    link::{Group, Member},
};
use mod_api::{
    HookId,
    wire::{Id, Reader, WireError, Writer},
};
use std::{
    cell::Cell,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use wasmtime::{
    AsContextMut, Caller, Engine, Extern, Func, Memory, Store, StoreContextMut, TypedFunc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallClass {
    Load,
    Hook,
    Worker,
    Callback,
}

pub(crate) struct State {
    limits: wasmtime::StoreLimits,
    cx: Box<Cx>,
    memory: Option<Memory>,
    replies: Vec<Writer>,
    inputs: Vec<Writer>,
    outputs: Vec<Vec<u8>>,
    imports: Vec<Vec<u8>>,
    depth: usize,
    path: Vec<(Arc<str>, u32)>,
    refusals: std::collections::HashSet<String>,
    epoch_budget: u64,
    group: usize,
    lending: Arc<AtomicUsize>,
}
// SAFETY: frames contain borrowed native objects only while the group mutex is held.
// They are pushed and popped synchronously on that thread; callbacks borrow the same
// locked store through a thread-local loan. The module pointer borrows call()'s self
// under the same scope. Between threads the frame stack is empty and module is null.
unsafe impl Send for State {}
impl Default for State {
    fn default() -> Self {
        Self {
            limits: wasmtime::StoreLimitsBuilder::new()
                .trap_on_grow_failure(false)
                .memory_size(256 << 20)
                .table_elements(1 << 20)
                .build(),
            cx: Box::default(),
            memory: None,
            replies: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            imports: Vec::new(),
            depth: 0,
            path: Vec::with_capacity(64),
            refusals: Default::default(),
            epoch_budget: 0,
            group: 0,
            lending: Arc::default(),
        }
    }
}

/// Explicit engine settings; the remaining defaults are covered by Wasmtime compatibility.
pub(crate) const ENGINE_CONFIG: (&str, bool) = ("epoch_interruption", true);

pub(crate) fn engine() -> wasmtime::Result<Engine> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(ENGINE_CONFIG.1);
    let engine = Engine::new(&config)?;
    let weak = engine.weak();
    std::thread::Builder::new()
        .name("wasm epoch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(1));
                let Some(engine) = weak.upgrade() else { break };
                engine.increment_epoch();
            }
        })?;
    Ok(engine)
}

#[derive(Default)]
struct Releases {
    pending: AtomicBool,
    ids: Mutex<Vec<Id>>,
}
impl Releases {
    fn push(&self, id: Id) {
        let mut ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
        ids.push(id);
        self.pending.store(true, Ordering::Release);
    }
    fn take(&self) -> Vec<Id> {
        if !self.pending.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        // A racing push either joins this drain or leaves pending set for the next
        // call. At worst the next call locks an empty queue; no release is lost.
        std::mem::take(&mut *self.ids.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

struct Running {
    group: Mutex<Group<State>>,
    poisoned: AtomicBool,
    lending: Arc<AtomicUsize>,
}
/// Only faults may leave guest state half-updated. Refusals returned normally.
#[derive(Debug)]
enum CallError {
    Refused(wasmtime::Error),
    Fault(wasmtime::Error),
}
impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) | Self::Fault(error) => write!(f, "{error:#}"),
        }
    }
}
impl From<WireError> for CallError {
    fn from(error: WireError) -> Self {
        Self::Refused(error.into())
    }
}
struct PoisonOnPanic<'a>(&'a AtomicBool);
impl Drop for PoisonOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}
struct FrameScope {
    state: *mut State,
    previous_module: *const WasmMod,
    running: Arc<Running>,
}
impl Drop for FrameScope {
    fn drop(&mut self) {
        // SAFETY: the store is exclusively locked or lent on this thread, and its
        // boxed state remains in place until this synchronous call scope ends.
        let state = unsafe { &mut *self.state };
        state.cx.frames.pop();
        state.path.pop();
        state.cx.module = self.previous_module;
        state.depth -= 1;
        if std::thread::panicking() {
            self.running.poisoned.store(true, Ordering::Relaxed);
        }
        if state.depth == 0 && self.running.poisoned.load(Ordering::Relaxed) {
            // Detach before dropping: native destructors may call a proxy again.
            drop(std::mem::take(&mut state.cx.held));
        }
    }
}
#[derive(Clone)]
pub struct WasmMod {
    id: Arc<str>,
    running: Arc<Running>,
    input: TypedFunc<u32, u32>,
    hook: TypedFunc<u32, u64>,
    pub subscribed: &'static [Option<HookId>],
    pub(crate) listeners: &'static [mod_api::Listener],
    released: Arc<Releases>,
    subscriptions: Arc<Mutex<std::collections::HashMap<(Id, u32), &'static [Option<HookId>]>>>,
}
/// A guest object never owns the store that may itself retain that object.
#[derive(Clone)]
pub(crate) struct WeakWasmMod {
    id: Arc<str>,
    running: Weak<Running>,
    input: TypedFunc<u32, u32>,
    hook: TypedFunc<u32, u64>,
    subscribed: &'static [Option<HookId>],
    pub(crate) listeners: &'static [mod_api::Listener],
    released: Arc<Releases>,
    subscriptions: Arc<Mutex<std::collections::HashMap<(Id, u32), &'static [Option<HookId>]>>>,
}
impl WeakWasmMod {
    /// The module-owned plan for a raw point's head call, until the module is dropped.
    pub(crate) fn is_stopped(&self) -> bool {
        self.running
            .upgrade()
            .is_some_and(|running| running.poisoned.load(Ordering::Relaxed))
    }
    /// Refusals answer unavailable without stopping the group; faults stop all members.
    pub(crate) fn call<R>(
        &self,
        class: CallClass,
        frame: Frame,
        id: u32,
        handler: Option<Id>,
        args: impl FnOnce(&mut Writer),
        reply: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        let module = WasmMod {
            id: self.id.clone(),
            running: self.running.upgrade()?,
            input: self.input.clone(),
            hook: self.hook.clone(),
            subscribed: self.subscribed,
            listeners: self.listeners,
            released: self.released.clone(),
            subscriptions: self.subscriptions.clone(),
        };
        module.call(class, frame, id, handler, args, reply)
    }
}
pub struct WasmHandler {
    pub index: Id,
    pub(crate) module: WeakWasmMod,
}
impl std::fmt::Debug for WasmHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmHandler")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}
impl WasmHandler {
    pub fn new(module: &WasmMod, index: Id) -> Self {
        Self {
            index,
            module: module.downgrade(),
        }
    }
    pub fn belongs_to(&self, module: &WasmMod) -> bool {
        self.module.running.ptr_eq(&Arc::downgrade(&module.running))
    }
    pub fn subscriptions(&self, owner: &'static str, export_id: u32) -> &'static [Option<HookId>] {
        if let Some(value) = self
            .module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(self.index, export_id))
            .copied()
        {
            return value;
        }
        let names = self
            .module
            .call(
                CallClass::Callback,
                Frame::default(),
                export_id,
                Some(self.index),
                |_| {},
                |r| r.take(),
            )
            .unwrap_or_default();
        let value = crate::boundary::subscriptions_of(owner, names);
        self.module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert((self.index, export_id), value);
        value
    }
}
impl Drop for WasmHandler {
    fn drop(&mut self) {
        self.module
            .subscriptions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(id, _), _| *id != self.index);
        self.module.released.push(self.index);
    }
}

// An import lends its context only while native code runs. Removing the pointer while
// guest code executes prevents double borrowing; a nested import installs a fresh node.
thread_local! { static LENT: Cell<*const Loan> = const { Cell::new(std::ptr::null()) }; }
struct Loan {
    group: usize,
    lending: Arc<AtomicUsize>,
    context: Cell<*mut StoreContextMut<'static, State>>,
    previous: *const Loan,
}
impl Drop for Loan {
    fn drop(&mut self) {
        self.lending.fetch_sub(1, Ordering::Relaxed);
        LENT.set(self.previous);
    }
}
struct Resumed {
    loan: *const Loan,
    context: *mut StoreContextMut<'static, State>,
}
impl Resumed {
    fn take(group: usize) -> Option<Self> {
        let mut ptr = LENT.get();
        while !ptr.is_null() {
            // SAFETY: nodes live on synchronous import stacks on this thread.
            let loan = unsafe { &*ptr };
            if loan.group == group {
                let context = loan.context.replace(std::ptr::null_mut());
                return (!context.is_null()).then_some(Self { loan: ptr, context });
            }
            ptr = loan.previous;
        }
        None
    }
    fn context(&mut self) -> StoreContextMut<'_, State> {
        // SAFETY: this loan removed the context exclusively until Drop restores it.
        unsafe { &mut *self.context }.as_context_mut()
    }
}
impl Drop for Resumed {
    fn drop(&mut self) {
        unsafe { &*self.loan }.context.set(self.context);
    }
}

fn host(
    mut caller: Caller<'_, State>,
    id: u32,
    ptr: u32,
    len: u32,
    out: u32,
    cap: u32,
) -> wasmtime::Result<u32> {
    let memory = caller
        .data()
        .memory
        .ok_or_else(|| wasmtime::Error::msg("host import during instantiation"))?;
    let end = (ptr as usize)
        .checked_add(len as usize)
        .ok_or_else(|| wasmtime::Error::msg("input range overflow"))?;
    let depth = caller.data().depth;
    if depth == 0 {
        return Err(wasmtime::Error::msg("host import outside a call"));
    }
    let mut input = std::mem::take(&mut caller.data_mut().imports[depth]);
    input.clear();
    input.extend_from_slice(
        memory
            .data(&caller)
            .get(ptr as usize..end)
            .ok_or_else(|| wasmtime::Error::msg("input outside memory"))?,
    );
    let mut reply = std::mem::take(&mut caller.data_mut().replies[depth]);
    reply.clear();
    let cx = &mut *caller.data_mut().cx as *mut Cx;
    // SAFETY: this import holds the locked/lent store; Cx stays boxed until the
    // synchronous native dispatch and any nested callbacks have returned.
    let resources = unsafe { crate::boundary::resource_scope(cx, false) };
    let mut context = caller.as_context_mut();
    let lending = context.data().lending.clone();
    lending.fetch_add(1, Ordering::Relaxed);
    let loan = Loan {
        lending,
        group: context.data().group,
        context: Cell::new((&mut context as *mut StoreContextMut<'_, State>).cast()),
        previous: LENT.get(),
    };
    LENT.set(&loan);
    let result = if id == mod_api::wire::HOST_RELEASE {
        let mut reader = Reader::new(&input);
        let id = reader.take::<Id>()?;
        if !reader.rest().is_empty() {
            return Err(WireError::Trailing.into());
        }
        unsafe { &mut *cx }
            .held
            .remove(id)
            .map(|_| ())
            .map_err(|e| wasmtime::Error::msg(format!("{e:?}")))
    } else if id == mod_api::wire::HOST_LOG {
        let mut reader = Reader::new(&input);
        let message: String = reader.take()?;
        if !reader.rest().is_empty() {
            return Err(WireError::Trailing.into());
        }
        // Release the Cx borrow before entering a possibly reentrant logger.
        let module = unsafe { (&*cx).module.as_ref().map(|module| module.id.clone()) }
            .ok_or_else(|| wasmtime::Error::msg("no calling module"))?;
        log::error!("wasm mod {module}: {message}");
        Ok(())
    } else {
        // SAFETY: Cx is boxed and stable; serve must release its frame borrows before
        // invoking native code that can synchronously reenter the guest.
        unsafe { serve(id, cx, &input, &mut reply) }
            .map_err(|e| wasmtime::Error::msg(format!("{e:?}")))
    };
    drop(loan);
    drop(resources);
    caller.data_mut().imports[depth] = input;
    result?;
    if let Some(error) = reply.error() {
        return Err(error.into());
    }
    let size = u32::try_from(reply.bytes().len())?;
    if size != 0 && size <= cap {
        memory.write(&mut caller, out as usize, reply.bytes())?;
    }
    caller.data_mut().replies[depth] = reply;
    Ok(size)
}
fn host_reply(mut caller: Caller<'_, State>, ptr: u32) -> wasmtime::Result<()> {
    let state = caller.data();
    let memory = state
        .memory
        .ok_or_else(|| wasmtime::Error::msg("no group memory"))?;
    let depth = state.depth;
    if depth == 0 {
        return Err(wasmtime::Error::msg("host reply outside a call"));
    }
    let reply = std::mem::take(&mut caller.data_mut().replies[depth]);
    let result = memory.write(&mut caller, ptr as usize, reply.bytes());
    caller.data_mut().replies[depth] = reply;
    result?;
    Ok(())
}

impl WasmMod {
    pub(crate) fn downgrade(&self) -> WeakWasmMod {
        WeakWasmMod {
            id: self.id.clone(),
            running: Arc::downgrade(&self.running),
            input: self.input.clone(),
            hook: self.hook.clone(),
            subscribed: self.subscribed,
            listeners: self.listeners,
            released: self.released.clone(),
            subscriptions: self.subscriptions.clone(),
        }
    }
    pub(crate) fn mod_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn is_stopped(&self) -> bool {
        self.running.poisoned.load(Ordering::Relaxed)
    }
    pub(crate) fn disable(&self) {
        // Publish stop before destroying native resources. Never wait on a store
        // loaned to native code: the outermost FrameScope clears it on return.
        self.running.poisoned.store(true, Ordering::Relaxed);
        if let Some(mut resumed) = Resumed::take(Arc::as_ptr(&self.running) as usize) {
            let mut store = resumed.context();
            if store.data().depth == 0 {
                drop(std::mem::take(&mut store.data_mut().cx.held));
            }
        } else if let Ok(mut group) = self.running.group.try_lock() {
            drop(std::mem::take(&mut group.store.data_mut().cx.held));
        }
    }
    pub(crate) fn instantiate(
        engine: &Engine,
        manifests: &[(&str, &horizon_mod_package::Manifest)],
        members: &[Member<'_>],
        cache: Option<&crate::module_cache::ModuleCache>,
    ) -> Result<Vec<Self>, Refusal> {
        let mut store = Store::new(engine, State::default());
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(30_000);
        let mut armed = false;
        let mut group = Group::link_cached_with(store, members, cache, |store, module, name| {
            // Compilation is not guest execution and must not consume its load deadline.
            if !armed {
                store.set_epoch_deadline(30_000);
                armed = true;
            }
            if module != "horizon" {
                return None;
            }
            match name {
                "host" => Some(Extern::Func(Func::wrap(store, host))),
                "host_reply" => Some(Extern::Func(Func::wrap(store, host_reply))),
                _ => None,
            }
        })
        .map_err(|e| Refusal::Runtime(e.to_string()))?;
        group.store.data_mut().memory = Some(group.memory);
        let mut exports = Vec::new();
        for (name, instance) in group.members.clone() {
            // The std member is a library, not a mod.
            if instance
                .get_func(&mut group.store, "horizon_layout")
                .is_none()
            {
                continue;
            }
            let load = |e: wasmtime::Error| Refusal::Runtime(format!("{e:#}"));
            let layout = instance
                .get_typed_func::<(), u64>(&mut group.store, "horizon_layout")
                .map_err(load)?
                .call(&mut group.store, ())
                .map_err(load)?;
            if layout != *mod_api::wire::LAYOUT {
                return Err(Refusal::Layout {
                    built: layout,
                    game: *mod_api::wire::LAYOUT,
                });
            }
            if instance
                .get_func(&mut group.store, "horizon_layout_ext")
                .is_some()
            {
                let built = instance
                    .get_typed_func::<(), u64>(&mut group.store, "horizon_layout_ext")
                    .map_err(load)?
                    .call(&mut group.store, ())
                    .map_err(load)?;
                if built != mod_points::POINT_LAYOUT {
                    return Err(Refusal::PointLayout {
                        built,
                        game: mod_points::POINT_LAYOUT,
                    });
                }
            }
            let input = instance
                .get_typed_func(&mut group.store, "horizon_in")
                .map_err(load)?;
            let hook = instance
                .get_typed_func(&mut group.store, "horizon_hook")
                .map_err(load)?;
            exports.push((name, input, hook));
        }
        // Register through the library's exported bridge, then seal every member's private
        // registry. No hook or mod initialization runs until the entire group is ready.
        for (name, instance) in group.members.clone() {
            if instance
                .get_func(&mut group.store, "horizon_mod_points_register")
                .is_some()
            {
                let refused = instance
                    .get_typed_func::<(), u32>(&mut group.store, "horizon_mod_points_register")
                    .and_then(|function| function.call(&mut group.store, ()))
                    .map_err(|e| Refusal::Runtime(format!("mod point registration: {e:#}")))?;
                if refused != 0 {
                    let injection = manifests
                        .iter()
                        .find(|(member, _)| *member == name)
                        .and_then(|(_, manifest)| {
                            manifest
                                .inject
                                .iter()
                                .filter(|injection| {
                                    manifest.mod_point_dependency(&injection.at).is_some()
                                })
                                .nth((refused - 1) as usize)
                                .map(|injection| format!("{}:{}", manifest.id, injection.id))
                        })
                        .ok_or_else(|| {
                            Refusal::Runtime(format!(
                                "{name}: invalid mod point registration refusal index {refused}"
                            ))
                        })?;
                    return Err(Refusal::Runtime(format!(
                        "{injection}: refused by library registry"
                    )));
                }
            }
        }
        // A member whose manifest listens at a library's points was built with the bridge that
        // registers them, and the library with the seal: a module missing either is refused, not
        // left silently unregistered.
        for (name, manifest) in manifests {
            for injection in &manifest.inject {
                let Some(library) = manifest.mod_point_dependency(&injection.at) else {
                    continue;
                };
                let mut exports = |member: &str, export: &str| {
                    let instance = group.members.iter().find(|(m, _)| m == member);
                    instance
                        .map(|(_, instance)| *instance)
                        .is_some_and(|instance| {
                            instance.get_func(&mut group.store, export).is_some()
                        })
                };
                let id = format!("{}:{}", manifest.id, injection.id);
                if !exports(name, "horizon_mod_points_register") {
                    return Err(Refusal::Runtime(format!(
                        "{id}: {name} has no mod point registration; rebuild it with `cargo \
                         horizon-mod build --wasm`"
                    )));
                }
                let sealed = manifests
                    .iter()
                    .find(|(_, other)| other.id == library)
                    .is_some_and(|(member, _)| exports(member, "horizon_mod_points_seal"));
                if !sealed {
                    return Err(Refusal::Runtime(format!(
                        "{id}: library {library} has no mod point seal in this group; rebuild it \
                         with `cargo horizon-mod build --wasm`"
                    )));
                }
            }
        }
        for (_, instance) in group.members.clone() {
            if instance
                .get_func(&mut group.store, "horizon_mod_points_seal")
                .is_some()
            {
                instance
                    .get_typed_func::<(), ()>(&mut group.store, "horizon_mod_points_seal")
                    .and_then(|function| function.call(&mut group.store, ()))
                    .map_err(|e| Refusal::Runtime(format!("mod point seal: {e:#}")))?;
            }
        }
        let lending = group.store.data().lending.clone();
        let running = Arc::new(Running {
            lending,
            group: Mutex::new(group),
            poisoned: AtomicBool::new(false),
        });
        running
            .group
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .store
            .data_mut()
            .group = Arc::as_ptr(&running) as usize;
        Ok(exports
            .into_iter()
            .map(|(name, input, hook)| Self {
                id: name.into(),
                running: running.clone(),
                input,
                hook,
                subscribed: &[],
                listeners: &[],
                released: Arc::default(),
                subscriptions: Arc::default(),
            })
            .collect())
    }
    /// Refusals answer unavailable without stopping the group; faults stop all members.
    pub(crate) fn call<R>(
        &self,
        class: CallClass,
        frame: Frame,
        id: u32,
        handler: Option<Id>,
        args: impl FnOnce(&mut Writer),
        reply: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        // Dispatch-owned native receivers can unwind before our outer call
        // guards. Their destructors must not reenter the still-lent guest.
        if std::thread::panicking() {
            self.disable();
            return None;
        }
        if self.running.poisoned.load(Ordering::Relaxed) {
            return None;
        }
        let key = Arc::as_ptr(&self.running) as usize;
        // A synchronous import can call any provider/handler, not just a named
        // callback trait. Such reentry shares the enclosing call's remaining budget.
        let mut resumed = Resumed::take(key);
        let mut locked = None;
        if resumed.is_none() {
            // Start the busy timeout on contention, not on the uncontended hot path.
            let mut start = None;
            loop {
                match self.running.group.try_lock() {
                    Ok(group) => {
                        locked = Some(group);
                        break;
                    }
                    Err(std::sync::TryLockError::Poisoned(e)) => {
                        locked = Some(e.into_inner());
                        break;
                    }
                    Err(_)
                        if class != CallClass::Worker
                            && (self.running.lending.load(Ordering::Relaxed) != 0
                                || start.get_or_insert_with(Instant::now).elapsed()
                                    >= Duration::from_secs(5)) =>
                    {
                        log::warn!("wasm mod {}: {}", self.id, Refusal::Busy);
                        return None;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }
        let mut store = match resumed.as_mut() {
            Some(resumed) => resumed.context(),
            None => {
                let store = &mut locked.as_mut()?.store;
                let budget = if class == CallClass::Load {
                    30_000
                } else {
                    100
                };
                store.set_epoch_deadline(budget);
                store.data_mut().epoch_budget = budget;
                store.as_context_mut()
            }
        };
        if self.running.poisoned.load(Ordering::Relaxed) {
            if store.data().depth == 0 {
                drop(std::mem::take(&mut store.data_mut().cx.held));
            }
            return None;
        }
        let budget = store.data().epoch_budget;
        let depth = store.data().depth + 1;
        if depth > 64 {
            let path = format!("{:?} -> ({}, {id}), limit 64", store.data().path, self.id);
            if store.data_mut().refusals.insert(path.clone()) {
                log::warn!("wasm depth refusal: {path}");
            }
            return None;
        }
        store.data_mut().depth = depth;
        if store.data().replies.len() <= depth {
            store.data_mut().replies.resize_with(depth + 1, Writer::new);
            store.data_mut().inputs.resize_with(depth + 1, Writer::new);
            store.data_mut().outputs.resize_with(depth + 1, Vec::new);
            store.data_mut().imports.resize_with(depth + 1, Vec::new);
        }
        store.data_mut().cx.frames.push(frame);
        store.data_mut().path.push((self.id.clone(), id));
        // self remains borrowed until this call returns. FrameScope restores the
        // enclosing module before this borrow ends, including on error or unwind.
        let previous_module =
            std::mem::replace(&mut store.data_mut().cx.module, self as *const Self);
        let scope = FrameScope {
            previous_module,
            state: store.data_mut(),
            running: self.running.clone(),
        };
        // SAFETY: FrameScope keeps the active module installed and the group
        // remains exclusively locked/lent until this resource scope is dropped.
        let resources = unsafe { crate::boundary::resource_scope(&mut *store.data_mut().cx, true) };
        let mut w = std::mem::take(&mut store.data_mut().inputs[depth]);
        let mut output = std::mem::take(&mut store.data_mut().outputs[depth]);
        // This guard unwinds before the resource scope, whose native destructors
        // must already see the group stopped.
        let panic_stop = PoisonOnPanic(&self.running.poisoned);
        let result = (|| -> Result<R, CallError> {
            let released = self.released.take();
            if !released.is_empty() {
                w.clear();
                w.put(&released);
                self.invoke(&mut store, mod_api::wire::RELEASE, &w, &mut output)?;
                if !output.is_empty() {
                    return Err(WireError::Trailing.into());
                }
            }
            w.clear();
            if let Some(handler) = handler {
                w.put(&handler);
            }
            args(&mut w);
            self.invoke(&mut store, id, &w, &mut output)?;
            let mut r = Reader::new(&output);
            let value = reply(&mut r)?;
            if !r.rest().is_empty() {
                return Err(WireError::Trailing.into());
            }
            Ok(value)
        })();
        store.data_mut().inputs[depth] = w;
        store.data_mut().outputs[depth] = output;
        if result.is_err() {
            horizon_mod_runtime::dispatch_failures::record_dispatch_failure();
        }
        let first_failure = matches!(result, Err(CallError::Fault(_)))
            && !self.running.poisoned.swap(true, Ordering::Relaxed);
        if let Err(CallError::Refused(error)) = &result {
            let reason = format!("{} hook {id}: {error:#}", self.id);
            if store
                .data_mut()
                .refusals
                .insert(format!("{} hook {id}", self.id))
            {
                log::warn!("wasm refusal: {reason}");
            }
        }
        drop(panic_stop);
        drop(resources);
        drop(scope);
        match result {
            Ok(value) => Some(value),
            Err(CallError::Refused(_)) => None,
            Err(CallError::Fault(e)) => {
                if first_failure {
                    log::error!(
                        "wasm mod {} stopped its group: {class:?}, depth {depth}, active epoch budget {budget}: {e:#}",
                        self.id
                    );
                }
                None
            }
        }
    }
    fn invoke(
        &self,
        store: &mut StoreContextMut<'_, State>,
        id: u32,
        input: &Writer,
        output: &mut Vec<u8>,
    ) -> Result<(), CallError> {
        if let Some(error) = input.error() {
            return Err(error.into());
        }
        let len = u32::try_from(input.bytes().len()).map_err(|e| CallError::Refused(e.into()))?;
        let ptr = self
            .input
            .call(&mut *store, len)
            .map_err(CallError::Fault)?;
        let memory = store
            .data()
            .memory
            .ok_or_else(|| CallError::Fault(wasmtime::Error::msg("no memory")))?;
        memory
            .write(&mut *store, ptr as usize, input.bytes())
            .map_err(|e| CallError::Fault(e.into()))?;
        let result = self.hook.call(&mut *store, id).map_err(CallError::Fault)?;
        let ptr = (result >> 32) as usize;
        let len = result as u32;
        if len == u32::MAX {
            let bytes = memory.data(&*store).get(ptr..).ok_or_else(|| {
                CallError::Refused(wasmtime::Error::msg("error reply outside memory"))
            })?;
            let message: String = Reader::new(bytes).take()?;
            return Err(CallError::Refused(wasmtime::Error::msg(message)));
        }
        let end = ptr
            .checked_add(len as usize)
            .ok_or_else(|| CallError::Refused(wasmtime::Error::msg("reply overflow")))?;
        output.clear();
        output.extend_from_slice(
            memory
                .data(&*store)
                .get(ptr..end)
                .ok_or_else(|| CallError::Refused(wasmtime::Error::msg("reply outside memory")))?,
        );
        Ok(())
    }
}
