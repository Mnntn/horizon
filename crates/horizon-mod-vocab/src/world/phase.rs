//! Restricted views for registration and data loading.
/// The owning boundary that starts a data load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DataLoadOrigin {
    /// Server data loading, including a reload.
    ServerDataLoad,
    /// Client resource preloading.
    ClientPreload,
}

/// The side that owns a resource-generation loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum LoopSide {
    /// A server loop.
    Server,
    /// A client loop.
    Client,
}
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

/// The permission boundary of the current owning thread. Only the game advances it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ModPhase {
    /// Package discovery, before declarations.
    Packages,
    /// Once-only declarations, without a Host.
    Registration,
    /// One side's startup, before its configs and world.
    Setup,
    /// Config loading: settings reads and scoped config files.
    Config,
    /// The owning game's normal runtime.
    World,
    /// Restricted data or asset loading, including published resource queries,
    /// returning to its entering phase.
    Reloading,
}

#[derive(Clone, Copy)]
struct PhaseState {
    phase: ModPhase,
    resume: Option<ModPhase>,
}

thread_local! {
    static PHASE: std::cell::Cell<PhaseState> = const { std::cell::Cell::new(PhaseState {
        phase: if cfg!(test) { ModPhase::World } else { ModPhase::Packages }, resume: None,
    }) };
}

/// A refused engine transition never changes the phase or dispatches a hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PhaseError {
    /// The token belongs to another thread.
    WrongThread,
    /// The requested edge is not part of the phase machine.
    Transition {
        /// Unchanged current phase.
        from: ModPhase,
        /// Refused destination.
        to: ModPhase,
    },
}
impl std::fmt::Display for PhaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mod phase transition refused: {self:?}")
    }
}
impl std::error::Error for PhaseError {}

// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// Engine-only authority for one thread; moving it does not transfer authority.
#[derive(Clone)]
#[non_exhaustive]
pub struct PhaseOwner {
    thread: std::thread::ThreadId,
}
impl PhaseOwner {
    /// Bind a new owning thread (or a loader) to its initial phase. Registration
    /// is process-owned; a new side starts at Setup after those tables are frozen.
    pub fn new(initial: ModPhase) -> Self {
        PHASE.set(PhaseState {
            phase: initial,
            resume: None,
        });
        Self {
            thread: std::thread::current().id(),
        }
    }
    /// A loader's temporary thread-local phase; the calling thread's state is restored.
    pub fn scope(initial: ModPhase) -> PhaseScope {
        let previous = PHASE.with(std::cell::Cell::get);
        PhaseScope {
            owner: Self::new(initial),
            previous,
            _thread: std::marker::PhantomData,
        }
    }
    /// Advance a legal edge on the bound thread, without dispatching anything.
    pub fn advance(&self, to: ModPhase) -> Result<(), PhaseError> {
        if self.thread != std::thread::current().id() {
            return Err(PhaseError::WrongThread);
        }
        PHASE.with(|cell| {
            let mut state = cell.get();
            let from = state.phase;
            match (from, to) {
                (ModPhase::Packages, ModPhase::Registration)
                | (ModPhase::Registration, ModPhase::Setup)
                | (ModPhase::Setup, ModPhase::Config)
                | (ModPhase::Config, ModPhase::World)
                | (ModPhase::World, ModPhase::Config) => {}
                (ModPhase::Setup | ModPhase::World, ModPhase::Reloading) => {
                    state.resume = Some(from);
                }
                (ModPhase::Reloading, _) if state.resume == Some(to) => {
                    state.resume = None;
                }
                _ => return Err(PhaseError::Transition { from, to }),
            }
            state.phase = to;
            cell.set(state);
            Ok(())
        })
    }
    /// Load configs before returning to normal runtime, including on failure.
    pub fn config(&self) -> Result<PhaseGuard<'_>, PhaseError> {
        self.advance(ModPhase::Config)?;
        Ok(PhaseGuard {
            owner: self,
            resume: ModPhase::World,
            _thread: std::marker::PhantomData,
        })
    }
    /// Nest a reload in Setup or World; errors and panics restore the caller's phase.
    pub fn reloading(&self) -> Result<PhaseGuard<'_>, PhaseError> {
        let resume = current_phase();
        self.advance(ModPhase::Reloading)?;
        Ok(PhaseGuard {
            owner: self,
            resume,
            _thread: std::marker::PhantomData,
        })
    }
}

// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// A temporary loader context, not authority transferred to a worker.
// exhaustive: engine-only constructor and private state
pub struct PhaseScope {
    owner: PhaseOwner,
    previous: PhaseState,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl std::ops::Deref for PhaseScope {
    type Target = PhaseOwner;
    fn deref(&self) -> &PhaseOwner {
        &self.owner
    }
}
impl Drop for PhaseScope {
    fn drop(&mut self) {
        PHASE.set(self.previous);
    }
}

// no wire: thread-bound phase authority or RAII restoration guard; never transferred as data.
/// Engine-only phase scope, restored during unwinding too.
// exhaustive: only the engine constructs it
pub struct PhaseGuard<'a> {
    owner: &'a PhaseOwner,
    resume: ModPhase,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        self.owner
            .advance(self.resume)
            .expect("scoped phase returns to its declared destination");
    }
}

