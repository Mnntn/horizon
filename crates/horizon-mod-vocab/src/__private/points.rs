//! Slot tables for injection points: the game's (in `mod_points`) and each mod crate's own (#2339),
//! one registry for all of them. A table is its slots, one `AtomicBool` per place, set once at the
//! seal where a mod listens. Registering a listener records its table, so the one [`seal`] after
//! the mods load sets every table that has listeners; a table nobody listens at is never touched,
//! and each of its points costs one load of a slot that stays `false`.
//!
//! Sealing orders each place's listeners: API ones before internals ones, a `replace` last among
//! the API ones, then priority descending, then registration order (the mods register in
//! dependency order), then inject id within a mod. Call sites keep mod-id tie ordering.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// Runs a point's listened path out of line: the code that builds its record and calls its
/// listeners stays out of the function, which nothing listening leaves small enough to inline.
#[cold]
#[inline(never)]
pub fn cold<R>(path: impl FnOnce() -> R) -> R {
    path()
}

/// One injection point table: its point slots, then its call-site slots.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct Table {
    points: &'static [AtomicBool],
    sites: &'static [AtomicBool],
    sealed: OnceLock<Sealed>,
}

impl Table {
    /// A table over `points` and `sites` (a mod crate's table has no sites).
    pub const fn new(points: &'static [AtomicBool], sites: &'static [AtomicBool]) -> Self {
        Self {
            points,
            sites,
            sealed: OnceLock::new(),
        }
    }

    /// Whether an active `replace` listener owns point `place`: the seal puts it last among the
    /// place's API listeners. A stopped or disabled owner no longer replaces, and the function
    /// runs as written.
    pub fn replaced(&self, place: usize) -> bool {
        let Some(sealed) = self.sealed.get() else {
            return false;
        };
        let slot = &sealed.slots[place];
        slot.entries[..slot.api_end].last().is_some_and(|entry| {
            entry.replace && sealed.listeners[entry.listener as usize].active()
        })
    }

    /// Runs the API functions and forwarding endpoints at point `place`, each under its owner's
    /// guard.
    #[cold]
    #[inline(never)]
    pub fn fire_api(&self, place: usize, call: &mut dyn FnMut(ApiKind)) {
        self.fire(place, true, &mut |kind| {
            if let Kind::Api(api) = kind {
                call(api);
            }
        });
    }

    /// Runs the internals listeners at point `place`, each through `call` with its kind and
    /// function. The slot casts a typed function back to the place's listener type, or calls an
    /// erased one with the place index and its receiver and raw record or return value pointers.
    #[cold]
    #[inline(never)]
    pub fn fire_raw(&self, place: usize, call: &mut dyn FnMut(RawKind)) {
        self.fire(place, false, &mut |kind| {
            if let Kind::Raw(raw) = kind {
                call(raw);
            }
        });
    }

    fn fire(&self, place: usize, api: bool, call: &mut dyn FnMut(Kind)) {
        let Some(sealed) = self.sealed.get() else {
            return;
        };
        let slot = &sealed.slots[place];
        let entries = if api {
            &slot.entries[..slot.api_end]
        } else {
            &slot.entries[slot.api_end..]
        };
        for entry in entries {
            let listener = &sealed.listeners[entry.listener as usize];
            if listener.active() {
                let _guard = super::enter(listener.id, entry.injection);
                call(entry.kind);
            }
        }
    }

    /// Invokes the next enabled listener of call site `site` under its mod guard. The generated
    /// typed slot recurses from the returned index, so the first listener is outermost, without
    /// allocating a chain.
    pub fn wrap<O>(
        &self,
        site: usize,
        from: usize,
        call: impl FnOnce(Option<(*const (), usize)>) -> O,
    ) -> O {
        if let Some(sealed) = self.sealed.get() {
            for (index, raw) in sealed.slots[self.points.len() + site]
                .entries
                .iter()
                .enumerate()
                .skip(from)
            {
                let listener = &sealed.listeners[raw.listener as usize];
                if !listener.active() {
                    continue;
                }
                let Kind::Raw(RawKind::Typed(function)) = raw.kind else {
                    unreachable!("only typed listeners register at sites")
                };
                let _guard = super::enter(listener.id, raw.injection);
                return call(Some((function, index + 1)));
            }
        }
        call(None)
    }

