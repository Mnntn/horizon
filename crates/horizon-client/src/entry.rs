//! Startup shared by the gameplay library and its executable.
#[path = "horizon_look.rs"]
mod horizon_look;
use crate::{crash, game, logging};
use horizon_mod_package::selection::{Argument, LaunchArguments};

/// Validated startup state. No native hooks or workers may run before prepare returns.
pub struct Startup {
    launch: LaunchArguments,
    options: game::Options,
    _capture: Option<horizon_mod_package::output::Capture>,
}

impl Startup {
    /// Validated launch context used by the platform's code loader.
    pub fn launch(&self) -> &LaunchArguments {
        &self.launch
    }
}

/// Holder mode runs first. The game's sole Options validation reads arguments/environment
/// and resource marker metadata only: no logging, account loading, mods or worker threads.
/// Account IO, cwd changes and command registration remain in Options::read, after leases.
pub fn prepare() -> Startup {
    horizon_mod_package::holder::enter();
    let result: crate::Result<_> = (|| {
        let mut command = std::process::Command::new(std::env::current_exe()?);
        command.args(std::env::args_os().skip(1));
        let launch = LaunchArguments::for_command(&command)?;
        let options = game::Options::validate(launch.clone())?;
        Ok((launch, options))
    })();
    let (launch, options) = result.unwrap_or_else(|error| {
        eprintln!("Horizon: {error}");
        std::process::exit(game::exit_code::USAGE);
    });
    let Some(options) = options else {
        std::process::exit(0);
    };
    let capture = horizon_mod_package::holder::acquire(&launch);
    Startup {
        launch,
        options,
        _capture: capture,
    }
}

/// Code-runtime services supplied by the platform at startup.
pub struct CodeRuntime {
    pub initialize: fn(&[mod_api::PackageInfo]),
    pub refusal: fn(&str) -> Option<String>,
    pub startup_refused: fn(&str) -> bool,
}
static CODE_RUNTIME: std::sync::OnceLock<CodeRuntime> = std::sync::OnceLock::new();
pub(crate) fn initialize_code(inventory: &[mod_api::PackageInfo]) {
    let runtime = CODE_RUNTIME
        .get()
        .expect("code initialization requires entry::start");
    (runtime.initialize)(inventory);
}
pub(crate) fn code_startup_refused(id: &str) -> bool {
    CODE_RUNTIME
        .get()
        .is_some_and(|runtime| (runtime.startup_refused)(id))
}
pub(crate) fn code_refusal(id: &str) -> Option<String> {
    CODE_RUNTIME.get().and_then(|runtime| (runtime.refusal)(id))
}