/// The phase of this thread, used in refusal diagnostics.
pub fn current_phase() -> ModPhase {
    PHASE.with(|cell| cell.get().phase)
}

/// The catalog-owned access class of a generated game operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum LoadAccess {
    /// No extra load access beyond the phase's metadata/settings exceptions.
    WorldOnly,
    /// Registry and immutable metadata available in setup and reload.
    Metadata,
    /// The existing client setup reads, not general reload permissions.
    ClientSetup,
    /// Published asset/data facts, including resource-cache queries.
    PublishedResources,
}

/// The default body of a host method: a test double that does not implement it gets the access
/// check, then a panic naming the method.
pub(crate) fn unprovided(method: &'static str, operation: &'static str, access: LoadAccess) -> ! {
    check_access(operation, access);
    unimplemented!("{}", method)
}

/// One cell read at the generated API boundary, not a tick scan or registry lookup.
/// A refused mod call panics so a listener cannot ignore a Result and continue.
#[inline]
pub fn check_access(operation: &str, access: LoadAccess) {
    assert!(
        access_allowed(operation, access),
        "mod API {operation} is unavailable during {:?}",
        current_phase()
    );
}

/// Whether an operation is available in this thread's current load phase.
#[inline]
pub fn access_allowed(operation: &str, access: LoadAccess) -> bool {
    let phase = current_phase();
    if phase == ModPhase::World {
        return true;
    }
    let metadata = access == LoadAccess::Metadata;
    let settings = operation == "setting";
    match phase {
        ModPhase::World => true,
        ModPhase::Setup => {
            settings
                || metadata
                || matches!(
                    access,
                    LoadAccess::ClientSetup | LoadAccess::PublishedResources
                )
        }
        ModPhase::Config => settings || operation == "config_files",
        ModPhase::Reloading => settings || metadata || access == LoadAccess::PublishedResources,
        ModPhase::Packages | ModPhase::Registration => false,
    }
}

/// Whether a package contains compiled code or content alone.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageKind {
    /// The package carries compiled code.
    Code,
    /// The package carries content alone: data and assets.
    Content,
}

/// Read-only package metadata available during registration.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone)]
#[non_exhaustive]
pub struct PackageInfo {
    pub(crate) id: String,
    pub(crate) version: String,
    pub(crate) kind: PackageKind,
    pub(crate) side: crate::Side,
    /// Its own namespace, then the others it declares.
    pub(crate) namespaces: Vec<String>,
    /// The ids it declares dependencies on.
    pub(crate) dependencies: Vec<String>,
}

impl PackageInfo {
    /// An owned package snapshot supplied by the loader.
    pub fn new(
        id: String,
        version: String,
        kind: PackageKind,
        side: crate::Side,
        namespaces: Vec<String>,
        dependencies: Vec<String>,
    ) -> Self {
        Self {
            id,
            version,
            kind,
            side,
            namespaces,
            dependencies,
        }
    }
    /// Package identifier.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// The package's version.
    pub fn version(&self) -> &str {
        &self.version
    }
    /// Whether this package contains code.
    pub fn kind(&self) -> PackageKind {
        self.kind
    }
    /// The package's declared side.
    pub fn side(&self) -> crate::Side {
        self.side
    }
    /// The package's own namespace followed by any additional namespaces.
    pub fn namespaces(&self) -> impl Iterator<Item = &str> {
        self.namespaces.iter().map(String::as_str)
    }
    /// Whether this package declares a dependency on `id`.
    pub fn depends_on(&self, id: &str) -> bool {
        self.dependencies.iter().any(|d| d == id)
    }
}

static PACKAGES: OnceLock<Arc<[PackageInfo]>> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_PACKAGES: std::cell::RefCell<Option<Arc<[PackageInfo]>>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with only the packages `ids` installed, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_packages<R>(ids: &[&str], f: impl FnOnce() -> R) -> R {
    let packages: Vec<_> = ids
        .iter()
        .map(|id| {
            PackageInfo::new(
                (*id).to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                crate::Side::Client,
                vec![(*id).to_owned()],
                vec![],
            )
        })
        .collect();
    with_test_package_inventory(packages, f)
}

#[cfg(any(test, feature = "testing"))]
pub(crate) fn with_test_package_inventory<R>(
    packages: Vec<PackageInfo>,
    f: impl FnOnce() -> R,
) -> R {
    let previous = TEST_PACKAGES.replace(Some(packages.into()));
    let result = f();
    TEST_PACKAGES.set(previous);
    result
}

fn inventory(native: &[PackageInfo], content: &[PackageInfo]) -> Vec<PackageInfo> {
    let mut packages = native.to_vec();
    for selected in content {
        if !packages.iter().any(|p| p.id() == selected.id()) {
            packages.push(selected.clone());
        }
    }
    // The code inventory follows the one installed native/wasm dispatch slice;
    // content-only packages remain after it in their selected layer order.
    packages.sort_by_key(|p| {
        crate::mods()
            .iter()
            .position(|entry| entry.id == p.id)
            .unwrap_or(usize::MAX)
    });
    packages
}

