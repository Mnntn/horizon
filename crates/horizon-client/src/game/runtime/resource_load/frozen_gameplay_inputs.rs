//! Owned fixture inputs only; never a registry wrapper or an installation.
use super::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, Weak},
};
type Inputs = (
    Arc<serde_json::Value>,
    BlockFacts,
    Vec<horizon_client::blocks::GameplayProjection>,
);
type FluidContext = Vec<(
    horizon_world::fluid::ModFluid,
    &'static horizon_world::fluid::ModFluidType,
)>;
type Store = Mutex<Option<(FluidContext, Inputs)>>;
fn owners() -> &'static Mutex<HashMap<PathBuf, Weak<Store>>> {
    static OWNERS: OnceLock<Mutex<HashMap<PathBuf, Weak<Store>>>> = OnceLock::new();
    OWNERS.get_or_init(Default::default)
}

pub(in crate::game) struct FrozenPreparedRoot {
    root: PathBuf,
    store: Arc<Store>,
}
/// One standard generation per test process; the Python scratch owner removes it
/// after exit because Rust does not drop statics.
pub(in crate::game) fn standard_root() -> PathBuf {
    static STANDARD: OnceLock<(PathBuf, FrozenPreparedRoot)> = OnceLock::new();
    let source =
        PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared test resources"))
            .canonicalize()
            .expect("canonical prepared test source");
    let scratch = PathBuf::from(std::env::var_os("HORIZON_FROZEN_SCRATCH").expect(
        "frozen fixtures require tools/ci/frozen_scratch.py with a disk-backed scratch parent",
    ));
    assert_eq!(
        std::env::temp_dir().canonicalize().unwrap(),
        scratch.canonicalize().unwrap(),
        "frozen scratch must be the wrapper-owned TMPDIR"
    );
    let (identity, owner) = STANDARD.get_or_init(|| {
        let owner =
            FrozenPreparedRoot::copy_from(&source).expect("plain prepared fixture snapshot");
        (source.clone(), owner)
    });
    assert_source_identity(&source, identity);
    owner.path().to_owned()
}
fn assert_source_identity(source: &Path, identity: &Path) {
    assert_eq!(
        source, identity,
        "standard frozen fixture source changed within one test process"
    );
}