/// Start with named mods and their function slots in installed dependency order.
pub fn start<G>(
    native: horizon_mod_package::runtime::Runtime,
    load: impl FnOnce(&horizon_mod_package::runtime::Runtime, &LaunchArguments) -> G,
    code: CodeRuntime,
    startup: Startup,
) {
    let _ = CODE_RUNTIME.set(code);
    let Startup {
        launch,
        options,
        _capture,
    } = startup;
    horizon_threads::configure_allocator();
    horizon_look::give();
    // OptiFine's and Polytone's pack formats (platform, docs/layers.md), before any pack loads.
    horizon_client::pack_formats::install();
    // horizon-extras (docs/forking.md): without the feature, no Horizon Settings or Mods page.
    if !cfg!(feature = "horizon-extras") {
        horizon_client::options_ui::HORIZON_EXTRAS
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
    horizon_client::telemetry::mark_start();
    // Opt-in telemetry keeps each crash report (docs/telemetry.md), whatever the mode.
    crash::on_report(horizon_client::telemetry::keep_crash);
    // Crates above the engine registered their hooks before this (horizon-official's `main`),
    // then the Horizon parts still in this crate; none registers after this point.
    game::player_hooks::register();
    horizon_hooks::engine::seal();
    horizon_client::spans::startup::init(std::time::Instant::now());
    horizon_client::spans::trace::start_from_env();
    let _loaded_code = load(&native, &launch);
    logging::set_game_home(launch.context.home.clone());
    // The dedicated server logs in its own folder, as vanilla's does; the game in its home.
    let (home, side, thread) = if launch
        .rest
        .iter()
        .any(|arg| matches!(arg, Argument::Dedicated(_)))
    {
        (launch.context.game_dir.clone(), "server", "Server thread")
    } else {
        // A player's own log stays separate; crash reports follow the shared home.
        if let Some(dir) = launch.rest.iter().find_map(|arg| match arg {
            Argument::Value(name, value) if name == "--player-dir" => Some(value),
            _ => None,
        }) {
            logging::set_player_dir(std::path::absolute(dir).unwrap_or_else(|_| dir.into()));
        }
        (launch.context.home.clone(), "client", "Render thread")
    };
    logging::init(
        logging::player_dir_override().unwrap_or(home.as_path()),
        thread,
    );
    // The log's first line names the build, so a bug report that carries the log says which one it was.
    log::info!("Horizon {}", crash::version());
    crash::install(home.clone(), side);
    if horizon_world::native_load_hooks::install(
        horizon_world::native_load_hooks::NativeLoadHooks {
            loot_table_load: |context, id, table| {
                if !mod_api::__private::listened(mod_api::HookId::LootTableLoad) {
                    return;
                }
                let reader = horizon_client_render::api_context::LoadReader(context);
                let context = mod_api::OwningLoad::new(
                    horizon_client_render::api_context::load_origin(context.origin()),
                    &reader,
                );
                mod_api::dispatch_loot_table_load(&context, id, table);
            },
            modify_default_components: |context, item, components| {
                if !mod_api::__private::listened(mod_api::HookId::ModifyDefaultComponents) {
                    return;
                }
                let reader = horizon_client_render::api_context::LoadReader(context);
                let context = reader.context();
                mod_api::dispatch_modify_default_components(&context, item, components);
            },
            loot_function: |kind, data, ctx, stack, tags| {
                crate::game::api_values::dispatch_loot_function(kind, data, ctx, stack, tags);
            },
            loot_condition: |kind, data, ctx, holds, tags| {
                crate::game::api_values::dispatch_loot_condition(kind, data, ctx, holds, tags);
            },
        },
    )
    .is_err()
    {
        log::warn!("Native data-load hooks already installed; keeping the first callbacks");
    }
    if horizon_world::native_worldgen_hooks::install(
        horizon_world::native_worldgen_hooks::NativeWorldgenHooks {
            alter_ground: crate::game::api_values::alter_ground,
        },
    )
    .is_err()
    {
        log::warn!("Native worldgen hooks already installed; keeping the first callbacks");
    }
    if horizon_items::native_enchantment_hooks::install(
        crate::enchantment_levels::get_enchantment_level,
    )
    .is_err()
    {
        log::warn!("The enchantment level hook is already installed; keeping the first");
    }
    install_client_hooks();
    raise_open_file_limit();
    if let Err(error) = game::run_with(options, native) {
        // A bad command line is the caller's mistake, not a crash: no report.
        let usage = error.is::<game::UsageError>();
        log::error!("Horizon: {error}");
        logging::flush();
        // The report's path is the last line on stderr (crash::SAVED_LINE).
        if !usage {
            crash::report_error(&home, side, &*error);
        }
        // Started from Explorer, the console window closes with the game:
        // keep the message on screen until it is read.
        #[cfg(windows)]
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            eprintln!("Press Enter to close.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(if usage {
            game::exit_code::USAGE
        } else {
            game::exit_code::ERROR
        });
    }
    horizon_hooks::engine::shutdown();
    logging::flush();
}