/// Records the package inventory once at startup; a second call panics.
pub fn install_packages(native: &[PackageInfo], content: &[PackageInfo]) {
    assert!(
        PACKAGES.set(inventory(native, content).into()).is_ok(),
        "package inventory already installed"
    );
}

/// Install the host's ordered package inventory in a guest at initialization.
// hidden: the guest runtime installs this once before dispatching hooks.
#[doc(hidden)]
pub fn install_guest_packages(packages: Vec<PackageInfo>) {
    let _ = PACKAGES.set(packages.into());
}

// no wire: shared immutable loader inventory; contexts query it through LoadReader rather than transferring its Arc storage.
/// The registries and settings registration reads: captured by the engine once, before the
/// first registration hook runs, and the same for every registration step. Lookups are binary
/// searches over sorted lists, so a clone of the context copies nothing.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RegistrationData {
    instance_dir: crate::InstanceDirectory,
    /// Built-in registry names in ID order, by registry name.
    pub(crate) registries: Arc<[(String, Arc<[String]>)]>,
    /// Each loaded mod's settings, by mod ID then key.
    pub(crate) settings: Arc<[(String, Arc<[(String, String)]>)]>,
}

impl RegistrationData {
    /// The engine's snapshot: `registries` maps a registry name to its names in ID order,
    /// `settings` a mod ID to its settings.
    pub fn new(
        registries: BTreeMap<String, Vec<String>>,
        settings: BTreeMap<String, BTreeMap<String, String>>,
        instance_dir: crate::InstanceDirectory,
    ) -> Self {
        Self {
            instance_dir,
            registries: registries
                .into_iter()
                .map(|(name, ids)| (name, ids.into()))
                .collect(),
            settings: settings
                .into_iter()
                .map(|(id, values)| (id, values.into_iter().collect()))
                .collect(),
        }
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        let at = self
            .registries
            .binary_search_by(|(known, _)| known.as_str().cmp(name));
        Some(self.registries[at.ok()?].1.clone())
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        let mod_at = self
            .settings
            .binary_search_by(|(known, _)| known.as_str().cmp(mod_id))
            .ok()?;
        let values = &self.settings[mod_at].1;
        let at = values
            .binary_search_by(|(known, _)| known.as_str().cmp(key))
            .ok()?;
        Some(values[at].1.clone())
    }
}

static REGISTRATION_DATA: OnceLock<RegistrationData> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_REGISTRATION_DATA: std::cell::RefCell<Option<RegistrationData>> = const { std::cell::RefCell::new(None) };
}
/// Runs `f` with `data` as the registration data, for tests.
#[cfg(any(test, feature = "testing"))]
pub fn with_test_registration_data<R>(data: RegistrationData, f: impl FnOnce() -> R) -> R {
    TEST_REGISTRATION_DATA.set(Some(data));
    let result = f();
    TEST_REGISTRATION_DATA.set(None);
    result
}

fn registration_data() -> Option<RegistrationData> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(data) = TEST_REGISTRATION_DATA.with_borrow(Clone::clone) {
        return Some(data);
    }
    REGISTRATION_DATA.get().cloned()
}

/// Records the registries and settings registration reads, once at startup, after the
/// package inventory and before the first registration hook; a second call panics.
pub fn install_registration_data(data: RegistrationData) {
    assert!(
        REGISTRATION_DATA.set(data).is_ok(),
        "registration data already installed"
    );
}

/// Read-only view of the startup package inventory, shared: cloning it copies no package.
// no wire: zero-byte phase token: decoding rebinds the installed package inventory rather than transferring entries.
#[derive(Clone)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct LoadedMods {
    pub(crate) entries: Arc<[PackageInfo]>,
}

impl LoadedMods {
    pub(crate) fn new() -> Self {
        #[cfg(any(test, feature = "testing"))]
        if let Some(entries) = TEST_PACKAGES.with_borrow(Clone::clone) {
            return Self { entries };
        }
        Self {
            entries: PACKAGES.get().cloned().unwrap_or_else(|| Arc::new([])),
        }
    }

    /// Package IDs in dispatch order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(PackageInfo::id)
    }

    /// Packages in dispatch order.
    pub fn iter(&self) -> impl Iterator<Item = &PackageInfo> {
        self.entries.iter()
    }

    /// Find a package by ID. This linear lookup is intended for registration, not hot paths.
    pub fn get(&self, id: &str) -> Option<&PackageInfo> {
        self.entries.iter().find(|entry| entry.id() == id)
    }
}

/// Why a registration read was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum RegistrationReadError {
    /// The engine supplied no registry or settings data to this context.
    Unavailable,
    /// The registry name is not a namespaced identifier (`minecraft:item`).
    InvalidRegistryName(String),
    /// No built-in registry has this name.
    UnknownRegistry(String),
    /// The setting key is empty or names a path rather than one key.
    InvalidSettingKey(String),
    /// No loaded package has this ID, so it has no settings to read.
    UnknownMod(String),
}
impl std::fmt::Display for RegistrationReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "registration read refused: {self:?}")
    }
}
impl std::error::Error for RegistrationReadError {}