    /// Sets the table's slots from its registrations: each place's listeners in order.
    fn seal(&self, mut listeners: Vec<Owner>, slots: Vec<(usize, Slot)>, stopped: &[String]) {
        for listener in &mut listeners {
            listener.mod_index = super::mod_index(listener.id);
            if stopped.iter().any(|id| id == listener.id) {
                listener.stopped.store(true, Ordering::Release);
            }
        }
        let points = self.points.len();
        let mut lists = vec![Vec::new(); points + self.sites.len()];
        for (place, slot) in slots {
            lists[place].push(slot);
        }
        for (place, list) in lists.iter_mut().enumerate() {
            if place < points {
                // A place's one `replace` listener runs after its other API listeners, so its
                // outcome is the one the function takes ([`Table::replaced`] reads it there).
                list.sort_by_key(|entry| {
                    (
                        std::cmp::Reverse(entry.kind.is_api()),
                        entry.replace,
                        std::cmp::Reverse(entry.priority),
                    )
                });
                for group in list.chunk_by_mut(|a, b| {
                    a.kind.is_api() == b.kind.is_api()
                        && a.replace == b.replace
                        && a.priority == b.priority
                        && a.listener == b.listener
                }) {
                    group.sort_by_key(|entry| entry.injection);
                }
                continue;
            }
            list.sort_by(|a, b| {
                b.priority
                    .cmp(&a.priority)
                    .then_with(|| {
                        listeners[a.listener as usize]
                            .id
                            .cmp(listeners[b.listener as usize].id)
                    })
                    .then_with(|| a.injection.cmp(b.injection))
            });
        }
        let sealed = self.sealed.get_or_init(|| Sealed {
            listeners: listeners.into(),
            slots: lists
                .into_iter()
                .map(|entries| SlotList {
                    api_end: entries.partition_point(|entry| entry.kind.is_api()),
                    entries: entries.into_boxed_slice(),
                })
                .collect(),
        });
        for (slot, list) in self.points.iter().chain(self.sites).zip(&sealed.slots) {
            slot.store(!list.entries.is_empty(), Ordering::Release);
        }
    }
}

/// A place in a table: a point's head or return, or a call site after the table's points.
#[derive(Clone, Copy)]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct Place {
    table: &'static Table,
    index: usize,
}

impl Place {
    /// Place `index` of `table`; a call site's index counts after the table's points.
    pub const fn new(table: &'static Table, index: usize) -> Self {
        Self { table, index }
    }

    /// The table the place is in.
    pub fn table(self) -> &'static Table {
        self.table
    }

    /// Its index in its table.
    pub fn index(self) -> usize {
        self.index
    }
}

struct Owner {
    id: &'static str,
    /// Resolved at the seal, so firing reads the owner's canonical enable flag without a scan.
    mod_index: Option<usize>,
    /// Set when the mod stopped (a trap in its wasm group, or the game giving up on it): its
    /// listeners are skipped from then on.
    stopped: AtomicBool,
}

impl Owner {
    fn active(&self) -> bool {
        !self.stopped.load(Ordering::Acquire) && self.mod_index.is_none_or(super::mod_index_on)
    }
}

/// How a slot calls an internals listener.
#[derive(Clone, Copy)]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub enum RawKind {
    /// A function of the place's listener type, cast back by the slot before calling it.
    Typed(*const ()),
    /// A runtime listener receiving the place index, receiver and raw record or return value.
    ///
    /// # Safety
    ///
    /// The caller must pass the registered place's index and valid pointers of that place's exact
    /// receiver and raw record or return types, respecting their mutability, aliasing, ownership
    /// and lifetime requirements for the call's duration. The receiver is null for a free or
    /// associated function, and at a borrowed-return place. A shared receiver must not be written
    /// to; its mutable pointer only erases the type, not its mutability.
    Erased(unsafe fn(usize, *mut (), *mut ())),
}

