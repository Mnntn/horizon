//! Entering a world without blocking the window: the deferred entry, world creation, the world
//! resources' background load, and the world's own content tables.
use super::*;

impl Game<'_> {
    /// The server's data-pack enchantments, or the local world's registry.
    pub(super) fn item_enchantments(&self) -> &Arc<horizon_client::enchant::Enchantments> {
        match self.remote.as_ref().and_then(|r| r.enchantments.as_ref()) {
            Some(e) => e,
            None => &self.stations().enchantments,
        }
    }

    pub(super) fn defer_world_entry(&mut self, entry: PendingWorldEntry) -> Result<bool> {
        self.poll_world_rest()?;
        if self.rest_loader.is_none() {
            return Ok(false);
        }
        self.perf.world_opening();
        if self.title.is_none() {
            let title = Title::new(
                std::env::current_dir()?.join("saves"),
                &self.options.resources,
            )
            .with_user(|| self.options.player_name());
            self.show_title(title);
        }
        self.pending_entry = Some(entry);
        log::info!("Horizon: world entry waiting for world resources");
        Ok(true)
    }

    pub(super) fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_integrated_start()?;
        self.poll_world_rest()?;
        self.poll_world_creation();
        if self.rest_loader.is_some() {
            return Ok(());
        }
        let on_world_screens = self.title.as_ref().is_some_and(|title| {
            !matches!(
                title.mode,
                horizon_client::title::Mode::Main | horizon_client::title::Mode::Multiplayer(_)
            )
        });
        world_loading::preload_world_data(
            on_world_screens,
            &self.options.resources,
            &self.pack.data_changes,
            self.pack.assets.registry,
            &self.res.core.item_data,
        );
        if std::mem::take(&mut self.pending_pack_reload) {
            self.reload_packs();
        }
        match self.pending_entry.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                cookies,
                lan,
                realm,
            }) => {
                self.connect_server_with_options(&address, &name, cookies, lan, realm)?;
            }
            Some(PendingWorldEntry::StudioReplay(path)) => {
                let path_str = path.to_string_lossy().to_string();
                if let Err(e) = self.open_studio_replay(&path_str) {
                    log::warn!("Horizon Studio: could not open {path_str}: {e}");
                }
            }
            None => {}
        }
        Ok(())
    }

    /// `CreateWorldScreen.openCreateWorldScreen`: read the world generation
    /// data the create screen describes, on a worker, then open the screen
    /// (`title.preparing`).
    pub(super) fn poll_world_creation(&mut self) {
        let Some(title) = self.title.as_ref() else {
            return;
        };
        if !title.preparing() {
            return;
        }
        if self.creation_job.is_none() {
            let dir = self.options.resources.join("worldgen");
            let registry = self.pack.assets.registry;
            // The chosen packs' worldgen over the game's own:
            // `applyNewPackConfig` reloads with the new pack config.
            let overrides = title
                .creation_packs()
                .map(|(dir, packs)| {
                    world_loading::creation_worldgen_overrides(
                        dir.as_deref(),
                        &self.options.resources,
                        &packs,
                        self.res.biomes(),
                        &self.pack.additional_data,
                    )
                })
                .unwrap_or_default();
            let data_source = self.pack.data_changes.clone();
            self.creation_job = std::thread::Builder::new()
                .name("world-creation".into())
                .spawn(move || {
                    horizon_client::vanillagen::shared_data_from(
                        &dir,
                        &overrides,
                        registry,
                        &data_source,
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string())
                })
                .ok();
            return;
        }
        if !self
            .creation_job
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
        {
            return;
        }
        let outcome = self.creation_job.take().and_then(|job| job.join().ok());
        if let Some(title) = self.title.as_mut() {
            match outcome {
                Some(Ok(())) => title.finish_preparing(),
                // `applyNewPackConfig`'s failure: back out of the read with
                // the message, the Create screen's options intact.
                outcome => {
                    let e = outcome
                        .and_then(|r| r.err())
                        .unwrap_or_else(|| "the read never ran".to_owned());
                    log::warn!("Horizon: preparing world creation failed: {e}");
                    // Clear the flag first: `failed` sets the error screen
                    // with Create as its back target, which
                    // `finish_preparing` would replace with a fresh world.
                    title.finish_preparing();
                    title.failed(format!(
                        "{}: {e}",
                        horizon_client::lang::get_or(
                            "createWorld.preparing",
                            "Preparing for world creation..."
                        )
                    ));
                }
            }
        }
    }

    pub(super) fn poll_world_rest(&mut self) -> Result<()> {
        if let Some(loader) = &self.rest_loader
            && loader.ready()?
        {
            self.rest_loader
                .take()
                .ok_or("missing world loader")?
                .finish()?;
        }
        Ok(())
    }

    // ---- the world's data packs' content tables ---------------------------

    /// The content tables the world plays with: its data packs' over the
    /// game's own when it has packs, `Resources.rest`'s own otherwise.
    pub(super) fn items(&self) -> &Items {
        world_items!(self)
    }
    pub(super) fn mob_suffocation(
        &self,
    ) -> &horizon_world::suffocation::SuffocationRegistry<'static> {
        self.world_tables
            .as_ref()
            .map_or_else(|| self.res.suffocation(), |t| &t.suffocation)
    }
    pub(super) fn recipes(&self) -> &Recipes {
        match &self.world_tables {
            Some(t) => &t.recipes,
            None => self.synced_recipes().unwrap_or_else(|| self.res.recipes()),
        }
    }
    pub(super) fn stations(&self) -> &horizon_client::stations::StationData {
        match &self.world_tables {
            Some(t) => &t.stations,
            None => self.res.stations(),
        }
    }

    /// The world's packs' content files, none without packs.
    /// The biome table the world plays with, `None` before a world opens.
    pub(super) fn packed_biomes(&self) -> Option<Arc<Biomes>> {
        let table = world_loading::packed_biomes(
            &self.world_root,
            &self.options.resources,
            self.res.biomes(),
            &self.level.enabled_features,
            &self.pack.additional_data,
        );
        (!Arc::ptr_eq(&table, self.res.biomes())).then_some(table)
    }
    /// What the chunk and far-terrain workers mesh with: the pack's context,
    /// over the world's biomes when it has its own.
    pub(super) fn worker_context(&self) -> Arc<WorkerContext> {
        match &self.world_biomes {
            Some(table) => self.pack.context_with(table.clone()),
            None => self.pack.context.clone(),
        }
    }
    pub(super) fn pack_files(&self) -> horizon_world::loot::PackOverlay<'_> {
        pack_files(&self.world_tables)
    }
    /// Load the world's data packs' content tables (`None`: the world has no
    /// packs, and `Resources.rest` stays in use). This re-reads the tables,
    /// so it runs once per world open and on `/reload`.
    pub(super) fn build_world_tables(
        &mut self,
        stack: &functions::PackStack,
        reload: bool,
        root: &std::path::Path,
        _data_switched: bool,
    ) -> Result<(Option<WorldTables>, horizon_client::health::HealthResources)> {
        let phase = self.mod_phase.clone();
        let _reload = phase
            .reloading()
            .expect("server tables on their owning thread");
        // The ask of this load, both here and on `/reload`: a native mod
        // recomputes the pack files it contributes every time.
        let mut contributed = Vec::new();
        self.native_data_pack_files(&mut contributed);
        let files = functions::overlay_files(stack, contributed);
        let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
        let conditions = horizon_world::conditions::LoadConditions::new(inventory.server_ids());
        let overlay = horizon_world::loot::PackOverlay {
            conditions: &conditions,
            files: &files,
        };
        let mut data_map_types = Vec::new();
        self.native_data_map_types(&mut data_map_types);
        self.data_map_registries = native_data_maps::registries_of(&data_map_types);
        let base = if reload {
            &self.health_resources
        } else {
            &self.res.motion().health
        };
        let health = base.with_packs(root, &overlay, reload)?;
        // With no pack, `res` holds the tables, unless content mods switched
        // live since it was loaded: their data is in the new root.
        let mut items = Items::load(
            root,
            self.pack.assets.registry,
            self.res.facts(),
            &overlay,
            self.res.item_defaults(),
        )?;
        class_tags::install(&mut items, self.res.facts());
        api_values::resolve_enchantment_registry(&items)?;
        // Item damage predicates use the same frozen registry as player hurt.
        Arc::make_mut(&mut items.enchantments).bind_damage_catalog(&health.damage);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = horizon_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = if reload {
            lan::prepare_server_sources(root, &self.res.core.item_data, &overlay)?
        } else {
            lan::load_sources(
                root,
                &self.res.core.item_data,
                self.options.execution_role.data_load_origin(),
                &overlay,
            )?
        };
        let recipes = Recipes::from_resolved(root, &items, &overlay, &sources)?;
        let mut stations = horizon_client::stations::StationData::from_sources(
            root,
            &items,
            &overlay,
            self.res.item_defaults(),
            &sources,
        );
        self.prepare_professions(root, &items, &mut stations)?;
        let data_maps = native_data_maps::DataMaps::load(
            &data_map_types,
            &overlay,
            &items.item_tags,
            &items.block_tags,
        );
        let causes_suffocation = items
            .block_tags
            .get("minecraft:causes_suffocation")
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        let suffocation = horizon_client::mobs::suffocation::bind(
            self.res.core.collision.clone(),
            &causes_suffocation,
        )?;
        // A registry whose pack tags don't load keeps the game's own tags
        // (`in_tag` falls back to them), rather than matching no tag at all.
        let pack_tags = WorldTables::PACK_TAG_REGISTRIES
            .into_iter()
            .filter_map(|registry| {
                match horizon_client::items::load_tags(root, registry, &overlay) {
                    Ok(tags) => Some((registry, tags)),
                    Err(e) => {
                        log::warn!("Horizon: the world's {registry} tags are not applied: {e}");
                        None
                    }
                }
            })
            .collect();
        let datapack_candidate = self.datapack_candidate(stack, &conditions)?;
        self.mods.datapack_candidate = datapack_candidate;
        Ok((
            Some(WorldTables {
                conditions,
                suffocation,
                files,
                items,
                recipes,
                stations,
                reload_sources: reload.then_some(sources),
                data_maps,
                pack_tags,
            }),
            health,
        ))
    }
    /// The entries of the mods' datapack registries in the winning pack files: decoded with
    /// each registry's schema and checked as one candidate, so a bad entry fails the load
    /// before any table is published. `None` when no mod declared a registry: nothing is
    /// scanned. A file whose `neoforge:conditions` fail is absent; the key is dropped before
    /// the schema sees the file.
    fn datapack_candidate(
        &self,
        stack: &functions::PackStack,
        conditions: &horizon_world::conditions::LoadConditions,
    ) -> Result<Option<mod_api::DatapackCandidate>> {
        let declarations = mod_api::__private::datapack_registry_declarations();
        if declarations.is_empty() {
            return Ok(None);
        }
        let keys: Vec<(&str, &str)> = declarations
            .iter()
            .filter_map(|declaration| declaration.key.split_once(':'))
            .collect();
        // Content files come before the world's enabled packs, lowest priority first.
        let packs: Vec<_> = std::iter::once(&stack.content)
            .chain(stack.enabled_packs())
            .collect();
        let files: Vec<(String, Vec<Vec<u8>>)> =
            horizon_client::datapack::registry_files(&packs, &keys)
                .into_iter()
                .map(|(path, versions)| {
                    let versions = versions
                        .into_iter()
                        .filter_map(|bytes| {
                            let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&bytes)
                            else {
                                // Malformed: the decoder reports it, naming the file.
                                return Some(bytes);
                            };
                            if !conditions.allows(&json) {
                                return None;
                            }
                            if let Some(object) = json.as_object_mut() {
                                object.remove("neoforge:conditions");
                            }
                            serde_json::to_vec(&json).ok()
                        })
                        .collect();
                    (path, versions)
                })
                .collect();
        mod_api::DatapackCandidate::build(declarations, &files)
            .map(Some)
            .map_err(|error| {
                format!(
                    "datapack registry {} entry {} ({}): {:?}",
                    error.registry, error.entry, error.file, error.reason
                )
                .into()
            })
    }
    /// A world open's tables, and the first generation of the registries the mods declared.
    pub(super) fn open_world_tables(
        &mut self,
        stack: &functions::PackStack,
        root: &std::path::Path,
    ) -> Result<()> {
        (self.world_tables, self.health_resources) =
            self.build_world_tables(stack, false, root, self.mods.data_switched)?;
        self.install_profession_save_snapshots();
        self.mods.datapack_registries = self.mods.datapack_candidate.take().map(|candidate| {
            Arc::new(mod_api::DatapackRegistrySet::first(
                candidate,
                horizon_client::datapack_net::next_session(),
            ))
        });
        horizon_client::datapack_net::serve(self.mods.datapack_registries.clone());
        Ok(())
    }
    /// Sends each joined Horizon client the snapshot of the set just published, built from the
    /// offer owned by its connection. A refused replacement ends that connection with its reason.
    fn send_datapack_snapshots(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        for guest in host
            .hub
            .guests
            .iter()
            .filter(|g| g.placed && g.participates_online() && g.brand == "horizon")
        {
            horizon_client::datapack_net::reload_connection(&guest.sender);
        }
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    /// Publish tables before function admission, leaving advancement rewards queued
    /// until the reload's new function store is available.
    pub(super) fn install_world_tables(&mut self, prepared: PreparedWorldReload) {
        let PreparedWorldReload {
            structure_spawns,
            tables,
            health,
            parts,
            wire,
            datapack_registries,
        } = prepared;
        lan::forget_server_sources();
        if let Some(tables) = &tables
            && let Some(sources) = &tables.reload_sources
        {
            let overlay = horizon_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
            };
            lan::install_server_sources(
                &self.options.resources,
                &self.res.core.item_data,
                &overlay,
                sources.clone(),
            );
        }
        self.world_tables = tables;
        self.install_profession_save_snapshots();
        self.world.pois.clear();
        for world in self.levels.parked_worlds() {
            world.pois.clear();
        }
        self.mods.datapack_registries = datapack_registries.map(Arc::new);
        horizon_client::datapack_net::serve(self.mods.datapack_registries.clone());
        self.send_datapack_snapshots();
        self.refresh_recipe_property_context();
        self.structure_spawns = structure_spawns;
        self.health_resources = health;
        self.server_items = None;
        self.install_parts(parts);
        self.damage_tag_generation = self.damage_tag_generation.wrapping_add(1);
        self.forget_book();
        horizon_client::structure_blocks::forget_loot_data();
        self.load_server_loot();
        // Before the players are sent the reloaded data below.
        self.sync_recipe_content();
        // PlayerList.reloadResources: the tags and recipes go to the players
        // and their advancements reload. The synchronized registries (the
        // dialogs, and the commands' ids of them) stay the hub's.
        lan::install_wire_reload(
            &self.options.resources,
            &self.res.core.item_data,
            &self.pack_files(),
            wire.clone(),
        );
        if self.lan.is_some()
            && let Some(sync) = wire
        {
            self.with_advancement_hub(|hub, owners| hub.reload_resources(sync, owners));
        }
        let overlay = pack_files(&self.world_tables);
        self.adv.tree = Arc::new(horizon_client::advancements::Tree::load_with(
            &self.options.resources.join("data"),
            &overlay,
        ));
        self.adv.reset_tree();
        self.native_tags_updated(mod_api::TagsUpdateCause::ServerDataLoad { reload: true });
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
        self.build_screen();
    }

    pub(super) fn join_world_rest(&mut self) -> Result<()> {
        if self.rest_loader.is_some() {
            let waited = Instant::now();
            log::info!("Horizon: waiting for world resources");
            while self.rest_loader.is_some() {
                self.poll_world_rest()?;
                if self.rest_loader.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            log::info!(
                "Horizon: world resources joined after {} ms",
                waited.elapsed().as_millis()
            );
        }
        if self.res.rest.get().is_none() {
            return Err("world resources are unavailable".into());
        }
        Ok(())
    }
}