/// A registration phase's view. Declarations go on the hook's event; there is
/// no world, handle constructor, transaction or game Host at this point.
///
/// Every registration hook (`register`, `component_types`, `blocks`, `items`,
/// `standalone_models` and the other `Register*` hooks) reads the same frozen view,
/// captured once before the first of them runs, so an answer never depends on
/// dispatch order or on which hook asks:
///
/// - [`mods`](Self::mods): the loaded packages.
/// - [`registry`](Self::registry): the built-in registries, as `registries.json` lists them.
///   Entries that mods declare during registration are not in it; a mod already has its own
///   declarations on its event or registrar.
/// - [`setting`](Self::setting): the settings on disk for the loaded packages when registration
///   began. Later writes are not seen.
///
/// ```compile_fail
/// use mod_api::{Host, RegistrationContext};
/// fn game(_: &dyn Host) {}
/// fn registration(context: &RegistrationContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, RegistrationContext};
/// fn registration(context: &RegistrationContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, RegistrationContext};
/// fn registration(context: &mut RegistrationContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
// exhaustive: its fields are private, so mods cannot build or destructure it
// no wire: phase context has a custom Wire layout for its derived directory value; other host tables rebind locally.
pub struct RegistrationContext {
    pub(crate) mods: LoadedMods,
    pub(crate) data: Option<RegistrationData>,
    pub(crate) declaration_refusals: std::sync::Mutex<Vec<String>>,
    pub(crate) self_refusal: std::sync::Mutex<Option<String>>,
    pub(crate) instance_dir: Option<crate::InstanceDirectory>,
}

impl RegistrationContext {
    pub(crate) fn new() -> Self {
        let data = registration_data();
        let instance_dir = data.as_ref().map(|data| data.instance_dir.clone());
        Self {
            mods: LoadedMods::new(),
            data,
            declaration_refusals: Default::default(),
            self_refusal: Default::default(),
            instance_dir,
        }
    }

    /// The instance directory captured before registration, identically on native
    /// and wasm. This is data only, not permission to access a filesystem.
    /// The value preserves the host OS's raw path encoding, including non-UTF-8 names.
    pub fn instance_dir(&self) -> Result<&crate::InstanceDirectory, RegistrationReadError> {
        self.instance_dir
            .as_ref()
            .ok_or(RegistrationReadError::Unavailable)
    }

    /// The loaded-mod view, independent of this phase's event or registrar.
    pub fn mods(&self) -> &LoadedMods {
        &self.mods
    }

    /// A built-in registry's names in ID order, for validating the keys a mod declares.
    pub fn registry(&self, name: &str) -> Result<Arc<[String]>, RegistrationReadError> {
        if crate::Identifier::parse_namespaced(name).is_none() {
            return Err(RegistrationReadError::InvalidRegistryName(name.to_owned()));
        }
        self.data()?
            .registry(name)
            .ok_or_else(|| RegistrationReadError::UnknownRegistry(name.to_owned()))
    }

    /// A loaded mod's setting as it was when registration began: `Ok(None)` when the mod
    /// has not set it. The key is one setting's name, not a path.
    pub fn setting(
        &self,
        mod_id: &str,
        key: &str,
    ) -> Result<Option<String>, RegistrationReadError> {
        if key.is_empty() || matches!(key, "." | "..") || key.contains(['/', '\\', '\0']) {
            return Err(RegistrationReadError::InvalidSettingKey(key.to_owned()));
        }
        let data = self.data()?;
        if self.mods.get(mod_id).is_none() {
            return Err(RegistrationReadError::UnknownMod(mod_id.to_owned()));
        }
        Ok(data.setting(mod_id, key))
    }

    fn data(&self) -> Result<&RegistrationData, RegistrationReadError> {
        self.data.as_ref().ok_or(RegistrationReadError::Unavailable)
    }
}

/// A data load's read-only view. The owning loader supplies its origin and
/// metadata; reading already-loaded data never creates this context.
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, Host};
/// fn game(_: &dyn Host) {}
/// fn load(context: &dyn DataLoadContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{prelude::EntitiesHost, DataLoadContext};
/// fn load(context: &dyn DataLoadContext) { let _ = EntitiesHost::entity(context, 1); }
/// ```
///
/// ```compile_fail
/// use mod_api::{DataLoadContext, HostTransaction};
/// fn load(context: &mut dyn DataLoadContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait DataLoadContext {
    /// The phase selected at the owning load boundary.
    fn origin(&self) -> DataLoadOrigin;

    /// Registry names in ID order, from this resource generation's snapshot.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;

    /// Resolve a fluid name against this load's registered fluid identities.
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey>;

    /// The fluid tags this load's resources hold (`data/<ns>/tags/fluid`):
    /// each tag id to the fluids it names, in the sources' order — what a
    /// recipe's fluid ingredient resolves its `#tag`s with. `None` where the
    /// load reads no recipe resources.
    fn fluid_tags(&self) -> Option<Arc<[(String, Arc<[String]>)]>> {
        None
    }

    /// A loaded mod's setting captured at the owning load boundary, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;

    /// The same loaded-mod view available during registration.
    fn mods(&self) -> LoadedMods;
}