// Function pointers: no data behind them, so they move between threads.
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Send for RawKind {}
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Sync for RawKind {}

/// The host available to a point's hook call; frame objects never cross as data.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub enum PointHost<'a> {
    /// A read-only point.
    Read(&'a dyn crate::Host),
    /// A server write point.
    Server(&'a mut dyn crate::ServerHost),
    /// A client write point.
    Client(&'a mut dyn crate::ClientHost),
    /// A point which runs on either side.
    Side(crate::HostSide<'a>),
    /// A free function without a host.
    None,
}

/// A checked hook endpoint. The runtime calls the ordinary hook export under its limits.
pub trait Forward: Send + Sync {
    /// Dispatches one generated listener id with its mutable wire record.
    fn forward(&self, id: u32, host: PointHost<'_>, record: &mut dyn crate::wire::Record);
}

/// A typed API function.
#[derive(Clone, Copy)]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub enum ApiKind {
    /// A function checked by the generated typed constructor before erasure.
    Typed(*const ()),
    /// A runtime endpoint in the same ordered list as native functions.
    Forwarded(&'static dyn Forward, u32),
}

// The pointer is a function; forwarding endpoints are Send + Sync resources.
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Send for ApiKind {}
#[allow(
    unsafe_code,
    reason = "Carried point-registry function-pointer erasure; ownership cleanup is tracked in #2587"
)]
unsafe impl Sync for ApiKind {}

#[derive(Clone, Copy)]
enum Kind {
    Api(ApiKind),
    Raw(RawKind),
}

impl Kind {
    fn is_api(self) -> bool {
        matches!(self, Self::Api(_))
    }
}

/// A listener in its place's list.
#[derive(Clone, Copy)]
struct Slot {
    listener: u32,
    kind: Kind,
    priority: i32,
    injection: &'static str,
    replace: bool,
}

struct SlotList {
    entries: Box<[Slot]>,
    api_end: usize,
}

struct Sealed {
    listeners: Box<[Owner]>,
    slots: Box<[SlotList]>,
}

/// One table's registrations while mods load.
struct OpenTable {
    table: &'static Table,
    listeners: Vec<Owner>,
    slots: Vec<(usize, Slot)>,
}

/// Every table's registrations while mods load, and stops that came before the seal.
struct Open {
    tables: Vec<OpenTable>,
    stopped: Vec<String>,
    /// Set once [`seal`] has run: registering is refused from then on, and stops go to the
    /// sealed tables in `tables`.
    sealed: bool,
}

/// Registering, stopping and sealing all hold this lock, so a registration either lands in the
/// sealed lists or is refused, and a stop before the seal still holds after it.
static OPEN: Mutex<Open> = Mutex::new(Open {
    tables: Vec::new(),
    stopped: Vec::new(),
    sealed: false,
});

fn lock() -> std::sync::MutexGuard<'static, Open> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Why a listener wasn't registered.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
pub enum RegisterError {
    /// The list was sealed: listeners register while the game loads, before its points are set.
    Sealed,
    /// The guest table requires a typed API listener.
    Unsupported,
    /// A second `replace` listener at a place that already has one: two overwrites can't both win,
    /// as with a mixin overwrite conflict. Names the mod and injection of each.
    Replaced {
        /// The mod and injection id already replacing the function.
        first: (&'static str, &'static str),
        /// The mod and injection id refused.
        second: (&'static str, &'static str),
    },
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sealed => f.write_str("the game's points were already set"),
            Self::Unsupported => f.write_str("the guest table requires a typed API listener"),
            Self::Replaced { first, second } => write!(
                f,
                "{}:{} replaces a function that {}:{} already replaces; only one replace \
                 listener can own it",
                second.0, second.1, first.0, first.1
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

/// A function or checked endpoint at an API, internals or site slot.
// no wire: point registry machinery runs in its owning process; listeners cross through their records, never this.
// exhaustive: generated point code builds and matches it in full.
pub struct SlotListener {
    id: &'static str,
    place: Place,
    kind: Kind,
    priority: i32,
    injection: &'static str,
    /// A `replace` listener: the function is its, so a second one at the place is refused.
    replace: bool,
}

impl SlotListener {
    /// Mod `id`'s internals listener `function` at `place`.
    ///
    /// # Safety
    ///
    /// `function` is a function of `place`'s raw listener type: `__<Point>Listener` at a head,
    /// `__<Point>ReturnListener` at a return. The place's slot casts it back to that type and calls
    /// it. The generated constructors check the type with a typed parameter before erasing it.
    #[allow(
        unsafe_code,
        reason = "Carried typed point registration contract; ownership cleanup is tracked in #2587"
    )]
    pub const unsafe fn new(id: &'static str, place: Place, function: *const ()) -> Self {
        Self::of(id, place, Kind::Raw(RawKind::Typed(function)))
    }

    /// Mod `id`'s erased runtime listener `function` at `place`.
    ///
    /// The callback gets the place's index, the receiver and the raw record at a head or the
    /// return value's place at a return (including `()` for a function with no explicit return
    /// type). The receiver is null for a free or associated function. At a borrowed-return place
    /// it is also null: only the value's place is passed, since the value may borrow the receiver.
    /// A shared receiver is passed as a mutable pointer only to erase its type, not its mutability.
    ///
    /// # Safety
    ///
    /// Given the pointers guaranteed by [`RawKind::Erased`], `function` must interpret them using
    /// `place`'s exact receiver and raw record or return types, respecting their mutability,
    /// aliasing, ownership and lifetime requirements. The pointers and any references derived
    /// from them must not escape the callback. In particular, a shared receiver must not be
    /// written to.
    #[allow(
        unsafe_code,
        reason = "Carried typed point registration contract; ownership cleanup is tracked in #2587"
    )]
    pub const unsafe fn erased(
        id: &'static str,
        place: Place,
        function: unsafe fn(usize, *mut (), *mut ()),
    ) -> Self {
        Self::of(id, place, Kind::Raw(RawKind::Erased(function)))
    }

    /// A typed call-site wrap listener, sorted by priority, mod id and injection id.
    ///
    /// # Safety
    ///
    /// `function` must have this site's exact generated listener type. Erased point callbacks
    /// cannot register as site listeners: their signature has no original continuation.
    #[allow(
        unsafe_code,
        reason = "Carried typed point registration contract; ownership cleanup is tracked in #2587"
    )]
    pub const unsafe fn site(
        id: &'static str,
        place: Place,
        priority: i32,
        injection: &'static str,
        function: *const (),
    ) -> Self {
        Self::of(id, place, Kind::Raw(RawKind::Typed(function))).ordered(priority, injection)
    }

    /// An API listener whose function has the place's exact generated function type.
    ///
    /// # Safety
    /// The generated constructor must check the function's host and record types before erasure.
    #[allow(
        unsafe_code,
        reason = "Carried typed point registration contract; ownership cleanup is tracked in #2587"
    )]
    pub const unsafe fn api(id: &'static str, place: Place, function: *const ()) -> Self {
        Self::of(id, place, Kind::Api(ApiKind::Typed(function)))
    }

    /// A checked runtime endpoint at an API place, dispatched through the hook machinery.
    pub const fn forwarded(
        id: &'static str,
        place: Place,
        endpoint: &'static dyn Forward,
        export: u32,
    ) -> Self {
        Self::of(id, place, Kind::Api(ApiKind::Forwarded(endpoint, export)))
    }

    const fn of(id: &'static str, place: Place, kind: Kind) -> Self {
        Self {
            id,
            place,
            kind,
            priority: 0,
            injection: "",
            replace: false,
        }
    }

    /// Orders a point by priority, then dependency order, then inject id within its mod.
    pub const fn ordered(mut self, priority: i32, injection: &'static str) -> Self {
        self.priority = priority;
        self.injection = injection;
        self
    }

    /// Marks a `replace` listener: registering a second one at the same place is refused with
    /// [`RegisterError::Replaced`].
    pub const fn replacing(mut self) -> Self {
        self.replace = true;
        self
    }

    /// The function, when it is a typed API listener.
    pub fn api_kind(&self) -> Option<ApiKind> {
        match self.kind {
            Kind::Api(kind) => Some(kind),
            Kind::Raw(_) => None,
        }
    }

    /// Where it listens.
    pub fn place(&self) -> Place {
        self.place
    }

    /// Its inject id.
    pub fn injection(&self) -> &'static str {
        self.injection
    }

    /// The mod's id.
    pub fn id(&self) -> &'static str {
        self.id
    }
}

