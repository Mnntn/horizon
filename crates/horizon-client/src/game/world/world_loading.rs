//! Prepare local worlds without blocking the window's event loop.
use super::*;
use horizon_client::vanillagen::TerrainResources;
use horizon_world::item_defaults::ItemData;

pub(super) struct WorldRequest {
    pub save_root: PathBuf,
    pub folder: String,
    pub name: Option<String>,
    pub seed: Option<i64>,
    pub creative: bool,
    pub create: Option<horizon_client::title::WorldOptions>,
    pub rule_registry: Arc<horizon_client::gamerules::RuleRegistry>,
    /// The account the world opens under, for an integrated server
    /// (`IntegratedServer.getSingleplayerProfile`): its name and UUID.
    owner: Option<(String, u128)>,
}

pub(super) struct PreparedWorld {
    pub save_root: PathBuf,
    /// The levels' shapes, from the world's dimension types.
    pub shapes: horizon_client::chunk::LevelShapes,
    pub level: Level,
    pub world: World,
    pub streaming: Streaming,
    pub save: Arc<SaveDir>,
    pub session_lock: horizon_client::save::SessionLock,
    pub rand_sequences: horizon_client::random_sequences::RandomSequences,
    pub locator: Option<Arc<VanillaTerrain>>,
    pub loot: horizon_client::structure_blocks::LootSnapshot,
    /// The owner's offline UUID and the account's, when an older world moved its
    /// owner at open (`playerdata::migrate_owner`): entities migrate as they load.
    pub owner_migration: Option<(u128, u128)>,
}

/// The dedicated server's `Main` takes level.dat_old without asking
/// (`getUnfixedDataTagWithFallback`): there is no world list to recover from. The integrated
/// server's world opens from that list, which offers the recovery instead, whatever the launch
/// configuration says.
fn fall_back_to_level_dat_old_without_asking(
    role: execution_role::ExecutionRole,
    save_root: &std::path::Path,
    folder: &str,
) {
    if role.is_dedicated_server() && horizon_client::save::fall_back_to_level_dat_old(save_root) {
        log::warn!("Horizon: level.dat of \"{folder}\" did not read; level.dat_old took its place");
    }
}

impl WorldRequest {
    pub fn new(
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&horizon_client::title::WorldOptions>,
        rule_registry: Arc<horizon_client::gamerules::RuleRegistry>,
    ) -> Result<Self> {
        Ok(Self {
            save_root: std::env::current_dir()?.join("saves").join(folder),
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned(),
            rule_registry,
            owner: None,
        })
    }

    /// The account this world opens under (`IntegratedServer.getSingleplayerProfile`):
    /// a world older builds made under the offline profile moves its owner to it at
    /// open. A dedicated server has no single-player owner and leaves it unset.
    pub fn owned_by(mut self, player: &str, owner: u128) -> Self {
        self.owner = Some((player.to_owned(), owner));
        self
    }

    pub fn prepare(
        self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        settings: &Settings,
        upgrade: &horizon_world::datafix::progress::UpgradeProgress,
    ) -> Result<PreparedWorld> {
        let _prepare_step = startup::step("world.prepare");
        // Before any chunk is read: a loaded container with a loot table is rolled as its chunk
        // is decoded (`save::roll_loaded_loot`), and the spawn chunks are read while the world
        // opens. Every world opens here: on the server thread, on a dedicated server.
        let loot = || {
            horizon_client::structure_blocks::loot_data(
                &options.resources,
                &horizon_world::loot::PackOverlay::NONE,
                res.item_defaults(),
            );
        };
        // The live features (bone meal's and saplings') are installed by `prepare_world`, over
        // the world's own data packs.
        if res.threads() <= 1 {
            loot();
            return self.prepare_world(res, pack, options, settings, upgrade);
        }
        // The loot data does not need the world: it loads beside it (the generator's decoration
        // keeps one core busy) and is done before it opens.
        std::thread::scope(|scope| {
            let loot = scope.spawn(loot);
            let prepared = self.prepare_world(res, pack, options, settings, upgrade);
            loot.join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            prepared
        })
    }