/// Immutable resource-generation reads supplied by an owning loader.
pub trait LoadReader: Sync {
    /// Registry names in ID order.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;
    /// Fluid tags and their members in source order.
    fn fluid_tags(&self) -> Arc<[(String, Arc<[String]>)]>;
    /// A captured setting, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's owning load, as a [`DataLoadContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoad<'a> {
    source: &'a dyn LoadReader,
    origin: DataLoadOrigin,
    mods: LoadedMods,
}

impl<'a> OwningLoad<'a> {
    /// Borrow a loader's immutable reader at its owning boundary.
    pub fn new(origin: DataLoadOrigin, source: &'a dyn LoadReader) -> Self {
        Self {
            source,
            origin,
            mods: LoadedMods::new(),
        }
    }
}

impl DataLoadContext for OwningLoad<'_> {
    fn origin(&self) -> DataLoadOrigin {
        self.origin
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.source.registry(name)
    }
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey> {
        crate::content::registry_keys::fluid_key(id)
    }
    fn fluid_tags(&self) -> Option<Arc<[(String, Arc<[String]>)]>> {
        Some(self.source.fluid_tags())
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> LoadedMods {
        self.mods.clone()
    }
}

/// The view of a hook fired inside a menu, recipe or player-list loop, where no
/// game is at hand: the side, the registries, the settings and the mod list.
/// The side that owns the loop builds it once per resource generation; a hook
/// never builds one, and its answers never change while it is held.
///
/// ```compile_fail
/// use mod_api::{Host, LoopContext};
/// fn game(_: &dyn Host) {}
/// fn each(context: &dyn LoopContext) { game(context); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, LoopContext};
/// fn each(context: &mut dyn LoopContext) {
///     let _: Result<(), ()> = context.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait LoopContext {
    /// Whether a mod-created recipe property set accepts an item ID. Unknown
    /// sets/items and vanilla set keys return false. Use this in
    /// `menu_slot_rules` for the same filter on server and client prediction.
    /// Answers come from the owning load, replaced by the server's synced
    /// generation on clients; no allocation or file IO occurs during a read.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool;

    /// The side whose loop fires the hook.
    fn side(&self) -> LoopSide;

    /// Registry names in ID order, from the resource generation's snapshot.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;

    /// A loaded mod's setting captured when this context was built, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;

    /// The same loaded-mod view available during registration.
    fn mods(&self) -> LoadedMods;
}

/// Immutable resource-generation reads supplied by an owning loop.
pub trait LoopReader: Sync {
    /// Mod-created recipe property membership from the owning generation.
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        let _ = (set, item);
        false
    }
    /// Registry names in ID order.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;
    /// A captured setting, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The game's loop, as a [`LoopContext`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningLoop<'a> {
    source: &'a dyn LoopReader,
    side: LoopSide,
    mods: LoadedMods,
}

impl<'a> OwningLoop<'a> {
    /// Borrow a loop's immutable reader at its owning boundary.
    pub fn new(side: LoopSide, source: &'a dyn LoopReader) -> Self {
        Self {
            source,
            side,
            mods: LoadedMods::new(),
        }
    }
}

impl LoopContext for OwningLoop<'_> {
    fn mod_recipe_property_contains(&self, set: &str, item: crate::ItemKey) -> bool {
        self.source.mod_recipe_property_contains(set, item)
    }
    fn side(&self) -> LoopSide {
        self.side
    }
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.source.registry(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        self.source.setting(mod_id, key)
    }
    fn mods(&self) -> LoadedMods {
        self.mods.clone()
    }
}

/// The view of a hook fired while a feature generates, where no game is at
/// hand: on a worldgen worker thread, or on the server thread when a sapling
/// grows. It carries the server's registries, its settings and the mod list;
/// the server builds it once per resource generation, a hook never builds one,
/// and its answers never change while it is held. There are no writes, no
/// level or entity handles and no transaction: what the generating region
/// holds is the hook's own arguments.
///
/// ```compile_fail
/// use mod_api::{Host, WorldgenHost};
/// fn game(_: &dyn Host) {}
/// fn generate(host: &dyn WorldgenHost) { game(host); }
/// ```
///
/// ```compile_fail
/// use mod_api::{HostTransaction, WorldgenHost};
/// fn generate(host: &mut dyn WorldgenHost) {
///     let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
/// }
/// ```
pub trait WorldgenHost {
    /// Registry names in ID order, from the server's resource generation.
    fn registry(&self, name: &str) -> Option<Arc<[String]>>;

    /// A loaded mod's setting captured when the server built this view, without file I/O.
    fn setting(&self, mod_id: &str, key: &str) -> Option<String>;

    /// The same loaded-mod view available during registration.
    fn mods(&self) -> LoadedMods;
}

// no wire: borrowed phase-scoped host readers; authority stays with their owning thread.
/// The server's world generation, as a [`WorldgenHost`].
// hidden: the game's side of the context, not part of the mod surface
#[doc(hidden)]
// exhaustive: its fields are private; only the game builds it
pub struct OwningWorldgen<'a> {
    registry: &'a dyn Fn(&str) -> Option<Arc<[String]>>,
    setting: &'a dyn Fn(&str, &str) -> Option<String>,
    mods: LoadedMods,
}