pub(crate) fn install_client_hooks() {
    game::api_values::install_stack_hooks();
    if !crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {
        menu_slot_rules: game::api_values::menu_slot_rules,
        item_stacked_on_other: |context, event, canceled| {
            game::api_values::stacked_on_other(context, event, canceled);
        },
        living_heal: |context, entity, amount, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingHeal) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_heal(&context, entity, amount, cancel);
        },
        living_breathe: |context, entity, kind, can_breathe, consume, refill| {
            if !mod_api::__private::listened(mod_api::HookId::LivingBreathe) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_breathe(&context, entity, kind, can_breathe, consume, refill);
        },
        farmland_trample: |context, entity, pos, state, distance, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::FarmlandTrample) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_farmland_trample(&context, entity, pos, state, distance, cancel);
        },
        has_mods: || !mod_api::__private::mods().is_empty(),
        player_name_format: |context, username, displayname| {
            use game::api_values::{TryIntoApi, TryIntoEngine};
            if !mod_api::__private::listened(mod_api::HookId::PlayerNameFormat) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let username = username.try_into_api().unwrap();
            let mut converted = displayname.try_into_api().unwrap();
            mod_api::dispatch_player_name_format(&context, &username, &mut converted);
            *displayname = converted.try_into_engine(&()).unwrap();
        },
        player_tab_list_name_format: |context, player, name, display_name| {
            use game::api_values::{TryIntoApi, TryIntoEngine};
            if !mod_api::__private::listened(mod_api::HookId::PlayerTabListNameFormat) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            let mut converted = display_name.try_into_api().unwrap();
            mod_api::dispatch_player_tab_list_name_format(&context, player, name, &mut converted);
            *display_name = converted.try_into_engine(&()).unwrap();
        },
        living_drown: |context, entity, kind, is_drowning, damage_amount, bubble_count, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingDrown) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_drown(
                &context,
                entity,
                kind,
                is_drowning,
                damage_amount,
                bubble_count,
                cancel,
            );
        },
        living_fall: |context, entity, kind, distance, multiplier, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFall) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_fall(&context, entity, kind, distance, multiplier, cancel);
        },
        living_jump: |context, entity, kind| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJump) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump(&context, entity, kind);
        },
        living_jump_power: |context, entity, kind, power| {
            if !mod_api::__private::listened(mod_api::HookId::LivingJumpPower) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_jump_power(&context, entity, kind, power);
        },
        living_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingSpeed) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_speed(&context, entity, kind, speed);
        },
        living_flying_speed: |context, entity, kind, speed| {
            if !mod_api::__private::listened(mod_api::HookId::LivingFlyingSpeed) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_living_flying_speed(&context, entity, kind, speed);
        },
        block_jump_factor: |context, state, factor| {
            crate::game::block_jump_factor(context, state, factor);
        },
        block_speed_factor: |context, state, factor| {
            crate::game::block_speed_factor(context, state, factor);
        },
        is_ladder: |context, entity, pos, state, ladder| {
            crate::game::is_ladder(context, entity, pos, state, ladder);
        },
        player_stays_on_ground_surface: |context, entity, staying| {
            if !mod_api::__private::listened(mod_api::HookId::PlayerStaysOnGroundSurface) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_player_stays_on_ground_surface(&context, entity, staying);
        },
        ender_entity_teleport: |context, entity, kind, from, target, cancel| {
            if !mod_api::__private::listened(mod_api::HookId::EnderEntityTeleport) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_ender_entity_teleport(&context, entity, kind, from, target, cancel);
        },
        fluid_flow: |context, fluid, pos, flow| {
            if !mod_api::__private::listened(mod_api::HookId::FluidFlow) {
                return;
            }
            let reader = horizon_client_render::api_context::LoopReader(context);
            let context = reader.context();
            mod_api::dispatch_fluid_flow(&context, fluid, pos, flow);
        },
    }) {
        log::warn!("Native client hooks already installed; keeping the first callbacks");
    }
}

/// The JVM raises its open-file limit to the hard limit as it starts
/// (`-XX:+MaxFDLimit`, on by default), so a vanilla server may hold as many
/// sockets and region files as the host allows, not the soft default (often
/// 1024 in a container). The same here; macOS caps it at `OPEN_MAX`, as the
/// JVM does there.
fn raise_open_file_limit() {
    #[cfg(unix)]
    {
        use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
        let limit = getrlimit(Resource::Nofile);
        let hard = if cfg!(target_os = "macos") {
            Some(limit.maximum.map_or(10_240, |h| h.min(10_240)))
        } else {
            limit.maximum
        };
        let raised = match (limit.current, hard) {
            (Some(soft), Some(hard)) => hard > soft,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if raised {
            let to = Rlimit {
                current: hard,
                maximum: limit.maximum,
            };
            match setrlimit(Resource::Nofile, to) {
                Ok(()) => log::info!(
                    "Open file limit raised from {:?} to {hard:?}",
                    limit.current
                ),
                Err(e) => log::warn!("Open file limit stays at {:?}: {e}", limit.current),
            }
        }
    }
}