    fn prepare_world(
        self,
        res: &Resources,
        pack: &PackResources,
        options: &Options,
        settings: &Settings,
        upgrade: &horizon_world::datafix::progress::UpgradeProgress,
    ) -> Result<PreparedWorld> {
        let Self {
            save_root,
            folder,
            name,
            seed,
            creative,
            create,
            rule_registry,
            owner,
        } = self;
        let name = name.as_deref();
        let create = create.map(|mut create| {
            create.set_additional_data_packs(pack.additional_data.clone());
            create
        });
        let create = create.as_ref();
        // openWorldLoadLevelData: level data that does not read is RecoverWorldDataScreen's to
        // handle (the title screen shows it); a world opened another way must not become a
        // new one with a random seed under the same name.
        fall_back_to_level_dat_old_without_asking(options.execution_role, &save_root, &folder);
        if let Some(issue) = horizon_client::save::level_data_needs_recovery(&save_root) {
            let why = match issue {
                horizon_client::save::DatIssue::Missing => "level.dat is missing".to_owned(),
                horizon_client::save::DatIssue::Unreadable(why) => why,
            };
            return Err(format!(
                "Failed to load world \"{folder}\": its level data does not read ({why}). \
                 Open it from the world list to try restoring level.dat_old."
            )
            .into());
        }
        // LevelSummary.isCompatible, which the dedicated server's `Main` checks before it fixes
        // a world: a world from another series is refused, never upgraded in place.
        if let Some(version) = horizon_client::save::incompatible_version(&save_root) {
            let version = if version.is_empty() {
                "unknown".to_owned()
            } else {
                version
            };
            return Err(format!(
                "Failed to load world \"{folder}\": it was last played in version {version}, \
                 which cannot be opened in this version."
            )
            .into());
        }
        horizon_client::save::upgrade_older_world_with(&save_root, upgrade)?;
        horizon_client::save::migrate_legacy(&save_root)?;
        let session_lock = horizon_client::save::SessionLock::acquire(&save_root)?;
        // RandomSequences: the world's data/minecraft/random_sequences.dat.
        let rand_sequences =
            horizon_client::save::read_dat(&save_root.join("data/minecraft/random_sequences.dat"))
                .and_then(|t| t.get("data").cloned())
                .map(|d| horizon_client::random_sequences::RandomSequences::from_nbt(&d))
                .unwrap_or_default();
        let existing = Level::read_with_registry(&save_root, rule_registry.clone());
        let created = existing.is_none();
        let level = existing.unwrap_or_else(|| Level {
            name: name.unwrap_or(&folder).to_owned(),
            // WorldOptions.randomSeed when no seed was given.
            seed: seed.unwrap_or_else(|| {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
                (t ^ (t >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64
            }),
            creative,
            day_time: 0,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            rule_registry,
            ..Level::default()
        });
        let mut level = level;
        // `IntegratedServer.setSingleplayerProfile`: a world older Horizon builds made
        // under the offline profile moves its owner to the account at open — the player
        // file, its `UUID` and level.dat's `singleplayer_uuid`. Its entities follow as
        // their chunks load (`owner_migration`, `entity_nbt::migrate_player_uuid`).
        let owner_migration = owner.and_then(|(player, account)| {
            let old = horizon_protocol::crypto::offline_uuid(&player);
            (old != account).then(|| {
                if created {
                    // `IntegratedServer.initServer` saves with the profile before the first
                    // tick: a world made under the account names it from its first level.dat.
                    level.player_uuid = Some(horizon_client::lan::playerdata::uuid_ints(account));
                } else if horizon_client::lan::playerdata::migrate_owner(
                    &save_root, &mut level, old, account,
                ) {
                    log::info!(
                        "Horizon: \"{folder}\" was made under the offline profile; \
                         its owner moved to the account's UUID"
                    );
                }
                (old, account)
            })
        });
        // CreateWorldScreen's choices, before streaming reads the generator.
        if created && let Some(o) = create {
            level.hardcore = o.hardcore;
            level.difficulty = horizon_client::title::DIFFICULTIES[o.difficulty()].into();
            level.allow_commands = Some(o.commands(creative));
            o.write_world_gen(&save_root, level.seed)?;
            // The DataPackConfig the screen chose: the picked packs on,
            // everything else disabled — the feature packs left out, and the
            // world packs the Data Packs screen held back (`install` copies
            // every pack of the folder, selected or not).
            let packs = horizon_client::datapack::feature_packs(&options.resources);
            level.data_packs = Some({
                let mut disabled: Vec<String> = packs
                    .iter()
                    .map(|p| p.id.clone())
                    .filter(|f| !o.data_packs.contains(f))
                    .collect();
                if let Some(dir) = o.pack_dir.as_deref() {
                    disabled.extend(
                        horizon_client::packs::folder(
                            std::path::Path::new(dir),
                            horizon_core::version::DATA_PACK_FORMAT,
                        )
                        .into_iter()
                        .map(|p| p.id)
                        .filter(|id| !o.data_packs.contains(id)),
                    );
                }
                (o.data_packs.clone(), disabled)
            });
            // WorldDataConfiguration's `enabled_features`: the flags of the
            // picked feature packs over vanilla's (`FeatureFlags.REGISTRY`'s
            // codec writes the whole set, `minecraft:vanilla` included, and
            // omits the tag when it is the default).
            level.enabled_features = o.enabled_features(&options.resources).into();
            // CreateWorldScreen copies the edited rules with the final flags,
            // even when the player never opened the rules screen.
            level.game_rules = o
                .rules
                .copy_with_features(&level.enabled_features)
                .by_name();
        }
        // Worlds made without the Create New World screen (--world, the
        // control socket) take its default: commands on for Creative only.
        if created {
            level.allow_commands.get_or_insert(creative);
        }
        if let Some(d) = &options.dimension {
            level.dimension = d.clone();
            level.position = None;
        }
        // Bone meal's and saplings' features, over the world's data packs like its generator:
        // a pack's tree is a feature here too. The same entries, so the bundle loads once.
        growth::install_live_features(
            res,
            pack,
            options,
            &live_feature_overrides(
                &save_root,
                &options.resources,
                res.biomes(),
                &level.enabled_features,
                &pack.additional_data,
            ),
        );
        let shapes = level_shapes(
            &save_root,
            &options.resources,
            res.biomes(),
            &pack.additional_data,
        );
        Dim::set_extra(shapes.extra.iter().map(|level| level.id));
        // The bounded registry-loader leaf slice: fail world preparation before streaming,
        // rather than letting the flat-generator fallback swallow a selected pack's codec error.
        let (overrides, _) = worldgen_overrides_of(
            &save_root,
            &options.resources,
            res.biomes(),
            &level.enabled_features,
            &pack.additional_data,
        );
        overrides.validate_fill_layers(pack.assets.registry)?;
        let dim = Dim::from_id(&level.dimension);
        let shape = shapes.get(dim.id());
        let streaming_step = startup::step("world.make_streaming");
        let loot = horizon_client::structure_blocks::LootSnapshot::default();
        let (mut streaming, save, locator) = make_streaming(
            res,
            pack,
            options,
            settings,
            &save_root,
            dim,
            shape,
            level.seed,
            &level.enabled_features,
            loot.clone(),
        )?;
        drop(streaming_step);
        streaming.radius = settings.render_distance;
        let mut world = World::new(res.facts().air, level.seed);
        world.attachments = save.load_attachments();
        world.shape = shape;
        world.fast_lava = horizon_world::dimension_type::DimensionFacts::of_level(
            horizon_core::LevelKey::parse(dim.id()).unwrap_or(horizon_core::LevelKey::OVERWORLD),
        )
        .fast_lava;
        world.game_time = level.time;
        world.day_time = options.time.unwrap_or(level.day_time);
        world.end_clock = level.end_clock.unwrap_or(level.time);
        world.weather = level.weather;
        if let Some(w) = options.weather.as_deref() {
            // /weather: clear, rain or thunder for a long duration.
            let (rain, thunder) = (w != "clear", w == "thunder");
            world.weather = horizon_client::world::Weather {
                clear_time: if w == "clear" { 6000 } else { 0 },
                rain_time: 6000,
                thunder_time: 6000,
                raining: rain,
                thundering: thunder,
                rain_level: if rain { (1.0, 1.0) } else { (0.0, 0.0) },
                thunder_level: if thunder { (1.0, 1.0) } else { (0.0, 0.0) },
            };
        }

        // Persist new metadata on the worker too, before publishing the world.
        if !save_root.join("level.dat").exists() {
            level.write(&save_root)?;
        }
        Ok(PreparedWorld {
            save_root,
            shapes,
            level,
            world,
            streaming,
            save,
            session_lock,
            rand_sequences,
            locator,
            loot,
            owner_migration,
        })
    }
}

pub(super) struct WorldLoader {
    handle: JoinHandle<std::result::Result<PreparedWorld, String>>,
}

impl WorldLoader {
    /// Prepare the world on its own thread. `upgrade` is what its upgrade has done
    /// (`UpgradeProgress`), which the window's `FileFixerProgressScreen` shows while the
    /// file fixers run, and where its Cancel stops them.
    pub fn start(
        request: WorldRequest,
        res: Arc<Resources>,
        pack: PackResources,
        options: Options,
        settings: Settings,
        upgrade: Arc<horizon_world::datafix::progress::UpgradeProgress>,
    ) -> Result<Self> {
        let handle = std::thread::Builder::new()
            .name("World initialization".into())
            .spawn(move || {
                request
                    .prepare(&res, &pack, &options, &settings, &upgrade)
                    .map_err(|e| e.to_string())
            })?;
        Ok(Self { handle })
    }

    pub fn finish(self) -> Result<PreparedWorld> {
        self.handle
            .join()
            .map_err(|_| "world initialization worker panicked")?
            .map_err(Into::into)
    }
}

/// How long the preloaded world data outlives the world screens: a player
/// who goes back to the title or to Multiplayer and opens no world for this
/// long gets the memory back.
const PRELOAD_KEPT: std::time::Duration = std::time::Duration::from_secs(60);

enum Preload {
    Idle,
    /// Read, or being read, on the "World data preload" thread. `left`: when
    /// the player last left the world screens.
    Started {
        thread: std::thread::JoinHandle<()>,
        left: Option<std::time::Instant>,
    },
    /// A world opened: the data is the world's now, and stays.
    Used,
}

static PRELOAD: std::sync::Mutex<Preload> = std::sync::Mutex::new(Preload::Idle);

/// Each frame of the title: read what every world opening loads and keeps
/// for the rest of the process (the worldgen bundle, the loot tables and the
/// registries the server sends) on a helper thread while the player is on the
/// world screens (`on_world_screens`: Singleplayer, Create World and their
/// dialogs), so it overlaps the time spent choosing a world. A world opened
/// meanwhile waits for the part being read instead of reading it again. Not
/// at the title screen itself: many players only go to Multiplayer, and
/// small machines need the memory. Dropped after `PRELOAD_KEPT` away from
/// the world screens without a world opening.
pub(super) fn preload_world_data(
    on_world_screens: bool,
    resources: &std::path::Path,
    data_source: &horizon_assets::resource_source::ResourceSource,
    registry: &'static horizon_world::registry::BlockRegistry,
    item_data: &Arc<ItemData>,
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    match &mut *preload {
        Preload::Used => {}
        Preload::Idle if on_world_screens => {
            let resources = resources.to_owned();
            let data_source = data_source.clone();
            let item_data = Arc::clone(item_data);
            let spawned = std::thread::Builder::new()
                .name("World data preload".into())
                .spawn(move || {
                    horizon_threads::behind_server();
                    // Errors are left for the world opening to report.
                    if let Ok(data) = horizon_client::vanillagen::shared_data_from(
                        &resources.join("worldgen"),
                        &Default::default(),
                        registry,
                        &data_source,
                    ) {
                        // The Overworld's feature decoration, which every
                        // seed shares: about 140 ms off opening the world.
                        let _ = data.prepare_decoration("minecraft:normal", "minecraft:overworld");
                    }
                    // The loot data's item tables and the server's data both
                    // read the recipe folder: held here, it is read once.
                    let _recipe_files =
                        horizon_client::json_folder::read(&resources.join("data/minecraft/recipe"));
                    // The preload is the game's own data; the world's packs'
                    // tables are built when it opens (finish_open_world).
                    let _ = horizon_client::structure_blocks::loot_data(
                        &resources,
                        &horizon_world::loot::PackOverlay::NONE,
                        item_data.defaults(),
                    );
                });
            match spawned {
                Ok(thread) => *preload = Preload::Started { thread, left: None },
                Err(e) => log::warn!("Horizon: world data preload: {e}"),
            }
        }
        Preload::Idle => {}
        Preload::Started { left, .. } if on_world_screens => *left = None,
        Preload::Started { thread, left } => {
            let left = *left.get_or_insert_with(std::time::Instant::now);
            if left.elapsed() >= PRELOAD_KEPT && thread.is_finished() {
                horizon_client::vanillagen::forget_shared_data();
                horizon_client::structure_blocks::forget_loot_data();
                super::lan::forget_sync_data();
                super::lan::forget_server_sources();
                horizon_threads::release_free_memory();
                *preload = Preload::Idle;
                log::info!("Horizon: preloaded world data dropped, no world opened");
            }
        }
    }
}

/// A launch that names a world (`--world`, `--quickPlaySingleplayer`): the
/// same preload, started as soon as the block registry and the biomes are
/// loaded, so it overlaps the rest of the startup instead of the world's
/// opening. Read for the world's own data packs and preset, the bundle and
/// decoration `make_streaming` asks for; a world they don't fit (one its
/// upgrade rewrites) reads its own when it opens.
pub(super) fn preload_launch_world(
    folder: &str,
    resources: &std::path::Path,
    data_source: &horizon_assets::resource_source::ResourceSource,
    registry: &'static horizon_world::registry::BlockRegistry,
    biomes: Arc<horizon_client::biome::Biomes>,
    item_data: Arc<ItemData>,
    additional: &[Arc<horizon_client::datapack::DataPack>],
) {
    let mut preload = PRELOAD.lock().unwrap_or_else(|e| e.into_inner());
    if !matches!(*preload, Preload::Idle) {
        return;
    }
    let Ok(dir) = std::env::current_dir() else {
        return;
    };
    let world_root = dir.join("saves").join(folder);
    let resources = resources.to_owned();
    let data_source = data_source.clone();
    let additional = additional.to_vec();
    let spawned = std::thread::Builder::new()
        .name("World data preload".into())
        .spawn(move || {
            // Not behind the server: the world's opening waits for it.
            let _step = startup::step("world.preload");
            let started = std::time::Instant::now();
            // Errors are left for the world opening to report. The world's
            // flags from its level.dat: the feature packs a flag forces on.
            let enabled_features = horizon_client::save::Level::read(&world_root)
                .map(|l| l.enabled_features)
                .unwrap_or_default();
            let (overrides, _) = worldgen_overrides_of(
                &world_root,
                &resources,
                &biomes,
                &enabled_features,
                &additional,
            );
            if let Ok(data) = horizon_client::vanillagen::shared_data_from(
                &resources.join("worldgen"),
                &overrides,
                registry,
                &data_source,
            ) {
                let preset = horizon_client::save::WorldGen::preset(&world_root);
                let _ = data.prepare_decoration(preset, "minecraft:overworld");
            }
            // As in `preload_world_data`: the loot data and the server's
            // data share one read of the recipe folder.
            let _recipe_files =
                horizon_client::json_folder::read(&resources.join("data/minecraft/recipe"));
            let _ = horizon_client::structure_blocks::loot_data(
                &resources,
                &horizon_world::loot::PackOverlay::NONE,
                item_data.defaults(),
            );
            log::info!(
                "Horizon: world data preloaded for the launch's world in {} ms",
                started.elapsed().as_millis()
            );
        });
    match spawned {
        Ok(thread) => *preload = Preload::Started { thread, left: None },
        Err(e) => log::warn!("Horizon: world data preload: {e}"),
    }
}

/// A world opens: what `preload_world_data` read is the world's from now on.
pub(super) fn world_data_used() {
    *PRELOAD.lock().unwrap_or_else(|e| e.into_inner()) = Preload::Used;
}

/// A server's world opens: the loot tables its first placed block or broken
/// container needs (`loot_data`) are read on a helper thread while the
/// connection is made, so the game thread never waits on them (~0.6 s on a
/// phone). Kept from then on, as a local world keeps them.
pub(super) fn preload_server_loot(resources: &std::path::Path, item_data: &Arc<ItemData>) {
    world_data_used();
    if horizon_client::structure_blocks::loaded_loot_data(item_data.defaults()).is_some() {
        return;
    }
    let resources = resources.to_owned();
    let item_data = Arc::clone(item_data);
    let spawned = std::thread::Builder::new()
        .name("Loot preload".into())
        .spawn(move || {
            horizon_threads::behind_server();
            let _ = horizon_client::structure_blocks::loot_data(
                &resources,
                &horizon_world::loot::PackOverlay::NONE,
                item_data.defaults(),
            );
        });
    if let Err(e) = spawned {
        log::warn!("Horizon: loot preload: {e}");
    }
}

/// The game's biomes with the world's enabled packs over them, from the same
/// pack list its worldgen is built from (`WorldgenOverrides` keeps only the
/// biomes this table knows).
pub(super) fn packed_biomes(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    base: &Arc<horizon_client::biome::Biomes>,
    enabled_features: &[String],
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> Arc<horizon_client::biome::Biomes> {
    let packs = super::pack_finders::data_repository(world_root, resources, additional);
    let enabled = horizon_client::datapack::enabled_ids(world_root, &packs, enabled_features);
    let enabled: Vec<_> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    let (overrides, _) = horizon_client::datapack::worldgen_overrides(&enabled, |_| true);
    // The mods' spawns too: their biome modifiers go over the packs' biomes.
    let overrides = horizon_client::vanillagen::biome_modifiers_over_packs(
        &resources.join("worldgen"),
        &overrides,
    )
    .unwrap_or(overrides);
    let packed: Vec<(String, serde_json::Value)> = overrides
        .entries("biomes")
        .filter_map(|(id, json)| Some((id.to_owned(), serde_json::from_str(json).ok()?)))
        .collect();
    if packed.is_empty() {
        return base.clone();
    }
    Arc::new(base.with_packs(packed.iter().map(|(i, v)| (i.as_str(), v))))
}
/// The worldgen entries the world's generator reads, for the live feature
/// registry: the same biome table and packs as `make_streaming`'s, so both share
/// one loaded bundle.
fn live_feature_overrides(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    base: &Arc<horizon_client::biome::Biomes>,
    enabled_features: &[String],
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> horizon_world::worldgen::overrides::WorldgenOverrides {
    let biomes = packed_biomes(world_root, resources, base, enabled_features, additional);
    worldgen_overrides_of(world_root, resources, &biomes, enabled_features, additional).0
}
/// The Create screen's chosen packs' worldgen entries over the game's own
/// (`applyNewPackConfig` reloads the world with the new pack config): the
/// packs the temp folder and the resources root hold, the selection the
/// Create screen made.
pub(super) fn creation_worldgen_overrides(
    pack_dir: Option<&std::path::Path>,
    resources: &std::path::Path,
    data_packs: &[String],
    biomes: &horizon_client::biome::Biomes,
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> horizon_world::worldgen::overrides::WorldgenOverrides {
    let empty = std::path::PathBuf::new();
    let packs =
        super::pack_finders::data_repository(pack_dir.unwrap_or(&empty), resources, additional);
    let enabled: Vec<_> = data_packs
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    let (overrides, _) =
        horizon_client::datapack::worldgen_overrides(&enabled, |b| biomes.contains(b));
    overrides
}
/// The worldgen entries of the world's enabled data packs, logging what
/// the generator leaves out of them.
fn world_worldgen_overrides(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    biomes: &horizon_client::biome::Biomes,
    enabled_features: &[String],
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> horizon_world::worldgen::overrides::WorldgenOverrides {
    let (overrides, warnings) =
        worldgen_overrides_of(world_root, resources, biomes, enabled_features, additional);
    for warning in warnings {
        log::warn!("Horizon: data pack worldgen: {warning}");
    }
    overrides
}

/// The worldgen entries of the world's enabled data packs, and what the
/// generator leaves out of them.
fn worldgen_overrides_of(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    biomes: &horizon_client::biome::Biomes,
    enabled_features: &[String],
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> (
    horizon_world::worldgen::overrides::WorldgenOverrides,
    Vec<String>,
) {
    let packs = super::pack_finders::data_repository(world_root, resources, additional);
    let enabled = horizon_client::datapack::enabled_ids(world_root, &packs, enabled_features);
    let enabled: Vec<_> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    horizon_client::datapack::worldgen_overrides(&enabled, |b| biomes.contains(b))
}

/// The shapes of the world's levels: their dimension types, the world's
/// enabled packs' stems, then saved preset/editor stems, over vanilla's. A refused type keeps the
/// vanilla shape, with a warning.
pub(super) fn level_shapes(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    biomes: &horizon_client::biome::Biomes,
    additional: &[Arc<horizon_client::datapack::DataPack>],
) -> horizon_client::chunk::LevelShapes {
    let packs = super::pack_finders::data_repository(world_root, resources, additional);
    let enabled_features = horizon_client::save::Level::read(world_root)
        .map(|l| l.enabled_features)
        .unwrap_or_default();
    let enabled = horizon_client::datapack::enabled_ids(world_root, &packs, &enabled_features);
    let enabled: Vec<_> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    // What the packs leave out is logged when the generator reads them.
    let (mut overrides, _) =
        horizon_client::datapack::worldgen_overrides(&enabled, |b| biomes.contains(b));
    horizon_client::save::merge_saved_world_gen_dimensions(world_root, &mut overrides);
    let shapes = shapes_of(&overrides);
    set_level_sea_levels(
        &overrides,
        horizon_world::worldgen::overrides::mod_dimensions(),
        resources,
    );
    shapes
}

/// The sea levels of the levels whose stem the mods or the data packs give: a mod's own
/// (`DimensionSettings::sea_level`), else the `sea_level` of the noise settings their stem names,
/// the packs' over the game's. The game's spawning, weather and maps then use them. A level of a
/// generator that reads no noise settings has its built-in level's, and the three built-in levels
/// too unless the packs replace their stem or their noise settings.
fn set_level_sea_levels(
    overrides: &horizon_world::worldgen::overrides::WorldgenOverrides,
    mods: &[horizon_world::worldgen::overrides::ModDimension],
    resources: &std::path::Path,
) {
    use horizon_world::worldgen::overrides::{DIMENSIONS, NOISE_SETTINGS, stem_sea_level};
    let read = |id: &str| -> Option<serde_json::Value> {
        if let Some((_, json)) = overrides
            .entries(NOISE_SETTINGS)
            .find(|(entry, _)| *entry == id)
        {
            return serde_json::from_str(json).ok();
        }
        let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let file = resources.join(format!(
            "data/{namespace}/worldgen/noise_settings/{path}.json"
        ));
        serde_json::from_slice(&std::fs::read(file).ok()?).ok()
    };
    let mut levels: Vec<(horizon_core::LevelKey, i32)> = Vec::new();
    for (id, stem) in overrides.entries(DIMENSIONS) {
        let own = mods.iter().find(|d| d.id == id).and_then(|d| d.sea_level);
        // The noise settings are what the terrain is built with; `sea_level` of the mod is for
        // a generator that has none.
        let stem = serde_json::from_str::<serde_json::Value>(stem).ok();
        // A superflat generator builds at -63 (`FlatLevelSource.getSeaLevel`) whatever its level
        // is told; a mod's explicit sea level still wins for it.
        let flat = stem
            .as_ref()
            .is_some_and(|stem| stem["generator"]["type"].as_str() == Some("minecraft:flat"));
        let sea = stem
            .and_then(|stem| stem_sea_level(&stem, &read))
            .or(own)
            .or(flat.then_some(-63));
        if let (Some(sea), Ok(key)) = (sea, horizon_core::LevelKey::parse(id)) {
            levels.push((key, sea));
        }
    }
    // A built-in level the packs gave no stem: its default noise settings, which a pack may replace.
    for (id, settings) in [
        ("minecraft:overworld", "minecraft:overworld"),
        ("minecraft:the_nether", "minecraft:nether"),
        ("minecraft:the_end", "minecraft:end"),
    ] {
        let Ok(key) = horizon_core::LevelKey::parse(id) else {
            continue;
        };
        if levels.iter().any(|(level, _)| *level == key) {
            continue;
        }
        let replaced = overrides
            .entries(NOISE_SETTINGS)
            .any(|(entry, _)| entry == settings);
        let sea = replaced
            .then(|| read(settings))
            .flatten()
            .and_then(|json| i32::try_from(json["sea_level"].as_i64()?).ok());
        if let Some(sea) = sea {
            levels.push((key, sea));
        }
    }
    horizon_world::level::set_sea_levels(levels);
}

/// The levels' shapes the mods' and packs' worldgen entries give.
fn shapes_of(
    overrides: &horizon_world::worldgen::overrides::WorldgenOverrides,
) -> horizon_client::chunk::LevelShapes {
    use horizon_client::chunk::{ExtraLevel, LevelShape, LevelShapes};
    let mut shapes = LevelShapes::VANILLA;
    for level in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        let shape = overrides.level_height(level).and_then(|h| {
            LevelShape::new(
                h.min_y,
                h.height,
                h.logical_height,
                h.has_skylight,
                h.has_ceiling,
            )
            .ok()
        });
        match shape {
            Some(shape) => *shapes.get_mut(level) = shape,
            None => log::warn!(
                "Horizon: data pack worldgen: the dimension type of {level} is not one the \
                 game accepts; the level keeps the vanilla build height"
            ),
        }
    }
    let extra: Vec<_> = overrides
        .entries(horizon_world::worldgen::overrides::DIMENSIONS)
        .filter(|(level, _)| !Dim::is_vanilla(level))
        .filter_map(|(level, stem)| {
            let shape = overrides.level_height(level).and_then(|h| {
                LevelShape::new(
                    h.min_y,
                    h.height,
                    h.logical_height,
                    h.has_skylight,
                    h.has_ceiling,
                )
                .ok()
            });
            if shape.is_none() {
                log::warn!(
                    "Horizon: data pack worldgen: the dimension type of {level} is not one the \
                     game accepts; the level is left out"
                );
            }
            // A stem's `type` is a dimension type's id, or the type itself.
            let stem = serde_json::from_str::<serde_json::Value>(stem).ok()?;
            let dimension_type = stem["type"].as_str().unwrap_or(level);
            Some(ExtraLevel {
                id: level.to_owned().leak(),
                dimension_type: dimension_type.to_owned().leak(),
                shape: shape?,
            })
        })
        .collect();
    shapes.extra = extra.leak();
    set_level_types(overrides, &shapes);
    shapes
}

/// The dimension types the mods and packs changed or added, for the levels `shapes` holds: what
/// `DimensionFacts::of_level` answers for them. A type the game refuses keeps the level's own.
fn set_level_types(
    overrides: &horizon_world::worldgen::overrides::WorldgenOverrides,
    shapes: &horizon_client::chunk::LevelShapes,
) {
    use horizon_core::LevelKey;
    let levels = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ]
    .into_iter()
    .chain(shapes.extra.iter().map(|level| level.id));
    let mut facts = Vec::new();
    for level in levels {
        match overrides.level_facts(level) {
            Some(Ok(found)) => facts.extend(LevelKey::parse(level).ok().map(|key| (key, found))),
            Some(Err(why)) => log::warn!(
                "Horizon: data pack worldgen: the dimension type of {level} sets a field the game                  refuses ({why}); the level keeps the type it would have without it"
            ),
            None => {}
        }
    }
    horizon_world::dimension_type::DimensionFacts::set_levels(facts);
    // The attributes and timelines (the sky, fog and cloud over the day) of the levels a mod or a
    // pack added.
    let attributes = shapes
        .extra
        .iter()
        .filter_map(|level| {
            let (_, json) = overrides.level_type_json(level.id)?;
            let named = json.get("attributes").is_some() || json.get("timelines").is_some();
            named.then(|| (level.id.to_owned(), json))
        })
        .collect();
    horizon_client::environment::Environment::set_level_attributes(attributes);
}

/// The terrain of a level whose stem names a generator a mod registered, the
/// stem being read as the game's own generator reads it: a pack's or mod's,
/// else the saved one. A stem naming any other generator is left to that
/// generator, which refuses all but noise.
fn mod_terrain(
    world_root: &std::path::Path,
    resources: &std::path::Path,
    additional: &[Arc<horizon_client::datapack::DataPack>],
    enabled_features: &[String],
    registry: &'static horizon_world::registry::BlockRegistry,
    biomes: &horizon_client::biome::Biomes,
    dim: Dim,
    shape: horizon_client::chunk::LevelShape,
    seed: i64,
) -> Option<horizon_client::modgen::ModTerrain> {
    use horizon_world::worldgen::{mod_generators::mod_chunk_generators, overrides::DIMENSIONS};
    let generators = mod_chunk_generators();
    if generators.is_empty() {
        return None;
    }
    let (overrides, _) =
        worldgen_overrides_of(world_root, resources, biomes, enabled_features, additional);
    let stem = overrides
        .entries(DIMENSIONS)
        .find(|(level, _)| *level == dim.id())
        .and_then(|(_, stem)| serde_json::from_str(stem).ok())
        .or_else(|| {
            horizon_client::save::world_gen_settings(world_root)
                .and_then(|s| s.get("dimensions")?.get(dim.id()).cloned())
                .map(|stem| horizon_client::scoreboard::tag_to_json(&stem))
        })?;
    horizon_client::modgen::ModTerrain::for_stem(
        &stem,
        generators,
        registry,
        |biome| biomes.id(biome),
        shape,
        seed,
    )
}

pub(super) fn make_streaming(
    res: &Resources,
    pack: &PackResources,
    options: &Options,
    settings: &Settings,
    world_root: &std::path::Path,
    dim: Dim,
    shape: horizon_client::chunk::LevelShape,
    seed: i64,
    enabled_features: &[String],
    loot: horizon_client::structure_blocks::LootSnapshot,
) -> Result<(Streaming, Arc<SaveDir>, Option<Arc<VanillaTerrain>>)> {
    let mut save = SaveDir::new(
        horizon_client::save::dimension_dir(world_root, dim.id()),
        shape,
    );
    save.enable_trusted_fluid_load();
    save.set_loot_source(loot.clone());
    let save = Arc::new(save);
    if dim == Dim::End {
        // Migrate before any chunk reader can cache the old block entities. A side file it
        // can't read stays where it is and costs only its old exits: the End still opens.
        if let Err(e) = save.migrate_end_gateway_exits() {
            log::warn!("Horizon: End gateway exits not migrated: {e}");
        }
    }
    // The world's packs over the game's biomes (`RegistryDataLoader`
    // layering): the generator names the ids and the client draws them.
    let biomes = packed_biomes(
        world_root,
        &options.resources,
        res.biomes(),
        enabled_features,
        &pack.additional_data,
    );
    let worldgen_dir = options.resources.join("worldgen");
    let terrain_resources = TerrainResources {
        data: Some(&pack.data_changes),
        dir: &worldgen_dir,
        registry: pack.assets.registry,
        biomes: &biomes,
        item_data: &res.core.item_data,
    };
    let mut locator = None;
    let flat = (dim == Dim::Overworld)
        .then(|| horizon_client::save::world_gen_settings(world_root))
        .flatten()
        .and_then(|s| {
            horizon_client::flatgen::FlatTerrain::from_settings(
                &s,
                pack.assets.registry,
                res.biomes(),
                shape,
            )
        });
    // `DebugLevelSource`: the overworld of the Debug preset.
    let debug = (dim == Dim::Overworld)
        .then(|| horizon_client::save::world_gen_settings(world_root))
        .flatten()
        .and_then(|s| {
            horizon_client::debugworld::DebugTerrain::from_settings(
                &s,
                pack.assets.registry,
                res.biomes(),
                shape,
            )
        });
    // PlaceCommand.placeJigsaw/placeStructure generate directly, independently
    // of WorldOptions.generateStructures. Keep their flat locator available
    // even when automatic structure starts are disabled.
    let automatic_structures =
        horizon_client::save::WorldGen::read(world_root).is_none_or(|w| w.structures);
    let flat_settings = (dim == Dim::Overworld)
        .then(|| horizon_client::save::world_gen_settings(world_root))
        .flatten()
        .and_then(|s| horizon_client::flatgen::FlatTerrain::pipeline_settings(&s));
    // `adjustGenerationSettings`: a superflat world with features or lakes is decorated
    // whether or not its structures generate.
    let flat_decorates = flat_settings
        .as_ref()
        .is_some_and(|s| s.features || s.lakes);
    // A generator that cannot be built (a layer block the registry lacks, a level shorter
    // than the noise) leaves the world on its flat ground, as it was before.
    let flat_vanilla = flat_settings.and_then(|settings| {
        let overrides = world_worldgen_overrides(
            world_root,
            &options.resources,
            &biomes,
            enabled_features,
            &pack.additional_data,
        );
        match VanillaTerrain::for_flat(
            terrain_resources,
            &overrides,
            seed,
            &settings,
            automatic_structures,
        ) {
            Ok(mut terrain) => {
                terrain.set_loot_source(loot.clone());
                terrain.add_mod_signs(res.facts());
                Some(Arc::new(terrain))
            }
            Err(e) => {
                log::warn!("Horizon: the superflat structures and features are unavailable: {e}");
                None
            }
        }
    });
    // The sea level the level's rules and the mods read is the one its generator builds with
    // (`ChunkGenerator.getSeaLevel`), so a pack's noise settings or a superflat world are seen:
    // the superflat generators' is -63 (`FlatLevelSource.getSeaLevel`).
    let register_sea_level = |sea_level: i32| {
        if let Ok(key) = horizon_core::LevelKey::parse(dim.id()) {
            horizon_world::level::register_sea_level(key, sea_level);
        }
    };
    if flat.is_some() || flat_vanilla.is_some() {
        register_sea_level(-63);
    }
    let mut streaming = if let Some(inner) = debug {
        Streaming::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some(inner) = flat_vanilla
        .as_ref()
        .filter(|_| automatic_structures || flat_decorates)
        .cloned()
    {
        if automatic_structures {
            inner.search_rings_behind();
        }
        inner.set_reference_file(world_root.join("data").join(format!(
            "horizon_structure_references_{}.json",
            dim.id().replace(':', "_")
        )));
        {
            let save = save.clone();
            inner.set_saved_references(Arc::new(move |name, x, z| {
                save.start_references(x, z, name)
            }));
        }
        locator = Some(inner.clone());
        Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some(inner) = flat {
        // A superflat world that neither decorates nor generates structures: its
        // layers alone. Only manual commands use the locator.
        locator = flat_vanilla;
        Streaming::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if options.test_terrain && dim == Dim::Overworld {
        let inner = Arc::new(TestTerrain::new(pack.assets.registry, res.biomes())?);
        Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else if let Some(inner) = mod_terrain(
        world_root,
        &options.resources,
        &pack.additional_data,
        enabled_features,
        pack.assets.registry,
        &biomes,
        dim,
        shape,
        seed,
    ) {
        Streaming::new(
            Arc::new(Persistent {
                inner: Arc::new(inner),
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    } else {
        // WorldOptions.generateStructures, from the world's settings.
        let structures =
            horizon_client::save::WorldGen::read(world_root).is_none_or(|w| w.structures);
        let preset = horizon_client::save::WorldGen::preset(world_root);
        let overrides = world_worldgen_overrides(
            world_root,
            &options.resources,
            &biomes,
            enabled_features,
            &pack.additional_data,
        );
        let saved_stem = horizon_client::save::world_gen_settings(world_root)
            .and_then(|s| s.get("dimensions")?.get(dim.id()).cloned())
            .map(|stem| horizon_client::scoreboard::tag_to_json(&stem));
        let mut inner = VanillaTerrain::for_world_with_settings(terrain_resources, &overrides, seed, preset, dim.id(), structures, saved_stem.as_ref()).map_err(|e| {
                if overrides.is_empty() {
                    format!("{e}\nPrepare the worldgen bundle with the launcher, or: horizon-prepare --prepare")
                } else {
                    format!("{e}\nThe world's data packs change its world generation: one of them may not be made for this version")
                }
            })?;
        inner.set_loot_source(loot);
        inner.add_mod_signs(res.facts());
        register_sea_level(inner.sea_level());
        let inner = Arc::new(inner);
        if dim == Dim::Overworld && structures {
            inner.search_rings_behind();
        }
        // Chunks an older version saved: the new chunks around them blend in.
        if dim == Dim::Overworld {
            let old = Arc::new(horizon_client::old_world::SavedOldWorld::new(
                save.clone(),
                pack.assets.clone(),
                res.biomes().clone(),
                inner.generator_data().clone(),
            ));
            // Saves of this folder take the old chunks' calculated blending data when
            // they are requested; a weak link, as the old world holds the folder.
            let weak = Arc::downgrade(&old);
            save.set_blending_lookup(Box::new(move |x, z| {
                weak.upgrade().and_then(|old| old.calculated(x, z))
            }));
            inner.set_old_world(old);
        }
        inner.set_reference_file(world_root.join("data").join(format!(
            "horizon_structure_references_{}.json",
            dim.id().replace(':', "_")
        )));
        {
            let save = save.clone();
            inner.set_saved_references(Arc::new(move |name, x, z| {
                save.start_references(x, z, name)
            }));
        }
        locator = Some(inner.clone());
        Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: biomes.clone(),
            }),
            pack.context_with(biomes.clone()),
            res.threads(),
        )
    };
    streaming.load_forced(
        save.load_forced_chunks(),
        save.load_mod_tickets(|id| mod_api::TicketController::from_id(id).map(|c| c.slot())),
    );
    streaming.radius = settings.render_distance;
    streaming.biome_blend = settings.biome_blend;
    streaming.smooth_lighting = settings.smooth_lighting;
    streaming.cutout_leaves = settings.cutout_leaves;
    streaming.opaque_ice = settings.horizon.opaque_ice;
    streaming.leaf_culling = settings
        .horizon
        .leaf_culling_effective(&settings.resource_packs);
    streaming.biome_zoom = BiomeZoom::new(obfuscate_seed(seed));
    streaming.sky_light = dim != Dim::Nether;
    streaming.nether_shading = dim == Dim::Nether;
    // A server draws nothing: neither the integrated server nor the
    // dedicated one meshes sections.
    streaming.mesh = options.execution_role.is_client();
    Ok((streaming, save, locator))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world whose level.dat does not read but whose level.dat_old does: only the
    /// dedicated server restores it unasked. Every role here carries the launch configuration
    /// (`options.dedicated`) a dedicated server and the integrated server thread both have, so
    /// it cannot be what decides.
    #[test]
    fn only_the_dedicated_role_takes_level_dat_old_unasked() {
        use execution_role::ExecutionRole::{Client, DedicatedServer, IntegratedServer};
        for (role, restored) in [
            (DedicatedServer, true),
            (IntegratedServer, false),
            (Client, false),
        ] {
            let dir = std::env::temp_dir().join(format!(
                "horizon-level-dat-old-{role:?}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            Level {
                name: "Recovered".into(),
                ..Level::default()
            }
            .write(&dir)
            .unwrap();
            std::fs::copy(dir.join("level.dat"), dir.join("level.dat_old")).unwrap();
            std::fs::write(dir.join("level.dat"), b"not a level").unwrap();
            let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
            options.dedicated = Some(Vec::new());
            options.execution_role = role;

            fall_back_to_level_dat_old_without_asking(options.execution_role, &dir, "world");

            assert_eq!(
                horizon_client::save::level_dat_issue(&dir, false).is_none(),
                restored,
                "{role:?}"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_levels_sea_level_comes_from_its_mod_or_its_noise_settings() {
        use horizon_client::datapack::worldgen_overrides_over;
        use horizon_world::worldgen::overrides::ModDimension;
        let level = |id: &str, stem: &str, sea_level| ModDimension {
            id: id.into(),
            stem: stem.into(),
            dimension_type: "{}".into(),
            sea_level,
        };
        let mods = [
            level(
                "test:own",
                r#"{"type":"test:own","generator":{"type":"test:floor"}}"#,
                Some(10),
            ),
            level(
                "test:inline",
                r#"{"type":"test:inline","generator":{"type":"minecraft:noise","settings":{"sea_level":20}}}"#,
                None,
            ),
            level(
                "test:plain",
                r#"{"type":"test:plain","generator":{"type":"test:floor"}}"#,
                None,
            ),
        ];
        let (overrides, _) = worldgen_overrides_over(&mods, &[], |_| true);
        set_level_sea_levels(&overrides, &mods, std::path::Path::new("."));
        let sea = |id: &str| {
            horizon_world::level::sea_level_of(horizon_core::LevelKey::parse(id).unwrap())
        };
        assert_eq!(sea("test:own"), 10);
        assert_eq!(sea("test:inline"), 20);
        assert_eq!(sea("test:plain"), 63);
        assert_eq!(sea("minecraft:the_nether"), 32);
        horizon_world::level::set_sea_levels(Vec::new());
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn creation_rule_selection_reaches_the_saved_world() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::gamerules::{GameRules, Rule, RuleRegistry, Value};
        let rule = Rule {
            name: "example:enabled".into(),
            category: "example:machines".into(),
            default: Value::Bool(true),
            min: 0,
            max: 0,
        };
        let registry =
            Arc::new(RuleRegistry::with_bool_rules(&["example:machines".into()], &[rule]).unwrap());
        let mut create = horizon_client::title::WorldOptions {
            rules: GameRules::with_registry(registry.clone(), &[]),
            ..Default::default()
        };
        assert!(create.rules.set("example:enabled", Value::Bool(false)));
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("creation-rules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        with_game(|game| {
            let request = WorldRequest {
                owner: None,
                save_root: dir.clone(),
                folder: "test".into(),
                name: Some("test".into()),
                seed: Some(1),
                creative: true,
                create: Some(create.clone()),
                rule_registry: registry.clone(),
            };
            assert!(Arc::ptr_eq(
                request.create.as_ref().unwrap().rules.registry(),
                &request.rule_registry
            ));
            game.load_world(request).unwrap();
            assert_eq!(
                game.ui.rules.get("example:enabled"),
                Some(Value::Bool(false))
            );
            assert!(Arc::ptr_eq(&game.level.rule_registry, &registry));
            game.save_game_with(true).unwrap();
        });
        assert_eq!(
            GameRules::load_with_registry(&dir, &[], registry).get("example:enabled"),
            Some(Value::Bool(false))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// NeoForge's `ModMismatchEvent`: a world saved with version 1 of a mod and
    /// opened with version 2 fires `mod_mismatch` once with both versions, a
    /// mod no longer loaded with no current version; the mod that marks its
    /// own mismatch resolved gets no warning while the missing one does. The
    /// save records the loaded list, so the reopened world fires nothing, as a
    /// world without the list doesn't.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_world_saved_with_other_mod_versions_fires_mod_mismatch() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::save::{
            Level,
            level_mods::{ModRecord, set_loaded},
        };
        use mod_api::{Hooks, HostSide, ModEntry, ModMismatch, ModMismatchEvent};
        use std::sync::{Arc, Mutex};

        struct Migrating(Arc<Mutex<Vec<Vec<ModMismatch>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Migrating {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                self.0.lock().unwrap().push(event.mismatches.to_vec());
                event.resolved.push("mismatch_test".to_owned().into());
            }
        }
        struct Erasing;
        #[mod_api::__subscriptions]
        impl Hooks for Erasing {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert_eq!(event.resolved[0].resolver, "mismatch_test");
                event.resolved.clear();
            }
        }
        struct Observing;
        #[mod_api::__subscriptions]
        impl Hooks for Observing {
            fn mod_mismatch(&self, _: HostSide, event: &mut ModMismatchEvent<'_>) {
                assert_eq!(event.resolved[0].resolver, "mismatch_test");
            }
        }
        let dir = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("mod-mismatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        horizon_client::logging::init(&dir, "main");
        let record = |id: &str, version: &str| ModRecord {
            id: id.into(),
            version: version.into(),
        };
        let mismatch = |mod_id: &str, saved: &str, current: Option<&str>| ModMismatch {
            mod_id: mod_id.into(),
            saved_version: saved.into(),
            current_version: current.map(str::to_owned),
        };
        let request = |root: &std::path::Path| {
            let mut request = WorldRequest::new(
                "test",
                Some("test"),
                Some(1),
                true,
                None,
                Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
            )
            .unwrap();
            request.save_root = root.to_owned();
            request
        };
        let warnings = || -> Vec<String> {
            horizon_client::logging::recent_lines()
                .into_iter()
                .filter(|line| line.contains("Horizon: mod "))
                .collect()
        };
        let heard = Arc::new(Mutex::new(Vec::new()));
        let level = || Level {
            name: "test".into(),
            ..Level::default()
        };
        // The loaded list is the whole process's: the test's goes away when it ends, even
        // when it fails, so the tests after it save no list.
        struct Restore(Option<Vec<ModRecord>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                set_loaded(self.0.take());
            }
        }
        // Before the loaded mods are known: a world without the list.
        let _restore = Restore(set_loaded(None));
        let plain = dir.join("plain");
        level().write(&plain).unwrap();
        assert_eq!(Level::read(&plain).unwrap().saved_mods, None);
        // Saved with version 1 of the mod, and with one since gone.
        let changed = dir.join("changed");
        set_loaded(Some(vec![
            record("mismatch_test", "1.0.0"),
            record("gone", "0.5.0"),
        ]));
        level().write(&changed).unwrap();
        set_loaded(Some(vec![record("mismatch_test", "2.0.0")]));
        mod_api::testing::with_mods(
            vec![
                ModEntry::new("mismatch_test", Box::new(Migrating(heard.clone()))),
                ModEntry::new("erasing", Box::new(Erasing)),
                ModEntry::new("observing", Box::new(Observing)),
            ]
            .into(),
            || {
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    let lines = warnings();
                    assert_eq!(
                        *heard.lock().unwrap(),
                        vec![vec![
                            mismatch("mismatch_test", "1.0.0", Some("2.0.0")),
                            mismatch("gone", "0.5.0", None),
                        ]]
                    );
                    assert!(
                        lines.iter().any(|line| line.contains(
                            "Horizon: mod gone was 0.5.0 when the world was saved, now missing"
                        )),
                        "{lines:?}"
                    );
                    assert!(
                        !lines.iter().any(|line| line.contains("mod mismatch_test")),
                        "{lines:?}"
                    );
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&changed).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
                // An unchanged list still fires.
                with_game(|game| {
                    game.load_world(request(&changed)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                });
                // A world without the list fires nothing and gets the list on its save.
                with_game(|game| {
                    game.load_world(request(&plain)).unwrap();
                    assert_eq!(heard.lock().unwrap().len(), 2);
                    game.save_game_with(true).unwrap();
                });
                assert_eq!(
                    Level::read(&plain).unwrap().saved_mods,
                    Some(vec![record("mismatch_test", "2.0.0")])
                );
            },
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mods_level_opens_with_no_data_packs() {
        use horizon_client::chunk::{LevelShape, LevelShapes};
        use horizon_client::datapack::worldgen_overrides_over;
        let (none, _) = worldgen_overrides_over(&[], &[], |_| true);
        assert_eq!(shapes_of(&none), LevelShapes::VANILLA);

        let pocket = horizon_world::worldgen::overrides::ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket","generator":{"type":"minecraft:noise"}}"#.into(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"logical_height":320,"has_skylight":false,"has_ceiling":true}"#
                    .into(),
            sea_level: None,
        };
        let (overrides, _) = worldgen_overrides_over(&[pocket], &[], |_| true);
        let shapes = shapes_of(&overrides);
        assert!(shapes.has("test:pocket"));
        assert_eq!(
            shapes.get("test:pocket"),
            LevelShape::new(-128, 448, 320, false, true).unwrap()
        );
        assert_eq!(shapes.dimension_type("test:pocket"), "test:pocket");
        assert_eq!(shapes.overworld, LevelShape::OVERWORLD);
    }

    /// A registered level's stem finds its generator through the world's
    /// worldgen entries; the Overworld's noise stays the game's. Ignored as it
    /// installs the process-wide tables.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_stem_naming_a_mods_generator_opens_on_mod_terrain() {
        use horizon_client::streaming::ChunkGenerator as _;
        use horizon_world::worldgen::{
            mod_generators::{
                ChunkAccess, ModChunkGenerator, ModChunkGenerators, install_mod_chunk_generators,
            },
            overrides::{ModDimension, install_mod_dimensions},
        };
        struct Floor;
        impl ModChunkGenerator for Floor {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let stone = chunk.state("stone");
                chunk.set_block([0, chunk.min_y(), 0], stone);
            }
        }
        install_mod_dimensions(vec![ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket","generator":{"type":"test:floor"}}"#.into(),
            dimension_type: r#"{"min_y":-128,"height":448,"logical_height":448}"#.into(),
            sea_level: None,
        }])
        .unwrap();
        let floor: Arc<dyn ModChunkGenerator> = Arc::new(Floor);
        install_mod_chunk_generators(ModChunkGenerators::from([("test:floor".to_owned(), floor)]))
            .map_err(|_| "installed twice")
            .unwrap();

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let biomes = horizon_client::biome::Biomes::load(
            &horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        )
        .unwrap();
        let registry = horizon_client::assets::Assets::load(&root)
            .unwrap()
            .registry;
        let world =
            std::env::temp_dir().join(format!("horizon-mod-terrain-{}", std::process::id()));
        std::fs::create_dir_all(&world).unwrap();
        let (overrides, _) = worldgen_overrides_of(&world, &root, &biomes, &[], &[]);
        let shapes = shapes_of(&overrides);
        let open = |dim: Dim| {
            let shape = shapes.get(dim.id());
            mod_terrain(&world, &root, &[], &[], registry, &biomes, dim, shape, 0)
        };
        let pocket = Dim::Extra(mod_api::__private::dimension_key("test:pocket"));
        let terrain = open(pocket).expect("the stem names a registered generator");
        let chunk = terrain.finish(terrain.proto(&mut (), 0, 0));
        assert_eq!(
            chunk.block(0, -128, 0),
            registry.default_state("minecraft:stone").unwrap()
        );
        assert!(open(Dim::Overworld).is_none());
        let _ = std::fs::remove_dir_all(world);
    }

    #[test]
    fn worker_panics_reach_the_client() {
        let loader = WorldLoader {
            handle: std::thread::spawn(|| panic!("broken world initialization")),
        };
        assert_eq!(
            loader.finish().err().unwrap().to_string(),
            "world initialization worker panicked"
        );
    }

    #[test]
    fn prepared_world_can_cross_threads() {
        fn send<T: Send>() {}
        send::<PreparedWorld>();
    }
}

#[cfg(test)]
mod unused_feature_startup_tests {
    use super::*;

    #[test]
    #[ignore = "requires prepared resources and scratch homes/cwd"]
    fn actual_world_prepare_rejects_unused_invalid_feature() {
        let resources = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(resources.clone()),
            Vec::new(),
            None,
            &resources,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = resources;
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        for (case, height) in [("valid", 0), ("invalid", -1)] {
            let root = std::env::current_dir().unwrap().join(case);
            let pack = root.join("datapacks/h259");
            std::fs::create_dir_all(pack.join("data/h259/worldgen/feature")).unwrap();
            std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"H259 unused feature","min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
            let json = format!(
                r#"{{"type":"minecraft:fill_layer","height":{height},"state":"minecraft:stone"}}"#
            );
            std::fs::write(
                pack.join("data/h259/worldgen/feature/unused_probe.json"),
                &json,
            )
            .unwrap();
            let mut level = Level {
                seed: 259,
                data_packs: Some((vec!["vanilla".into(), "file/h259".into()], vec![])),
                ..Level::default()
            };
            level.write(&root).unwrap();
            horizon_client::save::write_world_gen_settings(
                &root,
                259,
                horizon_client::save::WorldGen {
                    preset: horizon_client::save::WorldPreset::Flat,
                    structures: false,
                    bonus_chest: false,
                },
            )
            .unwrap();
            let (overrides, _) = worldgen_overrides_of(
                &root,
                &options.resources,
                loaded.resources.biomes(),
                &level.enabled_features,
                &loaded.pack.additional_data,
            );
            assert_eq!(
                overrides
                    .entries("features")
                    .find(|(id, _)| *id == "h259:unused_probe")
                    .unwrap()
                    .1,
                json
            );
            assert!(overrides.entries("placed_features").next().is_none());
            assert_eq!(
                horizon_client::save::WorldGen::read(&root).unwrap().preset,
                horizon_client::save::WorldPreset::Flat
            );
            let request = WorldRequest {
                save_root: root,
                folder: case.into(),
                name: None,
                seed: Some(259),
                creative: false,
                create: None,
                owner: None,
                rule_registry: Level::default().rule_registry,
            };
            let result = request.prepare(
                &loaded.resources,
                &loaded.pack,
                &options,
                &settings,
                &horizon_world::datafix::progress::UpgradeProgress::new(),
            );
            if height == 0 {
                assert!(result.is_ok(), "valid unused registry entry must prepare");
                drop(result);
            } else {
                let error = match result {
                    Ok(_) => panic!(
                        "unused invalid h259:unused_probe was accepted by actual world preparation"
                    ),
                    Err(error) => error.to_string(),
                };
                assert!(error.contains("h259:unused_probe"), "{error}");
            }
        }
    }
}

#[cfg(test)]
mod unused_feature_preservation_tests {
    use super::*;

    #[test]
    #[ignore = "requires prepared resources and scratch homes/cwd"]
    fn startup_leaf_preservation_other_id_and_unrelated_feature() {
        let resources = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(resources.clone()),
            Vec::new(),
            None,
            &resources,
            &settings,
            Instant::now(),
            false,
            execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = resources;
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let root = std::env::current_dir().unwrap().join("preservation");
        let pack = root.join("datapacks/h259");
        let entries = pack.join("data/controls/worldgen/feature");
        std::fs::create_dir_all(&entries).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"H259 preservation controls","min_format":[121,0],"max_format":[121,0]}}"#).unwrap();
        let leaf = r#"{"type":"minecraft:fill_layer","height":1,"state":"minecraft:stone"}"#;
        let unrelated = r#"{"type":"minecraft:no_op"}"#;
        std::fs::write(entries.join("another_id.json"), leaf).unwrap();
        std::fs::write(entries.join("unrelated.json"), unrelated).unwrap();
        let mut level = Level {
            seed: 259,
            data_packs: Some((vec!["vanilla".into(), "file/h259".into()], vec![])),
            ..Level::default()
        };
        level.write(&root).unwrap();
        horizon_client::save::write_world_gen_settings(
            &root,
            259,
            horizon_client::save::WorldGen {
                preset: horizon_client::save::WorldPreset::Flat,
                structures: false,
                bonus_chest: false,
            },
        )
        .unwrap();
        let (mut overrides, _) = worldgen_overrides_of(
            &root,
            &options.resources,
            loaded.resources.biomes(),
            &level.enabled_features,
            &loaded.pack.additional_data,
        );
        assert_eq!(
            overrides.entries("features").collect::<Vec<_>>(),
            vec![
                ("controls:another_id", leaf),
                ("controls:unrelated", unrelated),
            ]
        );
        // Labeled native preservation controls, not additional Java runtime coverage.
        overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap();
        // The existing winning-entry map still owns precedence, not the validation phase.
        overrides.set(
            "features",
            "controls:another_id",
            r#"{"type":"minecraft:fill_layer","height":-1,"state":"minecraft:stone"}"#.into(),
        );
        let error = overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap_err();
        assert!(error.to_string().contains("controls:another_id"));
        overrides.set("features", "controls:another_id", leaf.into());
        overrides
            .validate_fill_layers(loaded.pack.assets.registry)
            .unwrap();
        let request = WorldRequest {
            save_root: root,
            folder: "preservation".into(),
            name: None,
            seed: Some(259),
            creative: false,
            create: None,
            owner: None,
            rule_registry: Level::default().rule_registry,
        };
        assert!(
            request
                .prepare(
                    &loaded.resources,
                    &loaded.pack,
                    &options,
                    &settings,
                    &horizon_world::datafix::progress::UpgradeProgress::new(),
                )
                .is_ok(),
            "preservation: other-id leaf and unrelated no_op must still prepare"
        );
    }
}