impl<'a> OwningWorldgen<'a> {
    /// The server's registries and mod settings, read through the game's worldgen context.
    pub fn new(
        registry: &'a dyn Fn(&str) -> Option<Arc<[String]>>,
        setting: &'a dyn Fn(&str, &str) -> Option<String>,
    ) -> Self {
        Self {
            registry,
            setting,
            mods: LoadedMods::new(),
        }
    }
}

impl WorldgenHost for OwningWorldgen<'_> {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        (self.registry)(name)
    }
    fn setting(&self, mod_id: &str, key: &str) -> Option<String> {
        (self.setting)(mod_id, key)
    }
    fn mods(&self) -> LoadedMods {
        self.mods.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_dispatch_slice_keeps_order_in_registration_config_and_reload() {
        use crate::*;
        use std::sync::Mutex;
        struct Trace {
            id: &'static str,
            calls: Arc<Mutex<Vec<(&'static str, &'static str)>>>,
        }
        #[crate::__subscriptions]
        impl Hooks for Trace {
            fn register(&self, _: &mut dyn Registrar, _: &RegistrationContext) {
                assert_eq!(current_phase(), ModPhase::Registration);
                self.calls.lock().unwrap().push(("register", self.id));
            }
            fn config_loading(&self, _: HostSide<'_>, _: &mut ConfigLoadingEvent<'_>) {
                assert_eq!(current_phase(), ModPhase::Config);
                self.calls.lock().unwrap().push(("config", self.id));
            }
            fn add_server_reload_listeners(
                &self,
                _: &dyn Host,
                _: &mut AddServerReloadListenersEvent<'_>,
            ) {
                assert_eq!(current_phase(), ModPhase::Reloading);
                self.calls.lock().unwrap().push(("reload", self.id));
            }
        }
        struct Server;
        impl Host for Server {}
        mod_api::impl_host_areas!(Server);
        impl ServerHost for Server {}
        mod_api::impl_server_host_areas!(Server);
        // The loader supplies a dependency-resolved slice; phase dispatch must retain it.
        let order = [1, 0, 2];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ids = ["z", "lib", "a"];
        let mods: Mods = order
            .into_iter()
            .map(|i| {
                ModEntry::new(
                    ids[i],
                    Box::new(Trace {
                        id: ids[i],
                        calls: calls.clone(),
                    }),
                )
            })
            .collect::<Vec<_>>()
            .into();
        crate::install(mods.clone());
        let owner = PhaseOwner::new(ModPhase::Packages);
        owner.advance(ModPhase::Registration).unwrap();
        crate::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
        owner.advance(ModPhase::Setup).unwrap();
        let mut host = Server;
        for _world in 0..2 {
            {
                let _config = owner.config().unwrap();
                dispatch_config_loading(
                    HostSide::Server(&mut host),
                    &ConfigFile {
                        mod_id: "lib".into(),
                        kind: ConfigKind::Server,
                        file: "lib-server.toml".into(),
                        text: String::new(),
                    },
                );
            }
            for _reload in 0..2 {
                let _reload = owner.reloading().unwrap();
                dispatch_add_server_reload_listeners(&host, &mut ServerListenerRegistry::new());
            }
        }
        let expected = [
            "register", "config", "reload", "reload", "config", "reload", "reload",
        ]
        .into_iter()
        .flat_map(|moment| ["lib", "z", "a"].map(|id| (moment, id)))
        .collect::<Vec<_>>();
        assert_eq!(*calls.lock().unwrap(), expected);
    }

    #[test]
    fn phase_edges_are_exhaustive_and_refusal_leaves_state_unchanged() {
        use ModPhase::*;
        let phases = [Packages, Registration, Setup, Config, World, Reloading];
        let edges = [
            (Packages, Registration),
            (Registration, Setup),
            (Setup, Config),
            (Config, World),
            (World, Config),
            (Setup, Reloading),
            (World, Reloading),
        ];
        for from in phases {
            for to in phases {
                let owner = PhaseOwner::new(from);
                let result = owner.advance(to);
                assert_eq!(
                    result.is_ok(),
                    edges.contains(&(from, to)),
                    "{from:?} -> {to:?}"
                );
                assert_eq!(current_phase(), if result.is_ok() { to } else { from });
            }
        }
        let owner = PhaseOwner::new(Packages);
        let other = owner.clone();
        assert_eq!(
            std::thread::spawn(move || other.advance(Registration))
                .join()
                .unwrap(),
            Err(PhaseError::WrongThread)
        );
        assert_eq!(current_phase(), Packages);
    }

    #[test]
    fn reloads_restore_setup_or_world_and_world_config_can_repeat() {
        use ModPhase::*;
        let owner = PhaseOwner::new(Packages);
        owner.advance(Registration).unwrap();
        owner.advance(Setup).unwrap();
        {
            let _reload = owner.reloading().unwrap();
            assert_eq!(current_phase(), Reloading);
            assert!(owner.reloading().is_err());
        }
        assert_eq!(current_phase(), Setup);
        assert!(owner.advance(Registration).is_err());
        for _world in 0..2 {
            {
                let _config = owner.config().unwrap();
                assert_eq!(current_phase(), Config);
                check_access("setting", LoadAccess::WorldOnly);
            }
            assert_eq!(current_phase(), World);
            for _reload in 0..2 {
                let _ = std::panic::catch_unwind(|| {
                    let _reload = owner.reloading().unwrap();
                    assert_eq!(current_phase(), Reloading);
                    check_access("block_state_default", LoadAccess::Metadata);
                    panic!("failed prepare");
                });
                assert_eq!(current_phase(), World);
            }
        }
        {
            let _loader = PhaseOwner::scope(Setup);
            assert_eq!(current_phase(), Setup);
        }
        assert_eq!(current_phase(), World);
    }

    #[test]
    fn fluid_state_facts_follow_metadata_access() {
        use ModPhase::*;
        for phase in [Packages, Registration, Setup, Config, Reloading, World] {
            let _owner = PhaseOwner::new(phase);
            for operation in [
                "fluid_legacy_block",
                "fluid_state_definition",
                "fluid_state_validate",
                "fluid_type_properties",
            ] {
                let result = std::panic::catch_unwind(|| {
                    check_access(operation, LoadAccess::Metadata);
                });
                assert_eq!(result.is_ok(), matches!(phase, Setup | Reloading | World));
            }
        }
    }

    #[test]
    fn early_world_handle_and_transaction_calls_panic_naming_the_phase() {
        use crate::{Host, HostTransaction, ServerHost, prelude::*};
        struct HostDouble;
        impl Host for HostDouble {}
        mod_api::impl_host_areas!(HostDouble);
        impl ServerHost for HostDouble {}
        mod_api::impl_server_host_areas!(HostDouble);
        for phase in [
            ModPhase::Registration,
            ModPhase::Config,
            ModPhase::Reloading,
            ModPhase::Setup,
        ] {
            let _owner = PhaseOwner::new(phase);
            for action in [0, 1, 2, 3] {
                let panic = std::panic::catch_unwind(|| {
                    let mut host = HostDouble;
                    match action {
                        0 => {
                            let _ = host.entity(1);
                        }
                        1 => {
                            let _ = host.level("minecraft:overworld");
                        }
                        2 => {
                            let _ = host.player(1);
                        }
                        _ => {
                            let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
                        }
                    }
                })
                .expect_err("early access must panic, not return a refusal");
                let message = panic.downcast_ref::<String>().unwrap();
                assert!(message.contains(&format!("{phase:?}")), "{message}");
            }
        }
        let _owner = PhaseOwner::new(ModPhase::Registration);
        let refused = std::panic::catch_unwind(|| HostDouble.block_state_id("stone")).unwrap_err();
        assert!(
            refused
                .downcast_ref::<String>()
                .unwrap()
                .contains("Registration")
        );
    }

    #[test]
    fn inventory_keeps_first_content_package_after_code_mods() {
        let native = [PackageInfo::new(
            "runtime".into(),
            "3".into(),
            PackageKind::Code,
            crate::Side::Client,
            vec!["runtime".into()],
            vec![],
        )];
        let package = |version: &str| {
            PackageInfo::new(
                "plain".into(),
                version.into(),
                PackageKind::Content,
                crate::Side::Client,
                vec!["plain".into()],
                vec![],
            )
        };
        let entries = inventory(&native, &[package("1"), package("2")]);
        let view = LoadedMods {
            entries: entries.into(),
        };
        assert_eq!(view.ids().collect::<Vec<_>>(), ["runtime", "plain"]);
        let runtime = view.get("runtime").unwrap();
        assert_eq!(runtime.kind(), PackageKind::Code);
        assert_eq!(runtime.version(), "3");
        assert_eq!(runtime.side(), crate::Side::Client);
        let plain = view.get("plain").unwrap();
        assert_eq!(plain.kind(), PackageKind::Content);
        assert_eq!(plain.version(), "1");
        assert_eq!(plain.side(), crate::Side::Client);
        assert_eq!(plain.namespaces().collect::<Vec<_>>(), ["plain"]);
        assert!(!plain.depends_on("runtime"));
        assert!(view.get("missing").is_none());
    }

    #[test]
    fn a_loop_context_reads_its_side_registries_and_mods() {
        let source = TestReader {
            registries: std::collections::BTreeMap::from([(
                "item".to_owned(),
                vec!["a:b".into()].into(),
            )]),
            tags: Arc::from([]),
        };
        with_test_packages(&["one", "two"], || {
            let context = OwningLoop::new(LoopSide::Client, &source);
            assert_eq!(context.side(), LoopSide::Client);
            assert_eq!(
                context.registry("item").as_deref(),
                Some(&["a:b".to_owned()][..])
            );
            assert_eq!(context.registry("block"), None);
            assert_eq!(context.setting("one", "mode"), None);
            assert_eq!(context.mods().ids().collect::<Vec<_>>(), ["one", "two"]);
        });
    }

    #[test]
    fn registration_hooks_read_one_snapshot_and_refuse_bad_reads() {
        use crate::*;
        use std::sync::Mutex;

        /// Every answer the context gives this hook, for the keys the test names.
        fn probe(cx: &RegistrationContext) -> Vec<String> {
            let registry = |name| format!("{:?}", cx.registry(name));
            let setting = |id, key| format!("{:?}", cx.setting(id, key));
            vec![
                format!(
                    "{:?}",
                    cx.instance_dir()
                        .map(|dir| dir == &crate::InstanceDirectory::Unix(vec![47, 255]))
                ),
                registry("minecraft:item"),
                registry("minecraft:missing"),
                registry("item"),
                registry(""),
                setting("reader", "mode"),
                setting("reader", "unset"),
                setting("other", "mode"),
                setting("absent", "mode"),
                setting("reader", ""),
                setting("reader", ".."),
                setting("reader", "../mode"),
                setting("reader", "a\\b"),
            ]
        }
        struct Reader(Arc<Mutex<Vec<String>>>);
        #[crate::__subscriptions]
        impl Hooks for Reader {
            fn register(&self, _: &mut dyn Registrar, cx: &RegistrationContext) {
                *self.0.lock().unwrap() = probe(cx);
            }
            fn component_types(
                &self,
                cx: &RegistrationContext,
                event: &mut ComponentTypesEvent<'_>,
            ) {
                *event.types = probe(cx);
            }
        }

        let data = RegistrationData::new(
            BTreeMap::from([(
                "minecraft:item".to_owned(),
                vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()],
            )]),
            BTreeMap::from([
                (
                    "reader".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "fast".to_owned())]),
                ),
                (
                    "other".to_owned(),
                    BTreeMap::from([("mode".to_owned(), "slow".to_owned())]),
                ),
            ]),
            crate::InstanceDirectory::Unix(vec![47, 255]),
        );

        let registered = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "reader",
            Box::new(Reader(registered.clone())),
        )]
        .into();
        crate::install(mods.clone());
        let expected = [
            "Ok(true)",
            r#"Ok(["minecraft:air", "minecraft:stone"])"#,
            r#"Err(UnknownRegistry("minecraft:missing"))"#,
            r#"Err(InvalidRegistryName("item"))"#,
            r#"Err(InvalidRegistryName(""))"#,
            r#"Ok(Some("fast"))"#,
            "Ok(None)",
            r#"Ok(Some("slow"))"#,
            r#"Err(UnknownMod("absent"))"#,
            r#"Err(InvalidSettingKey(""))"#,
            r#"Err(InvalidSettingKey(".."))"#,
            r#"Err(InvalidSettingKey("../mode"))"#,
            r#"Err(InvalidSettingKey("a\\b"))"#,
        ];
        with_test_packages(&["reader", "other"], || {
            with_test_registration_data(data, || {
                crate::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false)
                    .unwrap();
                assert_eq!(*registered.lock().unwrap(), expected);
                let mut types = Vec::new();
                dispatch_component_types(&RegistrationContext::new(), &mut types);
                assert_eq!(types, expected, "a later step reads the same snapshot");
            });
            // No engine view: every read is refused, and a bad name or key still says so.
            crate::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
            let refused = registered.lock().unwrap().clone();
            assert_eq!(refused[0], "Err(Unavailable)");
            assert_eq!(refused[1], "Err(Unavailable)");
            assert_eq!(refused[3], r#"Err(InvalidRegistryName("item"))"#);
            assert_eq!(refused[5], "Err(Unavailable)");
            assert_eq!(refused[9], r#"Err(InvalidSettingKey(""))"#);
        });
    }
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestReader {
    pub registries: std::collections::BTreeMap<String, Arc<[String]>>,
    pub tags: Arc<[(String, Arc<[String]>)]>,
}
#[cfg(test)]
impl LoadReader for TestReader {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn fluid_tags(&self) -> Arc<[(String, Arc<[String]>)]> {
        self.tags.clone()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}
#[cfg(test)]
impl LoopReader for TestReader {
    fn registry(&self, name: &str) -> Option<Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn client_only_attribute_does_not_shift_both_side_registry() {
    use crate::Sentiment;
    use crate::parts::{AttributesRegistrar, PartRegistrar};
    let packages = [("alpha", crate::Side::Client), ("zeta", crate::Side::Both)]
        .into_iter()
        .map(|(id, side)| {
            PackageInfo::new(
                id.to_owned(),
                "1".to_owned(),
                PackageKind::Code,
                side,
                vec![id.to_owned()],
                vec![],
            )
        })
        .collect::<Vec<_>>();
    TEST_PACKAGES.set(Some(packages.into()));
    let mut alpha = PartRegistrar::new("alpha");
    let mut zeta = PartRegistrar::new("zeta");
    assert_eq!(
        alpha.declare_attribute("decoration", 0.0, 0.0, 1.0, false, Sentiment::Neutral),
        Err(crate::AttributeRegistrationError::WrongSide)
    );
    assert!(
        zeta.declare_attribute("mana", 0.0, 0.0, 1.0, true, Sentiment::Positive)
            .is_ok()
    );
    assert!(alpha.attributes.is_empty());
    assert_eq!(zeta.attributes[0].id, "zeta:mana");
    TEST_PACKAGES.set(None);
}