#[test]
fn changed_standard_source_is_refused() {
    assert!(
        std::panic::catch_unwind(|| assert_source_identity(
            Path::new("second"),
            Path::new("first")
        ))
        .is_err()
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn standard_generation_is_shared_across_callers_and_threads() {
    let first = standard_root();
    let second = std::thread::spawn(standard_root).join().unwrap();
    assert_eq!(first, second);
    assert_ne!(
        first,
        PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap())
            .canonicalize()
            .unwrap()
    );
}

impl FrozenPreparedRoot {
    /// Copies bytes, rejecting links/special files. No permission change can reach
    /// the source. Only this owned generation is eligible, never an environment path.
    pub(in crate::game) fn copy_from(source: &Path) -> std::io::Result<Self> {
        if !std::fs::symlink_metadata(source)?.is_dir()
            || source.join("horizon-mod-content.json").exists()
            || !horizon_mod_package::enabled::EnabledMods::read(source)?.is_empty()
        {
            return Err(std::io::Error::other(
                "frozen fixture requires a plain vanilla directory without enabled content packages",
            ));
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-frozen-inputs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root)?;
        let mut owner = Self {
            root,
            store: Arc::new(Mutex::new(None)),
        };
        copy_tree(source, &owner.root)?;
        owner.root = owner.root.canonicalize()?;
        owners()
            .lock()
            .unwrap()
            .insert(owner.root.clone(), Arc::downgrade(&owner.store));
        Ok(owner)
    }
    pub(in crate::game) fn path(&self) -> &Path {
        &self.root
    }
}
fn copy_tree(source: &Path, dest: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            std::fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
            let mut permissions = target.metadata()?.permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(target, permissions)?;
        } else {
            return Err(std::io::Error::other(
                "frozen fixture does not support links or special files",
            ));
        }
    }
    let mut permissions = dest.metadata()?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(dest, permissions)
}
impl Drop for FrozenPreparedRoot {
    fn drop(&mut self) {
        owners().lock().unwrap().remove(&self.root);
        fn thaw(path: &Path) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
            }
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        thaw(&entry.path());
                    }
                }
            }
        }
        thaw(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(super) fn load(
    root: &Path,
    decode: impl FnOnce() -> std::result::Result<Inputs, String>,
) -> std::result::Result<Inputs, String> {
    let store = owners().lock().unwrap().get(root).and_then(Weak::upgrade);
    let Some(store) = store else {
        return decode();
    };
    let mut inputs = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Facts also consult the process-global fluid table. Retain its immutable
    // entries as part of the key, not just the frozen bytes/numeric registry.
    let fluids = horizon_world::fluid::mod_fluids();
    if let Some((known_fluids, inputs)) = inputs.as_ref()
        && *known_fluids == fluids
    {
        return Ok(inputs.clone());
    }
    let decoded = decode()?; // Failures are never retained.
    // Registration during a decode must not publish a mixed-context cache entry.
    *inputs = (fluids == horizon_world::fluid::mod_fluids()).then(|| (fluids, decoded.clone()));
    Ok(decoded)
}

#[test]
fn owned_generation_errors_and_views() {
    let source = std::env::temp_dir().join(format!("horizon-input-source-{}", std::process::id()));
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("input"), "first").unwrap();
    let owner = FrozenPreparedRoot::copy_from(&source).unwrap();
    let root = owner.path().to_owned();
    let make = || {
        Ok((
            Arc::new(serde_json::json!(1)),
            BlockFacts {
                states: Vec::new(),
                air: horizon_world::registry::BlockStateId(0),
                woods: HashMap::new(),
                sign_texts: HashMap::new(),
            },
            Vec::new(),
        ))
    };
    assert!(load(&root, || Err("broken".into())).is_err());
    assert!(
        std::panic::catch_unwind(|| {
            let _ = load(&root, || panic!("decode interrupted"));
        })
        .is_err()
    );
    let (_, mut facts, _) = load(&root, make).unwrap();
    facts.air = horizon_world::registry::BlockStateId(99);
    assert_eq!(load(&root, || panic!("cache miss")).unwrap().1.air.0, 0);
    std::fs::write(source.join("input"), "second").unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("input")).unwrap(),
        "first"
    );
    assert!(root.metadata().unwrap().permissions().readonly());
    assert!(
        root.join("input")
            .metadata()
            .unwrap()
            .permissions()
            .readonly()
    );
    assert!(
        !source
            .join("input")
            .metadata()
            .unwrap()
            .permissions()
            .readonly()
    );
    let second = FrozenPreparedRoot::copy_from(&source).unwrap();
    assert_eq!(
        std::fs::read_to_string(second.path().join("input")).unwrap(),
        "second"
    );
    assert!(load(second.path(), || Err("distinct generation".into())).is_err());
    let decodes = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let decodes = &decodes;
            let path = second.path();
            scope.spawn(move || {
                assert_eq!(
                    *load(path, || {
                        decodes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let mut inputs = make()?;
                        inputs.0 = Arc::new(serde_json::json!(2));
                        inputs.1.air = horizon_world::registry::BlockStateId(2);
                        Ok(inputs)
                    })
                    .unwrap()
                    .0,
                    serde_json::json!(2)
                );
            });
        }
    });
    assert_eq!(decodes.load(std::sync::atomic::Ordering::SeqCst), 1);

    assert_eq!(
        *load(&root, || panic!("first owner miss")).unwrap().0,
        serde_json::json!(1)
    );
    let second_hit = load(second.path(), || panic!("second owner miss")).unwrap();
    assert_eq!(*second_hit.0, serde_json::json!(2));
    assert_eq!(second_hit.1.air.0, 2);
    drop(owner);
    assert!(load(&root, || Err("owner dropped".into())).is_err());
    assert!(load(&source, || Err("arbitrary root".into())).is_err());
    std::fs::remove_dir_all(source).unwrap();
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn cache_hits_keep_games_installations_and_parts_fresh() {
    use crate::game::native_entity_refs::tests::with_game_at;
    use horizon_client::chunk::Chunk;
    use horizon_core::position::BlockPos;
    struct Soil(bool);
    #[mod_api::horizon_part]
    impl mod_api::BlockPart for Soil {
        fn can_sustain_plant(
            &self,
            _: &dyn mod_api::LevelHost,
            _: &mut mod_api::CanSustainPlantEvent,
        ) -> Option<bool> {
            Some(self.0)
        }
    }
    struct FixtureMod(Arc<std::sync::atomic::AtomicBool>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for FixtureMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let glass = r.block_key("minecraft:glass");
            r.block_part(
                glass,
                Soil(self.0.load(std::sync::atomic::Ordering::SeqCst)),
            );
        }
    }
    let source = std::env::var_os("HORIZON_TEST_RESOURCES").unwrap();
    let owner = FrozenPreparedRoot::copy_from(Path::new(&source)).unwrap();
    let mut previous_epoch = None;
    let mut previous_registry = None;
    for _ in 0..2 {
        let allow = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mods: Arc<[mod_api::ModEntry]> = vec![mod_api::ModEntry::new(
            "fixture_soil",
            Box::new(FixtureMod(allow.clone())),
        )]
        .into();
        mod_api::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        // The initial installed Part denies. Only the fixture's repeat registration
        // can publish this changed answer, before resolve_parts binds it to states.
        allow.store(true, std::sync::atomic::Ordering::SeqCst);
        with_game_at(owner.path().to_owned(), |game| {
            assert!(game.world.chunks.is_empty());
            let epoch = game.native_lifecycle.epoch().unwrap();
            assert_ne!(previous_epoch, Some(epoch));
            previous_epoch = Some(epoch);
            let registry = game.pack.assets.registry;
            let identity = registry as *const _ as usize;
            assert_ne!(previous_registry, Some(identity));
            previous_registry = Some(identity);
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let flower = registry.default_state("minecraft:dandelion").unwrap();
            let glass = registry.default_state("minecraft:glass").unwrap();
            game.world.set(5, 64, 5, glass).unwrap();
            // Vanilla refuses glass; only the freshly registered and resolved Part
            // allows this. A hit must not skip either installation step.
            assert!(game.with_level(|b, level, ticks| {
                b.can_survive(level, ticks, BlockPos::new(5, 65, 5), flower)
            }));
        });
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn fluid_registration_refreshes_frozen_facts() {
    // This registration deliberately shadows a vanilla block. Never let it
    // contaminate other tests in the caller's process-wide fluid registry.
    const CHILD: &str = "HORIZON_FROZEN_FLUID_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "game::resource_load::frozen_gameplay_inputs::fluid_registration_refreshes_frozen_facts", "--ignored"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            child.status.success(),
            "isolated fluid contract exited {}",
            child.status
        );
        return;
    }
    use horizon_client::blocks::Fluid;
    use horizon_world::fluid::{ModFluidRules, ModFluidType, register_mod_fluid};
    let source = std::env::var_os("HORIZON_TEST_RESOURCES").unwrap();
    let owner = FrozenPreparedRoot::copy_from(Path::new(&source)).unwrap();
    let (registry, _) = horizon_world::level::data::load_registry_with_flags(owner.path()).unwrap();
    let stone = registry.default_state("minecraft:stone").unwrap();
    let decode = || {
        let json: serde_json::Value =
            horizon_world::json_file::read(&owner.path().join("block-behaviours.json"))
                .map_err(|e| e.to_string())?;
        let (facts, projection) =
            BlockFacts::load_with_projection(owner.path(), &registry, &json, false)
                .map_err(|e| e.to_string())?;
        Ok((Arc::new(json), facts, projection))
    };
    assert_eq!(
        load(owner.path(), decode).unwrap().1.states[stone.0 as usize].fluid,
        Fluid::None
    );
    register_mod_fluid(ModFluidType {
        id: "minecraft:stone".into(),
        flowing_id: "fixture:flowing_stone".into(),
        bucket: None,
        rules: ModFluidRules {
            tick_delay: 5,
            drop_off: 1,
            slope_find_distance: 4,
            converts_to_source: false,
        },
    })
    .unwrap();
    assert_eq!(
        load(owner.path(), decode).unwrap().1.states[stone.0 as usize].fluid,
        Fluid::Modded
    );
    assert_eq!(
        load(owner.path(), || panic!("same fluid context must hit"))
            .unwrap()
            .1
            .states[stone.0 as usize]
            .fluid,
        Fluid::Modded
    );
}