/// Registers a function or checked endpoint in its place's table, in the same order for native
/// and wasm owners.
pub fn register(slot: &SlotListener) -> Result<(), RegisterError> {
    let mut open = lock();
    if open.sealed {
        return Err(RegisterError::Sealed);
    }
    let table = match open
        .tables
        .iter()
        .position(|open| std::ptr::eq(open.table, slot.place.table))
    {
        Some(index) => &mut open.tables[index],
        None => {
            open.tables.push(OpenTable {
                table: slot.place.table,
                listeners: Vec::new(),
                slots: Vec::new(),
            });
            open.tables.last_mut().expect("just pushed")
        }
    };
    let place = slot.place.index;
    if slot.replace
        && let Some((_, first)) = table
            .slots
            .iter()
            .find(|(at, other)| *at == place && other.replace)
    {
        return Err(RegisterError::Replaced {
            first: (table.listeners[first.listener as usize].id, first.injection),
            second: (slot.id, slot.injection),
        });
    }
    let listener = match table.listeners.iter().position(|owner| owner.id == slot.id) {
        Some(index) => index,
        None => {
            table.listeners.push(Owner {
                id: slot.id,
                mod_index: None,
                stopped: AtomicBool::new(false),
            });
            table.listeners.len() - 1
        }
    } as u32;
    table.slots.push((
        place,
        Slot {
            listener,
            kind: slot.kind,
            priority: slot.priority,
            injection: slot.injection,
            replace: slot.replace,
        },
    ));
    Ok(())
}