impl Game<'_> {
    pub(super) fn active_enchantment_definitions(&self) -> &horizon_client::enchant::Enchantments {
        match self.remote.as_ref().and_then(|r| r.enchantments.as_ref()) {
            Some(e) => e,
            None => &self.stations().enchantments,
        }
    }
    /// Build and publish one server snapshot for this world load or reload.
    pub(super) fn load_server_loot(&mut self) {
        self.server_items = None;
        let overlay = pack_files(&self.world_tables);
        let loot = if mod_api::__private::mods().is_empty() {
            horizon_client::structure_blocks::world_loot_data(
                &self.options.resources,
                &overlay,
                self.res.item_defaults(),
            )
        } else {
            match horizon_world::loot::LootData::load(
                &self.options.resources,
                &overlay,
                self.res.item_defaults(),
            ) {
                Ok(mut loot) => {
                    let context = match lan::server_data_load(
                        &self.options.resources,
                        &self.res.core.item_data,
                    ) {
                        Ok(context) => context,
                        Err(error) => {
                            log::warn!("Horizon: server loot inventory unavailable: {error}");
                            self.server_loot.set(None);
                            return;
                        }
                    };
                    loot.apply_native_table_loads(&context);
                    Some(Arc::new(loot))
                }
                Err(e) => {
                    log::warn!("Horizon: server loot tables unavailable: {e}");
                    None
                }
            }
        };
        if !mod_api::__private::mods().is_empty()
            && let Some(loot) = &loot
        {
            let mut items = self.items().clone();
            items.set_loot_tables(loot.tables().clone());
            self.server_items = Some(Arc::new(items));
        }
        self.server_loot.set(loot);
    }

    pub(super) fn prepare_parts(&self, items: &Items) -> Result<PreparedParts> {
        let registry = self.pack.assets.registry;
        let block_tags = &items.block_tags;
        let tags = native_world_read::prepare_block_tags(block_tags);
        let parts = mod_api::__private::prepare_parts(
            block_tags,
            &items.item_tags,
            mod_api::__private::KnownIds {
                block: &|id| registry.default_state(id).is_some(),
                item: &|id| items.items.contains_key(id),
            },
        )
        .map_err(|refused| match refused {
            mod_api::__private::ResolvePartsError::ItemKeys(errors) => format!(
                "Mod item keys refused:\n{}",
                errors
                    .into_iter()
                    .map(|e| format!("mod {} requested unknown item {}", e.mod_id, e.id))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            mod_api::__private::ResolvePartsError::Parts(errors) => {
                format!("Mod parts refused:\n{}", errors.join("\n"))
            }
            _ => "Mod parts refused".to_owned(),
        })?;
        let block_states = api_values::prepare_block_states(registry)?;
        let cauldrons = self.prepare_cauldron_interactions(items)?;
        Ok(PreparedParts {
            cauldrons,
            block_states,
            tags,
            parts,
        })
    }
    /// Reload the content tables with the world's packs, as
    /// `/reload` reloads a server's (`ReloadableServerResources`).
    pub(super) fn prepare_world_reload(
        &mut self,
        stack: &functions::PackStack,
        root: &std::path::Path,
        data_switched: bool,
    ) -> Result<PreparedWorldReload> {
        let (tables, health) = self.build_world_tables(stack, true, root, data_switched)?;
        let datapack_registries = self
            .mods
            .datapack_candidate
            .take()
            .map(|candidate| match &self.mods.datapack_registries {
                Some(old) => old
                    .reload(candidate)
                    .map_err(|error| format!("datapack registry reload: {error:?}")),
                None => Ok(mod_api::DatapackRegistrySet::first(
                    candidate,
                    horizon_client::datapack_net::next_session(),
                )),
            })
            .transpose()?;
        let items = tables
            .as_ref()
            .map_or_else(|| self.res.items(), |tables| &tables.items);
        let parts = self.prepare_parts(items)?;
        let overlay = pack_files(&tables);
        let kept = self
            .lan
            .as_ref()
            .map(|host| host.hub.lan.shared.sync())
            .or_else(lan::cached_reload_sync);
        let wire = if let Some(kept) = kept {
            let sources = match tables
                .as_ref()
                .and_then(|tables| tables.reload_sources.clone())
            {
                Some(sources) => sources,
                None => lan::prepare_server_sources(root, &self.res.core.item_data, &overlay)?,
            };
            Some(
                lan::prepare_wire_reload(root, &self.res.core.item_data, &overlay, &kept, &sources)
                    .map_err(|error| format!("wire reload: {error}"))?,
            )
        } else {
            None
        };
        Ok(PreparedWorldReload {
            structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns::load(root),
            tables,
            health,
            parts,
            wire,
            datapack_registries,
        })
    }

    /// Resolve the mods' parts against the tags this game plays with (`/reload` again): mods
    /// that attach parts to a block or item it doesn't have are an error.
    pub(super) fn resolve_parts(&self) -> Result<()> {
        let prepared = self.prepare_parts(self.items())?;
        self.install_parts(prepared);
        Ok(())
    }
    /// Ask the parts of each block `isPathfindable` once per state and type: the answer depends
    /// on nothing else, so path finding and spawning read it with no dispatch.
    pub(super) fn install_parts(&self, prepared: PreparedParts) {
        cauldron_interactions::install(prepared.cauldrons);
        api_values::install_block_states(prepared.block_states);
        native_world_read::install_block_tags(prepared.tags);
        mod_api::__private::install_resolved_parts(prepared.parts);
        self.resolve_pathfindable();
    }
    pub(super) fn resolve_pathfindable(&self) {
        let asked = mod_api::__private::has_scoped_hook(mod_api::HookId::IsPathfindable);
        let data = &self.res.rest().mob_data;
        data.set_pathfindable(asked, |state, kind, mut pathfindable| {
            self.native_is_pathfindable(state, kind, &mut pathfindable);
            pathfindable
        });
    }
}