/// Stops mod `id`'s listeners in every table: none of them runs again, whether the tables are
/// sealed yet or not. A trap that stops a wasm group and the game giving up on a native mod both
/// end here.
pub fn stop(id: &str) {
    let mut open = lock();
    if !open.sealed {
        open.stopped.push(id.to_owned());
        return;
    }
    for table in &open.tables {
        let Some(sealed) = table.table.sealed.get() else {
            continue;
        };
        for listener in sealed.listeners.iter().filter(|listener| listener.id == id) {
            listener.stopped.store(true, Ordering::Release);
        }
    }
}

/// Seals every table, so no listener registers after it, and sets each slot a mod listens at. The
/// game calls it once, after the mods load.
pub fn seal() {
    let mut open = lock();
    if open.sealed {
        return;
    }
    open.sealed = true;
    let stopped = std::mem::take(&mut open.stopped);
    for table in &mut open.tables {
        let (listeners, slots) = (
            std::mem::take(&mut table.listeners),
            std::mem::take(&mut table.slots),
        );
        table.table.seal(listeners, slots, &stopped);
    }
}

/// Adapts a replacement listener to a point's ordinary typed listener without registry values.
// hidden: generated mod-point adapters share replacement semantics without exposing registry types.
#[doc(hidden)]
#[macro_export]
macro_rules! replacement_listener {
    (cancel, host, $listener:path) => {
        |host, args| {
            $listener(host, args);
            args.cancel = true;
        }
    };
    (cancel, none, $listener:path) => {
        |args| {
            $listener(args);
            args.cancel = true;
        }
    };
    (result, host, $listener:path) => {
        |host, args| {
            args.result = ::core::option::Option::Some($listener(host, args));
        }
    };
    (result, none, $listener:path) => {
        |args| {
            args.result = ::core::option::Option::Some($listener(args));
        }
    };
}
