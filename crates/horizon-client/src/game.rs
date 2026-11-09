//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
use key_codes::{
    container_key_clones, drop_held_behind_dialog, follow_named_keymap, gameplay_has_keyboard,
    glfw_button, glfw_modifiers, keyboard_button,
};
use loading_screen::{
    App, Loading, leave_fullscreen_after, missing_resources, run_event_loop, unclean_start_recovery,
};
use resource_load::{Progress, launch_world, load_resources, named_scope, synchronous_startup};

/// The content tables the world plays with, as field-level borrows: the
/// world's data packs' over the game's own. The `Game::items`/`recipes`/
/// `stations` methods read the same tables, but taking `&self` blocks a
/// `&mut self.…` in the same scope; these expand to a match on the fields,
/// which borrows stay disjoint from.
macro_rules! world_items {
    ($s:ident) => {
        match (&$s.server_items, &$s.world_tables) {
            (Some(items), _) => &**items,
            (None, Some(t)) => &t.items,
            (None, None) => $s.res.items(),
        }
    };
}
macro_rules! world_recipes {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.recipes,
            None => $s.res.recipes(),
        }
    };
}
macro_rules! world_stations {
    ($s:ident) => {
        match &$s.world_tables {
            Some(t) => &t.stations,
            None => $s.res.stations(),
        }
    };
}
/// The biome table the world plays with (`Game::biomes`), as a field-level
/// borrow: the world's packs' over the game's own.
macro_rules! world_biomes {
    ($s:expr) => {
        match &$s.world_biomes {
            Some(b) => b,
            None => $s.res.biomes(),
        }
    };
}

/// The world's packs' content files, as a field-level borrow (see the macros).
fn pack_files(world_tables: &Option<WorldTables>) -> horizon_world::loot::PackOverlay<'_> {
    match world_tables {
        Some(t) => horizon_world::loot::PackOverlay {
            conditions: &t.conditions,
            files: &t.files,
        },
        None => horizon_world::loot::PackOverlay::NONE,
    }
}

include!(concat!(env!("OUT_DIR"), "/game_modules.rs"));
/// The game's side of the injection points (#589), whose slots and listeners live in `mod_points`.
#[doc(hidden)]
pub mod points {
    /// A `side = "both"` point's host, out of line: each point's listener call stays a few bytes.
    /// Called only by the `#[point]`s the stage adds to its copy.
    #[inline(never)]
    #[allow(dead_code, reason = "used by the staged copy's points")]
    pub(super) fn host_side<'a>(game: &'a mut super::Game<'_>) -> mod_api::HostSide<'a> {
        super::execution_role::host_side(game)
    }

    /// An engine value's `mod_api` mirror for a point record, through the boundary's conversion.
    #[allow(dead_code, reason = "used by the staged copy's points")]
    pub(super) fn mirror_in<T: super::api_values::TryIntoApi + ?Sized>(
        value: &T,
    ) -> Option<T::Api> {
        value.try_into_api().ok()
    }

    /// A listener's `mod_api` value as the engine's, validated as every mod value is; a value the
    /// engine refuses leaves the engine's own in place.
    #[allow(dead_code, reason = "used by the staged copy's points")]
    pub(super) fn mirror_out<A: super::api_values::TryIntoEngine<()>>(
        value: &A,
        point: &str,
    ) -> Option<A::Engine> {
        let engine = value.try_into_engine(&()).ok();
        if engine.is_none() {
            log::warn!(
                "point {point}: a listener's value has no engine form; the engine's is kept"
            );
        }
        engine
    }

    /// An engine value with no `mod_api` form at this call: its listeners are skipped this once.
    #[cold]
    #[allow(dead_code, reason = "used by the staged copy's points")]
    pub(super) fn unconverted(point: &str) {
        log::warn!("point {point}: a value has no API form; its listeners were skipped");
    }
}
mod host;
mod native_hooks;
use event_loop::EventLoopCtl;
use horizon_client::random_source;
use loading_window::{
    build_renderer, loading_frame, loading_renderer, prepare_renderer_textures, start_game,
};
// `Action` here is the title screen's buttons (`title::Action`).
use self::api_values::TryIntoApi;
pub(crate) use api_values::{block_jump_factor, block_speed_factor};
use horizon_client::actions::Action as Mapping;
use horizon_client::held::Source;
use horizon_client::spans::startup;
use horizon_client::{
    Result,
    assets::{Assets, RenderAssets},
    audio::Audio,
    biome::Biomes,
    block_entity_render::{
        self, BlockEntityRenderer, CacheOptions, Scene as BlockEntityScene, StaticCacheFrameStats,
        StaticSectionSnapshot,
    },
    blocks::{BlockFacts, Fluid},
    clock::FixedStep,
    collisions,
    containers::{BlockEntity, Providers},
    crafting::Recipes,
    creative::{Creative, Names},
    entities::{Experience, ExperienceOrb, ItemEntity},
    environment::Environment,
    first_person::{Hand, SwingAnimation},
    furnace::FurnaceCooking,
    gfx::{Frame, Renderer},
    graphics::EntityCullingMode,
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, hud_overlays,
    items::{ItemStack, Items},
    keymapping::{KeyAction, SNEAK, SPRINT},
    light::LightTables,
    mesher::{MeshTables, Vertex},
    mobs::{Mob, MobType, MobTypes},
    models,
    motion::{MotionEvent, MotionResources, PlayerMotion},
    mouse::{LookOptions, MouseLook},
    particles::Particle,
    save::{Level, Persistent, SaveDir},
    settings::Settings,
    spans,
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    testgen::TestTerrain,
    title::{Action, Title},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    geometry::{Aabb, Direction},
    position::BlockPos,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_mod_package::selection::{Argument, LaunchArguments};
pub use native_block_entity_draw::BlockEntityDraw;
pub use native_claimed_items::{HeldItemSlot, ModEvent};
pub use native_components::NativeComponentError;
pub use native_configs::ConfigFile;
pub use native_data_maps::DataMapType;
pub use native_hooks::{PhantomSpawnResult, Shake, SleepProblem};
pub use native_mob_draw::{MobRender, PlayerRender};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    error::EventLoopError,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use {
    horizon_core::dimension::Dimension as LevelDimension,
    horizon_world::{
        block_collision::BlockCollisionRegistry,
        collision_context::{CollisionContext, CollisionEntity, CollisionItem},
        entity_obstruction::{EntityIdentity, EntityObstructionWorld, ObstructionEntity},
        fluid::FluidState,
        level::{
            Behaviours, BucketContent, EntityQuery, Landing, LevelAccess, LevelEvent, PlaceContext,
            PlaceOutcome, PlacementContext, PlayerContext, PlayerEffect, Ticks,
            flags as level_flags,
        },
        registry::BlockStateId,
    },
};

/// The time a frame spends uploading chunk meshes when the player set no
/// Chunk Updates or Upload Budget: a burst past it waits for the next frames.
const AUTO_UPLOAD_TIME: Duration = Duration::from_millis(2);
/// `AUTO_UPLOAD_TIME` while the view's terrain is still arriving (a world's
/// open, a teleport): the meshes waiting then are the view itself, not edits
/// to it, so a frame gives them more of its time to fill the view sooner.
const AUTO_UPLOAD_TIME_ARRIVING: Duration = Duration::from_millis(6);
/// A frame's finished meshes up to this many upload at once, unqueued.
const AUTO_UPLOADS_AT_ONCE: usize = 8;
/// The most changed columns far terrain waits on behind a mod's hold (`Game::lod_held`): past
/// it, they go in as ever.
const HELD_FAR_COLUMNS: usize = 1 << 13;

#[derive(Clone)]
struct Options {
    resources: PathBuf,
    distance: i32,
    distance_override: Option<i32>,
    seed: i64,
    creative: bool,
    test_terrain: bool,
    /// `--pregen`: enable the command even when the saved Extras switch is off.
    pregen: bool,
    frames: Option<u64>,
    /// A splitscreen player: the split window paces every player's frames, not each its own.
    paced_by_split: bool,
    /// A splitscreen player other than P1: its windows are the split's (P1 alone decides
    /// fullscreen; this game neither asks for it nor adopts it into its options).
    split_guest: bool,
    /// A splitscreen guest drawn in Window 1, the game's own window: its F11 leaves that
    /// window to P1.
    split_in_main_window: bool,
    /// `--world-frames N`: exit after N frames drawn in the world (smoke runs; the world may load slowly).
    world_frames: Option<u64>,
    screenshot_after: Option<u64>,
    timeout: Duration,
    report: Option<PathBuf>,
    world: Option<String>,
    look: Option<(f32, f32)>,
    time: Option<i64>,
    weather: Option<String>,
    dimension: Option<String>,
    /// Options.pauseOnLostFocus off (F3+P), for scripted testing.
    no_pause: bool,
    /// Control socket path (`--control`), for `horizon-ctl`.
    control: Option<PathBuf>,
    /// `HORIZON_SPIKES=MS`: print the timed spans of frames slower than MS.
    spikes: Option<f32>,
    /// Render into a texture instead of the window (`--offscreen`), so a
    /// GPU the display cannot present from can still be benchmarked. The
    /// window stays hidden and never grabs the system pointer.
    offscreen: bool,
    /// `--headless` (#4951): no display, no window and no renderer. The game runs its own
    /// loop (`game/runtime/headless.rs`) instead of winit's, so it starts where no display server
    /// is (an SSH session, a server). It counts as `offscreen` for everything else (an
    /// automated run that never shows a window or grabs the pointer).
    headless: bool,
    /// `--width` and `--height` (vanilla's own arguments): the window's
    /// starting size, 1280x720 by default.
    window_size: [u32; 2],
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    server: Option<String>,
    /// `--name`, `HORIZON_PLAYER_NAME` or the signed-in account's name; else
    /// [`Options::player_name`] uses the kept offline name.
    player_name: Option<String>,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    dedicated: Option<Vec<String>>,
    /// `--transfers SETTINGS`: a dedicated server takes a proxy's hand-over requests
    /// (`server.toml`'s `[transfer]`, which horizon-server passes; lan/arrivals.rs).
    transfers: Option<horizon_client::lan::arrivals::Settings>,
    /// `--benchmark SCENARIO`: run the benchmark, write its report and quit.
    benchmark: Option<String>,
    /// Content selection and the compiled native runtime shared with resource loaders.
    mods: mods::ModsState,
    /// What this Game executes; independent of its current server connection.
    execution_role: execution_role::ExecutionRole,
    /// `--fullscreen` (vanilla's): the window opens fullscreen whatever options.txt says,
    /// as vanilla's DisplayData does, without saving it there.
    fullscreen: bool,
    /// `--game-dir DIR`: the folder `read` makes the working directory (saves, options.txt...);
    /// none for a splitscreen player's game, which shares P1's working directory.
    game_dir: Option<PathBuf>,
    /// `--quickPlaySingleplayer`, checked once the game folder is known.
    quick_play: Option<String>,
    session_file: Option<String>,
    /// `--session-stdin`: the session file's JSON comes on standard input instead.
    session_stdin: bool,
}
impl Options {
    /// Whether this `Game` is the process's client start: not a dedicated server, and not
    /// a splitscreen guest, whose `run` is another local player joining the same process.
    fn is_client_start(&self) -> bool {
        self.execution_role.is_client() && !self.split_guest
    }
    /// The name to join servers with: the one given, else the offline name kept
    /// next to options.txt (created on first use).
    fn player_name(&self) -> String {
        self.player_name.clone().unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| horizon_client::offline_name::generate(0),
                |_| {
                    horizon_client::offline_name::load_or_create(
                        &horizon_client::logging::player_dir(),
                    )
                },
            )
        })
    }
    /// The command line, then what it asks of the process: the game folder, the account.
    /// A bad command line is a [`launch_api::UsageError`] (exit code 2, no crash report).
    fn read(launch: LaunchArguments) -> Result<Option<Self>> {
        let Some(mut o) =
            Self::from_launch(launch).map_err(|e| launch_api::UsageError(e.to_string()))?
        else {
            return Ok(None);
        };
        // Paths given on the command line stay relative to where the game was started.
        if o.game_dir.is_some() {
            o.control = o.control.map(std::path::absolute).transpose()?;
        }
        // Refused here, before the account's file is used up or anything loads.
        if let Some(path) = &o.control {
            horizon_client::control::check_path(path).map_err(|e| {
                launch_api::UsageError(format!("--control-socket {}: {e}", path.display()))
            })?;
        }
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        let session = if o.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| horizon_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            o.session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| horizon_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                o.player_name = Some(session.name.clone());
                horizon_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                horizon_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {}
            Err(e) => eprintln!("Horizon: not signed in: {e}"),
        }
        split::launcher::read_now();
        if let Some(dir) = o.game_dir.take() {
            // These too stay relative to where the game was started.
            o.resources = std::path::absolute(&o.resources)?;
            o.report = o.report.map(std::path::absolute).transpose()?;
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::env::set_current_dir(&dir))
                .map_err(|e| format!("game folder {}: {e}", dir.display()))?;
        }
        // Vanilla's quick play (Main, QuickPlay.joinSingleplayerWorld): only a
        // world that exists opens; else the title screen. --world also creates one.
        if let Some(world) = o.quick_play.take() {
            if std::env::current_dir()?
                .join("saves")
                .join(&world)
                .join("level.dat")
                .is_file()
            {
                o.world = Some(world);
            } else {
                eprintln!("Quick play: there is no world {world} in saves/");
            }
        }
        Ok(Some(o))
    }
    /// The command line alone: no file is read and nothing changes. `None` for `--help` and
    /// `--launch-api`, which print and end the run.
    #[cfg(test)]
    fn parse(args: impl Iterator<Item = String>) -> Result<Option<Self>> {
        Self::from_launch(LaunchArguments::parse(
            args,
            &std::env::current_dir()?,
            &horizon_client::logging::game_home(),
            std::env::var_os("HORIZON_HOME").as_deref(),
        )?)
    }

    fn from_launch(LaunchArguments { context, rest }: LaunchArguments) -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                std::path::Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION)
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            creative: false,
            test_terrain: false,
            pregen: false,
            frames: None,
            paced_by_split: false,
            split_guest: false,
            split_in_main_window: false,
            world_frames: None,
            screenshot_after: None,
            timeout: Duration::from_secs(120),
            report: None,
            world: None,
            look: None,
            time: None,
            weather: None,
            dimension: None,
            no_pause: false,
            control: None,
            offscreen: false,
            headless: false,
            window_size: [1280, 720],
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME")
                .ok()
                .filter(|n| !n.is_empty()),
            dedicated: None,
            transfers: None,
            benchmark: None,
            mods: mods::ModsState {
                dev: context.dev_mods,
                ..Default::default()
            },
            execution_role: execution_role::ExecutionRole::Client,
            fullscreen: false,
            game_dir: Some(context.game_dir),
            quick_play: None,
            session_file: std::env::var("HORIZON_SESSION_FILE").ok(),
            session_stdin: false,
        };
        for argument in rest {
            match argument {
                Argument::Switch(arg) => match arg.as_str() {
                    "--help" | "-h" => {
                        println!(
                            "Horizon — Minecraft Java Edition {} in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..128 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --width N, --height N   Window size (default 1280 x 720)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --world-frames N        Exit after N frames drawn in the world (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --benchmark SCENARIO    Run a benchmark (standard, village, forest, ocean, mountain), write its report to benchmarks/ and quit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --quickPlaySingleplayer WORLD  Open this world of saves/ at start, only if it exists (vanilla's)\n  --quickPlayMultiplayer HOST[:PORT]  The same as --server (vanilla's name)\n  --name NAME             Player name for multiplayer (default: generated once, kept in horizon-player.txt)\n  --session-file PATH     Signed-in account from the launcher (read once, then deleted)\n  --session-stdin         The same account as JSON on standard input\n  --game-dir DIR          Game folder: saves, options.txt, packs, mods (default: the working directory)\n  --home DIR              Logs and crash reports (default $HORIZON_HOME, else the platform data folder)\n  --player-dir DIR        This player's own files when several play in one game folder: options.txt, logs, horizon-player.txt, servers.dat, screenshots\n  --fullscreen            Open the window fullscreen\n  --launch-api            Print the Launch API version and options as JSON (docs/api/launch.md)\n  --dev-mod PATH          A content mod (.hmod or folder) to develop: always on\n  --pregen                Add Horizon's /pregen command (docs/pregeneration.md)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause.",
                            horizon_core::VANILLA_VERSION
                        );
                        return Ok(None);
                    }
                    "--launch-api" => {
                        println!("{}", launch_api::reply(&horizon_client::crash::version()));
                        return Ok(None);
                    }
                    "--fullscreen" => o.fullscreen = true,
                    "--session-stdin" => o.session_stdin = true,
                    "--creative" => o.creative = true,
                    "--test-terrain" => o.test_terrain = true,
                    "--no-pause" => o.no_pause = true,
                    "--offscreen" => o.offscreen = true,
                    "--headless" => {
                        o.headless = true;
                        o.offscreen = true;
                    }
                    "--control" => o.control = Some(horizon_client::control::default_path()),
                    // Single player always runs on its server thread: a launcher's saved arguments
                    // may still name the old switches.
                    "--server-thread" => {}
                    "--in-frame" | "--no-server-thread" => eprintln!(
                        "Horizon: {arg} was removed: single player runs on its server thread."
                    ),
                    // Horizon's own commands (/pregen), off by default as on vanilla.
                    "--pregen" => {
                        o.pregen = true;
                        horizon_client::command::vanilla::set_horizon_commands(true);
                    }
                    _ => return Err(format!("{arg}: unknown option").into()),
                },
                Argument::Dedicated(args) => {
                    o.execution_role = execution_role::ExecutionRole::DedicatedServer;
                    o.dedicated = Some(args);
                    break;
                }
                Argument::Value(arg, value) => {
                    // A launcher shows the error to the player: it names the option.
                    let read = || -> Result<()> {
                        match arg.as_str() {
                            "--resources" => o.resources = value.into(),
                            "--render-distance" | "--radius" => {
                                o.distance = value.parse()?;
                                o.distance_override = Some(o.distance);
                                let max = horizon_client::options_file::MAX_RENDER_DISTANCE;
                                if !(2..=max).contains(&o.distance) {
                                    return Err(format!("render distance must be 2..{max}").into());
                                }
                            }
                            "--seed" => {
                                o.seed = value.parse().unwrap_or_else(|_| {
                                    // Text seeds hash like String.hashCode.
                                    value
                                        .encode_utf16()
                                        .fold(0i32, |h, c| {
                                            h.wrapping_mul(31).wrapping_add(i32::from(c))
                                        })
                                        .into()
                                })
                            }
                            "--frames" => o.frames = Some(value.parse()?),
                            "--world-frames" => {
                                o.world_frames = Some(value.parse()?);
                                // A frame-limited run in every other respect; --timeout-seconds still applies.
                                o.frames.get_or_insert(u64::MAX);
                            }
                            "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                            "--smoke-report" => o.report = Some(value.into()),
                            "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                            "--width" => o.window_size[0] = value.parse::<u32>()?.max(1),
                            "--height" => o.window_size[1] = value.parse::<u32>()?.max(1),
                            // Vanilla opens only a level id that is one folder of saves/. --world, for
                            // harnesses, also takes a path (tools/bundle_tooltip_e2e.py).
                            "--world" => o.world = Some(value),
                            "--quickPlaySingleplayer" => {
                                let mut parts = std::path::Path::new(&value).components();
                                if !matches!(
                                    (parts.next(), parts.next()),
                                    (Some(std::path::Component::Normal(_)), None)
                                ) {
                                    return Err("not a folder name of saves/".into());
                                }
                                o.quick_play = Some(value);
                            }
                            // main has set this player's own folder (logging::set_player_dir).
                            "--player-dir" => {}

                            "--benchmark" => {
                                if horizon_client::benchmark::scenes(&value).is_none() {
                                    return Err(format!(
                                    "unknown benchmark {value}: standard, village, forest, ocean or mountain"
                                )
                                .into());
                                }
                                o.benchmark = Some(value);
                            }
                            "--server" | "--quickPlayMultiplayer" => o.server = Some(value),
                            "--name" => o.player_name = Some(value),
                            "--session-file" => o.session_file = Some(value),
                            "--control-socket" => o.control = Some(value.into()),
                            "--transfers" => {
                                o.transfers =
                                    Some(horizon_client::lan::arrivals::Settings::parse(&value)?)
                            }
                            "--time" => o.time = Some(value.parse()?),
                            "--weather" => o.weather = Some(value),
                            "--dimension" => {
                                o.dimension = Some(if value.contains(':') {
                                    value
                                } else {
                                    format!("minecraft:{value}")
                                })
                            }
                            "--look" => {
                                let (y, p) =
                                    value.split_once(',').ok_or("--look takes YAW,PITCH")?;
                                o.look = Some((y.parse()?, p.parse()?));
                            }
                            _ => return Err("unknown option".into()),
                        }
                        Ok(())
                    };
                    read().map_err(|e| format!("{arg}: {e}"))?;
                }
            }
        }
        Ok(Some(o))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Playing,
    GameModeSwitcher,
    Paused,
    Options,
    Chat,
    Creative,
    Inventory,
    Dead,
    /// The Statistics screen.
    Stats,
    /// The Advancements screen.
    Advancements,
    /// WorldOptionsScreen (game/network/lan_screen.rs).
    WorldOptions,
    /// Play with Friends, from the pause menu's Friends button
    /// (`friends_screen`, game/network/friends.rs).
    Friends,
    /// `SocialInteractionsScreen` (`social_screen`, game/network/social.rs).
    Social,
    /// CommandBlockEditScreen (game/administration/command_blocks.rs).
    CommandBlock,
    /// A game master block's edit screen (game/interface/block_editor.rs).
    BlockEditor,
    /// ServerLinksDialogScreen: a server's links (game/network/server_links.rs).
    ServerLinks,
    /// ConfirmLinkScreen for a server link (game/network/server_links.rs).
    ConfirmLink,
    /// DialogScreen.WarningScreen: the note about custom screens.
    DialogWarning,
    /// WinScreen: the end poem and the credits (game/interface/win.rs).
    Win,
    /// DebugOptionsScreen, F3+F6 (game/interface/debug_options.rs).
    DebugOptions,
    /// A screen of a native mod's own (`mod_screen_open`): it draws and
    /// takes its input through the screen hooks.
    Mod,
    /// PvP & HUD's widget editor (game/interface/hud_editor.rs).
    HudEditor,
    /// Horizon Studio script screen (`studio_screen_open`): canvas draws
    /// itself, takes input through the screen hooks, does not draw chat.
    Studio,
}
impl Screen {
    /// The id native mods see for this screen (`ScreenView.id`, the
    /// `screen_render_post` hook): one per variant, and a hook's contract, so
    /// it stays the same from release to release. A new variant must add one.
    fn id(self) -> &'static str {
        match self {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "game_mode_switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Chat => "chat",
            Screen::Creative => "creative",
            Screen::Inventory => "inventory",
            Screen::Dead => "dead",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world_options",
            Screen::Friends => "friends",
            Screen::Social => "social",
            Screen::CommandBlock => "command_block",
            Screen::BlockEditor => "block_editor",
            Screen::ServerLinks => "server_links",
            Screen::ConfirmLink => "confirm_link",
            Screen::DialogWarning => "dialog_warning",
            Screen::Win => "win",
            Screen::DebugOptions => "debug_options",
            Screen::Mod => "mod",
            Screen::HudEditor => "hud_editor",
            Screen::Studio => "studio",
        }
    }
}
/// The vanilla `state` a `transformer` table entry offers to `ability`: hoe and
/// shovel tables have one action each; the axe's strip, scrape and wax-off lists
/// are one untagged list in `BlockTransformers.bootstrap`, so its particle names it.
fn vanilla_tool_state(
    transformer: &str,
    particle: &str,
    ability: mod_api::ToolAbility,
    state: BlockStateId,
) -> Option<BlockStateId> {
    use mod_api::ToolAbility::*;
    let action = match transformer {
        "minecraft:axe" => match particle {
            "scrape" => AxeScrape,
            "wax_off" => AxeWaxOff,
            _ => AxeStrip,
        },
        "minecraft:shovel" => ShovelFlatten,
        "minecraft:hoe" => HoeTill,
        _ => return None,
    };
    (action == ability).then_some(state)
}

struct Mining {
    pos: [i32; 3],
    progress: f32,
    /// The latest destroy-speed query, reused for server crack feedback.
    per_tick: f32,
    ticks: u32,
}
impl Mining {
    fn stage(&self) -> Option<u8> {
        (self.progress > 0.0).then(|| ((self.progress * 10.0) as u8).min(9))
    }
}
#[derive(Clone, Copy)]
struct TransformContext<'a> {
    world: &'a World,
    player: Option<u32>,
    hand: mod_api::Hand,
    blocking: bool,
    simulated: bool,
}

#[derive(Clone, Copy)]
struct Hit {
    pos: [i32; 3],
    face: [i32; 3],
    point: [f64; 3],
    /// The ray started inside the block (`BlockHitResult.isInside`).
    inside: bool,
}
impl Hit {
    /// The hit as block-use hooks hear it (`UseOnContext`'s hit result).
    fn native(&self) -> mod_api::BlockHit {
        mod_api::BlockHit::new(self.point, self.face, self.inside)
    }
    /// The block the clicked face leads to (`pos.relative(direction)`).
    fn beside(&self) -> [i32; 3] {
        [
            self.pos[0] + self.face[0],
            self.pos[1] + self.face[1],
            self.pos[2] + self.face[2],
        ]
    }
}
/// What a falling block's tick did (`FallingBlockEntity.tick`).
enum FallStep {
    Falling,
    Gone,
    /// Gave up falling: drops itself as an item here.
    Dropped(Vec3),
    /// Hit the ground in this block.
    Landed([i32; 3]),
}
/// A guest's right click comes as one packet per hand and part
/// (`Minecraft.startUseItem`): `ServerGamePacketListenerImpl.handleUseItemOn`
/// does the block's and `ItemStack.useOn`'s part, `handleUseItem` the
/// item's own (`Item.use`). Running both for either would use the item twice.
#[derive(Clone, Copy)]
enum GuestUse {
    OnBlock(Hand),
    Item(Hand),
    /// `handleInteract`: `Player.interactOn` with the hand, nothing else (the
    /// client sends UseItem itself when the entity passed), on the entity the
    /// packet named: `ServerGamePacketListenerImpl.handleInteract` resolves
    /// `packet.entityId()` once and range checks it, so the click never picks
    /// an entity again from the view.
    Entity(Hand, InteractTarget),
}

/// The entity an interact packet names and where its client clicked it (the
/// packet's location, measured from the entity's position): what
/// `ServerGamePacketListenerImpl.handleInteract` takes from
/// `ServerboundInteractPacket` and hands to `Player.interactOn`, which passes
/// the point on to `interactAt`.
#[derive(Clone, Copy)]
struct InteractTarget {
    id: i32,
    location: [f64; 3],
}

/// The client world as seen by the vanilla block-behaviour library.
struct GameLevel<'w> {
    gateway_servers: &'w mut horizon_client::fast_hash::FxHashMap<
        [i32; 3],
        horizon_client::block_entity_anim::EndGateway,
    >,
    border: &'w horizon_core::geometry::border::BorderGeometry,
    world: &'w mut World,
    command_block_transition: Option<command_blocks::CommandBlockTransition>,
    redstone_experiments: bool,
    facts: &'w BlockFacts,
    collision: &'w BlockCollisionRegistry<'w>,
    events: &'w mut Vec<LevelEvent>,
    dimension: LevelDimension,
    darken: i32,
    /// The environment of the level's dimension (`SUN_ANGLE`).
    environment: &'w Environment,
    raining: bool,
    thundering: bool,
    /// The world's enabled feature flags as this side knows them (`LevelReader.enabledFeatures`).
    enabled_features: &'w std::sync::Arc<[String]>,
    player_box: Option<([f64; 3], [f64; 3])>,
    /// The LAN guests in this level who trigger blocks (not spectators).
    guest_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Other players that block placement during a guest turn, including live
    /// ancestor bodies; None outside one, where `guest_boxes` are those players.
    building_player_boxes: Option<Vec<([f64; 3], [f64; 3])>>,
    player_collision: Option<PlayerCollision>,
    /// Minecart boxes (detector rails).
    cart_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The item frames a comparator reads through a conductor.
    item_frames: &'w [horizon_client::entities::ItemFrame],
    /// The minecarts, in `cart_boxes` order, for a detector rail's comparator.
    carts: &'w [minecarts::Minecart],
    /// Arrow and thrown trident boxes (wooden buttons).
    arrow_boxes: Vec<([f64; 3], [f64; 3])>,
    /// Every entity on the walk; see `entity::for_each_entity`.
    entities: entity::EntityLists<'w>,
    /// End crystals, which aren't on the walk yet.
    crystals: &'w [dragon::Crystal],
    /// The fishing hooks here, which aren't on the walk either. None ignores
    /// block triggers, so each counts on a plate or tripwire. The flag says
    /// whether it also reaches `entityInside` (`applyEffectsFromBlocks`): not
    /// a hook on an entity, whose tick skips that step.
    trigger_hooks: Vec<(Aabb, bool)>,
    /// The lightning bolts here: none ignores block triggers
    /// (`Entity.isIgnoringBlockTriggers`).
    trigger_extras: Vec<Aabb>,
    biomes: &'w Biomes,
    seed: i64,
    behaviours: &'w Behaviours,
    items: &'w Items,
    /// Cooking recipes' experience, which a removed furnace pays out.
    recipes: &'w horizon_client::crafting::Recipes,
    /// Container loot tables (`unpackLootTable`).
    loot: Option<std::sync::Arc<horizon_world::loot::LootData>>,
    registry: &'static horizon_world::registry::BlockRegistry,
    block_registries: &'w horizon_client::command::args::Registries,
    rules: horizon_world::level::GameRules,
    /// `Difficulty.getId()` (peaceful 0 .. hard 3).
    difficulty: i32,
    /// The feet of the players who keep fire spreading around them: the local
    /// player and the LAN guests in this level, spectators aside.
    fire_players: Vec<[f64; 3]>,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    rand_sequences: &'w mut horizon_client::random_sequences::RandomSequences,
    pending: &'w mut spill::Pending,
    spill: &'w mut spill::Spill,
    /// Where the chunk tickets let blocks tick.
    tickets: horizon_client::ticket_levels::Levels,
    random_tick_sections: &'w mut horizon_client::random_tick_sections::RandomTickSections,
    /// Players with each chest open (a trapped chest's signal), also what a
    /// recount of the openers replaces (`ContainerOpenersCounter.recheckOpeners`).
    lid_openers: &'w mut HashMap<(i32, i32, i32), i32>,
    /// The containers players have open at this moment, by position: the local
    /// player's and the guests' (`getEntitiesWithContainerOpen`). A double
    /// chest is in it as both halves.
    open_containers: &'w [(i32, i32, i32)],
    /// The chests copper golems have open, a double chest as both halves. They
    /// keep their own count (`chest_openers`), so they stay out of `open_containers`.
    golem_chests: &'w [(i32, i32, i32)],
    /// `Level.isClientSide`: a client's copy of the level (a guest of a LAN
    /// world, a window on its server thread, a dedicated server's client).
    client_side: bool,
    /// Only the integrated or dedicated server dispatches native observations.
    native_authoritative: bool,
    native_player: Option<u128>,
    /// The acting player's entity id, beside `native_player`.
    native_player_entity: Option<i32>,
    /// `Level.isDebug`: `setBlock` changes nothing.
    debug: bool,
    native_sounds: &'w mut Vec<creatures::NativeSound>,
    native_particles: &'w mut Vec<lan_particles::QueuedParticle>,
    sound_seeds: Option<&'w horizon_client::lan::hub::Hub>,
    drop_entity: Option<mod_api::EntityRef>,
    /// The block a mod's machine breaks and the tool it breaks it with
    /// (`native_machine_destroy_block`): `Block.dropResources` rolls that
    /// block's loot with the tool instead of with none. Neighbours that pop
    /// off because of the break drop as with no tool, as in vanilla.
    drop_tool: Option<((i32, i32, i32), Option<ItemStack>)>,
    /// Only this destroyed block has an entity as its loot `this` parameter.
    /// Neighbour/default/machine drops keep the no-entity context.
    drop_breaker_at: Option<BlockPos>,
    /// Where `Block.dropResources` puts the drops instead of popping them,
    /// while a mod's machine breaks a block (`native_machine_destroy_block`).
    drop_into: Option<Vec<ItemStack>>,
}
struct PlayerCollision {
    y: f64,
    descending: bool,
    fall_distance: f64,
    main_hand_item: Arc<str>,
    main_hand_count: i32,
    leather_boots: bool,
}
/// The boxes of the entities that block building, as obstruction candidates.
struct BuildingObstruction(Vec<Aabb>);
impl EntityObstructionWorld for BuildingObstruction {
    type Error = std::convert::Infallible;
    fn obstruction_candidates(
        &mut self,
        _excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> std::result::Result<Vec<ObstructionEntity>, Self::Error> {
        Ok(self
            .0
            .iter()
            .filter(|b| b.intersects(bounds))
            .enumerate()
            .map(|(i, &bounds)| ObstructionEntity {
                identity: EntityIdentity(i as u64),
                root_vehicle: EntityIdentity(i as u64),
                bounds,
                removed: false,
                spectator: false,
                blocks_building: true,
            })
            .collect())
    }
}
/// Whether a block's successful use is `InteractionResult.SUCCESS_SERVER`,
/// which a client does not predict: `useWithoutItem` of a bed, a light
/// block, a charged respawn anchor or a sign. Every other block use the
/// level answers with SUCCESS is `SUCCESS` (`SwingSource.PREDICTED`).
fn server_only_block_use(
    block: &horizon_items::blocks::StateFacts,
    branch: horizon_world::level::UseBranch,
) -> bool {
    branch == horizon_world::level::UseBranch::WithoutItem
        && [
            "AbstractBedBlock",
            "LightBlock",
            "RespawnAnchorBlock",
            "SignBlock",
        ]
        .iter()
        .any(|class| block.has_class(class))
}
/// An entity's box (`EntityDimensions.makeBoundingBox`): `width` wide and
/// `height` tall, standing on `feet`.
fn point_box(feet: [f64; 3], width: f64, height: f64) -> Aabb {
    Aabb::new(
        Vec3::new(feet[0] - width / 2.0, feet[1], feet[2] - width / 2.0),
        Vec3::new(
            feet[0] + width / 2.0,
            feet[1] + height,
            feet[2] + width / 2.0,
        ),
    )
}
impl GameLevel<'_> {
    /// `LilyPadBlock.entityInside`: the boxes of the boats that can break a pad.
    fn boat_trigger_boxes(&self) -> impl Iterator<Item = Aabb> + '_ {
        self.entities.boats.iter().map(boats::Boat::bounds)
    }
    /// How many entities in `area` press plates and tripwires, or with
    /// `LIVING` only the living ones; a constant so a list of other kinds
    /// compiles to no loop at all.
    fn count_triggers<const LIVING: bool>(&self, area: Aabb) -> usize {
        use entity::{Entity as _, for_each_entity};
        let mut count = 0;
        for_each_entity!(self.entities, entity => {
            if (!LIVING || entity.is_living())
                && entity.triggers_blocks()
                && area.intersects(entity.bounds())
            {
                count += 1;
            }
        });
        count
    }
    /// `DetectorRailBlock.getAnalogOutputSignal`: a powered rail reads the
    /// minecart on it, a command block cart first (its success count), else the first container cart's fullness.
    fn detector_rail_signal(&self, pos: BlockPos, powered: bool) -> i32 {
        if !powered {
            return 0;
        }
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        let area = Aabb::new(
            Vec3::new(x + 0.2, y, z + 0.2),
            Vec3::new(x + 0.8, y + 0.8, z + 0.8),
        );
        let mut container = None;
        for ((lo, hi), cart) in self.cart_boxes.iter().zip(self.carts) {
            let bounds = Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            );
            if !area.intersects(bounds) {
                continue;
            }
            match cart.kind {
                minecarts::CartKind::CommandBlock => return cart.command.success_count,
                minecarts::CartKind::Chest | minecarts::CartKind::Hopper => {
                    // Counted here, for the cart a comparator reads, not for
                    // every cart on every block tick.
                    container = container.or_else(|| Some(self.cart_signal(cart)));
                }
                _ => {}
            }
        }
        container.unwrap_or(0)
    }
    /// A container minecart's comparator signal: its open menu's slots while
    /// someone has it open, else its own.
    fn cart_signal(&self, cart: &minecarts::Minecart) -> i32 {
        let key = (i32::MAX - 15, minecarts::CART_KEY_Y, cart.id as i32);
        let slots = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Chest(open)) => open,
            _ => &cart.items,
        };
        minecarts::Minecart::container_signal(slots, self.items)
    }
    /// Every entity box that blocks building: the players who aren't
    /// spectators, the minecarts and the `Blockers` (a removed mob or a marker
    /// armor stand, whose box is empty, doesn't).
    fn building_blocker_boxes(&self) -> Vec<Aabb> {
        let boxed = |(lo, hi): &([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        let b = self.entities;
        self.player_box
            .iter()
            .chain(
                self.building_player_boxes
                    .as_ref()
                    .unwrap_or(&self.guest_boxes),
            )
            .chain(&self.cart_boxes)
            .map(boxed)
            .chain(b.mobs.iter().filter(|m| !m.removed).map(Mob::bounds))
            .chain(
                b.armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
            .chain(b.boats.iter().map(boats::Boat::bounds))
            .chain(self.crystals.iter().map(dragon::Crystal::bounds))
            .chain(b.tnts.iter().map(tnt::PrimedTnt::bounds))
            .chain(
                b.falling
                    .iter()
                    .map(|f| horizon_client::falling_block::bounds(f.position)),
            )
            .collect()
    }
}
impl LevelAccess for GameLevel<'_> {
    fn within_world_border(&self, pos: BlockPos) -> bool {
        self.border
            .is_within_bounds(f64::from(pos.x), f64::from(pos.z), 0.0)
    }
    fn redstone_experiments(&self) -> bool {
        self.redstone_experiments
    }
    fn allow_hopper_pickup_item(&self, pos: BlockPos, item: &str, count: u32) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_hopper_pickup_item(self.dimension, pos, item, count, &mut cancel);
        }
        !cancel
    }
    fn has_native_block_entity(&self, state: BlockStateId) -> bool {
        self.world.block_entities.stateful_entity(state.0)
    }
    fn ensure_block_entity(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        if !self.world.block_entities.stateful_entity(state.0)
            || self.world.block_entities.get(&key).is_some()
        {
            return;
        }
        if let Some(entity) = self.new_block_entity_for_state(state) {
            self.world.block_entities.insert(key, entity);
        }
    }
    fn observe_block_changed(&self, pos: BlockPos, old: BlockStateId, new: BlockStateId) {
        if self.native_authoritative {
            self.native_block_changed(self.dimension, pos, old, new);
        }
    }
    fn neighbor_notify(&self, pos: BlockPos, skipped: Option<Direction>) -> bool {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::NeighborNotify)
        {
            return false;
        }
        let mut cancel = false;
        let state = self.block_state(pos);
        self.native_neighbor_notify(self.dimension, pos, state, skipped, &mut cancel);
        cancel
    }
    fn observe_neighbor_changed(
        &self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        changed: horizon_world::level::BlockId,
        moved: bool,
    ) {
        if self.native_authoritative
            && api_values::block_state::block_listens(mod_api::HookId::NeighborChanged, state)
        {
            use api_values::TryIntoApi;
            let Ok(changed) = self.behaviours.table.default_state(changed).try_into_api() else {
                return;
            };
            self.native_neighbor_changed(ticks, self.dimension, pos, state, changed.block(), moved);
        }
    }
    fn affect_neighbors_after_removal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
        updates: &mut Vec<horizon_core::neighbour_update::NeighbourUpdate>,
    ) {
        if self.native_authoritative {
            self.native_affect_neighbors_after_removal(self.dimension, pos, state, moved, updates);
        }
    }
    fn allow_spread(
        &self,
        kind: horizon_world::level::Spread,
        from: BlockPos,
        to: BlockPos,
    ) -> bool {
        let mut canceled = false;
        if self.native_authoritative {
            let kind = match kind {
                horizon_world::level::Spread::Fluid => "fluid",
                horizon_world::level::Spread::Fire => "fire",
            };
            self.native_block_spread(self.dimension, kind, from, to, &mut canceled);
        }
        !canceled
    }
    fn fluid_place_block(
        &self,
        pos: BlockPos,
        liquid_pos: BlockPos,
        state: BlockStateId,
    ) -> Option<BlockStateId> {
        let mut new_state = state;
        let mut cancel = false;
        if self.native_authoritative {
            self.native_fluid_place_block(
                self.dimension,
                pos,
                liquid_pos,
                state,
                &mut new_state,
                &mut cancel,
            );
        }
        (!cancel).then_some(new_state)
    }
    fn allow_fluid_level_change(
        &self,
        pos: BlockPos,
        fluid: &'static str,
        from: i32,
        to: i32,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_fluid_level_change(self.dimension, pos, fluid, from, to, &mut cancel);
        }
        !cancel
    }
    fn create_fluid_source(&self, pos: BlockPos, state: BlockStateId, vanilla: bool) -> bool {
        let mut can_convert = vanilla;
        if self.native_authoritative {
            self.native_create_fluid_source(self.dimension, pos, state, &mut can_convert);
        }
        can_convert
    }
    fn mod_fluid_rules(
        &self,
        pos: BlockPos,
        fluid: horizon_world::fluid::ModFluid,
        registered: horizon_world::fluid::ModFluidRules,
    ) -> horizon_world::fluid::ModFluidRules {
        if !self.native_authoritative {
            return registered;
        }
        Self::answered_fluid_rules(self.dimension, pos, fluid, registered, |call| {
            self.native_fluid_rules(call)
        })
    }
    fn allow_block_ignite(&self, pos: BlockPos, cause: &'static str) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_ignite(self.dimension, self.native_player, pos, cause, &mut cancel);
        }
        !cancel
    }
    fn block_grow_feature<'a>(
        &mut self,
        pos: BlockPos,
        feature: Option<&'a str>,
    ) -> Option<std::borrow::Cow<'a, str>> {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::BlockGrowFeature)
        {
            return feature.map(std::borrow::Cow::Borrowed);
        }
        let mut feature = feature.map(str::to_owned);
        let mut cancel = false;
        self.native_block_grow_feature(self.dimension, pos, &mut feature, &mut cancel);
        feature.filter(|_| !cancel).map(std::borrow::Cow::Owned)
    }
    fn crop_grow_pre(&mut self, pos: BlockPos, state: BlockStateId, vanilla: bool) -> bool {
        if !self.native_authoritative || !mod_api::__private::listened(mod_api::HookId::CropGrowPre)
        {
            return vanilla;
        }
        let mut result = mod_api::TriState::Default;
        self.native_crop_grow_pre(self.dimension, pos, state, &mut result);
        result.or(vanilla)
    }
    fn crop_grow_post(&mut self, pos: BlockPos, original: BlockStateId) {
        if self.native_authoritative && mod_api::__private::listened(mod_api::HookId::CropGrowPost)
        {
            let state = self.block_state(pos);
            self.native_crop_grow_post(self.dimension, pos, original, state);
        }
    }
    fn allow_block_burn(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_burn(self.dimension, pos, &mut cancel);
        }
        !cancel
    }
    fn allow_leaves_decay(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_leaves_decay(self.dimension, pos, &mut cancel);
        }
        !cancel
    }
    fn allow_moisture_change(&self, pos: BlockPos, from: i32, to: i32) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_moisture_change(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    fn observe_sponge_absorb(&self, pos: BlockPos) {
        if self.native_authoritative {
            self.native_sponge_absorb(self.dimension, pos);
        }
    }
    fn allow_block_fade(&self, pos: BlockPos, from: BlockStateId, to: BlockStateId) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_fade(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    fn allow_block_form(&self, pos: BlockPos, from: BlockStateId, to: BlockStateId) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_form(self.dimension, pos, from, to, &mut cancel);
        }
        !cancel
    }
    fn note_block_change(
        &self,
        pos: BlockPos,
        instrument: u8,
        old: i32,
        mut new: i32,
    ) -> Option<i32> {
        let mut cancel = false;
        if self.native_authoritative {
            let context = mod_api::NoteBlockChange {
                dimension: self.dimension,
                pos,
                instrument: mod_api::NoteInstrument::ALL[usize::from(instrument)],
                old_note: old,
            };
            self.native_note_block_change(&context, &mut new, &mut cancel);
        }
        (!cancel).then_some(new)
    }
    fn note_block_play(
        &self,
        pos: BlockPos,
        by_player: bool,
        instrument: &mut u8,
        note: &mut i32,
    ) -> bool {
        use mod_api::NoteInstrument;
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::NoteBlockPlay)
        {
            return true;
        }
        let mut played_instrument = NoteInstrument::ALL[usize::from(*instrument)];
        let mut played = *note;
        let mut cancel = false;
        let (player, player_entity_id) = if by_player {
            (self.native_player, self.native_player_entity)
        } else {
            (None, None)
        };
        self.native_note_block_play(mod_api::__private::NoteBlockPlayCall {
            dimension: self.dimension,
            player,
            player_entity_id,
            pos,
            instrument: &mut played_instrument,
            note: &mut played,
            cancel: &mut cancel,
        });
        *instrument = played_instrument as u8;
        // A note out of 0 to 24 keeps the block's own.
        if (0..=24).contains(&played) {
            *note = played;
        }
        !cancel
    }
    fn allow_portal_spawn(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        shape: &horizon_world::level::PortalShape,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative && mod_api::__private::listened(mod_api::HookId::PortalSpawn) {
            let b = shape.bottom_left;
            let shape = mod_api::PortalShape::from(mod_api::__private::built::PortalShape {
                x_axis: shape.x_axis,
                bottom_left: [b.x, b.y, b.z],
                width: shape.width,
                height: shape.height,
            });
            self.native_portal_spawn(self.dimension, pos, state, shape, &mut cancel);
        }
        !cancel
    }
    fn allow_bell_ring(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_bell_ring(self.dimension, self.native_player, pos, &mut cancel);
        }
        !cancel
    }
    fn allow_cauldron_level_change(
        &self,
        pos: BlockPos,
        from: i32,
        to: i32,
        reason: &'static str,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_cauldron_level_changed(mod_api::__private::CauldronLevelChangedCall {
                dimension: self.dimension,
                player: self.native_player,
                pos,
                from_level: from,
                to_level: to,
                reason,
                cancel: &mut cancel,
            });
        }
        !cancel
    }
    fn allow_piston_move(
        &self,
        piston: BlockPos,
        direction: horizon_core::geometry::Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) -> bool {
        let mut canceled = false;
        if self.native_authoritative {
            let step = direction.normal();
            self.native_piston_pre(mod_api::__private::PistonPreCall {
                dimension: self.dimension,
                piston,
                direction: step,
                extending,
                push,
                destroy,
                canceled: &mut canceled,
            });
        }
        !canceled
    }
    fn observe_piston_moved(
        &self,
        piston: BlockPos,
        direction: horizon_core::geometry::Direction,
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
    ) {
        if self.native_authoritative {
            let step = direction.normal();
            self.native_piston_post(self.dimension, piston, step, extending, push, destroy);
        }
    }
    fn adjust_signal_source(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut source = vanilla;
        if self.native_authoritative {
            self.native_is_signal_source(state, &mut source);
        }
        source
    }
    fn has_weak_power_hook(&self) -> bool {
        self.native_authoritative
            && mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldCheckWeakPower)
    }
    fn adjust_should_check_weak_power(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: bool,
    ) -> bool {
        let mut check = vanilla;
        self.native_should_check_weak_power(self.dimension, pos, state, direction, &mut check);
        check
    }
    fn has_redstone_wire_connection_hook(&self) -> bool {
        self.native_authoritative
            && mod_api::__private::has_scoped_hook(mod_api::HookId::ShouldRedstoneWireConnectTo)
    }
    fn adjust_should_redstone_wire_connect_to(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Option<Direction>,
        vanilla: bool,
    ) -> bool {
        let mut connected = vanilla;
        self.native_should_redstone_wire_connect_to(
            self.dimension,
            pos,
            state,
            direction,
            &mut connected,
        );
        connected
    }
    fn has_connection_hook(&self) -> bool {
        self.native_authoritative
            && mod_api::__private::has_scoped_hook(mod_api::HookId::ConnectsTo)
    }
    fn adjust_connects_to(
        &self,
        kind: horizon_core::connection_kind::ConnectionKind,
        pos: BlockPos,
        state: BlockStateId,
        neighbour: BlockStateId,
        direction: Direction,
        face_solid: bool,
        vanilla: bool,
    ) -> bool {
        let mut connected = vanilla;
        self.native_connects_to(native_hooks::ConnectsToCall {
            dimension: self.dimension,
            pos,
            state,
            kind,
            neighbour,
            direction,
            face_solid,
            connected: &mut connected,
        });
        connected
    }
    fn adjust_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        direct: bool,
        vanilla: i32,
    ) -> i32 {
        native_level_signals::adjust_signal(direction, vanilla, |face, signal| {
            if self.native_authoritative {
                self.native_block_signal(self.dimension, pos, state, face, direct, signal);
            }
        })
    }
    fn adjust_has_analog_output(&self, state: BlockStateId, vanilla: bool) -> bool {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_has_analog_output(state, &mut output);
        }
        output
    }
    fn adjust_analog_output(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        vanilla: i32,
    ) -> i32 {
        let mut output = vanilla;
        if self.native_authoritative {
            self.native_analog_output(self.dimension, pos, state, direction, &mut output);
        }
        output.clamp(0, 15)
    }
    fn before_random_tick(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_random_tick(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    fn before_tick_block(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: &mut BlockStateId,
        handled: &mut bool,
    ) {
        if self.native_authoritative {
            // Any block the hook writes moves the world's revision on.
            let revision = self.world.revision;
            self.native_tick_block(ticks, self.dimension, pos, *state, handled);
            if !*handled && self.world.revision != revision {
                *state = self.block_state(pos);
            }
        }
    }
    fn before_update_shape(
        &self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockStateId,
        new: &mut Option<BlockStateId>,
    ) {
        if self.native_authoritative {
            self.native_update_shape(
                ticks,
                native_hooks::UpdateShapeCall {
                    dimension: self.dimension,
                    pos,
                    state,
                    direction,
                    neighbour_pos,
                    neighbour,
                    new,
                },
            );
        }
    }
    #[allow(clippy::option_option)]
    fn before_state_for_placement(
        &self,
        state: BlockStateId,
        context: &PlacementContext,
        fluid: FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        if self.native_authoritative
            && let Some(fluid) = native_world_read::api_fluid_state(fluid)
        {
            self.native_state_for_placement(self.dimension, state, context, fluid, answer);
        }
    }
    fn before_can_survive(&self, pos: BlockPos, state: BlockStateId, survives: &mut Option<bool>) {
        if self.native_authoritative {
            self.native_can_survive(self.dimension, pos, state, survives);
        }
    }
    fn before_may_place_on(
        &self,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        may: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_may_place_on(self.dimension, state, ground, ground_pos, may);
        }
    }
    fn is_client_side(&self) -> bool {
        self.client_side
    }
    fn is_valid_bonemeal_target(&self, pos: BlockPos, state: BlockStateId) -> Option<bool> {
        // Asked on both sides, as vanilla asks `isValidBonemealTarget`: a
        // client predicts the use with it. Only a mod answers.
        let mut valid = None;
        self.native_is_valid_bonemeal_target(self.dimension, pos, state, &mut valid);
        valid
    }
    fn is_bonemeal_success(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        let mut success = false;
        if self.native_authoritative {
            self.native_is_bonemeal_success(self.dimension, pos, state, &mut success);
        }
        success
    }
    fn perform_bonemeal(&mut self, ticks: &mut Ticks, pos: BlockPos, state: BlockStateId) {
        if self.native_authoritative {
            self.native_perform_bonemeal(ticks, self.dimension, pos, state);
        }
    }
    fn before_fire_odds(&self, state: BlockStateId, odds: &mut Option<(i32, i32)>) {
        if self.native_authoritative {
            self.native_fire_odds(self.dimension, state, odds);
        }
    }
    fn before_smoke_source(&self, state: BlockStateId, smoke: &mut Option<bool>) {
        if self.native_authoritative {
            self.native_smoke_source(self.dimension, state, smoke);
        }
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
    fn precipitation(&self, pos: BlockPos) -> Option<horizon_world::level::Precipitation> {
        self.behaviours.precipitation_info(self, pos)
    }
    fn is_raining_at(&self, pos: BlockPos) -> bool {
        self.behaviours.is_raining_at(self, pos)
    }
    /// `getEntitiesWithContainerOpen` is not empty: a player or guest has the
    /// container open, or a copper golem has the chest open
    /// (`CopperGolem.hasContainerOpen`).
    fn container_open(&self, pos: BlockPos) -> bool {
        let key = (pos.x, pos.y, pos.z);
        self.open_containers.contains(&key) || self.golem_chests.contains(&key)
    }
    /// `ContainerOpenersCounter.recheckOpeners` for the block entity at `pos`,
    /// through `ChestBlock.tick`, `EnderChestBlock.tick` and `BarrelBlock.tick`:
    /// the count a recount of the players with that container open finds, and
    /// the count it replaces. Only the block entities that keep a counter and
    /// a scheduled recheck answer; anything else is `None`.
    fn recheck_container_openers(&mut self, pos: BlockPos) -> Option<(i32, i32)> {
        use horizon_world::level::classes::Class;
        let data = self.behaviours.data(self.world.block(pos.x, pos.y, pos.z));
        if !(data.is(Class::Chest) || data.is(Class::EnderChest) || data.is(Class::Barrel)) {
            return None;
        }
        Some(crate::game::block_entity_anims::recheck_opener_count(
            self.lid_openers,
            self.open_containers,
            (pos.x, pos.y, pos.z),
        ))
    }

    fn block_entity_analog_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        direction: Direction,
    ) -> i32 {
        if let Some(s) = self.registry.state(state)
            && s.name == "minecraft:detector_rail"
        {
            return self.detector_rail_signal(
                pos,
                s.properties.get("powered").is_some_and(|v| v == "true"),
            );
        }
        if let Some(signal) = command_blocks::analog_signal(self.world, (pos.x, pos.y, pos.z)) {
            return signal;
        }
        if let Some(signal) = sculk::sensor_signal(self.world, self.behaviours, state, pos) {
            return signal;
        }
        if let Some((_, comparator)) = jukebox::jukebox_signals(self.world, self.items, pos) {
            return comparator;
        }
        if let Some(signal) = creaking::heart_signal(self.world, pos) {
            return if self.registry.state(state).is_some_and(|s| {
                s.properties
                    .get("creaking_heart_state")
                    .is_some_and(|v| v == "uprooted")
            }) {
                0
            } else {
                signal
            };
        }
        if let Some(signal) = books::lectern_signal(self.world, pos) {
            return signal;
        }
        if let Some(signal) =
            machines::shelf_signal(self.world, self.registry, state, pos, direction)
        {
            return signal;
        }
        machines::container_signal(self.world, self.behaviours, self.items, self.registry, pos)
    }
    fn item_frame_analog_output(&self, pos: BlockPos, facing: Direction) -> Option<i32> {
        let mut frames = self
            .item_frames
            .iter()
            .filter(|f| f.pos == [pos.x, pos.y, pos.z] && f.facing == facing as u8);
        let frame = frames.next().filter(|_| frames.next().is_none())?;
        // ItemFrame.getAnalogOutput.
        Some(
            frame
                .item
                .as_ref()
                .map_or(0, |_| i32::from(frame.rotation % 8) + 1),
        )
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        native_level_signals::block_entity_signal(self.world, self.items, self.lid_openers, pos)
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }
    fn world_seed(&self) -> i64 {
        self.seed
    }
    fn day_time(&self) -> i64 {
        self.world.day_time
    }
    fn game_rules(&self) -> horizon_world::level::GameRules {
        self.rules
    }
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    /// `ServerLevel.canSpreadFireAround`: a player (not a spectator) closer
    /// than the `fire_spread_radius_around_player` rule to the block's corner
    /// (`ChunkMap.anyPlayerCloseEnoughTo`); -1 spreads everywhere.
    fn can_spread_fire_around(&self, pos: BlockPos) -> bool {
        let radius = self.rules.fire_spread_radius_around_player;
        radius == -1
            || self.fire_players.iter().any(|p| {
                let d = [
                    p[0] - f64::from(pos.x),
                    p[1] - f64::from(pos.y),
                    p[2] - f64::from(pos.z),
                ];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < f64::from(radius) * f64::from(radius)
            })
    }
    fn increased_fire_burnout(&self, pos: BlockPos) -> bool {
        self.world
            .chunks
            .get(&(pos.x >> 4, pos.z >> 4))
            .is_some_and(|chunk| {
                self.biomes
                    .get(chunk.biome(pos.x, pos.y, pos.z))
                    .increased_fire_burnout
            })
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        // `Level.setBlock`: a server level of the Debug preset never changes.
        if self.debug && !self.client_side {
            return None;
        }
        if !self.world.loaded(pos.x, pos.z)
            || pos.y < self.world.bottom_y()
            || pos.y >= self.world.top_y()
        {
            return None;
        }
        let previous = self.world.block(pos.x, pos.y, pos.z);
        self.world
            .set_with_notification(pos.x, pos.y, pos.z, state, false)
            .ok()?;
        Some(previous)
    }
    fn send_block_updated(&mut self, pos: BlockPos) {
        if (self.client_side || self.tickets.block_ticking(pos.x >> 4, pos.z >> 4))
            && let Some(log) = &mut self.world.block_log
        {
            log.push((pos.x, pos.y, pos.z));
        }
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    fn min_y(&self) -> i32 {
        self.world.shape.min_y
    }
    fn max_y(&self) -> i32 {
        // Exclusive, as LevelAccess::max_y documents.
        self.world.shape.max_y()
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.world.random
    }
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 15)
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn sun_angle(&self, _pos: BlockPos) -> f32 {
        self.environment
            .sample(&horizon_client::environment::Sample {
                day_time: self.world.day_time,
                ..Default::default()
            })
            .sun_angle
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn is_thundering(&self) -> bool {
        self.thundering
    }
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
    /// `Level.isUnobstructed`: the placed state's collision shape against the player.
    /// `Level.isUnobstructed(state, pos, CollisionContext.of(player))`: no
    /// entity that blocks building is in the state's collision shape.
    fn is_unobstructed(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let player = self.player_collision.as_ref();
        let context = player.map_or_else(
            || CollisionContext::empty(false),
            |player| {
                CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(0),
                        kind: horizon_world::collision_context::EntityContextKind::Player,
                        y: player.y,
                        descending: player.descending,
                        main_hand: Some(CollisionItem {
                            item: &player.main_hand_item,
                            count: player.main_hand_count,
                        }),
                    },
                    false,
                )
            },
        );
        let mut scene = collisions::SceneCollisions {
            world: &*self.world,
            binding: self.collision,
            subject: player.map(|player| horizon_client::collisions::PowderSnowSubject {
                fall_distance: player.fall_distance,
                falling_block: false,
                leather_boots: player.leather_boots,
                walkable_type: false,
                lava_walker: false,
            }),
        };
        let Ok(shape) = self
            .collision
            .collision_shape_with_context(&mut scene, state, pos, &context)
        else {
            return false;
        };
        if shape.is_empty() {
            return true;
        }
        let shape = shape.moved(horizon_core::geometry::Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        horizon_world::entity_obstruction::is_unobstructed(
            &mut BuildingObstruction(self.building_blocker_boxes()),
            None,
            &shape,
        )
        .unwrap_or(false)
    }
    /// `ServerLevel.shouldTickBlocksAt`: only the block-ticking ticket range.
    fn should_tick_blocks_at(&self, chunk: horizon_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z)
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded`: block ticking, a
    /// ticking chunk, and its entities loaded.
    fn is_ticking_chunk(&self, chunk: horizon_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z)
            && ticking_chunk(self.world, chunk.x, chunk.z)
            && !self.world.entities_pending.contains(&(chunk.x, chunk.z))
    }
    /// Entities that trigger pressure plates, tripwires, detector rails and
    /// wooden buttons: the players (the host and the LAN guests), the mobs,
    /// the minecarts and the other entities that don't ignore block triggers,
    /// and for `EntityQuery::Arrows` the arrows.
    fn count_entities(&self, area: Aabb, query: EntityQuery) -> usize {
        let hit = |(lo, hi): &([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ))
        };
        let carts = || self.cart_boxes.iter().filter(|b| hit(b)).count();
        let others = || {
            self.crystals
                .iter()
                .map(dragon::Crystal::bounds)
                .chain(self.trigger_hooks.iter().map(|&(bounds, _)| bounds))
                .chain(self.trigger_extras.iter().copied())
                .filter(|b| area.intersects(*b))
                .count()
        };
        let arrows = || self.arrow_boxes.iter().filter(|b| hit(b)).count();
        let players = || {
            self.player_box
                .iter()
                .chain(&self.guest_boxes)
                .filter(|b| hit(b))
                .count()
        };
        match query {
            EntityQuery::Minecart => carts(),
            EntityQuery::Arrows => arrows(),
            EntityQuery::Mobs => players() + self.count_triggers::<true>(area),
            EntityQuery::Everything => players() + self.count_triggers::<false>(area) + others(),
        }
    }
    fn emit(&mut self, event: LevelEvent) {
        match event {
            // Block.dropResources draws its loot and spots right away.
            // `popResource` only drops server-side (`!level.isClientSide`):
            // a client's moveBlocks (a piston's block event) skips it.
            LevelEvent::DropResources { pos, state } if !self.client_side => {
                self.drop_resources_now(pos, state)
            }
            LevelEvent::DropResources { .. } => {}
            // `ClientLevel.playSeededSound` plays only when the excluded
            // player is this client's own: the block's `playSound(null, ...)`
            // (a piston's move, a note) reaches it from the server instead.
            LevelEvent::Sound { .. } if self.client_side => {}
            // NoteBlock.triggerEvent plays the head's sound with a null
            // player, so only the server's level plays it; a guest or a
            // remote client hears it from the sound packet.
            LevelEvent::BlockEvent {
                kind: "note_custom_sound",
                ..
            } if self.client_side => {}
            // Block.popResource of the block's own item (tall flowers, flower beds).
            LevelEvent::BlockEvent {
                pos,
                state,
                kind: "pop_block_item",
            } => self.pop_block_item(pos, state),
            // ComposterBlock.extractProduce: bone meal above the composter.
            LevelEvent::Player {
                pos,
                effect: PlayerEffect::DropItem(item),
                ..
            } => self.pop_above(pos, item),
            // Picking a sweet berry bush or cave vines: the harvest loot,
            // popped before the pick sound draws its pitch.
            LevelEvent::Player {
                pos,
                state,
                effect: PlayerEffect::InteractLoot(table),
            } => self.drop_interact_loot(pos, state, table),
            event => self.events.push(event),
        }
    }
    fn has_mod_block_entity(&self, pos: BlockPos) -> bool {
        // Asked on every block change: no lookup until a mod block entity exists.
        self.world.block_entities.has_mods()
            && matches!(
                self.world.block_entities.get(&(pos.x, pos.y, pos.z)),
                Some(BlockEntity::Mod(_))
            )
    }
    /// `ShulkerBoxBlockEntity.isClosed`: the lid state the block-entity tick
    /// copied from `be_anims` (`ShulkerBoxBlock.getBlockSupportShape`).
    fn shulker_is_closed(&self, pos: BlockPos) -> Option<bool> {
        self.world
            .shulkers
            .get(&[pos.x, pos.y, pos.z])
            .map(|s| s.status == horizon_client::block_entity_anim::ShulkerStatus::Closed)
    }
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        if let Some(change) = &mut self.command_block_transition
            && change.preserve(self.world, pos, old)
        {
            // Returning 0 skips remove_entity's side effects once; command blocks have no contents or drops.
            return 0;
        }
        self.remove_entity(pos, old, side_effects)
    }
    fn load_block_entity(&mut self, pos: BlockPos, mut nbt: horizon_core::nbt::Compound) {
        use horizon_core::nbt::Tag;
        for (k, v) in [("x", pos.x), ("y", pos.y), ("z", pos.z)] {
            nbt.insert(k.into(), Tag::Int(v));
        }
        let block = self.world.block(pos.x, pos.y, pos.z);
        let Some(block) = self.registry.state(block) else {
            return;
        };
        let name = &block.name;
        let tag = Tag::Compound(nbt);
        if let Some((key, entity)) = horizon_client::save::decode_block_entity_for_block(&tag, name)
        {
            self.world.block_entities.insert(key, entity);
            if tag.get("LootTable").is_some() {
                self.emit(LevelEvent::ContainerLootTable { pos, tag });
            }
        }
    }
    fn create_feature_spawner(&mut self, pos: BlockPos, nbt: horizon_core::nbt::Compound) {
        // Guard at the owner/insert boundary as well as the live feature sink.
        // Existing entries of every class retain the prior no-op behavior.
        if self
            .registry
            .state(self.world.block(pos.x, pos.y, pos.z))
            .is_none_or(|state| state.name != "minecraft:spawner")
            || self
                .world
                .block_entities
                .contains_key(&(pos.x, pos.y, pos.z))
        {
            return;
        }
        self.load_block_entity(pos, nbt);
    }
    fn set_command_block_powered(&mut self, pos: BlockPos, powered: bool) -> Option<(bool, bool)> {
        command_blocks::set_powered(self.world, self.registry, (pos.x, pos.y, pos.z), powered)
    }
    /// `PotentSulfurBlockEntity.resetCountdown`.
    fn reset_potent_sulfur_countdown(&mut self, pos: BlockPos) {
        if let Some(BlockEntity::PotentSulfur(sulfur)) =
            self.world.block_entities.get_mut(&(pos.x, pos.y, pos.z))
        {
            sulfur.waiting_countdown = -1;
        }
    }
}
/// A landing's block game event, emitted after its world write is applied.
struct LandingVibration {
    event: &'static str,
    position: BlockPos,
    state: BlockStateId,
    from_entity: bool,
}

/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
    /// `Entity.yRot` and `xRot` (`/tp`, `/data get ... Rotation`).
    pub rotation: [f32; 2],
    /// When it joined its list (`horizon_client::join_seq`); set by the list.
    pub(super) join: horizon_client::join_seq::JoinSeq,
    /// Its entity id: from the entity counter on the server, the server's on
    /// a client.
    id: u32,
    state: BlockStateId,
    /// `DATA_START_POS`: spawn position used to select weighted block models.
    start_pos: BlockPos,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    /// `FallingBlockEntity.time`: signed, a save can start it below zero.
    age: i32,
    /// `Entity.isNoGravity`.
    no_gravity: bool,
    /// `FallingBlockEntity.tick` handles portals after moving, before landing.
    portal: horizon_client::nether_portal::PortalState,
    /// `FallingBlockEntity.dropItem`.
    drop_item: bool,
    /// `Entity.fallDistance` (powder snow holds a block that fell far at 0.9).
    fall_distance: f64,
    /// `Entity.onGround`: whether its last move was stopped going down.
    on_ground: bool,
    /// `Entity.stuckSpeedMultiplier`: a cobweb it is in scales its next move.
    stuck: Vec3,
    damage: horizon_client::falling_block::Damage,
    /// The UUID and compound it was loaded with, kept while it moves.
    saved: Option<Box<([i32; 4], horizon_core::nbt::Tag)>>,
}
horizon_mod_runtime::impl_attachment_lifetime!(FallingBlock);

impl FallingBlock {
    /// Where `FallingBlockRenderer` draws the block's model corner, `alpha`
    /// into the tick: centred on the entity, then moved by `offset`, the
    /// block's random model offset (`ModelBlockRenderer.tesselateBlock`:
    /// `BlockState.getOffset` of `BlockPos.containing(x, maxY, z)`), so a
    /// pointed dripstone keeps the place it hung at while it falls.
    fn model_origin(
        &self,
        alpha: f64,
        offset: impl Fn(BlockStateId, i32, i32) -> Option<[f64; 3]>,
    ) -> glam::DVec3 {
        let [dx, dy, dz] = offset(
            self.state,
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        )
        .unwrap_or([0.0; 3]);
        glam::DVec3::new(
            self.previous.x + (self.position.x - self.previous.x) * alpha - 0.5 + dx,
            self.previous.y + (self.position.y - self.previous.y) * alpha + dy,
            self.previous.z + (self.position.z - self.previous.z) * alpha - 0.5 + dz,
        )
    }
}

/// A block entity's `mark_motion` id: its position, packed into the 32 bits beside the tag
/// (x and z wrap at 2048 blocks and y at 1024; two block entities that far apart in one frame's
/// view are told apart by their vertex counts, as `PackEntities` matches only equal runs).
fn motion_block_entity(pos: [i32; 3]) -> u64 {
    motion_at(Game::MOTION_BLOCK_ENTITY, pos)
}

/// `tag` with a block position packed into its low 32 bits (`motion_block_entity`).
fn motion_at(tag: u64, pos: [i32; 3]) -> u64 {
    let [x, y, z] = pos.map(|c| c as u32);
    tag | u64::from((x & 0x7FF) | (z & 0x7FF) << 11 | (y & 0x3FF) << 22)
}

/// `FallingBlockRenderer.shouldRender` drops a falling block whose own state is
/// the state of the cell it is in (`level.getBlockState(entity.blockPosition())`):
/// a `NoGravity` block, or one that has just landed, sits inside an identical
/// block, and vanilla draws no model for it.
fn falling_block_visible(state: BlockStateId, cell: BlockStateId) -> bool {
    state != cell
}

/// The data loaded once at startup and never changed: blocks, items,
/// recipes, biomes, mobs, collision and motion. Read-only and `Send + Sync`,
/// so the client and an integrated server thread can share one copy.
pub struct Resources {
    core: WorldCore,
    rest: std::sync::OnceLock<WorldRest>,
}

struct WorldCore {
    item_data: Arc<horizon_world::item_defaults::ItemData>,
    facts: BlockFacts,
    biomes: Arc<Biomes>,
    environment: Environment,
    collision: Arc<BlockCollisionRegistry<'static>>,
    motion: MotionResources,
    suffocation: horizon_world::suffocation::SuffocationRegistry<'static>,
    spawn_registries: horizon_client::mobs::finalize::SpawnRegistries,
    threads: usize,
}

struct WorldRest {
    items: Items,
    recipes: Recipes,
    transformers: horizon_client::block_transform::Transformers,
    mob_types: HashMap<String, Arc<MobType>>,
    mob_data: horizon_client::mobs::MobData,
    providers: Providers,
    block_entity_render: BlockEntityRenderer,
    animate: horizon_client::animate_tick::AnimateTables,
    fog: horizon_client::fog::FogTables,
    music: horizon_client::music::MusicTables,
    ambient: horizon_client::ambience::AmbientTables,
    nether_environment: Environment,
    end_environment: Environment,
    behaviours: Behaviours,
    /// The player model's right and left arms.
    player_arm: [Option<horizon_client::mobs::Part>; 2],
    /// The slim model's right and left arms (3 pixels wide).
    player_arm_slim: [Option<horizon_client::mobs::Part>; 2],
    player_type: Option<Arc<horizon_client::mobs::MobType>>,
    /// `SpinAttackEffectModel` (Riptide's spinning boxes).
    spin_attack: Option<horizon_client::mobs::Part>,
    stations: horizon_client::stations::StationData,
}

/// The content tables a world's data packs change, loaded when the world
/// opens and on `/reload`: `ReloadableServerResources` loads a server's
/// recipes, loot tables and tags from the world's pack stack, over the
/// resource root's own. Worlds without packs use `Resources.rest` directly.
struct WorldTables {
    conditions: horizon_world::conditions::LoadConditions,
    suffocation: horizon_world::suffocation::SuffocationRegistry<'static>,
    /// The pack files the tables were built from (a native mod's among
    /// them), for the advancement tree reload and the loot data.
    files: Vec<(String, Vec<Vec<u8>>)>,
    items: Items,
    recipes: Recipes,
    stations: horizon_client::stations::StationData,
    reload_sources: Option<Arc<horizon_client::crafting::ResolvedSources>>,
    /// The values of the data map types native mods registered
    /// (`native_data_map_types`).
    data_maps: native_data_maps::DataMaps,
    /// The entity type, biome, structure and damage type tags with the packs'
    /// over the game's own, by registry (`TagLoader.loadTagsForExistingRegistries`):
    /// what the predicates' `#tag`s of those registries read.
    pack_tags: HashMap<&'static str, HashMap<String, HashSet<String>>>,
}

struct PreparedParts {
    block_states: Arc<api_values::block_state::Index>,
    tags: HashMap<String, HashSet<mod_api::BlockKey>>,
    parts: Option<mod_api::Parts>,
}

struct PreparedWorldReload {
    structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns,
    tables: Option<WorldTables>,
    health: horizon_client::health::HealthResources,
    parts: PreparedParts,
    wire: Option<Arc<horizon_client::lan::data::SyncData>>,
}

impl WorldTables {
    /// The registries [`WorldTables::pack_tags`] holds.
    const PACK_TAG_REGISTRIES: [&'static str; 4] = [
        "entity_type",
        "worldgen/biome",
        "worldgen/structure",
        "damage_type",
    ];
}

/// The equipment slot of `armor[index]`: head, chest, legs, feet.
/// `doHurtEquipment`'s `durabilityDamage` before the cast to int: `max(1, damage / 4)`.
pub(super) fn durability_damage(damage: f32) -> f32 {
    (damage / 4.0).max(1.0)
}
/// The durability damage a mod left for `slot` (`mod_api::ArmorHurt::new_damage`, cast to int
/// as vanilla casts it); an entry it removed keeps vanilla's `fallback`.
pub(super) fn answered_wear(
    entries: &[mod_api::ArmorHurt],
    slot: horizon_core::equipment_slot::EquipmentSlot,
    fallback: u32,
) -> u32 {
    entries
        .iter()
        .find(|e| e.slot == slot)
        .map_or(fallback, |e| e.new_damage as u32)
}

pub(super) fn armor_slot(index: usize) -> horizon_core::equipment_slot::EquipmentSlot {
    use horizon_core::equipment_slot::EquipmentSlot;
    match index {
        0 => EquipmentSlot::Head,
        1 => EquipmentSlot::Chest,
        2 => EquipmentSlot::Legs,
        _ => EquipmentSlot::Feet,
    }
}

/// Everything the title and menus draw: the GUI atlas with its font and the
/// title panorama. Kept apart from the world bundle so the title can appear
/// before the block sprites, models and gameplay tables are ready.
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<horizon_client::gfx::panorama::Faces>,
}

/// What a resource pack reload replaces (`Minecraft.reloadResourcePacks`):
/// block models and sprites with their meshing tables, item models,
/// translations and the textures read on the CPU. Each game holds its own;
/// cloning shares the loaded data.
#[derive(Clone)]
pub struct PackResources {
    source: horizon_assets::resource_source::ResourceSource,
    data_source: horizon_assets::resource_source::ResourceSource,
    data_changes: horizon_assets::resource_source::ResourceSource,
    additional_data: Arc<[Arc<horizon_client::datapack::DataPack>]>,
    assets: Arc<Assets>,
    block_registries: Arc<horizon_client::command::args::Registries>,
    render: Arc<RenderAssets>,
    context: Arc<WorkerContext>,
    layer_names: Arc<HashMap<u32, String>>,
    /// Item definitions, baked item models and special renderers.
    item_renderer: Arc<horizon_client::item_render::ItemRenderer>,
    names: Arc<Names>,
    /// clouds.png: (width, height, filled cells).
    clouds: Arc<(u32, u32, Vec<bool>)>,
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    forcefield: Arc<(u32, u32, Vec<u8>)>,
    /// OptiFine custom sky layers (`optifine/sky`) with their textures; empty without them.
    custom_sky: Arc<horizon_client::custom_sky::CustomSky>,
    /// The overworld's, Nether's and End's attribute stacks with the packs' Polytone biome and
    /// dimension modifiers (#11486), when a pack has some: what the frame draws.
    environments: Option<Arc<[Environment; 3]>>,
    /// The packs' Polytone custom particles and block emitters (#11486), when a pack has some.
    polytone_particles: Option<Arc<horizon_client::particles::polytone::Registry>>,
    /// The packs' Polytone post chains (#11486), when a pack has some.
    polytone_post: Option<Arc<horizon_client::polytone_post::PolytonePost>>,
    /// The packs' Polytone custom colours (`polytone/colors.json`).
    polytone_colors: Arc<horizon_client::polytone_colors::PackColors>,
}

/// A quick craft drag as the screen draws it: each dragged slot's count and
/// whether the slot caps it (None: hidden), and what stays in hand.
struct DragView {
    slots: HashMap<usize, Option<(u32, bool)>>,
    left: Option<u32>,
}
fn equipment_modifier_applies(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "armor" && slot != "mainhand" && slot != "offhand")
        || (group == "hand" && (slot == "mainhand" || slot == "offhand"))
}
/// Ctrl+Q reads the physical keyboard, independently of sprint key mappings.
/// A chunk whose ticking future can complete (`ChunkMap.prepareTickingChunk`):
/// lit, with its neighbours loaded, as `postprocess_chunks` waits for.
fn ticking_chunk(world: &World, x: i32, z: i32) -> bool {
    let Some(center) = world.chunks.get(&(x, z)) else {
        return false;
    };
    center.lit && world.chunks.surrounded((x, z))
}
/// `TicketStorage.canTicketExpire` for a `PORTAL` ticket: its chunk's holder is
/// ready for saving once the futures of its level are done, the chunk lit and
/// its neighbours loaded out to one chunk at a ticking level
/// (`prepareTickingChunk`) and two at an entity-ticking one
/// (`prepareEntityTickingChunk`). The ticket's own level stands for the
/// holder's: the area it waits for is the one its own streaming loads.
fn portal_ticket_can_expire(
    world: &World,
    ticket: &horizon_client::ticket_levels::PortalTicket,
) -> bool {
    use horizon_client::ticket_levels::{BLOCK_TICKING, ENTITY_TICKING};
    let r = match ticket.level {
        l if l <= i32::from(ENTITY_TICKING) => 2,
        l if l == i32::from(BLOCK_TICKING) => 1,
        _ => 0,
    };
    let (x, z) = ticket.chunk;
    world.chunks.get(&(x, z)).is_some_and(|c| c.lit)
        && (-r..=r).all(|dz| (-r..=r).all(|dx| world.chunks.contains_key(&(x + dx, z + dz))))
}
/// Key events carry the scancode's US-layout code, which ignores XKB options like
/// `caps:swapescape`. For the keys whose meaning the keymap may remap, take the code
/// from the keysym the keymap resolved instead. This includes function keys on
/// remapped media rows; ordinary character bindings still use their physical key.
fn follow_keymap(event: &mut WindowEvent) {
    if let WindowEvent::KeyboardInput { event, .. } = event {
        follow_named_keymap(&event.logical_key, &mut event.physical_key);
    }
}

/// Q throws the whole stack with Control down: the same modifier word the
/// screens read (`has_control_down`), never a latched sprint toggle.
fn drop_whole_stack(modifiers: winit::keyboard::ModifiersState) -> bool {
    modifiers.control_key()
}

fn select_creative_tab(
    creative: &mut Creative,
    clicks: &mut horizon_client::container_screen::Clicks,
    tab: usize,
) {
    creative.select(tab);
    *clicks = Default::default();
    // Tabs are outside the panel; their release must not drop a carried stack.
    clicks.skip_release();
}

/// The icon the Select World list shows for the world in play
/// (`GameRenderer.tryTakeScreenshotIfNeeded`): its path in the save, and when
/// the last frame grab was asked for.
struct WorldIcon {
    path: PathBuf,
    last_attempt: Option<Instant>,
}

#[derive(Debug, PartialEq)]
struct QueuedLevelEvent {
    kind: i32,
    pos: [i32; 3],
    data: i32,
    source: Option<horizon_world::level::LevelEventPlayer>,
}

/// Physical and scaled GUI dimensions last reported by `window_resize`.
#[derive(Clone, Copy, PartialEq)]
struct ResizeState {
    size: winit::dpi::PhysicalSize<u32>,
    gui_size: [u32; 2],
}

impl ResizeState {
    /// Records `self` as the last report and says whether it differs from the previous one.
    fn differs_from(self, last: &mut Option<Self>) -> bool {
        last.replace(self) != Some(self)
    }
}

/// Temporary block-contact collections rebuilt on every server tick. Keeping
/// their backing storage on `Game` avoids reallocating them while the world is
/// ticking; the wrapper around `tick_blocks` restores the scratch even when a
/// phase returns an error.
#[derive(Default)]
struct TickScratch {
    ticking: Vec<(i32, i32)>,
    inside: Vec<BlockPos>,
    standing: Vec<(BlockPos, bool, f64)>,
    boat_cells: Vec<BlockPos>,
    touched: Vec<BlockPos>,
    sensing: Vec<BlockPos>,
    rest: Vec<BlockPos>,
    later: Vec<BlockPos>,
}

struct Game<'a> {
    /// Shared with an integrated server; `collision` and `motion` borrow from it.
    res: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pending_entry: Option<PendingWorldEntry>,
    /// `DebugEntryChunkGeneration`'s `lastPos` and `result`: F3's `Density` line is sampled
    /// again only when the feet block (or the dimension) changes.
    debug_density: RefCell<debug_keys::DensityCache>,
    pending_pack_reload: bool,
    /// HDR, Smooth Gradients, picked path and outcome at the last frame attempt, including a failed
    /// replacement (`follow_frame`).
    frame_attempted: render_settings::FrameAttempt,
    /// When `resourcepacks/` was last looked at for the open pack screen, and what it held
    /// (`watch_pack_folder`).
    pack_folder: Option<(
        std::time::Instant,
        Vec<(String, u64, Option<std::time::SystemTime>)>,
    )>,
    /// A pack reload staged on a worker (`LoadingOverlay`): the frame draws
    /// its progress until `drive_pack_reload` applies it.
    pack_reload: Option<packs::PackReload>,
    /// A server's resource packs (game/world/server_packs.rs).
    server_packs: server_packs::ServerPackState,
    /// The link a chat click asks to open (`ConfirmLinkScreen`).
    link_prompt: Option<String>,
    /// A command the player didn't type, waiting for their answer
    /// (game/network/remote_chat.rs).
    command_prompt: Option<remote_chat::CommandPrompt>,
    /// A server's Code of Conduct to accept (game/interface/code_of_conduct.rs).
    code_of_conduct: Option<code_of_conduct::Prompt>,
    dialogs: dialogs::DialogState,
    /// Single player's server while it loads its world (`Minecraft.doWorldLoad`).
    integrated_starting: Option<integrated::Starting>,
    world_renderer_ready: bool,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    /// This world's frozen damage definitions and reloadable tag snapshot.
    pub(super) health_resources: horizon_client::health::HealthResources,
    damage_tag_generation: u64,
    /// The registries native mods registered data map types for, as of the
    /// server tables' last build: what `data_maps_updated` fires for.
    data_map_registries: Vec<&'static str>,
    /// This game's resource packs (`reload_packs` replaces them).
    pack: PackResources,
    /// The title and menu bundle (GUI atlas, font and panorama).
    gui: UiResources,
    instance_groups: std::cell::RefCell<native_instance_groups::InstanceGroupStorage>,
    standalone_geometry: native_block_entity_draw::StandaloneGeometry,
    /// `FogRenderer`'s evolving state.
    fog: horizon_client::fog::FogState,
    screen_fx: horizon_client::screen_effects::ScreenEffects,
    /// `LocalPlayer.itemActivation`: the totem flying up to the screen.
    item_activation: horizon_client::item_activation::ItemActivation,
    weather_parameters: horizon_client::weather::ParameterCache,
    panorama_rotation: horizon_client::gfx::panorama::Rotation,
    world: World,
    /// The biome table the world plays with: the game's own with the world's
    /// packs over it (`Biomes::with_packs`), `None` before a world opens.
    world_biomes: Option<Arc<Biomes>>,
    /// `world_biomes` as the biome files made it, before `native_biome_colors`.
    world_biomes_base: Option<Arc<Biomes>>,
    streaming: Streaming,
    lod: Option<horizon_client::lod::Lod>,
    lod_source_attempted: bool,
    lod_observe: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    /// Columns the single-player server changed in bulk inside a mod's hold on the terrain
    /// (`native_world_hold`): far terrain takes them in when the hold lets go of them.
    lod_held: horizon_client::fast_hash::FxHashMap<(i32, i32), Arc<horizon_client::chunk::Chunk>>,
    /// The server level far terrain keeps the chunks of.
    lod_level: Option<String>,
    /// The server's BlueMap far terrain draws from, when the player named one
    /// or Find BlueMap found it.
    lod_blue_map: Option<Arc<horizon_client::bluemap::BlueMap>>,
    /// Beacon beam sections kept after their chunk unloads, so far terrain
    /// LOD still draws their beams (`beacon_geometry` refreshes them).
    far_beacons: HashMap<(i32, i32, i32), Vec<(u32, i32)>>,
    /// The detail revision, centre and radius `Lod::update` last had covered for.
    lod_covered_at: Option<(u64, (i32, i32), i32)>,
    /// Whether the view's columns have all arrived, and how new it is: large
    /// translucent pages wait for both before they shrink (`set_terrain_loading`).
    view_arrival: horizon_client::streaming::ViewArrival,
    options: Options,
    player: Option<PlayerMotion>,
    spawn: Option<Vec3>,
    clock: FixedStep,
    previous_position: Vec3,
    /// `yRotO` / `xRotO` of the local player: the view at the start of the tick,
    /// moved by `Entity.turn` and a boat's `clampRotation` as the view is
    /// (`LocalPlayer.getRopeHoldPosition` eases the held lead from it).
    rope_rot_old: (f32, f32),
    eye: f32,
    previous_eye: f32,
    yaw: f32,
    pitch: f32,
    inventory: Vec<Option<ItemStack>>,
    selected: usize,
    carried: Option<ItemStack>,
    craft_grid: Vec<Option<ItemStack>>,
    craft_size: usize,
    /// The crafting table the 3x3 grid belongs to (`CraftingMenu.access`).
    craft_table: Option<[i32; 3]>,
    /// A quick craft drag over inventory slots: (button, game slots so far).
    inv_drag: Option<(i32, Vec<usize>)>,
    /// `AbstractContainerScreen`'s quick craft and skip-release state.
    inv_clicks: horizon_client::container_screen::Clicks,
    /// The Mouse Tweaks extra's drag and wheel (`tweaks`).
    tweaks: tweaks::Tweaks,
    /// The wheel and hovered slot of `BundleMouseActions`.
    bundle_mouse: bundles::BundleMouse,
    /// The last inventory click, for the double click that gathers a stack.
    last_inv_click: Option<(std::time::Instant, usize, bool)>,
    /// `lastQuickMoved`: what the last Shift click moved, which a Shift
    /// double click moves every stack of.
    last_quick_moved: Option<ItemStack>,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    /// The open container and the slots it had when it opened:
    /// `Container.stillValid` holds only while every block entity the menu
    /// took is still there, so a half of a double chest (or the container)
    /// going away closes it.
    open_container_slots: Option<((i32, i32, i32), usize)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    station: Option<stations::Station>,
    /// Status effects and thrown potions (game/items/potions.rs).
    potions: potions::PotionState,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    /// The other players' cracks (`LevelRenderer.destroyingBlocks`): breaker id to the block,
    /// the stage (0..=9) and the game time it last changed.
    destroying: HashMap<i32, ([i32; 3], u8, i64)>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    /// The game mode; `creative` mirrors `mode == Creative`.
    mode: horizon_client::command::args::GameMode,
    keys: horizon_client::held::HeldKeys,
    // Physical input is separate from remapped and toggled movement keys.
    physical_keys: HashSet<KeyCode>,
    /// Keys whose first press a mod cancelled (`key_mapping_pre`): their repeats are never held.
    rejected_keys: HashSet<KeyCode>,
    physical_buttons: HashSet<MouseButton>,
    drag_button: Option<MouseButton>,
    /// Control + left click as a right click (macOS).
    right_click_emulation: horizon_client_input::mouse::RightClickEmulation,
    left: bool,
    right: bool,
    cursor: (f32, f32),
    /// The cursor icon the window shows (`widgets::take_cursor` each frame).
    cursor_icon: horizon_client::widgets::Cursor,
    captured: bool,
    /// Native grab mode used by winit. Linux window managers can drop the
    /// grab without a focus event, leaving `captured` true but stale.
    capture_mode: Option<CursorGrabMode>,
    /// The window held the pointer when it lost focus: the system lets a
    /// grabbed pointer go with the focus, and a window that comes back still
    /// wanting it takes it again (`MouseHandler.grabMouse`, whose grab
    /// outlives the window's focus).
    regrab_on_refocus: bool,
    /// Only the local player owning keyboard and mouse changes the native pointer.
    keyboard_mouse: bool,
    /// Offscreen rendering into a visible splitscreen window still accepts native input.
    split_window: bool,
    /// This view's corner in its shared window; cursor warps include the offset.
    view_at: [u32; 2],
    /// Splitscreen: what it draws over this player's view (its badge, questions, notices).
    split_hud: split::Hud,
    /// Splitscreen: this player's own Microsoft account session (one of the launcher's other
    /// accounts), in place of the process's (P1's) for its connections and skin.
    split_auth: Option<std::sync::Arc<horizon_client::auth::AuthSession>>,
    focused: bool,
    /// The visible window of a game an agent started, until a person focuses it: it opened
    /// without the focus, and its pointer grab stays inside the game as `--offscreen`'s does,
    /// so the person's pointer is never taken, hidden or moved.
    unclaimed_window: bool,
    /// The window's last focus change during a scripted run, applied once the run ends.
    focus_after_run: Option<bool>,
    /// Whether the window currently accepts input method events.
    ime_allowed: bool,
    /// The candidate window's area last given to the window, in pixels.
    ime_area: Option<[i32; 4]>,
    /// When the window lost focus, while it is still without
    /// (`Minecraft.lastActiveTime`: pausing waits 500 ms).
    focus_lost: Option<Instant>,
    /// The keyboard's modifier state (`KeyEvent.modifiers`).
    modifiers: winit::keyboard::ModifiersState,
    /// The window's fullscreen request and reported state.
    fullscreen: horizon_client::window::FullscreenState,
    /// The monitor whose video modes `settings.video_modes` holds.
    video_modes: horizon_client::window::VideoModeList,
    /// MouseHandler's accumulated look and wheel state.
    mouse: MouseLook,
    /// An agent moved the mouse (`mouse-move`) since the pointer was last
    /// grabbed or released: its motion turns the view like a grabbed pointer.
    agent_pointer: bool,
    /// `Options.smoothCamera` (cinematic camera), not persisted.
    smooth_camera: bool,
    /// Sneak, sprint, attack and use as ToggleKeyMappings.
    toggles: horizon_client::keymapping::ToggleKeys,
    /// A world screen with the pointer captured was open last frame.
    was_playing: bool,
    /// Minecraft.pause: a single-player screen that pauses the game is open.
    pause: bool,
    debug: bool,
    /// `horizon-ctl gpu-passes`: time Horizon's own passes on the GPU with F3 shut.
    gpu_passes: bool,
    screen: Screen,
    observed_screen: Option<native_hooks::ObservedScreen>,
    /// The layout the last frame drew the inventory's effect panels with.
    effects_layout: std::cell::Cell<Option<horizon_client::inventory_effects::EffectsLayout>>,
    window: Option<Arc<Window>>,
    /// What `window_resize` last reported: the physical size and the GUI settings behind the scaled size.
    resize_state: Option<ResizeState>,
    renderer: Option<Renderer>,
    /// Avoid an endless rebuild loop if the new device is lost as well.
    last_device_recovery: Option<Instant>,
    /// CPU geometry storage retained across frames, including empty/title frames.
    entity_vertices: Vec<Vertex>,
    /// Horizon Extras' dynamic lights this frame (`dynamic_lights.rs`), and how brightly each
    /// item glows (built the first time the extra is on).
    dynamic_lights: Vec<[f32; 4]>,
    luminance: std::cell::OnceCell<horizon_client::dynamic_light::Luminance>,
    /// Name tags drawn over everything (`RenderPipelines.TEXT_SEE_THROUGH`).
    see_through_vertices: Vec<Vertex>,
    /// The first-person player's model, which only a native pack's shadow pass draws.
    shadow_only_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// Runs of `entity_vertices` by entity, for a shader pack's per-entity draws; recorded only
    /// while `pack_ids` is set (game/interface/shader_packs.rs).
    entity_spans: std::cell::RefCell<Vec<horizon_client::gfx::pack_draw::EntitySpan>>,
    /// Which entity this frame's entity vertices belong to (`Frame::entity_motion`).
    entity_motion: std::cell::RefCell<Vec<(u32, u64)>>,
    /// The GPU-posed vertex count at each of `entity_motion`'s marks (`Frame::entity_motion_posed`).
    entity_motion_posed: std::cell::RefCell<Vec<u32>>,
    /// Mob models posed on the GPU (`Frame::skin`, `mobs::skin`): the models stored there,
    /// and this frame's draws.
    skin_models: std::cell::RefCell<horizon_client::mobs::skin::SkinModels>,
    skin_frame: std::cell::RefCell<horizon_client::gfx::entity_skin::SkinFrame>,
    /// The glowing entities' silhouettes this frame (game/entities/glowing.rs).
    glowing_vertices: std::cell::RefCell<Vec<Vertex>>,
    /// Client mods' sounds (`play_local_sound`): those waiting for the next audio update
    /// (`tick_sound_engine`), the stops and fades asked of them, and their handles.
    mod_sounds: native_local_sounds::ModSounds,
    /// A client mod asked to clear the particles (`clear_particles`), done before
    /// the next frame or particle tick (`apply_mod_particle_clear`).
    mod_clear_particles: std::cell::Cell<bool>,
    /// The shader pack's entity and block entity ids, while one draws entities.
    pack_ids: Option<shader_packs::PackIds>,
    started: Instant,
    /// `SpectatorGui`: the menu a spectator opens with the hotbar keys.
    spectator_gui: horizon_client::spectator_menu::SpectatorGui,
    previous: Instant,
    /// When the last frame's work ended: what runs before the next one starts (`idle`, the
    /// event loop) shows in HORIZON_SPIKES as the time between frames.
    frame_end: Instant,
    /// FramerateLimitTracker.latestInputTime.
    last_input: Instant,
    /// Controllers (Horizon's own).
    pad: pad::Pads,
    frames: u64,
    /// The glint clock and options, read once at the start of each frame
    /// (game/interface/item_draw.rs).
    glint_frame: horizon_client::item_render::GlintFrame,
    /// Mod block entities' geometry their mods kept
    /// (game/modding/native_block_entity_draw.rs).
    kept_block_entities: std::cell::RefCell<native_block_entity_draw::KeptGeometry>,
    /// Prepared block-entity copies for this frame, in entity-stream order.
    prepared_model_draws: std::cell::RefCell<Vec<horizon_client::gfx::PreparedModelDraw>>,
    world_geometry: Option<horizon_client::world_geometry::WorldGeometry>,
    /// `DebugRenderer.renderers`: the ids `debug_renderer_draw` is asked about.
    debug_renderers: Vec<String>,
    /// `DebugRenderer.lastDebugEntriesVersion`: none before the first view of a world.
    debug_renderers_version: Option<u64>,
    /// The next place in the HUD's layer order (`gui_layers::order`) whose mod layers are not
    /// drawn yet this frame; `None` while the HUD is hidden or outside its pass.
    hud_layer_next: Option<usize>,
    /// The mod block entities' view distances, asked of the parts once (`native_block_entity_draw`).
    mod_view_distances: std::cell::OnceCell<native_block_entity_draw::ViewDistances>,
    /// `item_level` for one frame, game time, day time and dimension.
    item_level_cache: std::cell::RefCell<
        Option<(
            (u64, i64, i64, &'static str),
            horizon_assets::item_model::LevelState,
        )>,
    >,
    /// Frames drawn with the player in the world, for `--world-frames`.
    world_frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    /// The sprite animation clock (`TextureManager.tick`): counts only the ticks the level
    /// runs normally, so `/tick freeze` holds it and `/tick step` moves it on.
    texture_ticks: u64,
    health_hud: hud_layout::HealthHud,
    /// What ends the game with an error; kept as it is, so main can tell a debug crash.
    error: Option<Box<dyn std::error::Error>>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    /// Must precede session_lock: dropping the writer drains writes while the world is locked.
    player_autosaves: horizon_client::lan::playerdata::Autosaves,
    session_lock: Option<horizon_client::save::SessionLock>,
    /// The prepared world is installed. The session lock is taken earlier so tag
    /// reads see ready tables, and is not this marker: a final save before the
    /// prepared `Level` is installed would replace the world with placeholder state.
    world_adopted: bool,
    audio: Audio,
    /// The side, registries and mod settings of this game's resource generation, which
    /// its menus lend to the hooks they fire inside a loop; rebuilt by `refresh_loop_context`.
    loop_context: Arc<horizon_world::native_load_context::LoopContext>,
    setting_overrides: std::cell::RefCell<std::collections::BTreeMap<(String, String), String>>,
    /// The native damage hooks every level of this game lends to its hits.
    damage_hooks: horizon_client::health::DamageHooks,
    music: horizon_client::music::MusicManager,
    /// The tracks native mods play over the music (`native_music_layers`).
    music_layers: horizon_client::music::MusicLayers,
    /// `ToastManager.nowPlayingToast`: there while Music Toast is not Never.
    now_playing: Option<horizon_client::toasts::NowPlayingToast>,
    /// The Music Toast option the toast was last set up for.
    music_toast_state: u8,
    music_notes: horizon_client::toasts::MusicNotes,
    ambience: horizon_client::ambience::Ambience,
    /// The sound engine was told the game paused (pauseAllExcept).
    sound_paused: bool,
    /// Minecarts whose `MinecartSoundInstance` is playing.
    cart_sounds: HashSet<u32>,
    /// The minecart the riding sounds were started for (`LocalPlayer.startRiding`).
    riding_cart_sounds: Option<u32>,
    /// The happy ghast its riding sound was started for (`LocalPlayer.startRiding`).
    riding_sound: Option<u32>,
    /// `LocalPlayer.wasFallFlying` for the elytra's flying sound.
    was_fall_flying: bool,
    /// Bees whose buzz is playing, and whether it is the aggressive one.
    bee_sounds: HashMap<u32, bool>,
    /// Guardians whose attack hum is playing.
    guardian_sounds: HashSet<u32>,
    /// Remote guardian eye targets reused between ticks.
    guardian_look_scratch: Vec<(usize, [f64; 3])>,
    dropped: horizon_client::join_seq::JoinList<ItemEntity>,
    next_entity: u32,
    /// The last entity id when this level's entity pass began: an item with a
    /// later one was added during that pass and first ticks on the next
    /// (`EntityTickList`). `u32::MAX` until a tick has run.
    ticked_entities_through: u32,
    /// The items `tick_blocks` dropped while applying the entities' block
    /// contacts (a boat breaking a lily pad). Vanilla runs those in each
    /// entity's own tick (`applyEffectsFromBlocks`), inside the entity pass,
    /// so they too first tick on the next tick.
    contact_drops: std::ops::Range<u32>,
    particles: horizon_client::particles::ParticleEngine,
    /// `ItemPickupParticle`s: taken items, orbs and arrows flying to their collector.
    pickups: pickups::Pickups,
    /// Sign text being edited (`SignEditScreen`).
    sign_edit: Option<sign_edit::SignEditor>,
    /// An open book, book and quill or lectern screen.
    book: Option<books::BookScreen>,
    /// The last click on the book screen, for double clicks.
    book_last_click: Option<std::time::Instant>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    /// LevelLoadingScreen.smoothedProgress while the player is not placed yet.
    loading_progress: f32,
    /// `CreateWorldScreen.openCreateWorldScreen`'s data read, on a worker,
    /// while the title says `createWorld.preparing`.
    creation_job: Option<std::thread::JoinHandle<std::result::Result<(), String>>>,
    /// When `loading_progress` last eased (the server thread's load, 50 ms a step).
    loading_progress_at: Option<std::time::Instant>,
    /// A received packet started or ended a piston's move: `poll_remote`
    /// syncs the moving pistons once its packets are handled, not once per packet.
    moving_pistons_stale: bool,
    /// The world spawn search's origin chunk and fallback height, once known.
    spawn_origin: Option<((i32, i32), i32)>,
    /// `native_create_spawn_position` was asked, and a mod chose the spawn.
    spawn_hook_fired: bool,
    spawn_hook_cancelled: bool,
    spawn_spiral: horizon_client::world_spawn::ChunkSpiral,
    /// A new player's spawn search, while its candidates' chunks load.
    player_spawn: Option<horizon_client::world_spawn::PlayerSpawnFinder>,
    /// Chunks a respawn search waits for this tick, kept loading beside the
    /// player's own (taken by `update_world`).
    spawn_wait: Vec<(i32, i32)>,
    mobs: horizon_client::join_seq::JoinList<Mob>,
    /// `mobs` by network id, for the packets that name one (`game/mobs/mob_index.rs`).
    mob_index: mob_index::MobIndex,
    creatures: creatures::Creatures,
    sculk: sculk::SculkState,
    raiding: raids::Raiding,
    attack_ticks: u32,
    /// The main hand's item at the last player tick, whose attack damage and
    /// speed modifiers apply (`LivingEntity.detectEquipmentUpdates`): a hotbar
    /// swap just before an attack keeps them, with the new item's enchantments
    /// (attribute swapping).
    attack_item: Option<Arc<str>>,
    /// Mobs a player hurt (`LivingEntity.lastHurtByPlayer`), by player UUID.
    last_hurt_by_player: HashMap<u32, combat::PlayerCredit>,
    /// Leashed mobs a player killed, with the killer's UUID: the lead drops
    /// on the corpse's next tick (`Leashable.tickLeash`), for the mods. With
    /// the corpse's dimension: each level's tick prunes only its own.
    lead_killers: HashMap<u32, (u128, Dim)>,
    /// Animals a player fed into love (`Animal.loveCause`), with the guest it was by
    /// connection (None: the host). Breeding credits them, wherever they are.
    love_cause: HashMap<u32, Option<u32>>,
    /// Mobs whose death (`LivingEntity.die`) has been handled.
    /// Looked up per mob each tick: a fast hasher (never iterated).
    died: horizon_client::fast_hash::FxHashSet<u32>,
    /// Client-ticked block-entity animation state (`game/blocks/block_entity_anims.rs`).
    be_anims: horizon_client::block_entity_anim::BlockEntityAnims,
    /// Item being used (eaten): hotbar slot and ticks used.
    using: Option<(usize, u32)>,
    /// `getUseDuration` when that use started, as a mod left it
    /// (`game/items/item_use.rs`): consumables and mod items count to it.
    use_duration: i32,
    /// A mod's `Start` or `Tick` set that use's ticks left (`game/items/item_use.rs`).
    use_ticks_set: bool,
    /// The crossbow charge's start and middle loading sounds played so far
    /// (`game/items/crossbow.rs`): each charge plays each of them once, however
    /// far a mod's tick moved the charge at a time.
    crossbow_sounds: [bool; 2],
    /// Camera.oldFovModifier / fovModifier.
    /// `Camera` (eye height and field-of-view easing, view placement).
    view: horizon_client::camera::Camera,
    /// `FramerateLimiter` and its clock.
    limiter: horizon_client::window::FramerateLimiter,
    frame_clock: horizon_client::window::SystemClock,
    /// Hud.scopeScale (spyglass overlay size).
    scope_scale: f32,
    /// Duration of the last frame in ticks (GUI easing).
    frame_ticks: f32,
    title: Option<Title>,
    /// Armor slots: head, chest, legs, feet.
    armor: [Option<ItemStack>; 4],
    /// The off hand (Inventory slot 40, saved as slot 150; menu slot 504 here).
    offhand: Option<ItemStack>,
    /// The body and saddle slots (`EquipmentSlot.BODY`, `SADDLE`; Inventory
    /// slots 41 and 42, saved as 151 and 152). The hands and armor stay in
    /// the fields above. Nothing in the game reads these yet; they are kept
    /// for mods and saves.
    equipment: horizon_items::equipment::Equipment,
    /// `ItemStack.popTime` of the nine hotbar stacks, then the off hand: set to 5
    /// when a slot gains items, counted down each tick.
    hotbar_pop: [u8; 10],
    /// The hand an item use acts with (`InteractionHand`): the off hand only
    /// for its turn in `use_item` and while its item is being used.
    hand: Hand,
    /// The shield's item cooldown after an axe disabled it (ticks).
    shield_cooldown: u32,
    /// `Player.getCooldowns()`: the `use_cooldown` component's entries, what
    /// `itemDecorations` shades a slot by. A server's players have their own
    /// (`GuestPlay::cooldowns`), swapped in for their turn.
    cooldowns: horizon_client::cooldowns::Cooldowns,
    /// The source of a command run through /execute (the player otherwise).
    command_source: Option<commands::execute::Source>,
    native_command_filter: bool,
    titles: titles::Titles,
    /// `Hud.toolHighlightTimer`: the selected item name.
    tool_highlight: horizon_client::hud_overlays::ToolHighlight,
    /// `Hud.autosaveIndicatorValue`.
    saving_indicator: horizon_client::hud_overlays::SavingIndicator,
    /// This game's world is autosaving this tick (`MinecraftServer.isSaving`).
    autosaving: bool,
    /// `MinecraftServer.ticksUntilAutosave`: the ticks left before the next
    /// autosave, counted down a tick at a time.
    ticks_until_autosave: i64,
    applied_modifiers: Vec<(String, String)>,
    experience: Experience,
    orbs: horizon_client::join_seq::JoinList<ExperienceOrb>,
    xp_delay: u32,
    ordered: ordered_tick::OrderedTicks,
    level_up_sound: horizon_client::entities::LevelUpSound,
    /// The arm swing (`LivingEntity.SwingState`) and how many swings started.
    swing: horizon_client::first_person::SwingState,
    swings_started: u32,
    /// Swings of uses whose client predicts them (`SwingSource.PREDICTED`):
    /// a server does not send these back to the player who swung.
    predicted_swings: u32,
    /// The first-person equip heights and the items the hands show (with
    /// the slot each came from).
    heights: horizon_client::first_person::HandHeights,
    /// The hands' lag behind the view (`LocalPlayer.xBob`, `yBob`); a new
    /// LocalPlayer (joining, respawning, changing dimension) starts at 0.
    sway: horizon_client::first_person::HandSway,
    visible_hands: [(usize, Option<ItemStack>); 2],
    /// Whether the main and off hand's slots were set to a new stack that
    /// the hand doesn't show yet (`Game::hand_stack_replaced`).
    hands_replaced: [bool; 2],
    /// `Minecraft`'s attack handling, attack clicks since the last tick and
    /// the attack strength tickers the hand reads.
    clicks: horizon_client::first_person::Clicks,
    attack_clicks: u32,
    /// Use clicks since the last tick; `handleKeybinds` consumes them after the attack clicks.
    use_clicks: u32,
    hand_strength: horizon_client::attack_strength::AttackStrength,
    settings: Settings,
    tutorial: tutorial::Tutorial,
    tutorial_toasts: Vec<tutorial::HintToast>,
    /// The `SystemToastId.NARRATOR_TOGGLE` toast, wherever the player is.
    narrator_toast: Option<horizon_client::toasts::SystemToast>,
    /// The open screen's narration (`Screen`'s narration state and timing).
    narration: horizon_client::screen_narrator::ScreenNarrator,
    /// Where the options screen returns: true = title, false = pause menu.
    options_from_title: bool,
    dragging: bool,
    /// `ClientAvatarState`: walked distance and view bob.
    avatar: horizon_client::camera::AvatarBob,
    flash: u32,
    /// OptiFine custom sky layers' eased brightness (`custom_sky.rs`).
    custom_sky: custom_sky::CustomSkyView,
    /// `ClientLevel.rainSoundTime`: ticks of rain since the last rain sound.
    rain_sound_time: i32,
    /// Bolts that struck here and sky flashes they made, in total (`status`).
    lightning_seen: [u32; 2],
    /// The head of the bed while sleeping; the visual timer also survives waking.
    sleeping: Option<[i32; 3]>,
    sleep_timer: u32,
    /// The beds players sleep in that this game marked `occupied`, by level
    /// (game/blocks/beds.rs).
    player_beds: Vec<(Dim, [i32; 3])>,
    /// `SleepStatus`'s active and sleeping players at the last announcement.
    sleep_seen: (i64, i64),
    dim: Dim,
    world_root: PathBuf,
    /// The owner's offline-to-account UUID move of this world, when it opened under
    /// the account (`playerdata::migrate_owner`): entities the world loads still name
    /// the player by the offline UUID until their chunk is saved again.
    owner_migration: Option<(u128, u128)>,
    /// The world's data packs' content tables, when it has packs (`None`
    /// keeps `Resources.rest`'s own in use).
    world_tables: Option<WorldTables>,
    /// Server-owned loot and item snapshots, independent of client preloads.
    server_loot: horizon_client::structure_blocks::LootSnapshot,
    server_items: Option<Arc<Items>>,
    /// The world is superflat (`ServerLevel.isFlat`, for every dimension).
    world_flat: bool,
    /// The world is the Debug preset (`Level.isDebug`, for every dimension): its blocks
    /// cannot be changed and nothing ticks.
    world_debug: bool,
    /// A server's players on their way to another level, each waiting for
    /// the destination to load around them.
    server_trips: Vec<server_travel::Trip>,
    /// Players a command sends to another level, once it has run.
    level_teleports: Vec<server_travel::LevelTeleport>,
    /// Teleports a command made inside a level, for `native_player_teleported`
    /// once the player's move has reached their client (`leave_guest`).
    teleport_notes: Vec<server_travel::TeleportNote>,
    /// A server's levels other than the one in place (game/world/levels.rs).
    levels: levels::Levels,
    /// Which pass of a server tick runs (`tick_server`): the first one reads
    /// the network, the last one ends the players' tick.
    level_pass: levels::Pass,
    /// Nether portals: the player's, items' and mobs' (game/blocks/nether_portal.rs).
    portals: nether_portal::Portals,
    /// The respawn point when it is not the world spawn or a bed: a respawn
    /// anchor (`game/blocks/respawn_anchor.rs`).
    respawn_config: Option<respawn_anchor::RespawnConfig>,
    /// The next enter_dimension call comes from an End portal.
    end_portal_travel: bool,
    /// The open WinScreen (game/interface/win.rs).
    win: Option<win::Win>,
    end: end::EndState,
    block_ticks: Ticks,
    tick_scratch: TickScratch,
    tick_chunks: horizon_client::tick_chunk_membership::TickChunkMembership,
    tickets: horizon_client::ticket_levels::ChunkTickets,
    random_tick_sections: horizon_client::random_tick_sections::RandomTickSections,
    level_events: Vec<LevelEvent>,
    falling: horizon_client::join_seq::JoinList<FallingBlock, false>,
    /// Blocks the server shows for a moment (`ClientboundAddTransientBlockPacket`).
    transient_blocks: transient_blocks::TransientBlocks,
    paintings: horizon_client::join_seq::JoinList<horizon_client::entities::Painting>,
    painting_variants: paintings::PaintingVariants,
    item_frames: horizon_client::join_seq::JoinList<horizon_client::entities::ItemFrame>,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    /// The `map=true` frame variants.
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stands: horizon_client::join_seq::JoinList<horizon_client::entities::ArmorStand>,
    /// Worn armor as of the last equipment check (frost walker).
    location_armor: [Option<ItemStack>; 4],
    /// `Cushion` decoration entities (game/entities/cushions.rs).
    cushions: horizon_client::join_seq::JoinList<horizon_client::entities::Cushion>,
    /// Entities of types Horizon does not simulate.
    others: horizon_client::join_seq::ResidentList<commands::OtherEntity>,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    /// `WaypointStyleManager`.
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    /// `ClientWaypointManager`: the locator bar's waypoints.
    waypoints: horizon_client::locator_bar::ClientWaypoints,
    /// The camera the last frame drew from (position, yaw, pitch, fov), for the locator bar.
    locator_camera: Option<([f64; 3], f32, f32, f32)>,
    native_camera_frame: Option<mod_api::Camera>,
    /// The yaw and pitch a native mod's camera drew the last frame from (`native_camera_pose`),
    /// none when it drew from vanilla's.
    posed_rotation: Option<(f32, f32)>,
    /// `LocalPlayer.postEffects`: the chains the server has the screen run
    /// (`ClientboundPostEffectsPacket`), in order.
    post_effects: Vec<String>,
    /// The game time (ticks and partial) of the last frame, for `PolyDeltaTime`.
    polytone_post_ticks: f64,
    /// `GameRenderer.spectatedEntityPostEffect` (`checkEntityPostEffect`).
    spectated_post_effect: Option<&'static str>,
    /// `LocalPlayer.experienceDisplayStartTick`: (last progress, seen a change, tick).
    experience_display: (f32, bool, u64),
    boats: horizon_client::join_seq::JoinList<boats::Boat, false>,
    /// The boats a mob's lead can hang on, for this tick's mob ticks.
    leash_boats: Vec<(u32, horizon_client::mobs::leash::HolderState)>,
    boat_models: boats::BoatModels,
    /// EndCrystalModel's baked layer.
    crystal_model: Option<horizon_client::mobs::Part>,
    /// CushionModel's baked layer.
    cushion_model: Option<horizon_client::mobs::Part>,
    /// Models drawn only once such an entity shows up (lead knots, wind charges...).
    entity_models: EntityModels,
    carts: horizon_client::join_seq::JoinList<minecarts::Minecart, false>,
    /// The boats and minecarts `native_vehicle_moved` follows (lan_vehicles.rs).
    watched_vehicles: Vec<lan_vehicles::WatchedVehicle>,
    /// Songs playing from jukeboxes (tracked sound handles).
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: Vec<lightning::Bolt>,
    maps: maps::Maps,
    /// The ender dragon fight (game/mobs/dragon.rs).
    fight: dragon::Fight,
    dragon: horizon_client::join_seq::ResidentSlot<dragon::Dragon>,
    crystals: horizon_client::join_seq::JoinList<dragon::Crystal>,
    breath_clouds: horizon_client::join_seq::ResidentList<dragon::Breath>,
    dragon_model: Option<horizon_client::mobs::Part>,
    /// Map textures waiting for the next HUD batch to upload them.
    map_patches: Vec<horizon_client::font::Patch>,
    name_tag_layouts: names::NameTagLayouts,
    text_displays: std::collections::HashMap<i32, text_displays::RemoteTextDisplay>,
    interactions: std::collections::HashMap<i32, interactions::RemoteInteraction>,
    /// The fence knots a server tracks to this client, by entity id, at their
    /// block (`LeashFenceKnotEntity`).
    remote_knots: HashMap<u32, [i32; 3]>,
    /// The entity ids a hosted server gives the knots its mobs hang on, by
    /// block, while a lead is tied there.
    knot_ids: HashMap<[i32; 3], u32>,
    text_display_vertices: usize,
    /// Player skin downloads, and which skin each atlas skin slot holds
    /// (least recently shown first).
    skin_downloads: RefCell<horizon_client::skins::Downloader>,
    /// `ProfileResolver.Cached`: `Dynamic` profiles (name- or id-only heads
    /// and mannequins) resolve through the services in the background.
    skin_profiles: RefCell<horizon_client::skins::Resolver>,
    skin_slots: horizon_client::skins::SkinSlots,
    /// 3D skin layers: each player's outer layer as one slab per painted
    /// texel, built once per (skin, slim) (`mobs::skin_layers`).
    skin_3d: RefCell<skin_models::SkinModels>,
    /// Runtime trim slots and the last completed preparation for this atlas.
    trim_slots: horizon_client::trims::TrimSlots,
    cart_models: minecarts::CartModels,
    /// The boat the player rides.
    riding: Option<u32>,
    /// Vehicles that gained a rider this tick whose `START_RIDING_TRIGGER` was
    /// not fired yet (`game/entities/boarding.rs`), for a guest's turn.
    started_riding: Vec<u32>,
    /// A rotation-only write (`/rotate`, `Entity.forceSetRotation`) to the player of this
    /// turn: `leave_guest` sends it as `ClientboundPlayerRotationPacket`, not a teleport.
    rotation_forced: bool,
    /// The mount the last `position_rider` put the player on, to tell the tick
    /// a seat is taken (`Entity.moveTo`, no camera sweep) from the ones that
    /// follow it (`positionRider`, interpolated as usual).
    posed_rider: Option<u32>,
    /// The vehicle the player's file names, waiting for it to be in the world
    /// (`ServerPlayer.loadAndSpawnParentVehicle`).
    pending_vehicle: Option<[i32; 4]>,
    /// The ender inventory (27 slots) and the ender chest showing it.
    ender_items: Vec<Option<ItemStack>>,
    ender_open: Option<(i32, i32, i32)>,
    /// Block entity data of broken blocks waiting for their dropped item.
    pending: spill::Pending,
    /// Item entities spawned and block entities removed while the block
    /// behaviour ran (spilled contents, popped resources).
    spill: spill::Spill,
    /// The recipe book of the open screen (game/items/recipe_book.rs).
    recipe_ui: recipe_book::RecipeUi,
    /// The horse inventory (game/interface/mount_menu.rs).
    mount_menu: mount_menu::MountMenu,
    /// The player's fishing hook.
    hook: Option<fishing::Hook>,
    /// The other players' hooks a server tracks to this client.
    remote_hooks: Vec<fishing::RemoteHook>,
    tridents: horizon_client::join_seq::JoinList<trident::ThrownTrident>,
    rockets: horizon_client::join_seq::JoinList<fireworks::Rocket>,
    /// Ominous trial spawners' item spawners.
    ominous_spawners:
        horizon_client::join_seq::ResidentList<ominous_item_spawner::OminousItemSpawner>,
    creaking: creaking::CreakingState,
    /// Monster spawners' display mobs (`game/mobs/spawner.rs`).
    spawners: spawner::SpawnerState,
    /// Reused candidate buffers for natural spawning.
    spawn_scratch: natural_spawning::SpawnScratch,
    trident_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    bee_stinger_model: Option<horizon_client::mobs::Part>,
    tnts: horizon_client::join_seq::JoinList<tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    /// The open `host.transaction`s, innermost last (`transactions.rs`).
    transactions: Vec<transactions::Level>,
    /// Backing blocks whose contents notice is on the call stack, innermost last.
    contents_changing: Vec<(mod_api::LevelRef, BlockPos)>,
    /// The screen of a mod block's menu open on this client (`native_screen`).
    mod_screen: Option<native_hooks::OpenNativeScreen>,
    /// The players with each mod container open, by level (`tick_mod_container_viewers`).
    mod_container_viewers: HashMap<(&'static str, (i32, i32, i32)), u32>,
    /// Where item entities are, while the block entities tick (hoppers).
    item_columns: Option<machines::ItemColumns>,
    /// The item columns for one pass of the players' pickups
    /// (`tick_guests`), rebuilt after any write to the items (a pickup, a
    /// command, an explosion): see `item_columns`.
    pickup_columns: Option<machines::StampedItemColumns>,
    /// Where the mobs are, while the block entities tick (potent sulfur gas).
    pass_mobs: Option<potent_sulfur::PassMobs>,
    /// Block-entity positions retained between ticks for client effects.
    spawner_tick_positions: Vec<horizon_client::block_entities::Pos>,
    trial_spawner_tick_positions: Vec<horizon_client::block_entities::Pos>,
    creative_screen: Creative,
    /// `CreativeModeInventoryScreen.ignoreTextInput`: the chat key opened the
    /// search tab, so the letter it carries is not typed into the box.
    creative_ignore_text: bool,
    control: Option<horizon_client::control::Server>,
    /// The Agent Control setting the socket was last opened or closed for (`agent_settings`).
    agent_control_on: bool,
    /// When an agent's last command came.
    agent_last: Option<Instant>,
    frame_stats: horizon_client::control::FrameStats,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    /// `screenshot-sequence` while it runs.
    screenshot_sequence: Option<screenshot_sequence::Sequence>,
    /// `render-capture DIR`: replied once the renderer wrote the scene.
    pending_render_capture: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>)>,
    /// Recent frame times for the performance overlay.
    frame_graph: horizon_client::perf_overlay::FrameGraph,
    /// Horizon's extras (minimap, auto-reconnect; mods' counters): `game/interface/extras.rs`.
    extras: extras::ExtrasState,
    /// The PvP & HUD options' state (`game/entities/pvp.rs`).
    pvp: pvp::PvpState,
    /// Horizon mods (`game/modding/mods/mod.rs`), none running unless turned on.
    mods: mods::ModsState,
    /// The in-game browser's worker, separate from the cloneable reload inputs.
    mod_jobs: mod_browser::Jobs,
    /// Every item and recipe, for mods (`game/modding/native_recipes.rs`), made once.
    recipe_tables: native_recipes::RecipeTables,
    /// The item panel beside container screens (`game/modding/native_panel.rs`).
    panel: native_panel::Panel,
    /// Horizon's own shader pack, while one is picked (`game/modding/native_shaders.rs`).
    native_shaders: native_shaders::NativeShaders,
    /// The items native client mods claim and the events they haven't polled (game/modding/native_claimed_items.rs).
    native_input: native_claimed_items::ModInput,
    /// The native lifecycle bookkeeping.
    native_lifecycle: native_lifecycle::NativeLifecycle,
    /// The mobs lent to the hook calls running now, before they join a level
    /// (game/modding/native_unspawned_mob.rs).
    lent_mobs: RefCell<native_unspawned_mob::LentMobs>,
    /// Entities whose copies are held out of storage; their handles refuse until stored back.
    held_out: Vec<u32>,
    /// Native mods' config files this game loaded (game/modding/native_configs.rs).
    native_configs: native_configs::NativeConfigs,
    mod_phase: mod_api::__private::PhaseOwner,
    /// Native mods' messages waiting (`game/modding/native_net.rs`).
    native_net: native_net::NativeNet,
    /// The mod whose own screen is open (`Screen::Mod`).
    native_mod_screen: Option<String>,
    /// Meshed sections waiting for an upload budget (Horizon settings).
    pending_uploads: Vec<((i32, i32, i32), horizon_client::mesher::SectionMesh)>,
    /// This frame's view frustum, for culling entities and block entities.
    view_frustum: Option<horizon_client::gfx::Frustum>,
    /// Whether the renderer's section-occlusion graph is active for this view.
    entity_section_visibility_valid: bool,
    /// The camera section in the current world layout.
    entity_section_camera: Option<(i32, i32, i32)>,
    sync_occlusion_culler: Option<horizon_client::voxel_occlusion::Culler>,
    async_occlusion_culler: Option<Box<occlusion_culling::AsyncOcclusionCuller>>,
    async_occlusion_culling_failed: bool,
    async_particle_culler: RefCell<particle_culling::ParticleAsyncState>,
    /// `Renderer::pack_shadow_casters` for this frame's entities: (keep what the culler hides,
    /// build the player's model in first person); (false, false) without a shader pack.
    pack_shadow_casters: (bool, bool),
    /// A scripted flight from the control socket (`fly`).
    flight: Option<control::Flight>,
    /// The running agent action from the control socket (game/runtime/agent/mod.rs).
    agent: agent::Agent,
    /// Widget commands waiting for the frame after a click (game/runtime/ui_control.rs).
    ui_wait: ui_control::UiWait,
    /// What happened, for `events` and `wait-for` (game/runtime/events.rs).
    events: events::Events,
    /// Who caused each input event: a person, an agent or the game (game/runtime/input_trace.rs).
    input_trace: input_trace::InputTrace,
    /// A repro recording, export or replay (game/runtime/repro.rs).
    repro: repro::Repro,
    /// `HORIZON_TICK_SCRIPT`'s scenario (game/world/tick_script.rs).
    tick_script: tick_script::TickScript,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((Dim, i64, u32, u32), i32)>>,
    /// The last water plane `water_plane` found, kept while none is in reach.
    water_plane_memo: std::cell::Cell<Option<f32>>,
    /// The camera's block and the second `water_plane` last scanned at.
    water_plane_scan: std::cell::Cell<Option<([i32; 3], i64)>>,
    /// The mobs' box reach while the boats and minecarts tick, with the mob count it was
    /// worked out for (`vehicle_mob_reach`).
    vehicle_mob_reach: std::cell::Cell<Option<(usize, horizon_client::mobs::BoxReach)>>,
    persist: persist::EntityMemory,
    /// Missing profession IDs already reported for this world session.
    missing_professions: HashSet<String>,
    entity_attachments: HashMap<[i32; 4], horizon_mod_runtime::attachments::Attachments>,
    entity_initialization: entity_initialization::EntityInitialization,
    /// Typed per-level membership follows dimensions, not the temporary world swap.
    attachment_indexes: attachment_index::Indexes,
    /// Connected to a server: the world comes from the network (game/network/remote/mod.rs).
    remote: Option<remote::Remote>,
    /// Active packet replay session (game/studio.rs).
    pub(super) studio_replay: Option<horizon_client::studio::ReplayDriver>,
    pub(super) studio_freecam: horizon_client::studio::StudioFreecam,
    pub(super) studio_timeline: horizon_client::studio::TimelineUi,
    pub(super) studio_topbar: horizon_client::studio::TopBarUi,
    pub(super) studio_rmb_down: bool,
    studio_camera_track_dirty: bool,
    /// Camera perspective to restore after leaving replay playback.
    studio_replay_previous_camera_type: Option<horizon_client::camera::CameraType>,
    /// Replay opened from the title screen owns the synthetic remote session.
    studio_replay_standalone: bool,
    last_replay_tick: Option<Instant>,
    pub(super) studio_virtual_tick_ms: f32,
    /// This world is open to the network (game/network/lan/mod.rs).
    lan: Option<lan::LanHost>,
    /// Single player's server thread, which `remote` is connected to.
    integrated: Option<integrated::Integrated>,
    /// The shapes of the open world's levels, from its dimension types.
    level_shapes: horizon_client::chunk::LevelShapes,
    /// This game is that server thread's (game/world/integrated.rs).
    serving: Option<integrated::Serving>,
    /// The open world is shared behind an invite code (game/network/friends.rs).
    #[cfg(feature = "friends")]
    friends: Option<friends::Sharing>,
    /// The Social Interactions screen and the players whose chat is hidden (game/network/social.rs).
    social: social::Social,
    world_options: Option<lan_screen::WorldOptions>,
    /// Chat Settings opened from RestrictionsScreen: Done returns to World Options.
    options_to_world_options: bool,
    /// HDR's settings opened with their key from the game: Done returns to the game.
    options_to_game: bool,
    command_block_screen: Option<command_blocks::CommandBlockScreen>,
    block_editor: Option<block_editor::BlockEditScreen>,
    /// `StructureTemplateManager.structureRepository`: the templates in memory
    /// by id, a miss included (game/administration/structure_blocks.rs).
    structure_templates: HashMap<String, Option<structure_blocks::TemplateEntry>>,
    structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns,
    /// The game tests read so far, by id (the test instance registry).
    test_defs: std::cell::RefCell<HashMap<String, Option<test_instances::TestDef>>>,
    /// `test_defs` holds the world's whole test instance registry
    /// (`snapshot_test_defs`), so a miss is final.
    test_defs_snapshot: bool,
    /// `GameTestBlockHighlightRenderer.markers`: block, text, removal time.
    test_highlights: HashMap<(i32, i32, i32), (String, std::time::Instant)>,
    /// `GameTestTicker.SINGLETON`: the game tests running on this server.
    game_tests: game_tests::GameTests,
    /// KeyboardHandler's debug key state (game/interface/debug_keys.rs), the physical
    /// keys held, and the debug renderers F3+B and F3+G switch on.
    debug_keys: horizon_client::debug_keys::DebugKeys,
    /// The F3 overlay's charts (F3+2, F3+3).
    debug_charts: horizon_client::debug_chart::DebugCharts,
    /// `Minecraft.debugEntries`: what the overlay and the debug renderers
    /// show (`debug-profile.json`, F3+F6).
    debug_entries: horizon_client::debug_entries::DebugEntries,
    /// `DebugOptionsScreen` while it is open (game/interface/debug_options.rs).
    debug_options: Option<debug_options::DebugOptionsUi>,
    /// `DebugEntryMemory`'s readings (game/interface/debug_lines.rs).
    memory_meter: std::cell::RefCell<horizon_client::debug_overlay::MemoryMeter>,
    /// The F3 overlay's vertices between rebuilds (`draw_debug_overlay_under`).
    debug_paint: RefCell<horizon_client::debug_overlay::PaintCache>,
    /// The F3 overlay's last reading of the refresh rate, and when it was taken.
    debug_refresh: std::cell::Cell<Option<(Instant, Option<u32>)>>,
    /// `Minecraft.gpuUtilization`: the last timed frame's GPU time over its
    /// CPU time, in percent.
    gpu_utilization: f64,
    /// GPU Culling's Auto mode: what it has decided from the frames' CPU and GPU times.
    gpu_culling_auto: horizon_client::autotune::GpuCullingAuto,
    /// The mobs `mob_geometry` drew last frame (`lastEntityRenderStateCount`).
    rendered_mobs: std::cell::Cell<usize>,
    /// Whether a native mod requested background blur for this frame (`apply_screen_blur`).
    screen_blur_requested: std::cell::Cell<bool>,
    /// `ServerChunkCache.lastSpawnState` of each level: its spawnable chunks
    /// and each `MobCategory`'s count.
    spawn_counts: HashMap<Dim, (i32, [i32; 8])>,
    /// The natural spawning policy mods set on each level (`LevelRef::set_spawn_limits`,
    /// `set_spawn_flags`); a level with none spawns as vanilla. Kept while the world is open.
    spawn_policies: HashMap<mod_api::DimensionKey, natural_spawning::SpawnPolicy>,
    /// PauseScreen.showPauseMenu (false after F3+Escape).
    pause_menu: bool,
    pause_focus: Option<u8>,
    death_focus: Option<u8>,
    /// The screen ([`Game::key_screen`]) the menu focus belongs to. Another
    /// screen in front drops it, as `Gui.setScreen` drops the focus of the
    /// screen it leaves (`Screen.clearFocus`).
    focus_screen: u64,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    previous_mode: Option<horizon_client::command::args::GameMode>,
    gamemode_switcher: Option<horizon_client::game_mode_switcher::GameModeSwitcher>,
    /// The system clipboard (`KeyboardHandler.clipboardManager`).
    clipboard: horizon_client::clipboard::SystemClipboard,
    /// The game is handling a key, mouse or typed-text event, from the window or the
    /// control socket: the only time a mod reaches the clipboard (`with_user_input`).
    user_input: std::cell::Cell<bool>,
    /// Select All in the append-only text fields.
    text_selection: horizon_client::clipboard::AppendSelection,
    /// The world was saved for the application ending (`save_for_exit`).
    saved_for_exit: bool,
    pending_save: Option<saving::ClosingSave>,
    /// Scheduled end of the server tick (`MinecraftServer.haveTime`).
    server_tick_deadline: Option<Instant>,
    /// The last world frame reached the swap chain. A window the compositor
    /// stops showing gets no frames, and the event loop must not spin on it.
    frame_drawn: bool,
    /// The world list's icon in progress (`GameRenderer.tryTakeScreenshotIfNeeded`):
    /// `None` without a local world, or once the save has its icon.png.
    world_icon: Option<WorldIcon>,
    /// The block a guest is acting on (game/network/lan_actions/interaction.rs).
    forced_hit: Option<Hit>,
    /// `Minecraft.hitResult` while the clicks of a tick act (game/network/remote/mod.rs
    /// `remote_handle_input`): picked once, so a creative break does not
    /// move the use that follows it past the broken block.
    tick_hit: Option<Option<Hit>>,
    /// The mob of that `hitResult`, picked with the item held at the pick
    /// (`LocalPlayer.raycastHitResult`: a spear's `attack_range`), before the
    /// tick's number keys change it.
    tick_mob: Option<Option<(usize, f64, [f64; 3])>>,
    /// Number keys since the last tick (`keyHotbarSlots[i].consumeClick`):
    /// `Minecraft.handleKeybinds` selects them after the tick's pick.
    hotbar_presses: Vec<usize>,
    /// `Player.entityInteractionRange` and `blockInteractionRange`, refreshed
    /// each tick from the attribute (a server's reach modifier, creative's
    /// +2): the pick helpers' `&self` reach.
    entity_reach: f64,
    block_reach: f64,
    /// The part of a right click a guest's packet asks for (game/network/lan_actions/interaction.rs).
    guest_use: Option<GuestUse>,
    /// The last `use_hand` that ended the click did so with `FAIL`, which
    /// does not consume the action (handleUseItemOn's ANY_BLOCK_USE).
    use_failed: bool,
    /// Level events of this tick for the guests, with their excluded source.
    lan_events: Vec<QueuedLevelEvent>,
    /// Jukebox level events 1010 (song name) and 1011 (`None`) for the guests;
    /// the song's network id is only known once the hub is in hand.
    lan_songs: Vec<([i32; 3], Option<String>)>,
    lan_cushion_breaks: Vec<(horizon_world::registry::BlockStateId, [f64; 3])>,
    /// Block events (position, b0, b1, state) and entity events (id, event)
    /// for the guests.
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8)>,
    /// Successful crits (attacking guest, hit entity), queued while the hub is taken.
    lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// Spear stabs that landed (the stabbing guest), queued while the hub is
    /// taken: `broadcastEntityEvent(player, 2)`.
    lan_kinetic_hits: Vec<Option<u32>>,
    /// `LivingEntity.recentKineticEnemies`: when each player (a guest, or the
    /// window's own) last stabbed each entity this use.
    stabbed: HashMap<(Option<u32>, u32), i64>,
    /// The other players the acting player's spear can reach this turn,
    /// taken from the hub before it (`spear_players`).
    stab_players: Vec<spear_players::StabPlayer>,
    /// Spear stabs on players worked out in a guest's turn, landed once the
    /// hub is back (`land_player_stabs`).
    player_stabs: Vec<spear_players::StabBatch>,
    /// The movement a guest's client reported this tick
    /// (`ServerPlayer.getKnownMovement`), for their turn.
    known_movement: Vec3,
    /// `LivingEntity.lastKineticHitFeedbackTime` of the entities that felt a
    /// spear's hit (`spear::OWN` is this window's player), in game time.
    kinetic_feedback: HashMap<i32, i64>,
    /// Global level events for every guest (`ServerLevel.globalLevelEvent`).
    lan_global_events: Vec<(i32, [i32; 3], i32)>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_acting: Option<u32>,
    /// A mod asked for the command trees again (`native_refresh_commands`):
    /// the next server tick asks every mod for its commands and resends them.
    native_commands_refresh: bool,
    /// The abilities of the server player whose turn it is (`as_guest`): their record's
    /// copy, which the turn reads and writes and `leave_guest` stores. None outside a turn.
    acting_abilities: Option<horizon_client::lan::abilities::PlayerAbilities>,
    /// Other players in the current guest turn, including borrowed ancestor bodies.
    lan_building_boxes: building_players::BuildingPlayers,
    /// A guest's `/save-all`: saved once their turn ends (`run_guest_command`),
    /// when their own state is back in their record.
    save_after_guest_turn: bool,
    /// Guests whose records are displaced by nested player swaps, outer first.
    lan_guest_swaps: Vec<u32>,
    /// The active player's map; a guest turn moves it from its owning record.
    player_attachments: horizon_mod_runtime::attachments::ResidentAttachments,
    /// The block the acting player is breaking (`break_block`): its destroy
    /// event, and the ones before it, are the player's own (`Level.levelEvent`
    /// with the player excluded); the events after it are not.
    lan_break_pos: Option<[i32; 3]>,
    /// Where a player's break puts the block's own drops instead of popping
    /// them (`native_player_break_block` with `into_inventory`).
    break_collect: Option<Vec<ItemStack>>,
    /// The name of the guest `lan_acting` acts for (`Player.getName`).
    lan_acting_name: Option<String>,
    /// Their UUID (`Player.getUUID`).
    lan_acting_uuid: Option<u128>,
    /// Where the mods refused to prime TNT: the sound and game event the level
    /// queued right after the prime are dropped (`prime_tnt_event`).
    tnt_refused: Option<BlockPos>,
    /// Their permission level: an op's above 0 (`PlayerList.isOp`).
    lan_acting_permission: Option<u8>,
    /// The spawn protection radius that holds the acting guest back
    /// (`DedicatedServer.isUnderSpawnProtection` before the position test):
    /// None for an operator, without operators, or off a dedicated server.
    lan_acting_spawn_protection: Option<i32>,
    /// Their entity id, which the hub (often taken out of `lan` while they
    /// act) would otherwise have to be asked for.
    lan_acting_entity: Option<u32>,
    /// Whether the player currently swapped into the game is connectionless.
    lan_acting_fake: bool,
    /// A fake player's `perform_action` in progress, and what it refused.
    fake_action: std::sync::Arc<native_server::FakeAction>,
    /// Command feedback to forward to a guest.
    chat_capture: Option<Vec<horizon_client::text::Text>>,
    /// Packets for the guest acting now, sent once its action returns.
    guest_packets: Vec<horizon_protocol::packets::play::Clientbound>,
    /// The acting guest's cooldowns, as (item, ticks): their packet needs the
    /// hub's item registry, which the guest's turn does not hold.
    guest_cooldowns: Vec<(Arc<str>, i32)>,
    /// The acting guest's own entity events (`ServerPlayer.connection.send`),
    /// sent with their entity id, which the guest's turn does not hold.
    guest_entity_events: Vec<i8>,
    /// The acting guest's equipment breaks (`onEquippedItemBroken`'s entity
    /// events), broadcast with their entity id once their turn ends.
    guest_equipment_breaks: Vec<i8>,
    /// Block sounds queued by the acting guest, excluding that guest.
    guest_block_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// `Player.playSound` of the acting guest (event, position, volume,
    /// pitch): `level.playSound(player, ...)` leaves the guest out, whose
    /// own client plays it.
    guest_player_sounds: Vec<(String, [f64; 3], f32, f32)>,
    /// Sounds bound to an entity this tick, for the players (`game/interface/level_sounds.rs`).
    bound_sounds: Vec<level_sounds::BoundSound>,
    /// `broadcastToAdmins` lines a guest's command queued, sent once it
    /// returns: the line, whether operators get it and whether the console
    /// logs it.
    admin_feedback: Vec<(horizon_client::text::Text, bool, bool)>,
    /// The server console is running a command: whether its feedback goes to
    /// the operators (`DedicatedServer.shouldInformAdmins`,
    /// `broadcast-console-to-ops`).
    console_broadcast: Option<bool>,
    /// The command the console is running came over RCON: its source is named `Rcon`
    /// (`RconConsoleSource.createCommandSourceStack`), not `Server`.
    rcon_source: bool,
    /// A dedicated server's operator, whitelist and ban lists, for `/op`,
    /// `/ban` and friends (`DedicatedPlayerList`).
    server_lists: Option<std::sync::Arc<std::sync::Mutex<dedicated::Lists>>>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    cmds: commands::extra::CommandExtras,
    /// The retained server reload listeners of the last successful reload.
    reload_resources: mod_api::RetainedListeners,
    ui: chat::UiState,
    combat: combat::CombatTracker,
    /// Each guest's own `CombatTracker`, out of `combat` between turns.
    guest_combat: std::collections::HashMap<u32, combat::CombatTracker>,
    mount: mounts::MountState,
    stats: stats::Stats,
    score: scoreboard::ScoreState,
    fns: functions::FnState,
    bosses: bossbars::BossState,
    /// `Hud.bossOverlay`: the bars this window is showing.
    boss_overlay: horizon_client::boss_overlay::BossOverlay,
    exec: commands::execute::ExecState,
    rand_sequences: horizon_client::random_sequences::RandomSequences,
    /// `BrushableBlockEntity`'s brush counts of the blocks being brushed.
    brushing: brush::Brushing,
    border: horizon_client::worldborder::WorldBorder,
    /// `WorldBorderRenderer` and its render state.
    border_renderer: horizon_client::border_render::BorderRenderer,
    /// The camera's 16-block section when the wall was last invalidated.
    border_section: Option<[i32; 3]>,
    border_state: horizon_client::border_render::BorderRenderState,
    /// `Hud.tabList`: the player list's header, footer and heart states.
    tab_list: horizon_client::tab_list::TabList,
    subtitles: subtitle_hud::SubtitleState,
    adv: advancements::AdvState,
    restore_player_state: bool,
    stats_ui: stats_screen::StatsUi,
    adv_ui: advancements_screen::AdvUi,
    /// The Server Links screens' state (game/network/server_links.rs).
    links_ui: server_links::LinksUi,
    camera_type: horizon_client::camera::CameraType,
    requested_camera_type: std::cell::Cell<Option<horizon_client::camera::CameraType>>,
    fangs: fangs::FangState,
    /// The local player as a model (third person, the inventory).
    player_model: Option<Mob>,
}
/// The open world's dimensions past vanilla's three. One per process, as a window
/// and its server thread name the same dimensions.
static EXTRA_DIMS: std::sync::RwLock<Vec<mod_api::DimensionKey>> =
    std::sync::RwLock::new(Vec::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Dim {
    Overworld,
    Nether,
    End,
    /// A dimension past vanilla's three, from a data pack or a mod.
    Extra(mod_api::DimensionKey),
}
impl Dim {
    /// The level as block behaviour and the mods name it.
    fn level(self) -> LevelDimension {
        match self {
            Self::Overworld => LevelDimension::Overworld,
            Self::Nether => LevelDimension::Nether,
            Self::End => LevelDimension::End,
            Self::Extra(_) => LevelDimension::of_level(
                horizon_core::LevelKey::parse(self.id())
                    .unwrap_or(horizon_core::LevelKey::OVERWORLD),
            ),
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
            Self::Extra(key) => {
                mod_api::__private::dimension_id(key).expect("the game's table made the key")
            }
        }
    }
    fn from_id(id: &str) -> Self {
        match id {
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::extra(id).unwrap_or(Self::Overworld),
        }
    }
    /// `Level.getSeaLevel` of the level: the sea level its noise settings give a mod's level, else
    /// the built-in level's.
    fn sea_level(self) -> i32 {
        Self::sea_level_of_id(self.id())
    }
    /// [`Self::sea_level`] of the level `id` names.
    fn sea_level_of_id(id: &str) -> i32 {
        horizon_core::LevelKey::parse(id).map_or(63, horizon_world::level::sea_level_of)
    }
    /// `DimensionType.getTeleportationScale`: how a coordinate in `source` scales into `target`,
    /// the source's `coordinate_scale` over the target's (8 into the Overworld from the Nether).
    fn teleportation_scale(source: Self, target: Self) -> f64 {
        use horizon_world::dimension_type::DimensionFacts;
        let facts = |dim: Self| {
            horizon_core::LevelKey::parse(dim.id())
                .ok()
                .map(DimensionFacts::of_level)
        };
        match (facts(source), facts(target)) {
            (Some(from), Some(to)) => DimensionFacts::teleportation_scale(from, to),
            _ => 1.0,
        }
    }
    /// The open world's dimension `id` past vanilla's three.
    fn extra(id: &str) -> Option<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        let key = extra
            .iter()
            .find(|key| mod_api::__private::dimension_id(**key) == Some(id))?;
        Some(Self::Extra(*key))
    }
    fn is_vanilla(id: &str) -> bool {
        matches!(
            id,
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        )
    }
    /// Whether the open world has the dimension `id`.
    fn exists(id: &str) -> bool {
        Self::is_vanilla(id) || Self::extra(id).is_some()
    }
    /// The open world's dimensions, in the order its server ticks them.
    fn all() -> Vec<Self> {
        let extra = EXTRA_DIMS.read().unwrap();
        [Self::Overworld, Self::Nether, Self::End]
            .into_iter()
            .chain(extra.iter().copied().map(Self::Extra))
            .collect()
    }
    /// The dimensions past vanilla's three of the world being opened.
    fn set_extra<'a>(ids: impl Iterator<Item = &'a str>) {
        *EXTRA_DIMS.write().unwrap() = ids.map(mod_api::__private::dimension_key).collect();
    }
    /// Adds the dimension `id` a remote server sent, which no local world told of, so that
    /// [`Self::from_id`] finds it.
    fn add_extra(id: &str) {
        if !Self::exists(id) {
            EXTRA_DIMS
                .write()
                .unwrap()
                .push(mod_api::__private::dimension_key(id));
        }
    }
    /// The dimension as mods' level handles name it.
    fn key(self) -> mod_api::DimensionKey {
        match self {
            Self::Overworld => mod_api::DimensionKey::OVERWORLD,
            Self::Nether => mod_api::DimensionKey::NETHER,
            Self::End => mod_api::DimensionKey::END,
            Self::Extra(key) => key,
        }
    }
    /// The level's shape in a world of vanilla's dimension types.
    fn shape(self) -> horizon_client::save::LevelShape {
        use horizon_client::save::LevelShape;
        match self {
            Self::Overworld => LevelShape::OVERWORLD,
            Self::Nether => LevelShape::NETHER,
            Self::End => LevelShape::END,
            Self::Extra(_) => LevelShape::OVERWORLD,
        }
    }
}
/// `Inventory.add` on a player's 36 slots (hotbar first) and off hand,
/// `max` the stack's own maximum: onto equal stacks first
/// (`getSlotWithRemainingSpace`: the selected slot, the off hand, then the
/// hotbar and the main inventory), then into empty slots of the inventory
/// (`getFreeSlot`, never the off hand). What does not fit stays in `stack`;
/// true when some of it is left.
fn inventory_add(
    inventory: &mut [Option<ItemStack>],
    offhand: &mut Option<ItemStack>,
    selected: usize,
    max: u32,
    stack: &mut ItemStack,
) -> bool {
    let order = std::iter::once(selected)
        .chain(std::iter::once(shield::OFFHAND_SLOT))
        .chain((0..36).filter(|&i| i != selected));
    for i in order {
        let slot = if i == shield::OFFHAND_SLOT {
            &mut *offhand
        } else {
            &mut inventory[i]
        };
        if let Some(slot) = slot
            && slot.same_item_same_components(stack)
            && slot.count < max
        {
            let moved = (max - slot.count).min(stack.count);
            slot.count += moved;
            stack.count -= moved;
            if stack.count == 0 {
                return false;
            }
        }
    }
    for slot in inventory.iter_mut().take(36) {
        if slot.is_none() {
            let moved = stack.count.min(max);
            *slot = Some(ItemStack {
                count: moved,
                ..stack.clone()
            });
            stack.count -= moved;
            if stack.count == 0 {
                return false;
            }
        }
    }
    true
}

fn facing_offset(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
/// The point a `ServerboundInteractPacket` carries: `EntityHitResult.getLocation()
/// - entity.position()`, measured from the entity's feet, as
/// `MultiPlayerGameMode.interact` computes it.
fn interact_location(point: Vec3, feet: Vec3) -> [f64; 3] {
    [point.x - feet.x, point.y - feet.y, point.z - feet.z]
}
/// How far along `dir` (within `reach`) a ray from `eye` enters `b`.
fn ray_box(eye: [f64; 3], dir: [f64; 3], reach: f64, b: Aabb) -> Option<f64> {
    let (min, max) = (b.min_position(), b.max_position());
    let (lo, hi) = ([min.x, min.y, min.z], [max.x, max.y, max.z]);
    let (mut t0, mut t1) = (0.0f64, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}
/// `Entity.calculateViewVector`, which `GameRenderer.pick` aims with: float radians through `Mth.sin` and
/// `Mth.cos`, vanilla's 65536-entry table. A placement's cursor and an interaction's point come from this ray, and
/// the exact trigonometry below moved them by up to ~1e-4 of a block.
fn pick_view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let v = horizon_world::view_direction::view_vector(pitch, yaw);
    [v.x, v.y, v.z]
}
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
/// `LivingEntity.canGlideUsing`: for each worn piece (head, chest, legs,
/// feet), whether it is a glider (its own component, or one the server
/// patched onto the stack) in the slot its `equippable` names, and will not
/// break with its next point of wear (`ItemStack.nextDamageWillBreak`).
fn gliding_pieces(items: &Items, armor: &[Option<ItemStack>; 4]) -> [bool; 4] {
    const SLOTS: [&str; 4] = ["head", "chest", "legs", "feet"];
    // `canGlideUsing`: the stack's own `glider` and `equippable` (its
    // components, a server's patch), else its item's.
    std::array::from_fn(|i| {
        armor[i]
            .as_ref()
            .is_some_and(|s| horizon_client::item_sync::can_glide_using(s, SLOTS[i], items))
    })
}
/// `Level.canHaveWeather`: sky light, no ceiling, and not the End (the one
/// level the rule names by id).
fn level_can_have_weather(sky_light: bool, has_ceiling: bool, is_end: bool) -> bool {
    sky_light && !has_ceiling && !is_end
}
/// `Camera.setRotation` (`rotationYXZ(π - yaw, -pitch, 0)`) for the player's
/// view, turned round in third-person front view (`Camera.setup`).
fn camera_rotation(
    camera_type: horizon_client::camera::CameraType,
    yaw: f32,
    pitch: f32,
) -> glam::Quat {
    let (view_yaw, view_pitch) =
        if camera_type == horizon_client::camera::CameraType::ThirdPersonFront {
            (yaw + 180.0, -pitch)
        } else {
            (yaw, pitch)
        };
    glam::Quat::from_euler(
        glam::EulerRot::YXZ,
        std::f32::consts::PI - view_yaw.to_radians(),
        -view_pitch.to_radians(),
        0.0,
    )
}
impl Game<'_> {
    /// The ticks shown by this frame; still mode follows world time.
    fn tick_blocks_with_scratch(&mut self, scratch: &mut TickScratch) -> Result<Option<usize>> {
        let mut laps = horizon_client::spans::laps();
        // ServerChunkCache.tick: the tickets that time out.
        let world = &self.world;
        self.tickets
            .tick_portal(|ticket| portal_ticket_can_expire(world, ticket));
        self.sync_tick_chunks();
        self.start_ticking_chunks();
        laps.lap("tb_tickets");
        let levels = self.tickets.levels();
        let bounds = levels.random_tick_bounds();
        // Keep the world map's chunk order, including empty chunks
        // (tick_chunk_random still draws for precipitation in those).
        scratch.ticking.clear();
        scratch
            .ticking
            .extend(self.world.chunks.keys().copied().filter(|&(x, z)| {
                if let Some((min_x, min_z, max_x, max_z)) = bounds
                    && (x < min_x || x > max_x || z < min_z || z > max_z)
                {
                    return false;
                }
                levels.random_ticking(x, z) && ticking_chunk(&self.world, x, z)
            }));
        laps.lap("tb_chunks");
        // Entity.checkInsideBlocks for the plates, tripwires, detector rails and
        // buttons under the players (the host and the LAN guests), the minecarts
        // and the arrows (the mobs and other entities are added in `with_level`
        // below).
        scratch.inside.clear();
        let guests = self.lan_block_trigger_boxes();
        let boxes = self
            .player
            .iter()
            .filter(|_| !self.spectator())
            .map(|p| {
                let w = f64::from(p.dimensions.width) / 2.0;
                let h = f64::from(p.dimensions.height);
                (
                    [p.position.x - w, p.position.y, p.position.z - w],
                    [p.position.x + w, p.position.y + h, p.position.z + w],
                )
            })
            .chain(self.carts.iter().map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            }))
            .chain(guests);
        for (lo, hi) in boxes {
            for x in (lo[0] + 1e-7).floor() as i32..=(hi[0] - 1e-7).floor() as i32 {
                for y in (lo[1] + 1e-7).floor() as i32..=(hi[1] - 1e-7).floor() as i32 {
                    for z in (lo[2] + 1e-7).floor() as i32..=(hi[2] - 1e-7).floor() as i32 {
                        scratch.inside.push(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        // AbstractArrow.applyEffectsFromBlocks: the blocks every arrow passed
        // through this tick (a wooden button is pressed by one inside it), and
        // the ones a thrown trident is in.
        for p in self
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.arrow.is_some())
        {
            scratch.inside.extend(horizon_client::arrows::blocks_inside(
                p.previous, p.position,
            ));
        }
        for t in &self.tridents {
            scratch.inside.extend(horizon_client::arrows::blocks_inside(
                t.position, t.position,
            ));
        }
        // BigDripleafBlock.entityInside also asks whether the player stands on
        // the ground and how high its feet are: the leaves its box's feet
        // layer is inside (the host and the LAN guests).
        scratch.standing.clear();
        let feet = self
            .player
            .iter()
            .filter(|_| !self.spectator())
            .map(|p| {
                (
                    [p.position.x, p.position.y, p.position.z],
                    p.ground.on_ground,
                    f64::from(p.dimensions.width) / 2.0,
                )
            })
            .chain(
                self.lan_guest_feet()
                    .into_iter()
                    .map(|(at, on_ground)| (at, on_ground, 0.3)),
            );
        for (at, on_ground, w) in feet {
            let y = (at[1] + 1e-7).floor() as i32;
            for x in (at[0] - w + 1e-7).floor() as i32..=(at[0] + w - 1e-7).floor() as i32 {
                for z in (at[2] - w + 1e-7).floor() as i32..=(at[2] + w - 1e-7).floor() as i32 {
                    scratch
                        .standing
                        .push((BlockPos::new(x, y, z), on_ground, at[1]));
                }
            }
        }
        // ServerLevel.tick ticks only the entities in entity-ticking chunks, and
        // applyEffectsFromBlocks runs in their tick: a mob or item past the
        // simulation distance presses, burns and slows on nothing.
        let entity_ticking = self.entity_ticking_area();
        let first_contact_drop = self.next_entity + 1;
        self.with_level(|b, level, ticks| {
            // LilyPadBlock.entityInside: a boat inside a lily pad breaks it.
            scratch.boat_cells.clear();
            for bounds in level.boat_trigger_boxes() {
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                for x in (lo.x + 1e-7).floor() as i32..=(hi.x - 1e-7).floor() as i32 {
                    for y in (lo.y + 1e-7).floor() as i32..=(hi.y - 1e-7).floor() as i32 {
                        for z in (lo.z + 1e-7).floor() as i32..=(hi.z - 1e-7).floor() as i32 {
                            scratch.boat_cells.push(BlockPos::new(x, y, z));
                        }
                    }
                }
            }
            for pos in scratch.boat_cells.drain(..) {
                b.boat_inside(level, ticks, pos);
            }
            // The mobs and the other entities (items, orbs, projectiles...) too.
            scratch.touched.clear();
            use entity::{Entity as _, for_each_entity};
            let mut touch = |bounds: Aabb| {
                let (lo, hi) = (bounds.min_position(), bounds.max_position());
                for x in (lo.x + 1e-7).floor() as i32..=(hi.x - 1e-7).floor() as i32 {
                    for y in (lo.y + 1e-7).floor() as i32..=(hi.y - 1e-7).floor() as i32 {
                        for z in (lo.z + 1e-7).floor() as i32..=(hi.z - 1e-7).floor() as i32 {
                            scratch.touched.push(BlockPos::new(x, y, z));
                        }
                    }
                }
            };
            for_each_entity!(level.entities, entity => {
                if entity.triggers_blocks() && entity.sweeps_blocks_inside() {
                    let at = entity.position();
                    if entity_ticking(at.x.floor() as i32, at.z.floor() as i32) {
                        touch(entity.bounds());
                    }
                }
            });
            for crystal in level.crystals {
                touch(crystal.bounds());
            }
            for &(bounds, applies) in &level.trigger_hooks {
                if applies {
                    touch(bounds);
                }
            }
            for &bounds in &level.trigger_extras {
                touch(bounds);
            }
            for pos in scratch.inside.drain(..) {
                b.entity_inside(level, ticks, pos);
            }
            // The touched blocks go once each, in (x, y, z) order. Only an
            // entity-sensing block does anything there, so the others are
            // passed over while no block has changed since they were looked
            // at (they are still not sensing), and only the few sensing ones
            // are sorted. Once a block changes, every later one goes, as the
            // whole sorted list would.
            let key = |p: &BlockPos| (p.x, p.y, p.z);
            scratch.sensing.clear();
            scratch.rest.clear();
            for pos in scratch.touched.drain(..) {
                if b.senses_entities(level.world.block(pos.x, pos.y, pos.z)) {
                    scratch.sensing.push(pos);
                } else {
                    scratch.rest.push(pos);
                }
            }
            scratch.sensing.sort_unstable_by_key(key);
            scratch.sensing.dedup();
            let looked = (
                level.world.revision,
                level.world.chunks.membership_revision(),
            );
            let mut i = 0;
            while i < scratch.sensing.len() {
                let pos = scratch.sensing[i];
                b.entity_inside(level, ticks, pos);
                i += 1;
                let now = (
                    level.world.revision,
                    level.world.chunks.membership_revision(),
                );
                if now != looked {
                    scratch.later.clear();
                    scratch.later.append(&mut scratch.rest);
                    scratch.later.extend(scratch.sensing.drain(i..));
                    scratch.later.retain(|p| key(p) > key(&pos));
                    scratch.later.sort_unstable_by_key(key);
                    scratch.later.dedup();
                    for pos in scratch.later.drain(..) {
                        b.entity_inside(level, ticks, pos);
                    }
                    break;
                }
            }
            for (pos, on_ground, feet_y) in scratch.standing.drain(..) {
                b.entity_on_big_dripleaf(level, ticks, pos, on_ground, feet_y);
            }
        });
        self.process_level_events()?;
        self.contact_drops = first_contact_drop..self.next_entity + 1;
        laps.lap("tb_entity_inside");
        // ServerLevel.tick: a Debug world runs no scheduled block or fluid ticks, and
        // ServerChunkCache.tickChunks no random ticks.
        let debug = self.world_debug;
        if !debug {
            self.tick_scheduled()?;
        }
        self.with_level(|b, level, ticks| {
            horizon_client::spans::time("random_ticks", || {
                for (x, z) in scratch.ticking.drain(..).filter(|_| !debug) {
                    // The last answer, asked from Y `from` at a world revision:
                    // the next ticking section and its mask. It stands for a
                    // later Y up to that section's until a block changes.
                    let mut last: Option<(u64, i32, Option<(i32, std::rc::Rc<_>)>)> = None;
                    let pos = horizon_core::position::ChunkPos::new(x, z);
                    b.tick_chunk_random_masked(level, ticks, pos, |level, from| {
                        let revision = level.world.revision;
                        let stands = last.as_ref().is_some_and(|(r, f, next)| {
                            *r == revision
                                && *f <= from
                                && next.as_ref().is_none_or(|(y, _)| from <= *y)
                        });
                        if !stands {
                            let next = level.world.chunks.get(&(x, z)).and_then(|chunk| {
                                level.random_tick_sections.next_masked(
                                    (x, z),
                                    chunk,
                                    level.facts,
                                    |id| b.random_tick_candidate(BlockStateId(id)),
                                    from,
                                )
                            });
                            last = Some((revision, from, next));
                        }
                        last.as_ref()
                            .and_then(|(_, _, next)| next.as_ref())
                            .map(|(y, mask)| (*y, Some(mask.clone())))
                    });
                }
            });
        });
        laps = horizon_client::spans::laps();
        let sent = self.lan_broadcast_changes();
        laps.lap("tb_broadcast");
        self.save_chunks_eagerly();
        self.with_level(|b, level, ticks| {
            b.run_block_events(level, ticks);
        });
        laps.lap("tb_block_events");
        self.process_level_events()?;
        laps.lap("tb_level_events");
        self.sync_moving_pistons();
        laps.lap("tb_pistons");
        Ok(sent)
    }

    fn tick_blocks(&mut self) -> Result<Option<usize>> {
        let mut scratch = std::mem::take(&mut self.tick_scratch);
        let result = self.tick_blocks_with_scratch(&mut scratch);
        self.tick_scratch = scratch;
        result
    }

    fn weather_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> Vec<horizon_client::weather::Vertex> {
        use horizon_client::weather;
        let (intensity, _) = self.view_weather(alpha as f32);
        if intensity <= 0.0 {
            return Vec::new();
        }
        let atlas = &self.gui.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (
            atlas.sprites.get("environment/rain").copied(),
            atlas.sprites.get("environment/snow").copied(),
        ) else {
            return Vec::new();
        };
        let (cx, cy, cz) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        // The 32x32 direction table supports at most a 15-block radius.
        let radius = self.settings.weather_radius.clamp(0, 15);
        let eye = camera.to_array();
        let mut rain = Vec::new();
        let mut snow_vertices = Vec::new();
        // Saturation keeps extreme camera coordinates from overflowing the loop.
        for z in cz.saturating_sub(radius)..=cz.saturating_add(radius) {
            for x in cx.saturating_sub(radius)..=cx.saturating_add(radius) {
                let top = self.rain_height(x, z);
                let y0 = cy.saturating_sub(radius).max(top);
                let y1 = cy.saturating_add(radius).max(top);
                if y1 == y0 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, cy, z) else {
                    continue;
                };
                let parameters = self.weather_parameters.get(x, z);
                let light = self.world.light(x, top.max(cy), z);
                let coords = i32::from(light & 15) << 4 | i32::from(light >> 4) << 20;
                // Only light brightening is needed here; UV phases stay zero.
                let lc = if snow {
                    let (block, sky) = (coords >> 4 & 15, coords >> 20 & 15);
                    ((block * 3 + 15) / 4) << 4 | ((sky * 3 + 15) / 4) << 20
                } else {
                    coords
                };
                let column = weather::Column {
                    x,
                    z,
                    bottom_y: y0,
                    top_y: y1,
                    u_offset: 0.0,
                    v_offset: 0.0,
                    light_coords: lc,
                };
                let r = if snow { snow_tex } else { rain_tex };
                let vertices = weather::vertices(
                    &column,
                    parameters,
                    snow,
                    eye,
                    radius,
                    intensity,
                    [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0],
                );
                if snow {
                    snow_vertices.extend(vertices);
                } else {
                    rain.extend(vertices);
                }
            }
        }
        // Vanilla draws every rain column before the snow ones.
        rain.extend(snow_vertices);
        rain
    }

    fn celestial_geometry(
        &self,
        sky: &horizon_client::environment::Sky,
        rain: f32,
    ) -> (Vec<Vertex>, Option<(glam::Mat4, f32)>) {
        let body_color =
            0x00FF_FFFF | (((1.0 - rain).clamp(0.0, 1.0) * 255.0).round() as u32) << 24;
        let mut out = Vec::new();
        let atlas = &self.gui.atlas;
        let frame = |angle: f32| {
            glam::Mat4::from_rotation_y((-90.0f32).to_radians())
                * glam::Mat4::from_rotation_x(angle.to_radians())
        };
        let mut body = |name: &str, angle: f32, size: f32, moon: bool| {
            let Some(r) = atlas.sprites.get(name).copied() else {
                return;
            };
            // How brightly HDR output lets the body shine (mesher::hdr_tier):
            // the moon at the low tier, not the sun's.
            let tier = if moon {
                horizon_client::mesher::LOW
            } else {
                horizon_client::mesher::VERY_BRIGHT
            };
            let m = frame(angle);
            let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
            // buildCelestialQuad (sun) vs buildMoonPhases: the moon's UVs run
            // the other way round.
            let uv = if moon {
                [[r.u1, r.v1], [r.u0, r.v1], [r.u0, r.v0], [r.u1, r.v0]]
            } else {
                [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]]
            };
            for ((x, z), uv) in corners.iter().zip(uv) {
                let p = m.transform_point3(glam::Vec3::new(*x, 100.0, *z));
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 2 | u32::from(tier) << 16,
                    color: body_color,
                    light: 0,

                    glint_uv: [0.0, 0.0],
                });
            }
        };
        body("environment/celestial/sun", sky.sun_angle, 30.0, false);
        let phases = [
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ];
        let phase = phases[(self.view_day_time() / 24000).rem_euclid(8) as usize];
        body(
            &format!("environment/celestial/moon/{phase}"),
            sky.moon_angle,
            20.0,
            true,
        );
        // SkyRenderer.renderStars: the static star buffer on the same frame.
        let stars =
            (sky.star_brightness > 0.0).then(|| (frame(sky.star_angle), sky.star_brightness));
        (out, stars)
    }

    pub(super) fn visual_ticks(&self) -> u64 {
        if self.clock.still() {
            self.world.game_time.max(0) as u64
        } else {
            self.ticks
        }
    }

    /// Frame time in milliseconds, fixed to world time in still mode.
    pub(super) fn visual_millis(&self) -> i64 {
        if self.clock.still() {
            self.visual_ticks() as i64 * 50
        } else {
            self.started.elapsed().as_millis() as i64
        }
    }

    /// `Level.canHaveWeather`, from the level's dimension type (sky light, no
    /// ceiling) and the End, which vanilla names by id. Every rain, thunder
    /// and lightning rule asks this, never the dimension's id.
    fn level_key(&self) -> horizon_core::LevelKey {
        horizon_core::LevelKey::builtin_or_overworld(self.dim.id())
    }
    /// The level's dimension type: a mod's or a data pack's own when it has one, else the
    /// built-in level's.
    fn dimension_type(&self) -> &horizon_world::dimension_type::DimensionFacts {
        // A client of a server has the facts its server sent for the level, which are its own,
        // not the table the server's levels (in this process, for an integrated server) read.
        if let Some(facts) = self.remote.as_ref().and_then(|r| r.level_facts.as_deref()) {
            return facts;
        }
        // Else the level the server named, whether or not the client has the mod that
        // registered it (`self.dim` is then the Overworld).
        let level = self
            .remote
            .as_ref()
            .map(|r| r.level.as_str())
            .filter(|level| !level.is_empty())
            .unwrap_or_else(|| self.dim.id());
        self.dimension_facts_for(
            horizon_core::LevelKey::parse(level).unwrap_or(horizon_core::LevelKey::OVERWORLD),
        )
    }
    /// A client only knows its current level's type; other keys use built-in defaults.
    fn dimension_facts_for(
        &self,
        key: horizon_core::LevelKey,
    ) -> &horizon_world::dimension_type::DimensionFacts {
        use horizon_world::dimension_type::DimensionFacts;
        if let Some(remote) = &self.remote {
            if horizon_core::LevelKey::parse(&remote.level).ok() == Some(key)
                && let Some(facts) = remote.level_facts.as_deref()
            {
                return facts;
            }
            DimensionFacts::of_builtin_level(key)
        } else {
            DimensionFacts::of_level(key)
        }
    }
    fn dimension_type_of(
        &self,
        dim: Dim,
    ) -> &'static horizon_world::dimension_type::DimensionFacts {
        horizon_world::dimension_type::DimensionFacts::of_level(
            horizon_core::LevelKey::parse(dim.id()).unwrap_or(horizon_core::LevelKey::OVERWORLD),
        )
    }
    /// `Level.isDarkOutside`.
    fn dark_outside(&self) -> bool {
        !self.dimension_type().has_fixed_time && self.sky_darken() >= 4
    }
    fn can_have_weather(&self) -> bool {
        level_can_have_weather(
            self.world.shape.sky_light,
            self.world.shape.has_ceiling,
            self.dim == Dim::End,
        )
    }
    /// The rain and thunder levels (`Level.getRainLevel`, `getThunderLevel`):
    /// the world's, where the level can have weather, else none.
    fn weather_levels(&self, partial: f32) -> (f32, f32) {
        if self.can_have_weather() {
            self.world.weather.levels(partial)
        } else {
            (0.0, 0.0)
        }
    }
    fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ])
    }
    /// Where targeting starts and points: the eyes along the view. Every
    /// crosshair target (blocks, fluids, entities, mod raycasts) is found
    /// along this ray, so a tracked controller can stand in for it.
    fn aim(&self) -> Option<([f64; 3], [f64; 3])> {
        // Minecraft.pick: from the camera entity's eyes along its view
        // (LivingEntity.getViewYRot: the head's yaw).
        if let Some(m) = self.camera_mob() {
            return Some((
                [m.position.x, m.eye_y(), m.position.z],
                pick_view_vector(m.head_yaw, m.head_pitch),
            ));
        }
        Some((self.eye_position()?, pick_view_vector(self.yaw, self.pitch)))
    }
    /// `Player.blockInteractionRange`: the block pick's and mining's reach.
    fn reach(&self) -> f64 {
        self.block_reach
    }
    /// Outline shapes that depend on more than the block state, as one box
    /// in block space: `LightBlock` and `ScaffoldingBlock.getShape` are a full
    /// block while the main hand holds the block's own item
    /// (`CollisionContext.isHoldingItem`), and `ShulkerBoxBlock.getShape`
    /// grows with the lid (`ShulkerBoxBlockEntity.getBoundingBox`).
    fn context_outline(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        player_context: bool,
        anims: &horizon_client::block_entity_anim::BlockEntityAnims,
    ) -> Option<[f64; 6]> {
        let name = &self.res.facts().get(state).name;
        if matches!(&**name, "minecraft:light" | "minecraft:scaffolding") {
            return (player_context && self.held().is_some_and(|s| *s.item == **name))
                .then_some([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        }
        if !name.ends_with("shulker_box") {
            return None;
        }
        let progress = anims.shulkers.get(&pos).map_or(0.0, |s| s.progress);
        let facing = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("facing").cloned());
        let d = facing_offset(facing.as_deref().unwrap_or("up"));
        let mut b = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for a in 0..3 {
            match d[a] {
                1 => b[a + 3] += 0.5 * f64::from(progress),
                -1 => b[a] -= 0.5 * f64::from(progress),
                _ => {}
            }
        }
        Some(b)
    }
    fn outline_context_kind(&self) -> horizon_world::collision_context::EntityContextKind {
        if self.camera_mob().is_some() {
            horizon_world::collision_context::EntityContextKind::Living
        } else {
            horizon_world::collision_context::EntityContextKind::Player
        }
    }
    /// A block's outline boxes in world space (`getShape` plus the random offset).
    fn outline_boxes(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        self.outline_boxes_in_context(&self.world, &self.be_anims, pos, true)
    }
    fn outline_boxes_in_context(
        &self,
        world: &World,
        anims: &horizon_client::block_entity_anim::BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> Vec<[f64; 6]> {
        self.outline_boxes_iter(world, anims, pos, player_context)
            .collect()
    }
    /// The same outline traversal without allocating a box list.
    fn outline_boxes_iter(
        &self,
        world: &World,
        anims: &horizon_client::block_entity_anim::BlockEntityAnims,
        pos: [i32; 3],
        player_context: bool,
    ) -> impl Iterator<Item = [f64; 6]> + '_ {
        let state = world.block(pos[0], pos[1], pos[2]);
        let declared = self.collision.cached_shapes().context_outline_shape(
            state,
            if player_context {
                self.outline_context_kind()
            } else {
                horizon_world::collision_context::EntityContextKind::Other
            },
        );
        let context = declared
            .is_none()
            .then(|| self.context_outline(pos, state, player_context, anims))
            .flatten();
        let f = self.res.facts().get(state);
        let o = perspective::outline_offset(
            &self.pack.context.tables,
            state,
            pos[0],
            pos[2],
            f.outline_offset,
        );
        let base = [
            f64::from(pos[0]) + o[0],
            f64::from(pos[1]) + o[1],
            f64::from(pos[2]) + o[2],
        ];
        let boxes = if context.is_some() || declared.is_some() {
            &[][..]
        } else {
            &f.outline[..]
        };
        context
            .map(|b| std::array::from_fn(|i| f64::from(pos[i % 3]) + b[i]))
            .into_iter()
            .chain(
                declared
                    .into_iter()
                    .flat_map(|shape| shape.boxes())
                    .map(move |b| {
                        let min = b.min_position();
                        let max = b.max_position();
                        let coords = [min.x, min.y, min.z, max.x, max.y, max.z];
                        std::array::from_fn(|i| base[i % 3] + coords[i])
                    }),
            )
            .chain(
                boxes
                    .iter()
                    .map(move |b| std::array::from_fn(|i| base[i % 3] + b[i])),
            )
    }
    /// The declared outline uses vanilla's inside-ray test and interaction override.
    fn clip_declared_outline(
        &self,
        state: BlockStateId,
        pos: [i32; 3],
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
    ) -> Option<Hit> {
        use horizon_core::geometry::{Aabb, Vec3};
        use std::borrow::Cow;
        let facts = self.res.facts().get(state);
        let cached = self.collision.cached_shapes();
        let shape = cached.context_outline_shape(state, self.outline_context_kind());
        let boxes: Cow<'_, [Aabb]> = if let Some(shape) = shape {
            Cow::Borrowed(shape.boxes())
        } else if let Some(b) = self.context_outline(pos, state, true, &self.be_anims) {
            Cow::Owned(vec![Aabb::new(
                Vec3::new(b[0], b[1], b[2]),
                Vec3::new(b[3], b[4], b[5]),
            )])
        } else {
            Cow::Owned(
                facts
                    .outline
                    .iter()
                    .map(|b| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
                    .collect(),
            )
        };
        let offset = if facts.outline_offset {
            self.pack
                .context
                .tables
                .offset(state, pos[0], pos[2])
                .unwrap_or([0.; 3])
        } else {
            [0.; 3]
        };
        let offset = Vec3::new(offset[0], offset[1], offset[2]);
        let interaction: Cow<'_, [Aabb]> = if let Some(shape) = cached.interaction_shape(state) {
            Cow::Borrowed(shape.boxes())
        } else {
            let facing = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("facing"))
                .map(String::as_str);
            Cow::Owned(
                interaction_shapes::boxes(&facts.name, facing)
                    .into_iter()
                    .map(|b| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
                    .collect(),
            )
        };
        let start = Vec3::new(origin[0], origin[1], origin[2]);
        let end = Vec3::new(
            origin[0] + dir[0] * reach,
            origin[1] + dir[1] * reach,
            origin[2] + dir[2] * reach,
        );
        let hit = horizon_world::interaction_shapes::clip_with_interaction_override(
            boxes.iter().map(|b| b.move_by(offset)),
            interaction.iter().copied(),
            start,
            end,
            BlockPos::new(pos[0], pos[1], pos[2]),
        )?;
        Some(Hit {
            pos,
            face: hit.direction.normal(),
            point: [hit.location.x, hit.location.y, hit.location.z],
            inside: hit.inside,
        })
    }
    /// Voxel traversal against non-air, non-fluid blocks, clipped against
    /// each block's outline shape (`BlockGetter.clip` with `ClipContext.Block.OUTLINE`).
    fn pick(&self) -> Option<Hit> {
        // A guest of a hosted world acts on the block their client targeted.
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        if let Some(hit) = self.tick_hit {
            return hit;
        }
        let (origin, dir) = self.aim()?;
        // GameRenderer.pick: the block ray runs out to the larger of the two
        // interaction ranges; filterHitResult then drops a hit past the
        // block's.
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
            .filter(|hit| {
                let distance: [f64; 3] = std::array::from_fn(|i| hit.point[i] - origin[i]);
                distance.iter().map(|v| v * v).sum::<f64>() <= self.block_reach.powi(2)
            })
    }
    /// The same traversal without `filterHitResult`: the block hit that
    /// bounds the entity ray in `GameRenderer.pick`, out to the larger
    /// interaction range — a wall between the two still hides the entity.
    fn pick_unfiltered(&self) -> Option<Hit> {
        if let Some(hit) = self.forced_hit {
            return Some(hit);
        }
        let (origin, dir) = self.aim()?;
        self.clip_ray(origin, dir, self.block_reach.max(self.entity_reach))
    }
    /// The first block outline hit within `reach` along the unit `dir`.
    fn clip_ray(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Hit> {
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| {
            if d == 0.0 {
                f64::INFINITY
            } else {
                (1.0 / d).abs()
            }
        });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 {
                f64::from(pos[i]) + 1.0
            } else {
                f64::from(pos[i])
            };
            t_max[i] = if dir[i] == 0.0 {
                f64::INFINITY
            } else {
                (boundary - origin[i]) / dir[i]
            };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts().get(state);
            if self.collision.cached_shapes().has_context_shapes(state) {
                if let Some(hit) = self.clip_declared_outline(state, pos, origin, dir, reach) {
                    return Some(hit);
                }
            } else if !self.res.facts().is_air(state)
                && (f.fluid == Fluid::None
                    || f.waterlogged
                    || &*f.name == "minecraft:kelp"
                    || f.render_shape == horizon_client::blocks::RenderShape::Model)
                && !matches!(
                    &*f.name,
                    "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
                )
            {
                // The nearest entry into one of the outline's boxes (slab method).
                let mut best: Option<(f64, [i32; 3])> = None;
                for b in self.outline_boxes(pos) {
                    let (mut t0, mut t1, mut axis_in, mut sign) = (0.0f64, f64::INFINITY, None, 0);
                    let mut miss = false;
                    for a in 0..3 {
                        if dir[a] == 0.0 {
                            miss |= origin[a] < b[a] || origin[a] > b[a + 3];
                            continue;
                        }
                        let (lo, hi) =
                            ((b[a] - origin[a]) / dir[a], (b[a + 3] - origin[a]) / dir[a]);
                        let (near, far) = if lo < hi { (lo, hi) } else { (hi, lo) };
                        if near > t0 {
                            t0 = near;
                            axis_in = Some(a);
                            sign = if dir[a] > 0.0 { -1 } else { 1 };
                        }
                        t1 = t1.min(far);
                    }
                    if miss || t0 > t1 || t0 > reach {
                        continue;
                    }
                    let mut normal = face;
                    if let Some(a) = axis_in {
                        normal = [0; 3];
                        normal[a] = sign;
                    }
                    if best.is_none_or(|(bt, _)| t0 < bt) {
                        best = Some((t0, normal));
                    }
                }
                if let Some((t, mut face)) = best {
                    // `BlockGetter.clipWithInteractionOverride`: the hit stays at the
                    // outline's point, with the face of the interaction shape when
                    // the ray enters that first.
                    let name = &*f.name;
                    let facing = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("facing"))
                        .map(String::as_str);
                    let inside = interaction_shapes::boxes(name, facing);
                    if !inside.is_empty() {
                        let at = pos.map(f64::from);
                        let world: Vec<[f64; 6]> = inside
                            .iter()
                            .map(|b| std::array::from_fn(|i| at[i % 3] + b[i]))
                            .collect();
                        if let Some((ti, normal)) =
                            interaction_shapes::entry(origin, dir, &world, reach, face)
                            && ti < t
                        {
                            face = normal;
                        }
                    }
                    return Some(Hit {
                        pos,
                        face,
                        point: [
                            origin[0] + dir[0] * t,
                            origin[1] + dir[1] * t,
                            origin[2] + dir[2] * t,
                        ],
                        // No face entered: the eyes are in the shape.
                        inside: face == [0; 3],
                    });
                }
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
    /// The stack in the hand that acts (`getItemInHand(hand)`).
    fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }
    fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }
    /// The slot of the hand that acts: the selected one, or `OFFHAND_SLOT`.
    /// `LivingEntity.canGlide`: some worn piece `gliding_pieces` lets glide,
    /// or a stack in a hand that glides there (`canGlideUsing` walks every
    /// `EquipmentSlot`).
    fn glider_worn(&self) -> bool {
        let items = self.res.items();
        let hand = |stack: Option<&ItemStack>, slot: &str| {
            stack.is_some_and(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
        };
        gliding_pieces(items, &self.armor).contains(&true)
            || hand(self.inventory[self.selected].as_ref(), "mainhand")
            || hand(self.offhand.as_ref(), "offhand")
    }
    /// `LivingEntity.updateFallFlying`: the glide's wear. `Util.getRandom`
    /// picks one of the gliders that will not break (from the entity's own
    /// random), and
    /// `ItemStack.hurtAndBreak` takes `points` of its durability (Unbreaking
    /// may save it). Callers skip creative.
    pub(super) fn wear_glider(&mut self, points: u32) {
        let gliding = gliding_pieces(self.res.items(), &self.armor);
        let pieces: Vec<usize> = (0..4).filter(|&i| gliding[i]).collect();
        if pieces.is_empty() {
            return;
        }
        // `Util.getRandom(slotsWithGliders, this.random)`: the entity's own
        // random, which never touches the level's stream.
        let pick = random_source::entity().next_int_bound(pieces.len() as i32);
        let pick = pieces[pick.unwrap_or(0) as usize];
        if let Some(stack) = self.armor[pick].clone() {
            let loss = self.wear(&stack, points);
            if let Some(piece) = self.armor[pick].as_mut() {
                piece.damage += loss;
            }
        }
    }
    /// `LivingEntity.canFreeze`'s armor half: any worn piece in
    /// `freeze_immune_wearables`.
    pub(crate) fn wears_freeze_immune(&self) -> bool {
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        self.armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)))
    }
    /// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners
    /// (aiStep, unless a spectator or a passenger): a corner inside a
    /// suffocating block sets a 0.1 speed towards the nearest side that is
    /// free, on that side's axis; a later corner overwrites an earlier one.
    /// Suffocating is a full collision cube that is also a full render cube,
    /// as `in_wall` has it.
    fn closest_space_nudge(&self) -> [Option<f64>; 2] {
        match self.player.as_ref() {
            Some(p) if !p.spectator && self.riding.is_none() => {
                closest_space_nudge(p, &self.world, self.res.facts())
            }
            _ => [None; 2],
        }
    }
    /// What the window decides for its player's body every tick: the look
    /// (Player.travel steers sprint-swimming by the pitch), Auto Jump (LocalPlayer.sendPosition), the slowdown of an
    /// item in use (LocalPlayer.modifyInput), the helmet's Respiration
    /// (decreaseAirSupply), the wings, leather boots on powder snow
    /// (PowderSnowBlock.canEntityWalkOnPowderSnow), freeze-immune wear
    /// (LivingEntity.canFreeze), the dimension's FAST_LAVA, the sleeping
    /// pose and blindness (Player.isMobilityRestricted, which stops a sprint).
    pub(crate) fn sync_player_body(&mut self) {
        let glider = self.glider_worn();
        let sleeping = self.sleeping.is_some();
        let item_in_use = self
            .using
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    self.offhand.as_ref()
                } else {
                    self.inventory.get(slot)?.as_ref()
                }
            })
            .and_then(|s| self.items().get(&s.item));
        let item_use_speed = item_in_use.map(|i| i.use_speed);
        let item_use_slows_sprint = item_in_use.is_some_and(|i| !i.can_sprint);
        let respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        let leather_boots = self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots");
        let freeze_immune = self.wears_freeze_immune();
        let burn_immune = self.wears_burn_from_stepping_immunity();
        let nudge = self.closest_space_nudge();
        let fast_lava = self.dimension_type().fast_lava;
        let blind = self.potions.effects.get("blindness").is_some();
        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.nudge = nudge;
        player.mobility_restricted = blind;
        player.yaw = self.yaw;
        player.pitch = self.pitch;
        player.auto_jump_enabled = self.settings.auto_jump;
        player.item_use_speed = item_use_speed;
        player.item_use_slows_sprint = item_use_slows_sprint;
        player.respiration = respiration;
        player.glider = glider;
        // On someone else's server the glide flag is the server's
        // (`DATA_SHARED_FLAGS_ID`, applied by the SetEntityData handler); the
        // game's own integrated server has no such round trip, so the client
        // keeps predicting the clear a tick after the landing glide.
        player.server_owns_glide = self.integrated.is_none();
        let _ = player.set_sleeping(sleeping);
        player.leather_boots = leather_boots;
        player.freeze_immune = freeze_immune;
        player.burn_from_stepping_immune = burn_immune;
        player.fluids.fast_lava = fast_lava;
    }
    fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }
    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    fn used_hand(&self) -> Option<Hand> {
        item_use::hand_of(self.using)
    }
    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }
    /// Inventory.add: returns the stack that did not fit.
    fn add_stack_to_inventory(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let tutorial_item = matches!(
            self.tutorial.step,
            tutorial::Step::FindTree | tutorial::Step::PunchTree | tutorial::Step::CraftPlanks
        )
        .then(|| stack.clone());
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
        let before = stack.count;
        let left = inventory_add(
            &mut self.inventory,
            &mut self.offhand,
            self.selected,
            max,
            &mut stack,
        );
        if stack.count < before {
            self.tutorial_get_item(tutorial_item.as_ref());
        }
        left.then_some(stack)
    }
    /// The block's first up face (or any face) and its fixed tint, for block
    /// models drawn as entities (falling blocks, carried blocks).
    fn falling_block_face_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let pack = &self.pack;
        let models = pack
            .render
            .model_catalog
            .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = pack.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts().get(state).name;
            if n.contains("leaves") || n.contains("vine") {
                0x48B518
            } else {
                0x91BD59
            }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    fn particle_sprite(&self, state: BlockStateId, pos: [i32; 3]) -> Option<(String, u32)> {
        let sprite = Self::block_particle_sprite(&self.pack, state)?;
        let chunks = &self.world.chunks;
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&horizon_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        let tint = self
            .pack
            .context
            .tables
            .terrain_particle_tint(
                state,
                pos,
                &region,
                world_biomes!(self),
                self.streaming.biome_zoom,
                self.streaming.biome_blend,
                self.streaming.nether_shading,
            )
            .unwrap_or(0xFFFFFF);
        Some((sprite, tint))
    }
    /// ClientLevel.addBreakingParticles: one crumb on the face being mined.
    fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        if self.settings.extras.hide_block_particles {
            return;
        }
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        // Keep client effects separate from the authoritative world RNG.
        let r = self.particles.random();
        let Some(p) = horizon_client::particles::breaking_position(r, pos, face, &boxes) else {
            return;
        };
        let mut particle = Particle::terrain(r, p, [0.0; 3], sprite, tint);
        particle.set_power(0.2);
        particle.scale(0.6);
        self.particles.push(particle);
    }
    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction. The mods hear
    /// the drops (`native_living_drops`) before they join the level.
    fn drop_inventory(&mut self, at: [f64; 3], dying: &creatures::DyingDrops<'_>) {
        let mut stacks: Vec<ItemStack> = Vec::new();
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::KEEP_INVENTORY)
        {
            stacks.extend(self.inventory.iter_mut().filter_map(Option::take));
            stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
            stacks.extend(self.offhand.take());
            stacks.extend(self.equipment.take_all());
            stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
            self.apply_equipment();
        }
        let stacks = stacks.into_iter().map(|stack| (stack, at)).collect();
        self.drop_death_items(dying, stacks, 40, |game| {
            let r = &mut game.world.random;
            let strength = f64::from(r.next_float()) * 0.5;
            let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
            Some([-angle.sin() * strength, 0.2, angle.cos() * strength])
        });
    }
    /// Item entity `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_dropped(&mut self, i: usize) -> ItemEntity {
        let e = self.dropped.remove(i);
        self.native_entity_left(e.id);
        e
    }
    /// Experience orb `i` leaves the level (`Entity.discard`); mods hear it.
    pub(super) fn remove_orb(&mut self, i: usize) -> ExperienceOrb {
        let o = self.orbs.remove(i);
        self.native_entity_left(o.id);
        o
    }
    /// The item entity `spawn_item` just added, unless a native mod kept it
    /// out of the level (`native_entity_join_level`).
    pub(super) fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        let id = self.next_entity;
        self.dropped.last_mut().filter(|d| d.id == id)
    }
    /// `new ItemEntity(level, x, y, z, stack, dx, dy, dz)`.
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) {
        self.add_item_entity(stack, position, Some(velocity), pickup_delay);
    }
    /// `new ItemEntity(level, x, y, z, stack)`: the item leaves with its own
    /// random's motion, which this returns.
    pub(super) fn spawn_item_own(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        pickup_delay: u32,
    ) -> [f64; 3] {
        self.add_item_entity(stack, position, None, pickup_delay)
    }
    fn add_item_entity(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
    ) -> [f64; 3] {
        self.add_item_entity_prepared(stack, position, velocity, pickup_delay, None)
    }
    /// Joins an item only after drop hooks have seen its constructor's motion.
    fn add_item_entity_prepared(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: Option<[f64; 3]>,
        pickup_delay: u32,
        random: Option<spill::ItemRandom>,
    ) -> [f64; 3] {
        let random = random.unwrap_or_else(|| spill::prepare_item_random(velocity));
        let item = stack.item.clone();
        let count = stack.count as i32;
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        self.persist.uuids.insert(self.next_entity, random.uuid);
        let (bob, yaw) = (random.bob, random.yaw);
        let velocity = velocity.unwrap_or(random.velocity);
        self.dropped.push(ItemEntity {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            rotation: [yaw, 0.0],
            pickup_delay: i64::from(pickup_delay),
            random: random.own.into(),
            ..ItemEntity::new(
                self.next_entity,
                stack,
                p,
                Vec3::new(velocity[0], velocity[1], velocity[2]),
                bob,
            )
        });
        // ServerLevel.addFreshEntity.
        if self.native_joins_flush(false, self.next_entity) {
            self.native_item_spawn(self.next_entity, &item, count, position);
        }
        velocity
    }
    /// The block entity at `pos` saved with its id and position
    /// (`saveWithFullMetadata`), or the saved one Horizon keeps as loaded:
    /// what a broken block's loot sees as `LootContextParams.BLOCK_ENTITY`.
    fn loot_block_entity(&self, pos: [i32; 3], block: &str) -> Option<horizon_core::nbt::Tag> {
        let key = (pos[0], pos[1], pos[2]);
        match self.world.block_entities.get(&key) {
            Some(entity) => Some(horizon_client::save::encode_block_entity(
                key, entity, block,
            )),
            None => spill::saved_entity(&self.world, pos),
        }
    }
    /// Where a loot roll at `origin` happens, for a mod's own loot kinds:
    /// the block entity it may copy from, and this level as
    /// `LootContext.getLevel` gives it (`Level.isRaining`: rain above 0.2;
    /// `isThundering`: thunder above 0.9; neither where `canHaveWeather` is
    /// false: a level with a ceiling or no sky light, or the End).
    fn loot_place_at(
        &self,
        origin: [f64; 3],
        block_entity: Option<horizon_core::nbt::Tag>,
    ) -> horizon_world::loot::LootPlace {
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Level.canHaveWeather: sky light, no ceiling and not the End.
        let weather =
            self.world.shape.sky_light && !self.world.shape.has_ceiling && self.dim != Dim::End;
        horizon_world::loot::LootPlace {
            origin: Some(origin),
            block_entity,
            level: Some(horizon_world::loot::LootLevel {
                dimension: self.dim.id().to_owned(),
                game_time: self.world.game_time,
                day_time: self.world.day_time,
                raining: weather && rain > 0.2,
                thundering: weather && thunder > 0.9,
            }),
        }
    }
    /// `Block.getDrops` for a block a player breaks: the block's loot table
    /// with the player as `this`, drawing from its server random sequence
    /// (`LootContext.Builder.create`), else the level random. The roll's
    /// origin is the block's centre and `block_entity` the entity it had.
    fn block_loot_at(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        held: Option<&ItemStack>,
        block_entity: Option<horizon_core::nbt::Tag>,
    ) -> Vec<ItemStack> {
        let dynamic = self.pending.dynamic(pos);
        let place = self.loot_place_at(pos.map(|v| f64::from(v) + 0.5), block_entity);
        let (_, entity) = self.native_clicker();
        let mut drops = horizon_world::loot::with_native_loot_place(place, || {
            let roll = horizon_client::items::BlockLootRoll {
                held,
                dynamic: &dynamic,
                breaker: true,
                ..Default::default()
            };
            self.roll_block_loot(pos, state, Some(entity as u32), roll)
        });
        let player = self.native_acting_id();
        self.native_block_loot(pos, state, Some(player), held, None, &mut drops);
        drops
    }
    /// Block.popResource
    fn pop_resource(&mut self, pos: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        // Block.popResource: Mth.nextDouble offsets from the level random; the
        // motion is `new ItemEntity(..)`'s own, from the item's random.
        let p = spill::pop_position(&mut self.world.random, pos);
        // Block.popResource draws its offsets before the private overload checks the gamerule.
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item_own(stack, p, 10);
    }
    /// `Block.popResourceFromFace`: the loot leaves the clicked face with the
    /// face's own motion instead of the item's random one.
    fn pop_resource_from_face(&mut self, pos: [i32; 3], face: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        let (p, v) = spill::pop_position_from_face(&mut self.world.random, pos, face);
        // The private overload checks the gamerule after the offsets are drawn.
        if !self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return;
        }
        self.spawn_item(stack, p, v, 10);
    }
    /// Player.drop: thrown from the eyes along the view direction.
    fn drop_held(&mut self, all: bool) {
        if self.eye_position().is_none() {
            return;
        }
        let spectator = self.spectator();
        let Some(thrown) = horizon_client::items::take_held_drop(
            &mut self.inventory[self.selected],
            all,
            spectator,
        ) else {
            return;
        };
        self.player_drop(thrown);
    }
    /// `Player.drop`: the statistics and a thrown item entity in front of the player.
    pub(super) fn player_drop(&mut self, thrown: ItemStack) -> Option<u32> {
        let eye = self.eye_position()?;
        // LivingEntity.drop: every drop swings the main hand, a menu's
        // THROW and a creative tab's drop included.
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        // ServerPlayer.drop: the drop statistics, only when an item entity came of it.
        let (item, count) = (thrown.item.clone(), thrown.count as i32);
        let dropped = self.throw_from_eyes(thrown, eye, false, true)?;
        self.award_stat("dropped", &item, count);
        self.award_custom_stat("drop", 1);
        Some(dropped)
    }
    /// `Player.drop(stack, false)`, as `ItemUtils.createFilledResult` drops what no longer fits
    /// in the inventory: thrown from the eyes, with no thrower, statistic or drop hook.
    pub(super) fn drop_plain(&mut self, stack: ItemStack) -> Option<u32> {
        self.drop_stack_from_eyes(stack, false)
    }
    /// `Player.drop(stack, false, thrown_from_hand)`: the stack thrown from the eyes along the
    /// view, the main hand swinging for every drop (`LivingEntity.drop`; `PREDICTED` only keeps
    /// the swing from being sent back to the player who made it). `thrown_from_hand` names the
    /// player as the thrower, as the drop key does. The item's id, none for an empty stack or a
    /// toss a mod canceled.
    pub(super) fn drop_stack_from_eyes(
        &mut self,
        stack: ItemStack,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        if stack.count == 0 {
            return None;
        }
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
        let eye = self.eye_position()?;
        self.throw_from_eyes(stack, eye, false, thrown_from_hand)
    }
    /// `Player.createItemStackToDrop`: an item entity thrown from the eyes
    /// along the view direction. `thrown_from_hand` (`drop(stack, true)`, the
    /// drop key) names the player as the thrower; `drop(stack, false)`, for
    /// what no longer fits in the inventory, does not.
    fn throw_from_eyes(
        &mut self,
        thrown: ItemStack,
        eye: [f64; 3],
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let r = &mut self.world.random;
        let v = horizon_client::entities::drop_motion(r, self.yaw, self.pitch, randomly);
        let y = eye[1] - f64::from(0.3f32);
        // ItemTossEvent: a canceled toss never enters the world; what was thrown is gone.
        // Only `drop(.., false)` tosses fire it: a death's scatter (`randomly`) does not.
        if !randomly {
            let player = self.native_acting_id();
            let mut canceled = false;
            self.native_item_toss(player, &thrown.item, thrown.count, &mut canceled);
            if canceled {
                return None;
            }
        }
        self.spawn_item(thrown, [eye[0], y, eye[2]], v, 40);
        let uuid = thrown_from_hand.then(|| self.acting_player_uuid());
        let item = self.just_spawned_item()?;
        item.thrower = uuid;
        Some(item.id)
    }
    /// The UUID of the player acting: the guest whose turn it is, else the host.
    pub(super) fn acting_player_uuid(&mut self) -> [i32; 4] {
        match self.lan_acting_uuid {
            Some(guest) => horizon_client::lan::playerdata::uuid_ints(guest),
            None => self.level.ensure_player_uuid(),
        }
    }
    /// `Entity.doWaterSplashEffect` for a dropped item. The client draws the
    /// bubble and splash particles (`Level.addParticle` is empty on a server).
    /// The server plays `entity.generic.splash` (`Entity.playSound`); a
    /// client's `playSeededSound` ignores that call.
    pub(super) fn apply_item_splash(&mut self, splash: horizon_client::entities::ItemSplash) {
        use horizon_client::entities::ItemSplash;
        let client = self.options.execution_role.is_client();
        match splash {
            ItemSplash::Sound { at, volume, pitch } if !client => {
                self.audio
                    .play("minecraft:entity.generic.splash", Some(at), volume, pitch);
            }
            ItemSplash::Particle {
                bubble,
                at,
                velocity,
            } if client => {
                use horizon_client::particles::{ParticleOptions, ParticleType};
                let kind = if bubble {
                    ParticleType::Bubble
                } else {
                    ParticleType::Splash
                };
                self.particles
                    .add(&ParticleOptions::simple(kind), at, velocity);
            }
            _ => {}
        }
    }
    fn tick_items(&mut self) {
        // EntityTickEvent.Pre for the items about to tick, and the Post after them.
        let mut round = self.native_tick_round();
        if round.listening() {
            let ticking = self.entity_ticking_area();
            let merging = self.remote.is_none();
            let mut i = 0;
            while let Some(item) = self.dropped.get(i) {
                let (id, x, z) = (
                    item.id,
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                );
                if !(merging && self.joined_during_entity_pass(id)) && ticking(x, z) {
                    let kind = native_ticks::TickKind::Fixed("minecraft:item");
                    self.native_tick_pre_batch(&mut round, id, kind);
                }
                i += 1;
            }
        }
        self.tick_dropped_items(&mut round);
        self.drain_entity_inside();
        self.native_tick_post_batch(round);
    }
    /// `Block.fallOn` for item `id`, which fell `distance` onto the block at `on`
    /// (`Entity.checkFallDamage`): a mod block's `fall_on` Part hears it, through the same
    /// `block_contact::fall` as a player's and a mob's landing. An item takes no fall damage.
    /// Nothing is lent or built unless a Part of that block listens.
    fn item_fall_on(&mut self, id: u32, on: horizon_core::position::BlockPos, distance: f64) {
        let state = self.world.block(on.x, on.y, on.z);
        if !horizon_client::block_contact::fall_listens(state) {
            return;
        }
        let mut lent = horizon_client::health::LentLevel::take(&mut self.world);
        // A native fall_on Part can unwind. Keep the persistent level carrier
        // outside the callback's temporary views, and return its progressed state
        // before propagating the original panic. This does not restore the item batch.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut level = horizon_client::health::DamageLevel::lent(
                &self.world,
                self.pack.assets.registry,
                &mut lent,
                Some(world_items!(self)),
                Some(&self.pack.block_registries),
                &self.damage_hooks,
            );
            let contact = horizon_client::block_contact::Contact {
                entity: id as i32,
                kind: "minecraft:item",
                state,
                position: on,
            };
            let _ = horizon_client::block_contact::fall(&mut level, contact, distance, 0.0);
        }));
        lent.give_back(&mut self.world);
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }
    /// An item that joined during this tick's entity pass: it first ticks on
    /// the next one.
    fn joined_during_entity_pass(&self, id: u32) -> bool {
        id > self.ticked_entities_through || self.contact_drops.contains(&id)
    }
    fn tick_dropped_items(&mut self, round: &mut native_ticks::TickRound) {
        let mut items = self.dropped.lift();
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut removed = vec![false; items.len()];
        let mut cells = horizon_client::entities::ItemCells::new(&items);
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        let mut obstacles =
            horizon_client::entities::ObstacleGrid::new(self.item_entity_collisions());
        let mut nearby = Vec::new();
        let mut bubble_tops = Vec::new();
        // `ItemUtils.onContainerDestroyed`: what a destroyed item held, and
        // where it lay, for adding to the level once its own tick is done.
        let mut destroyed: Vec<(ItemStack, Vec3)> = Vec::new();
        for i in 0..items.len() {
            if removed[i] {
                // Merged into an earlier item before its own tick: it never ticked.
                round.not_ticked(items[i].id);
                continue;
            }
            // `addFreshEntity` during the entity pass: the item ticks from the
            // next tick on (`EntityTickList`).
            if merging && self.joined_during_entity_pass(items[i].id) {
                continue;
            }
            if !ticking(
                items[i].position.x.floor() as i32,
                items[i].position.z.floor() as i32,
            ) || round.canceled(items[i].id)
            {
                continue;
            }
            // Entity.baseTick: the entity-level invulnerability window of
            // the saved data counts down (#916) — only for an item that
            // really ticks in this pass.
            self.tick_saved_invulnerable_time(items[i].id);
            let item = &mut items[i];
            // `getEntityCollisions(entity, bounds.expandTowards(movement))`: the boxes around
            // where the item can reach this tick (a block of slack for the next velocity).
            let reach = Vec3::new(
                item.velocity.x.abs() + 1.0,
                item.velocity.y.abs() + 1.0,
                item.velocity.z.abs() + 1.0,
            );
            obstacles.near(item.bounds().inflate(reach), &mut nearby);
            let no_physics = merging
                && item.inside_block(
                    &self.world,
                    self.collision,
                    &nearby,
                    Some(self.border.geometry()),
                );
            let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
            let mut entered = Vec::new();
            let (alive, discarded) = self.tick_item_with_block_contacts(
                item,
                escape_speed,
                &nearby,
                false,
                &mut |splash| entered.push(splash),
            );
            // Entity.doWaterSplashEffect: the server plays the sound (broadcast
            // to clients). Its addParticle is empty.
            for splash in entered {
                self.apply_item_splash(splash);
            }
            if discarded {
                removed[i] = true;
                cells.remove(i, item.previous);
                continue;
            }
            if merging && !no_physics {
                self.item_landing_event(item.id, item.previous, item.position, item.on_ground);
                self.entity_powder_snow_particles(
                    item.previous,
                    item.position,
                    item.bounds(),
                    item.velocity,
                    false,
                );
            }
            cells.moved(i, item.previous, item.position);
            // Entity.handleOnAboveBubbleColumn: the server sends the particles of
            // the surface of a bubble column the item is at the top of.
            if merging {
                bubble_tops.extend(horizon_client::motion::bubble_column_surfaces(
                    &self.world,
                    self.pack.assets.registry,
                    self.res.facts(),
                    self.collision,
                    item.bounds(),
                ));
            }
            // ItemEntity.tick: merge every 2 ticks while moving between
            // blocks, every 40 otherwise, before the lifetime check.
            let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
            let rate = if block(item.previous) == block(item.position) {
                40
            } else {
                2
            };
            if merging
                && (alive || item.age == 6000 && item.position.y > -128.0)
                && item.ticks.is_multiple_of(rate)
            {
                let max_stack = |stack: &ItemStack| self.stack_max_size(stack).max(1);
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    i,
                    &max_stack,
                );
            }
            let item = &mut items[i];
            let mut alive = alive || (item.age < 6000 && item.position.y > -128.0);
            if merging && !removed[i] && item.age >= 6000 && item.position.y > -128.0 {
                let mut extra_life = 0;
                self.native_item_expire(item.id, &item.stack, &mut extra_life);
                if extra_life > 0 {
                    item.age = 6000 - extra_life.min(32766);
                    alive = true;
                } else if self.serving.is_some() || self.options.dedicated.is_some() {
                    self.native_item_despawn(
                        item.id,
                        &item.stack.item,
                        item.stack.count as i32,
                        [item.position.x, item.position.y, item.position.z],
                    );
                }
            }
            if !alive || removed[i] {
                removed[i] = true;
                cells.remove(i, item.position);
                continue;
            }
            let item_id = item.id;
            let saved = [
                "minecraft:on_fire",
                "minecraft:lava",
                "minecraft:in_fire",
                "minecraft:cactus",
            ]
            .map(|damage| self.saved_refuses_damage(item_id, damage, false));
            let tags = world_items!(self);
            let registry = self.pack.assets.registry;
            let block_registries = &self.pack.block_registries;
            let hooks = &self.damage_hooks;
            let environment = item.environment_tick_with_hook(
                &mut self.world,
                self.res.facts(),
                registry,
                tags,
                |world, damage| {
                    let vanilla = match damage {
                        "minecraft:on_fire" => saved[0],
                        "minecraft:lava" => saved[1],
                        "minecraft:in_fire" => saved[2],
                        "minecraft:cactus" => saved[3],
                        _ => false,
                    };
                    let source = horizon_client::health::HurtSource {
                        damage_type: damage,
                        causing: 0,
                        direct: 0,
                        position: None,
                    };
                    horizon_client::health::DamageLevel::of(
                        world,
                        registry,
                        Some(tags),
                        Some(block_registries),
                        hooks,
                    )
                    .invulnerability_check(
                        horizon_client::health::Hurting::Entity(
                            item_id as i32,
                            "minecraft:item",
                            source,
                        ),
                        vanilla,
                    )
                },
            );
            if environment.should_play_lava_hurt_sound(item.ticks) {
                let pitch = 2.0 + self.world.random.next_float() * 0.4;
                self.audio.play(
                    "minecraft:entity.generic.burn",
                    Some([item.position.x, item.position.y, item.position.z]),
                    0.4,
                    pitch,
                );
            }
            if !environment.alive {
                removed[i] = true;
                cells.remove(i, item.position);
                for stack in environment.spilled {
                    destroyed.push((stack, item.position));
                }
            }
        }
        for pos in bubble_tops {
            self.bubble_column_particles(pos);
        }
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap_or(false));
        let Some(p) = self.player.as_ref() else {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.put_back_dropped(items);
            self.spawn_destroyed_contents(destroyed);
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let own = self.level.ensure_player_uuid();
        // The predicate's Game borrow ends before each yielded row's effects.
        // This cell is stack-local and never accessible to a native callback.
        let game = std::cell::RefCell::new(&mut *self);
        for item in items.extract_if_mut(|item| {
            let mut game = game.borrow_mut();
            let b = item.bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            // ItemEntity.playerTouch: no pickup delay, and the target (if any) is this player.
            if touching && item.pickup_delay == 0 && item.target.is_none_or(|t| t == own) {
                if game.native_pickup_refused(&item.stack) {
                    item.pickup_delay = 10;
                    return false;
                }
                let before = item.stack.count;
                let kind = item.stack.item.clone();
                let thrower = item.thrower;
                match game.add_stack_to_inventory(item.stack.clone()) {
                    None => {
                        game.award_stat("picked_up", &kind, before as i32);
                        if let Some(thrower) = thrower {
                            game.trigger_thrown_picked_up_by_player(&item.stack, thrower);
                        }
                        return true;
                    }
                    Some(rest) => {
                        if rest.count < before {
                            game.award_stat("picked_up", &kind, (before - rest.count) as i32);
                            if let Some(thrower) = thrower {
                                game.trigger_thrown_picked_up_by_player(&rest, thrower);
                            }
                            game.add_pickup(
                                pickups::Taken::Item(Box::new(item.clone())),
                                pickups::Collector::Player,
                            );
                            // handleTakeItemEntity: the random pitch of a full pickup.
                            let pitch = ((game.world.random.next_float()
                                - game.world.random.next_float())
                                * 0.7
                                + 1.0)
                                * 2.0;
                            game.audio
                                .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        }
                        item.stack = rest;
                    }
                }
            }
            false
        }) {
            let mut game = game.borrow_mut();
            game.add_pickup(
                pickups::Taken::Item(Box::new(item)),
                pickups::Collector::Player,
            );
            let pitch = ((game.world.random.next_float() - game.world.random.next_float()) * 0.7
                + 1.0)
                * 2.0;
            game.audio
                .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
        }
        self.put_back_dropped(items);
        self.spawn_destroyed_contents(destroyed);
    }
    /// The item entities `tick_dropped_items` took out, back in the level,
    /// before those its hooks spawned meanwhile (`addFreshEntity` appends).
    /// They keep their join numbers; the spawned ones got theirs on joining.
    fn put_back_dropped(&mut self, items: horizon_client::join_seq::JoinList<ItemEntity>) {
        self.dropped.restore(
            items,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
    }
    /// `ItemUtils.onContainerDestroyed`: each stack a destroyed item held
    /// becomes an item entity where that item lay (`new ItemEntity(level, x,
    /// y, z, stack)`), nudged by 0.2 less 0.1 per axis and pickable at once.
    fn spawn_destroyed_contents(&mut self, destroyed: Vec<(ItemStack, Vec3)>) {
        for (stack, at) in destroyed {
            let r = &mut self.world.random;
            let motion = [
                r.next_double() * 0.2 - 0.1,
                r.next_double() * 0.2 - 0.1,
                r.next_double() * 0.2 - 0.1,
            ];
            self.spawn_item(stack, [at.x, at.y, at.z], motion, 0);
        }
    }
    /// Run vanilla block behaviour against the world through `LevelAccess`.
    fn with_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        self.ensure_stateful_entity_table();
        let darken = self.sky_darken();
        let (rain, thunder) = self.weather_levels(1.0);
        // Spectators ignore block triggers (Entity.isIgnoringBlockTriggers).
        let spectator = self.spectator();
        let player_box = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            (
                [p.position.x - w, p.position.y, p.position.z - w],
                [
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ],
            )
        });
        let player_collision = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let main_hand = self.inventory.get(self.selected).and_then(Option::as_ref);
            PlayerCollision {
                y: p.position.y,
                descending: p.velocity.y < 0.0,
                fall_distance: p.fall_distance,
                main_hand_item: main_hand.map_or_else(
                    || Arc::from("minecraft:air"),
                    |stack| Arc::clone(&stack.item),
                ),
                main_hand_count: main_hand.map_or(0, |stack| stack.count as i32),
                leather_boots: self.armor[3]
                    .as_ref()
                    .is_some_and(|stack| stack.item.as_ref() == "minecraft:leather_boots"),
            }
        });
        let cart_boxes = self
            .carts
            .iter()
            .map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            })
            .collect();
        let arrow_boxes = self
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.arrow.is_some())
            .map(|p| p.position)
            .chain(self.tridents.iter().map(|t| t.position))
            .map(|at| {
                let b = horizon_client::arrows::arrow_bounds(at);
                (b.min_position().components(), b.max_position().components())
            })
            .collect();
        let guest_boxes = self.lan_block_trigger_boxes();
        let building_player_boxes = self
            .lan_acting
            .is_some()
            .then(|| self.lan_building_boxes.boxes(self.dim));
        let dimension = self.dim.level();
        let rules = self.level_rules();
        let fire_players: Vec<[f64; 3]> = self
            .player
            .iter()
            .filter(|_| !spectator)
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .chain(self.lan.iter().flat_map(|h| {
                h.hub
                    .guests
                    .simulation_iter()
                    .filter(|g| h.hub.here(g) && !g.spectator())
                    .map(|g| g.position)
            }))
            .collect();
        let mut trigger_hooks: Vec<(Aabb, bool)> = self
            .hook
            .iter()
            .map(|h| (h.bounds(), h.applies_block_effects()))
            .collect();
        self.lan_hook_triggers(&mut trigger_hooks);
        // EntityType.LIGHTNING_BOLT: 0 x 0.
        let trigger_extras: Vec<Aabb> = self
            .bolts
            .iter()
            .map(|b| point_box(b.at, 0.0, 0.0))
            .collect();
        // `ContainerOpenersCounter.getEntitiesWithContainerOpen`: the players
        // whose menu still holds a container of the block at a position. A
        // double chest answers for both halves (`ChestBlockEntity$1` takes a
        // `CompoundContainer`), and a spectator's menu counts for nothing.
        let mut open_containers: Vec<(i32, i32, i32)> = Vec::new();
        for key in self.container.into_iter().filter(|_| !spectator).chain(
            self.lan
                .iter()
                .flat_map(|h| {
                    h.hub
                        .guests
                        .simulation_iter()
                        .filter(|g| h.hub.here(g) && !g.spectator())
                })
                .filter_map(|g| g.menu.as_ref().and_then(|m| m.pos)),
        ) {
            open_containers.push(key);
            if let Some(other) = self.chest_other_half(key) {
                open_containers.push(other);
            }
        }
        // CopperGolem.hasContainerOpen: the chest a golem opened, and its other half.
        let mut golem_chests: Vec<(i32, i32, i32)> = Vec::new();
        for &key in self.world.chest_openers.keys() {
            golem_chests.push(key);
            golem_chests.extend(self.chest_other_half(key));
        }
        let native_authoritative = self.native_authoritative();
        let mut level = GameLevel {
            command_block_transition: None,
            gateway_servers: &mut self.be_anims.gateway_servers,
            border: self.border.geometry(),
            redstone_experiments: self
                .level
                .enabled_features
                .iter()
                .any(|feature| feature == "minecraft:redstone_experiments"),
            cart_boxes,
            item_frames: &self.item_frames,
            carts: &self.carts,
            arrow_boxes,
            entities: entity::entity_lists_of!(self),
            crystals: &self.crystals,
            trigger_hooks,
            trigger_extras,
            world: &mut self.world,
            facts: self.res.facts(),
            collision: self.collision,
            events: &mut self.level_events,
            dimension,
            darken,
            environment: match dimension {
                LevelDimension::Overworld => self.res.environment(),
                LevelDimension::Nether => self.res.nether_environment(),
                LevelDimension::End => self.res.end_environment(),
                LevelDimension::Custom(key) => horizon_client::environment::Environment::for_level(
                    key.id(),
                    self.res.environment(),
                ),
            },
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            enabled_features: self
                .remote
                .as_ref()
                .map_or(&self.level.enabled_features, |remote| &remote.features),
            player_box,
            guest_boxes,
            building_player_boxes,
            player_collision,
            biomes: world_biomes!(self),
            seed: self.level.seed,
            behaviours: self.res.behaviours(),
            items: world_items!(self),
            recipes: world_recipes!(self),
            // A client rolls no loot (LootTable.getRandomItems needs a
            // ServerLevel): it doesn't load the tables on its game thread.
            loot: if self.remote.is_some() {
                None
            } else {
                self.server_loot.get()
            },
            registry: self.pack.assets.registry,
            block_registries: &self.pack.block_registries,
            rules,
            difficulty: i32::from(self.ui.difficulty),
            fire_players,
            rand_sequences: &mut self.rand_sequences,
            pending: &mut self.pending,
            spill: &mut self.spill,
            tickets: self.tickets.levels(),
            random_tick_sections: &mut self.random_tick_sections,
            lid_openers: &mut self.lid_openers,
            open_containers: &open_containers,
            golem_chests: &golem_chests,
            client_side: self.remote.is_some(),
            native_authoritative,
            native_player: self.lan_acting_uuid,
            native_player_entity: self.lan_acting_entity.map(|id| id as i32),
            debug: self.world_debug,
            native_sounds: &mut self.creatures.lan_sounds,
            native_particles: &mut self.creatures.lan_particles,
            sound_seeds: self.lan.as_ref().map(|host| &host.hub),
            drop_tool: None,
            drop_entity: None,
            drop_breaker_at: None,
            drop_into: None,
        };
        let result = f(self.res.behaviours(), &mut level, &mut self.block_ticks);
        self.flush_spill();
        if self.lan.is_some() {
            self.flush_lan_particles();
            self.flush_native_sounds();
        }
        result
    }
    pub(super) fn native_block_ignite_allowed(
        &mut self,
        pos: BlockPos,
        cause: &'static str,
    ) -> bool {
        self.with_level(|_, level, _| level.allow_block_ignite(pos, cause))
    }
    pub(super) fn native_cauldron_level_change_allowed(
        &mut self,
        pos: BlockPos,
        from: i32,
        to: i32,
        reason: &'static str,
    ) -> bool {
        self.with_level(|_, level, _| level.allow_cauldron_level_change(pos, from, to, reason))
    }
    /// Level.setBlock(pos, state, UPDATE_ALL) with neighbour/shape updates.
    fn level_set(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.level_set_with_flags(pos, state, level_flags::UPDATE_ALL)
    }
    /// Level.setBlock with the caller's update flags.
    fn level_set_with_flags(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                flags,
            );
        });
        self.process_level_events()
    }
    /// `BlockState.canSurvive(level, pos)` for `state` at `pos` in the level in
    /// place: the rule a placement checks before `level_set` (a sapling on
    /// grass, a torch on a wall), with a mod block's own `native_can_survive`
    /// hook. False for an invalid state, an unloaded or out-of-height
    /// position, and on a client.
    ///
    /// This is the non-closure equivalent of `with_level(|b, level, ticks|
    /// b.can_survive(level, ticks, pos, state))`, for a wasm mod whose
    /// internals view cannot send a closure.
    // A wasm mod's staged internals table calls this; nothing in Horizon does.
    #[allow(dead_code)]
    fn level_can_survive(&mut self, pos: [i32; 3], state: BlockStateId) -> bool {
        if !self.is_logical_server()
            || !self.world.loaded(pos[0], pos[2])
            || !self.world.shape.contains_y(pos[1])
            || self.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        self.with_level(|b, level, ticks| {
            b.can_survive(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), state)
        })
    }
    fn attack_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let note = self.with_level(|b, level, ticks| {
            b.attack_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]))
        });
        // NoteBlock.attack awards the statistic.
        if note {
            self.award_custom_stat("play_noteblock", 1);
        }
        self.process_level_events()
    }
    /// `AbstractCandleBlock.getParticleOffsets`, in sixteenths of a block:
    /// `CandleBlock.PARTICLE_OFFSETS` by its `candles` (one to four wicks),
    /// then `CandleCakeBlock.PARTICLE_OFFSETS`, whose one wick sits a block up.
    fn candle_offsets(name: &str, candles: Option<usize>) -> &'static [[u8; 3]] {
        const OFFSETS: [&[[u8; 3]]; 5] = [
            &[[8, 8, 8]],
            &[[6, 7, 8], [10, 8, 7]],
            &[[8, 5, 10], [6, 7, 8], [9, 8, 7]],
            &[[7, 5, 9], [10, 7, 9], [6, 7, 6], [9, 8, 6]],
            &[[8, 16, 8]],
        ];
        if name.ends_with("candle_cake") {
            OFFSETS[4]
        } else {
            OFFSETS[candles.unwrap_or(1).clamp(1, 4) - 1]
        }
    }
    /// `AbstractCandleBlock.extinguish`: the smoke from each wick, at the
    /// block's `getParticleOffsets` (sixteenths of a block), rising 0.1.
    fn candle_smoke(&mut self, pos: [i32; 3], state: BlockStateId) {
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let candles = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("candles")?.parse::<usize>().ok());
        let offsets = Self::candle_offsets(&self.res.facts().get(state).name, candles);
        for o in offsets {
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Smoke),
                [
                    f64::from(pos[0]) + f64::from(o[0]) / 16.0,
                    f64::from(pos[1]) + f64::from(o[1]) / 16.0,
                    f64::from(pos[2]) + f64::from(o[2]) / 16.0,
                ],
                // AbstractCandleBlock.extinguish: ySpeed 0.1F, widened.
                [0.0, f64::from(0.1_f32), 0.0],
            );
        }
    }
    /// `LeverBlock.makeParticle` at full scale: a powered lever puffs one red
    /// dust on the clicking client (`LeverBlock.useWithoutItem`).
    fn lever_puff(&mut self, pos: [i32; 3]) {
        // The server's side of a guest's pull draws nothing.
        if self.lan_acting.is_some() {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let Some(registry_state) = self.pack.assets.registry.state(state) else {
            return;
        };
        let prop = |key: &str| registry_state.properties.get(key).map(String::as_str);
        if prop("powered") != Some("true") {
            return;
        }
        // FACING is horizontal; getConnectedDirection is the face's direction.
        let facing = match prop("facing") {
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => return,
        };
        let connected = match prop("face") {
            Some("ceiling") => [0, -1, 0],
            Some("floor") => [0, 1, 0],
            _ => facing,
        };
        let at = |base: i32, i: usize| {
            f64::from(base) + 0.5 + 0.1 * f64::from(-facing[i]) + 0.2 * f64::from(-connected[i])
        };
        let p = [at(pos[0], 0), at(pos[1], 1), at(pos[2], 2)];
        self.particles.add(
            &horizon_client::particles::ParticleOptions::dust([1.0, 0.0, 0.0], 1.0),
            p,
            [0.0; 3],
        );
    }
    /// Track which chunks are ticking (Ticks needs chunk load/unload).
    fn sync_tick_chunks(&mut self) {
        if !self.tick_chunks.synchronize(&self.world.chunks) {
            return;
        }
        // Move only the added keys out so registration can borrow all of Game.
        let added = self.tick_chunks.take_added();
        for &key in &added {
            self.register_chunk_ticks(key);
        }
        for &key in self.tick_chunks.removed() {
            self.random_tick_sections.remove(&key);
            self.portals.sections.remove(&key);
            self.block_ticks
                .remove_chunk(horizon_core::position::ChunkPos::new(key.0, key.1));
            self.block_ticks
                .comparator_outputs
                .retain(|p, _| (p.x >> 4, p.z >> 4) != key);
            self.hoppers.retain(|p, _| (p.0 >> 4, p.2 >> 4) != key);
            // Its moving pistons were saved with it and load back with it.
            self.block_ticks
                .moving_pistons
                .retain(|(p, _)| (p.x >> 4, p.z >> 4) != key);
        }
        self.tick_chunks.recycle_added(added);
    }
    /// `DistanceManager`'s tickets of the level in place, from every player in
    /// it (the local player, LAN guests, a server's players; a spectator the
    /// rule skips holds none), its forced chunks and its portal tickets. A game
    /// with neither a server nor a player (the headless fixtures) ticks
    /// everywhere.
    fn update_chunk_tickets(&mut self) {
        let rule = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPECTATORS_GENERATE_CHUNKS);
        let mut players: Vec<((i32, i32), bool)> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g))
                .filter(|g| !g.is_fake()) // fake players hold small forced tickets below
                .map(|g| (g.chunk(), g.spectator()))
                .collect()
        });
        if let Some(p) = &self.player {
            players.push((
                (
                    (p.position.x.floor() as i32) >> 4,
                    (p.position.z.floor() as i32) >> 4,
                ),
                self.spectator(),
            ));
        }
        // A spectator the rule skips is still a player: a game that has one
        // must not take the headless fixtures' tick-everywhere case below.
        let any_player = !players.is_empty();
        let sources = (!self.options.execution_role.is_client() || any_player).then(|| {
            horizon_client::ticket_levels::Sources {
                players: players
                    .into_iter()
                    .filter(|&(_, spectator)| game_mode::loads_chunks(rule, spectator))
                    .map(|(chunk, _)| chunk)
                    .collect(),
                forced: self.streaming.forced.iter().copied().collect(),
                // Filled from the `PORTAL` tickets by `update`.
                portal: Vec::new(),
                // EnderDragonFight.tick: while a player watches the fight, the
                // arena holds a `DRAGON` ticket of radius 9 at (0, 0), so its
                // island ticks out to 7 chunks whatever the watcher's
                // simulation distance. The fight lives where the type has one.
                dragon: (self.dimension_type().has_ender_dragon_fight
                    && self.fight.arena_watched())
                .then_some((0, 0)),
                view_distance: self.settings.render_distance.clamp(2, 32),
                simulation_distance: self.settings.simulation_distance.max(2),
            }
        });
        // The levels' changes are kept only for a listener, so that a tick
        // with none looks at no chunk.
        self.tickets.record_changes(
            self.is_logical_server()
                && mod_api::__private::listened(mod_api::HookId::ChunkTicketLevelUpdated),
        );
        self.tickets.update(sources);
        self.run_chunk_ticket_level_updates();
    }
    /// `ServerLevel.startTickingChunk`: a loaded chunk's saved ticks are
    /// scheduled from the game time at which it becomes a ticking chunk
    /// (block ticking, with its neighbours loaded). Chunks become ticking
    /// between ticks in vanilla: before this tick's clock advanced.
    fn start_ticking_chunks(&mut self) {
        let levels = self.tickets.levels();
        let world = &self.world;
        let started = self
            .tick_chunks
            .start_ticking(|(x, z)| levels.loaded_ticking(x, z) && ticking_chunk(world, x, z));
        let time = self.world.game_time - 1;
        for (x, z) in started {
            let pos = horizon_core::position::ChunkPos::new(x, z);
            self.block_ticks.blocks.unpack_chunk(pos, time);
            self.block_ticks.fluids.unpack_chunk(pos, time);
        }
    }

    /// `ServerLevel.tick`: moving block entities push after entity travel.
    fn tick_moving_pistons(&mut self) -> Result<()> {
        self.with_level(|b, level, ticks| b.tick_block_entities(level, ticks));
        let pushes = std::mem::take(&mut self.block_ticks.piston_pushes);
        self.push_piston_entities(pushes);
        self.process_level_events()?;
        self.sync_moving_pistons();
        Ok(())
    }
    /// Copies the moving-piston block entities into the world, where the
    /// collision queries and the renderer read them.
    ///
    /// A client runs this for every block change that ends a piston's move,
    /// so it stays linear in the pistons moving: a clock of a thousand pistons
    /// ends two thousand moves at once.
    fn sync_moving_pistons(&mut self) {
        use horizon_client::fast_hash::{FxHashMap, FxHashSet};
        let previous = std::mem::take(&mut self.world.moving_pistons);
        let current: Vec<_> = self
            .block_ticks
            .moving_pistons
            .iter()
            .map(|(p, piston)| ([p.x, p.y, p.z], piston.shape_state()))
            .collect();
        // The positions drawn by `current` or `fallback` so far.
        let mut drawn: FxHashSet<[i32; 3]> = current.iter().map(|(p, _)| *p).collect();
        let mut fallback: Vec<_> = previous
            .into_iter()
            .filter(|(pos, _)| !drawn.contains(pos))
            .collect();
        drawn.extend(fallback.iter().map(|(p, _)| *p));
        // A finished piston stays drawn until the chunk mesh shows the block
        // it placed, however many ticks that takes. Pistons share a few
        // sections, so each section is asked once.
        let older = std::mem::take(&mut self.world.moving_piston_fallback);
        let mut pending: FxHashMap<(i32, i32, i32), bool> = FxHashMap::default();
        for (pos, piston) in older {
            if drawn.contains(&pos) {
                continue;
            }
            let Some(section) = self.section_of(pos) else {
                continue;
            };
            if *pending
                .entry(section)
                .or_insert_with(|| self.section_mesh_pending_in(section))
            {
                drawn.insert(pos);
                fallback.push((pos, piston));
            }
        }
        self.world.moving_piston_fallback = fallback;
        self.world.moving_pistons = current;
    }
    /// `ServerLevel.tick`'s scheduled block then fluid ticks, one at a time:
    /// the side effects of each (a shrieker summoning a warden, drops, game
    /// events) happen before the next tick runs, as they do inline in vanilla.
    pub(super) fn tick_scheduled(&mut self) -> Result<()> {
        for fluids in [false, true] {
            let mut laps = horizon_client::spans::laps();
            let name = if fluids { "tb_fluids" } else { "tb_scheduled" };
            if !self.with_level(|b, level, ticks| b.scheduled_begin(level, ticks, fluids)) {
                laps.lap(name);
                continue;
            }
            loop {
                // Due ticks run against one level while each leaves nothing for
                // the game to apply (level events, spilled items or orbs): the
                // next one would see the same level built again.
                let more = self.with_level(|b, level, ticks| {
                    loop {
                        let more = b.scheduled_step(level, ticks, fluids);
                        if !more || !level.events.is_empty() || !level.spill.is_empty() {
                            break more;
                        }
                    }
                });
                if !self.level_events.is_empty() {
                    self.process_level_events()?;
                }
                if !more {
                    break;
                }
            }
            self.with_level(|b, _, ticks| b.scheduled_finish(ticks, fluids));
            laps.lap(name);
        }
        Ok(())
    }
    /// Side effects reported by the behaviour library, in vanilla order.
    fn process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.level_events);
        self.lan_record_events(&events);
        dispense_fixture::record_events(&events);
        for event in events {
            match event {
                LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data,
                    ..
                } => {
                    // LevelEventHandler: destroy particles and the block's
                    // break sound (none for air).
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.destroy_particles(p, state);
                    if self.res.facts().is_air(state) {
                        continue;
                    }
                    let sounds = self.res.facts().get(state).sounds.clone();
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio.play(
                        &sounds.break_sound,
                        Some(c),
                        (sounds.volume + 1.0) / 2.0,
                        sounds.pitch * 0.8,
                    );
                }
                LevelEvent::LevelEvent {
                    id: 3006,
                    pos,
                    data,
                    ..
                } => self.sculk_charge_event(pos, data),
                LevelEvent::LevelEvent { id: 3007, pos, .. } => self.shriek_event(pos),
                // LevelEventHandler 1010 and 1011: a jukebox's song starts
                // (`playJukeboxSong`) or stops, and the parrots nearby dance or stop.
                LevelEvent::LevelEvent {
                    id: 1010,
                    pos,
                    data,
                    ..
                } => self.jukebox_song_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 1011, pos, .. } => {
                    self.jukebox_stop_event([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 2019 and 2020: the crack on a face of the
                // block being mined, with its hit sound for 2020.
                LevelEvent::LevelEvent {
                    id: id @ (2019 | 2020),
                    pos,
                    data,
                    ..
                } => {
                    let face = horizon_core::geometry::Direction::ALL[data.clamp(0, 5) as usize];
                    self.breaking_block_effects([pos.x, pos.y, pos.z], face.normal(), id == 2020);
                }
                LevelEvent::LevelEvent {
                    id: 3008,
                    pos,
                    data,
                    ..
                } => self.brush_complete_event([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2004, pos, .. } => {
                    self.spawner_burst([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 1505: the growth particles, then the sound.
                LevelEvent::LevelEvent {
                    id: 1505,
                    pos,
                    data,
                    ..
                } => {
                    self.growth_particles(pos, data);
                    let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
                    self.audio
                        .play("minecraft:item.bone_meal.use", Some(at), 1.0, 1.0);
                }
                LevelEvent::LevelEvent {
                    id: 2015,
                    pos,
                    data,
                    ..
                } => self.dragon_egg_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2016,
                    pos,
                    data,
                    ..
                } => self.shulker_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2017,
                    pos,
                    data,
                    ..
                } => self.consume_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2018,
                    pos,
                    data,
                    ..
                } => self.enderman_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent { id: 2003, pos, .. } => {
                    self.eye_break_particles([pos.x, pos.y, pos.z]);
                }
                LevelEvent::LevelEvent {
                    id: 2013,
                    pos,
                    data,
                    ..
                } => self.smash_dust(pos, data),
                // LevelEventHandler 2002 and 2007: a potion's splash.
                LevelEvent::LevelEvent {
                    id: 2002,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, false),
                LevelEvent::LevelEvent {
                    id: 2007,
                    pos,
                    data,
                    ..
                } => self.potion_splash_particles(pos, data, true),
                LevelEvent::LevelEvent {
                    id: id @ 3002..=3005,
                    pos,
                    data,
                    ..
                } => self.block_particle_event(id, pos, data),
                LevelEvent::LevelEvent {
                    id: 1500,
                    pos,
                    data,
                    ..
                } => self.composter_fill_event(pos, data == 1),
                // LevelEventHandler 1504: the stalactite's drip particle.
                LevelEvent::LevelEvent { id: 1504, pos, .. } => self.dripstone_drip_particle(pos),
                // LevelEventHandler 2006: the dragon fireball's burst.
                LevelEvent::LevelEvent {
                    id: 2006,
                    pos,
                    data,
                    ..
                } => self.dragon_fireball_burst(pos, data),
                // LevelEventHandler 2009: a wet sponge drying.
                LevelEvent::LevelEvent { id: 2009, pos, .. } => {
                    self.puffs_above_block(pos, horizon_client::particles::ParticleType::Cloud)
                }
                // LevelEventHandler 2011 and 2012: a bee growing a crop, a
                // turtle egg laid.
                LevelEvent::LevelEvent {
                    id: 2011 | 2012,
                    pos,
                    data,
                    ..
                } => self.turtle_egg_particles(pos, data),
                // LevelEventHandler 2014: a straw bed destroyed, with no sound.
                LevelEvent::LevelEvent {
                    id: 2014,
                    pos,
                    data,
                    ..
                } => self.destroy_particles([pos.x, pos.y, pos.z], BlockStateId(data as u32)),
                // EnderDragonFight.spawnNewGateway: an always-visible burst
                // and a sound at the gateway for every nearby player.
                LevelEvent::LevelEvent { id: 3000, pos, .. } => {
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.particles.add_with(
                        &horizon_client::particles::ParticleOptions::simple(
                            horizon_client::particles::ParticleType::ExplosionEmitter,
                        ),
                        at,
                        [0.0; 3],
                        true,
                        true,
                    );
                    let random = self.particles.level_random();
                    let pitch = (1.0 + (random.next_float() - random.next_float()) * 0.2) * 0.7;
                    self.audio.play_unshared(
                        "minecraft:block.end_gateway.spawn",
                        Some(at),
                        10.0,
                        pitch,
                    );
                }
                // LevelEventHandler 3009: a sniffer egg cracking.
                LevelEvent::LevelEvent { id: 3009, pos, .. } => {
                    self.sniffer_egg_crack_particles(pos)
                }
                LevelEvent::LevelEvent {
                    id: id @ 3011..=3021,
                    pos,
                    data,
                    ..
                } => self.trial_level_event(id, [pos.x, pos.y, pos.z], data),
                // LevelEvent.SOUND_PORTAL_TRAVEL: LevelEventHandler.levelEvent
                // plays SimpleSoundInstance.forLocalAmbience (ambient, no
                // position). EndPortalBlock.entityInside travels through
                // TeleportTransition.PLAY_PORTAL_SOUND, which sends 1032 to
                // the traveling player. The packet's position is BlockPos.ZERO,
                // so a positional play would not be heard; at None is the
                // relative sound, and it is not captured for other players.
                LevelEvent::LevelEvent { id: 1032, .. } => {
                    let pitch = self.particles.level_random().next_float() * 0.4 + 0.8;
                    self.audio.play_on(
                        horizon_client::sound_engine::AMBIENT,
                        "minecraft:block.portal.travel",
                        None,
                        0.25,
                        pitch,
                    );
                }
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ]);
                    let sound = match id {
                        1503 => Some(("minecraft:block.end_portal_frame.fill", 1.0)),
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1033 => Some(("minecraft:block.chorus_flower.grow", 1.0)),
                        1034 => Some(("minecraft:block.chorus_flower.death", 1.0)),
                        1045 => Some(("minecraft:block.pointed_dripstone.land", 2.0)),
                        1052 => Some(("minecraft:block.sulfur_spike.land", 2.0)),
                        1046 => Some((
                            "minecraft:block.pointed_dripstone.drip_lava_into_cauldron",
                            2.0,
                        )),
                        1047 => Some((
                            "minecraft:block.pointed_dripstone.drip_water_into_cauldron",
                            2.0,
                        )),
                        // LevelEvent.SOUND_ZOMBIE_WOODEN_DOOR (attack, break).
                        1019 => Some(("minecraft:entity.zombie.attack_wooden_door", 2.0)),
                        1021 => Some(("minecraft:entity.zombie.break_wooden_door", 2.0)),
                        1026 => Some(("minecraft:entity.zombie.infect", 2.0)),
                        1027 => Some(("minecraft:entity.zombie_villager.converted", 2.0)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1029 => Some(("minecraft:block.anvil.destroy", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1042 => Some(("minecraft:block.grindstone.use", 1.0)),
                        1044 => Some(("minecraft:block.smithing_table.use", 1.0)),
                        // LevelEvent.SOUND_PAGE_TURN (a lectern's page).
                        1043 => Some(("minecraft:item.book.page_turn", 1.0)),
                        // LevelEvent.SOUND_SPLASH_POTION and its instant twin.
                        1053 | 1054 => Some(("minecraft:entity.splash_potion.break", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        let pitch = match id {
                            1009 => {
                                2.6 + (self.world.random.next_float()
                                    - self.world.random.next_float())
                                    * 0.8
                            }
                            // LevelEventHandler draws these from the client
                            // level's random, before 1501's and 1502's smoke.
                            1501 | 1502 => {
                                let r = self.particles.level_random();
                                2.6 + (r.next_float() - r.next_float()) * 0.8
                            }
                            1045 | 1046 | 1047 | 1052 => {
                                self.particles.level_random().next_float() * 0.1 + 0.9
                            }
                            1053 | 1054 => {
                                // LevelEventHandler's client random: impact audio
                                // must not change the server's bottle XP stream.
                                self.particles.level_random().next_float() * 0.1 + 0.9
                            }
                            1029 | 1030 | 1042 | 1043 | 1044 => {
                                self.world.random.next_float() * 0.1 + 0.9
                            }
                            1026 | 1027 => {
                                let r = self.particles.level_random();
                                (r.next_float() - r.next_float()) * 0.2 + 1.0
                            }
                            1019 | 1021 => {
                                (self.world.random.next_float() - self.world.random.next_float())
                                    * 0.2
                                    + 1.0
                            }
                            _ => 1.0,
                        };
                        if matches!(
                            id,
                            1019 | 1021
                                | 1026
                                | 1027
                                | 1029
                                | 1030
                                | 1045
                                | 1052
                                | 1053
                                | 1054
                                | 1501
                                | 1503
                        ) {
                            // Clients play these sounds from their level events;
                            // capturing it also sends a duplicate sound packet.
                            self.audio.play_unshared(name, c, volume, pitch);
                        } else {
                            self.audio.play(name, c, volume, pitch);
                        }
                    }
                    // LevelEventHandler draws smoke after the sound.
                    match id {
                        1501 => self.puffs_above_block(
                            pos,
                            horizon_client::particles::ParticleType::LargeSmoke,
                        ),
                        1502 => self.torch_burnout_smoke(pos),
                        1503 => self.end_portal_frame_smoke(pos),
                        _ => {}
                    }
                }
                LevelEvent::ContainerLootTable { pos, tag } => {
                    let key = (pos.x, pos.y, pos.z);
                    if let Some(mut entity) = self.world.block_entities.remove(&key) {
                        self.take_loot_table(key, &tag, &mut entity);
                        self.world.block_entities.insert(key, entity);
                        self.world.modified.insert((pos.x >> 4, pos.z >> 4));
                    }
                }
                // A prime the mods refused is silent: no fuse sound, no game event.
                LevelEvent::Sound { pos, ref sound, .. }
                    if self.tnt_refused == Some(pos)
                        && &**sound == "minecraft:entity.tnt.primed" => {}
                LevelEvent::GameEvent {
                    pos,
                    event: "minecraft:prime_fuse",
                    ..
                } if self.tnt_refused == Some(pos) => self.tnt_refused = None,
                LevelEvent::Sound {
                    pos,
                    sound,
                    volume,
                    pitch,
                } => {
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    let name = if sound.contains(':') {
                        sound.to_string()
                    } else {
                        format!("minecraft:{sound}")
                    };
                    self.audio.play(&name, Some(c), volume, pitch);
                }
                // Handled as the behaviour emits it (GameLevel::emit).
                LevelEvent::DropResources { pos, state } => {
                    self.with_level(|_, level, _| level.drop_resources_now(pos, state));
                }
                LevelEvent::FallingBlock {
                    pos,
                    state,
                    hurt_per_distance,
                } => {
                    let mut damage = horizon_client::falling_block::Damage::falling(
                        &self.res.facts().get(state).name,
                    );
                    if let Some(per_distance) = hurt_per_distance {
                        damage.hurt_entities = true;
                        damage.per_distance = per_distance;
                    }
                    let p = Vec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y),
                        f64::from(pos.z) + 0.5,
                    );
                    self.next_entity += 1;
                    self.falling.push(FallingBlock {
                        attachment_lifetime: Default::default(),
                        rotation: [0.0, 0.0],
                        join: Default::default(),
                        id: self.next_entity,
                        state,
                        start_pos: pos,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        age: 0,
                        no_gravity: false,
                        portal: Default::default(),
                        drop_item: true,
                        fall_distance: 0.0,
                        on_ground: false,
                        stuck: Vec3::ZERO,
                        damage,
                        saved: None,
                    });
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_block_effect_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep if self.beds_destroy_on_use() => {
                        self.destroy_bed_on_use([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::Sleep => self.try_sleep([pos.x, pos.y, pos.z]),
                    PlayerEffect::GiveItem(item) => {
                        if let Some(rest) = self.add_stack_to_inventory(ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        }) {
                            self.pop_resource([pos.x, pos.y, pos.z], rest);
                        }
                    }
                    // Popped as the behaviour emits it (GameLevel::emit).
                    PlayerEffect::DropItem(_) => {}
                    PlayerEffect::EatCake => {
                        // CakeBlock.eat
                        self.award_custom_stat("eat_cake_slice", 1);
                        if let Some(p) = self.player.as_mut() {
                            let food = &mut p.health.food;
                            food.food = (food.food + 2).min(20);
                            food.saturation = (food.saturation + 0.4).min(food.food as f32);
                        }
                    }
                    PlayerEffect::RespawnAnchor => {
                        self.respawn_anchor_used([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::InteractLoot(_) => {}
                },
                LevelEvent::PrimeTnt { pos, rot } => self.prime_tnt_event(pos, rot),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "dispense",
                } => self.dispense_from(pos, state)?,
                LevelEvent::BlockEvent {
                    pos, kind: "golem", ..
                } => self.check_golem_spawn([pos.x, pos.y, pos.z])?,
                LevelEvent::BlockEvent {
                    pos,
                    kind: "command_block",
                    ..
                } => self.command_block_tick(pos),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "structure_block",
                    ..
                } => self.structure_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block",
                    ..
                } => self.test_block_neighbor_changed((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "test_block_tick",
                    ..
                } => self.reset_test_block((pos.x, pos.y, pos.z)),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "brushable",
                    ..
                } => self.brushable_tick([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos, kind: "hatch", ..
                } => {
                    // FrogspawnBlock.hatchFrogspawn: 2-5 tadpoles.
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio
                        .play("minecraft:block.frogspawn.hatch", Some(at), 1.0, 1.0);
                    let count = 2 + self.world.random.next_int_bound(4).unwrap_or(0);
                    for _ in 0..count {
                        // getRandomTadpolePositionOffset: 0.2..0.8 inside the block.
                        let r = &mut self.world.random;
                        let off = |r: &mut horizon_core::random::LegacyRandom| {
                            f64::from(r.next_float()) * 0.6 + 0.2
                        };
                        let (dx, dz) = (off(r), off(r));
                        let spawn = [
                            f64::from(pos.x) + dx,
                            f64::from(pos.y) - 0.5,
                            f64::from(pos.z) + dz,
                        ];
                        let _ = self.spawn_mob(
                            "minecraft:tadpole",
                            spawn,
                            true,
                            creatures::SpawnSite::Plain,
                        );
                    }
                }
                LevelEvent::BlockEvent {
                    pos,
                    kind: "hatch_sniffer",
                    ..
                } => self.hatch_sniffer([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "hatch_turtles",
                } => self.hatch_turtles([pos.x, pos.y, pos.z], state),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "portal_spawn",
                    ..
                } => self.portal_spawn([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "spawn_ghastling",
                } => self.spawn_ghastling([pos.x, pos.y, pos.z], state),
                LevelEvent::GameEvent { pos, event, state } => {
                    self.sculk_level_event(pos, event, state)
                }
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "shrieker_respond",
                } => self.shrieker_block_event(pos, state),
                LevelEvent::BlockEntityEvent { pos, a, b, .. } => {
                    self.block_entity_event([pos.x, pos.y, pos.z], a, b)
                }
                // NoteBlock.triggerEvent: a NOTE particle above, coloured by
                // the note (`note / 24.0` in the x speed).
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "note_particle",
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let note = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("note")?.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    self.particles.add(
                        &ParticleOptions::simple(ParticleType::Note),
                        [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 1.2,
                            f64::from(pos.z) + 0.5,
                        ],
                        [note / 24.0, 0.0, 0.0],
                    );
                }
                // NoteBlock.getCustomSoundId: the skull above's note block sound,
                // played at volume 3 (vanilla's pitch for a head is 1).
                LevelEvent::BlockEvent {
                    pos,
                    kind: "note_custom_sound",
                    ..
                } => {
                    if let Some(BlockEntity::Skull(skull)) =
                        self.world.block_entities.get(&(pos.x, pos.y + 1, pos.z))
                        && let Some(sound) = skull.note_block_sound.as_deref()
                    {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        self.audio.play(sound, Some(at), 3.0, 1.0);
                    }
                }
                // `level.addParticle`: only the local player's own click draws it.
                LevelEvent::BlockEvent {
                    kind: "candle_smoke",
                    ..
                } if self.lan_acting.is_some() => {}
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "candle_smoke",
                } => self.candle_smoke([pos.x, pos.y, pos.z], state),
                // The block at `pos` has just replaced `state`: Block.pushEntitiesUp.
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "push_entities_up",
                } => self.push_entities_up(pos, state),
                LevelEvent::GrowTree { .. }
                | LevelEvent::PlaceFeature { .. }
                | LevelEvent::BlockEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Menus opened by PlayerEffect::OpenMenu.
    fn open_block_effect_menu(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Menu) {
            return;
        }
        let name = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let key = (pos[0], pos[1], pos[2]);
        if self.chest_menu_blocked(key, &name) {
            // The use still succeeds, but no menu, stat or piglin anger.
            return;
        }
        // BaseContainerBlockEntity.createMenu: a lock the opener cannot
        // unlock refuses the menu with vanilla's action bar and sound.
        if self.refuse_locked_container(key, &name) {
            return;
        }
        if &*name == "minecraft:lectern" {
            self.open_lectern(pos);
            return;
        }
        if horizon_client::command_block::Mode::of_block(&name).is_some() {
            self.open_command_block(key);
            return;
        }
        // The game master blocks' screens open on the player's own client
        // (`BlockEntity.usedBy`); the server has nothing to open.
        if matches!(
            &*name,
            "minecraft:structure_block"
                | "minecraft:jigsaw"
                | "minecraft:test_block"
                | "minecraft:test_instance_block"
        ) {
            return;
        }
        // SignBlock.useWithoutItem: the text editor, not a container.
        let block = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().get(block).sign_form().is_some() {
            self.use_sign_empty_hand(pos);
            return;
        }
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
            self.craft_table = Some(pos);
        } else if &*name == "minecraft:ender_chest" {
            self.open_ender_chest(key);
            // EnderChestBlock.useWithoutItem
            self.award_custom_stat("open_enderchest", 1);
            self.anger_piglins(true);
            // EnderChestBlockEntity.startOpen: the first opener's CONTAINER_OPEN
            // (ContainerOpenersCounter.incrementOpeners), as for chests below.
            if self.container_opener_event(key, true) {
                self.post_block_game_event(
                    "minecraft:container_open",
                    [key.0, key.1, key.2],
                    Some(horizon_client::mobs::Who::Player),
                    None,
                );
                self.container_edge_sound(key, true);
            }
        } else if matches!(&*name, "minecraft:beacon" | "minecraft:enchanting_table")
            && self.open_station(pos, &name)
        {
            // Station menus whose block has a block entity, not a container.
            self.world.block_entities.get_or_insert_with(key, || {
                BlockEntity::for_block(&name).unwrap_or(BlockEntity::Chest(Vec::new()))
            });
        } else if storage::is_shulker_box(&name) && !self.shulker_can_open(pos) {
            // ShulkerBoxBlock.canOpen: the lid has no room; nothing opens.
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.get_or_insert_with(key, || entity);
            horizon_client::spans::time("open_loot", || self.unpack_opened_loot(key, &name));
            let centre = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            horizon_client::spans::time("open_maps", || {
                self.resolve_container_maps(key, centre);
            });
            self.container = Some(key);
            self.open_container_slots = Some((key, self.open_container_size()));
            let first = self.container_opener_event(key, true);
            if first {
                self.post_block_game_event(
                    "minecraft:container_open",
                    [key.0, key.1, key.2],
                    Some(horizon_client::mobs::Who::Player),
                    None,
                );
            }
            let stat = match &*name {
                "minecraft:chest" => "open_chest",
                "minecraft:trapped_chest" => "trigger_trapped_chest",
                "minecraft:barrel" => "open_barrel",
                "minecraft:hopper" => "inspect_hopper",
                "minecraft:dropper" => "inspect_dropper",
                "minecraft:dispenser" => "inspect_dispenser",
                n if storage::is_shulker_box(n) => "open_shulker_box",
                _ => "",
            };
            if !stat.is_empty() {
                self.award_custom_stat(stat, 1);
            }
            // Opening a piglin-guarded container angers the piglins who see it.
            if self.guarded_by_piglins(&name) {
                self.anger_piglins(true);
            }
            if first {
                self.container_edge_sound(key, true);
            }
        } else if self.open_mod_menu(key) {
            // A mod block's container (`native_block_entity_container`).
            self.container = Some(key);
        } else if self.open_station(pos, &name) {
            // Workstation menus: game/blocks/stations.rs.
        } else {
            self.message = Some((
                format!(
                    "{} is not available yet",
                    name.trim_start_matches("minecraft:").replace('_', " ")
                ),
                Instant::now(),
            ));
            return;
        }
        self.set_screen(Screen::Inventory);
        self.capture(false);
        self.right = false;
    }
    /// FallingBlockEntity.tick: fall with 0.04 gravity and 0.98F drag, then land.
    /// One falling block's `tick`: places or drops its block in its own tick,
    /// so a block falling onto one that just landed lands on top of it. `i` is
    /// its place in the list; the caller has checked it is ticking.
    /// `entities` are the boxes that stop its move.
    fn tick_one_falling(&mut self, i: usize, entities: &[Aabb]) -> Result<()> {
        let entity_drops = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::ENTITY_DROPS);
        let mut f = self.falling.extract(self.falling[i].join).unwrap();
        let original = f.clone();
        match self.fall_step(&mut f, entity_drops, entities) {
            Ok(FallStep::Falling) => self.falling.put_back(i.min(self.falling.len()), f),
            Ok(FallStep::Gone) => {}
            Ok(FallStep::Dropped(at)) => self.drop_falling_block(f.state, at),
            Ok(FallStep::Landed(pos)) => self.finish_falling_block(f, pos),
            Err(error) => {
                horizon_client::join_seq::store_back(&mut f, original);
                self.falling.put_back(i.min(self.falling.len()), f);
                return Err(error);
            }
        }
        Ok(())
    }
    /// One `FallingBlockEntity.tick` move of `f`.
    fn fall_step(
        &mut self,
        f: &mut FallingBlock,
        entity_drops: bool,
        entities: &[Aabb],
    ) -> Result<FallStep> {
        if self.res.facts().is_air(f.state) {
            return Ok(FallStep::Gone);
        }
        f.previous = f.position;
        f.age = f.age.wrapping_add(1);
        f.velocity.y -= horizon_client::entity_rules::EntityRules::get_gravity(f);
        let mut wanted = f.velocity;
        // Entity.move: a cobweb it was in scales this move, which then
        // starts from rest.
        if f.stuck.length_squared() > 1.0e-7 {
            wanted = wanted.multiply(f.stuck);
            f.stuck = Vec3::ZERO;
            f.velocity = Vec3::ZERO;
        }
        let scene = collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: Some(collisions::PowderSnowSubject {
                fall_distance: f.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        };
        // Entity.isDescending: only sneaking entities descend through
        // scaffolding. Falling blocks do not sneak, so descending is always
        // false (Entity.isDescending returns isShiftKeyDown, fixes #7553).
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                kind: horizon_world::collision_context::EntityContextKind::Other,
                y: f.position.y,
                descending: false,
                main_hand: None,
            },
            false,
        );
        // On an error the caller keeps the block as it was before this tick.
        // Entity.collide: the world border's walls and the entities that
        // `canBeCollidedWith` it (a boat, a shulker) stop it too.
        let movement = collisions::EntityCollisions {
            blocks: scene,
            entities,
            border: Some((self.border.geometry(), [f.position.x, f.position.z])),
        }
        .resolve_with_context(
            horizon_client::falling_block::bounds(f.position),
            wanted,
            0.0,
            false,
            context,
        )?;
        let next = Vec3::new(
            f.position.x + movement.x,
            f.position.y + movement.y,
            f.position.z + movement.z,
        );
        let on_ground = wanted.y < 0.0 && movement.y != wanted.y;
        f.on_ground = on_ground;
        if movement.y < 0.0 {
            f.fall_distance -= f64::from(movement.y as f32);
        }
        let from = f.position;
        f.position = next;
        // Entity.move stops it on the axes it collided on.
        if movement.x != wanted.x {
            f.velocity.x = 0.0;
        }
        if movement.y != wanted.y {
            f.velocity.y = 0.0;
        }
        if movement.z != wanted.z {
            f.velocity.z = 0.0;
        }
        if on_ground {
            self.falling_block_impact(f);
            f.fall_distance = 0.0;
        }
        // applyEffectsFromBlocks: WebBlock.entityInside makes it stuck
        // (Entity.makeStuckInBlock), BubbleColumnBlock.entityInside lifts or
        // drags it. Entity.move records the move unless it was blocked short
        // of a tiny one.
        let moved = movement.length_squared();
        let recorded = moved > 1.0e-7 || wanted.length_squared() - moved < 1.0e-7;
        let world = &self.world;
        let facts = self.res.facts();
        let (registry, collision) = (self.pack.assets.registry, self.collision);
        let mut tops = Vec::new();
        let stuck = horizon_client::falling_block::apply_effects_from_blocks(
            from,
            f.position,
            recorded.then_some(wanted),
            &mut f.velocity,
            &mut f.fall_distance,
            &mut tops,
            |p| {
                use horizon_client::falling_block::Inside;
                if &*facts.get(world.block(p.x, p.y, p.z)).name == "minecraft:cobweb" {
                    Inside::Cobweb
                } else if let Some(column) =
                    horizon_client::motion::bubble_column_at(world, registry, facts, collision, p)
                {
                    Inside::BubbleColumn(column)
                } else {
                    Inside::Nothing
                }
            },
        );
        if let Some(multiplier) = stuck {
            f.stuck = multiplier;
        }
        for pos in tops {
            self.bubble_column_particles(pos);
        }
        self.entity_powder_snow_particles(
            from,
            f.position,
            horizon_client::falling_block::bounds(f.position),
            f.velocity,
            false,
        );
        // FallingBlockEntity.tick handles its portal after the move's block
        // effects, before placing/dropping on the floor under an End portal.
        let teleported = self.falling_through_portal(f);
        // FallingBlockEntity.forceTickAfterTeleportToDuplicate: crossing
        // either way through the End deliberately keeps the source's final
        // placement/drop branch, even though the source entity was removed.
        if let Some(target) = teleported
            && self.dim != Dim::End
            && target != Dim::End
        {
            return Ok(FallStep::Gone);
        }
        let y = next.y.floor() as i32;
        let outside = y <= self.world.bottom_y() || y >= self.world.top_y();
        if !on_ground && (f.age > 100 && outside || f.age > 600) {
            return Ok(if f.drop_item && entity_drops {
                FallStep::Dropped(next)
            } else {
                FallStep::Gone
            });
        }
        if on_ground {
            return Ok(FallStep::Landed([
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            ]));
        }
        // Entity.scale(getAirDrag()): 0.98F, widened before it scales motion.
        f.velocity = horizon_client::entity_rules::scale_air_drag(f.velocity);
        Ok(if teleported.is_some() {
            FallStep::Gone
        } else {
            FallStep::Falling
        })
    }
    /// `Entity.spawnAtLocation`: an item entity where the block was.
    fn drop_falling_block(&mut self, state: BlockStateId, at: Vec3) {
        // Entity.spawnAtLocation: an item entity where the block was.
        let item = self.res.facts().get(state).item.clone();
        if &*item != "minecraft:air" {
            // new ItemEntity(level, x, y, z, stack): the motion is from the
            // item's own random, not the level's.
            let stack = ItemStack {
                item,
                count: 1,
                damage: 0,
                extra: None,
            };
            self.spawn_item_own(stack, [at.x, at.y, at.z], 10);
        }
    }
    /// `DecoratedPotBlock.playerWillDestroy`: a pot mined with an item in
    /// `breaks_decorated_pots`, unless an enchantment prevents shattering, is
    /// cracked first (flags 260), so it shatters into its sherds.
    fn crack_mined_pot(&mut self, pos: [i32; 3], state: BlockStateId) -> BlockStateId {
        let registry = &self.pack.assets.registry;
        let Some(mut pot) = registry
            .state(state)
            .filter(|s| s.name == "minecraft:decorated_pot")
            .cloned()
        else {
            return state;
        };
        let Some(held) = self.held() else {
            return state;
        };
        let breaks = self
            .res
            .items()
            .item_tags
            .get("minecraft:breaks_decorated_pots")
            .is_some_and(|t| t.contains(&*held.item));
        let enchantments = &self.stations().enchantments;
        // `EnchantmentHelper.hasTag`.
        let kept = held
            .all_enchantments()
            .iter()
            .any(|(e, _)| enchantments.in_tag("minecraft:prevents_decorated_pot_shattering", e));
        if !breaks || kept {
            return state;
        }
        pot.properties.insert("cracked".into(), "true".into());
        let Ok(cracked) = registry.resolve(&pot) else {
            return state;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
        cracked
    }
    /// `ItemStack.mineBlock` for the held stack once a survival player has
    /// mined the block `name`: an item with a `tool` component counts as used
    /// and loses its `damage_per_block` on a block that doesn't break
    /// instantly (`ShearsItem`: on any block but fire).
    fn mine_block_with_held(&mut self, name: &str, hardness: f32) {
        let Some(stack) = self.inventory[self.selected].clone() else {
            return;
        };
        let Some(per_block) = self.items().mining_damage(&stack) else {
            return;
        };
        let max = self.items().mining_damage_capacity(&stack);
        let wears = if &*stack.item == "minecraft:shears" {
            !self.items().block_in_tag("minecraft:fire", name)
        } else {
            hardness != 0.0
        };
        if wears && per_block > 0 && max > 0 {
            self.damage_held(per_block);
        }
        self.award_stat("used", &stack.item, 1);
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.player_break_block(pos).map(|_| ())
    }
    /// `ServerPlayerGameMode.destroyBlock`: whether the block was removed.
    /// A break inside another (a mod breaking more blocks from its
    /// `native_destroy_block`) keeps the outer break's own events.
    pub(super) fn player_break_block(&mut self, pos: [i32; 3]) -> Result<bool> {
        let outer = self.lan_break_pos.replace(pos);
        let broken = self.with_player_actor(|game| game.break_block_as_player(pos));
        self.lan_break_pos = outer;
        broken
    }
    fn break_block_as_player(&mut self, pos: [i32; 3]) -> Result<bool> {
        // DebugStickItem.canDestroyBlock: a left click selects a property and
        // never destroys the block.
        if !self.debug_stick_can_destroy(pos)? {
            return Ok(false);
        }
        // ServerPlayerGameMode.destroyBlock.
        if self.block_action_restricted(pos) {
            return Ok(false);
        }
        if !self
            .res
            .items()
            .can_destroy_block(self.held(), self.creative)
        {
            return Ok(false);
        }
        // ServerPlayerGameMode.destroyBlock (and MultiPlayerGameMode.destroyBlock
        // on the client): a GameMasterBlock stays when the player cannot use
        // game master blocks (Player.canUseGameMasterBlocks).
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !self.can_use_game_master_blocks()
            && is_game_master_block(&self.res.facts().get(state).name)
        {
            return Ok(false);
        }
        // Native mods, once the game allowed the break: a canceled one keeps
        // the block, and a guest's acknowledgement puts it back in its world.
        let mut canceled = false;
        let mut exp_to_drop = None;
        self.native_break_block(pos, state, &mut canceled, &mut exp_to_drop);
        if canceled {
            return Ok(false);
        }
        self.pad_rumble(horizon_client::gamepad::Rumble::BlockBroken);
        let broken = state;
        let facts = self.res.facts().get(state).clone();
        let state = self.crack_mined_pot(pos, state);
        // Block.playerWillDestroy: mining their gold angers all nearby piglins.
        if self.guarded_by_piglins(&facts.name) {
            self.anger_piglins(false);
        }
        // The block entity as it was, for what the drops copy from it.
        let key = (pos[0], pos[1], pos[2]);
        let loot_entity = if self.creative {
            None
        } else {
            self.loot_block_entity(pos, &facts.name)
        };
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let name = self.world.block_names.get(&key).cloned();
            self.pending.record(pos, &entity, &facts.name, name);
        } else if let Some(tag) = spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        self.creaking_heart_broken(pos, true);
        let hive = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Beehive(h)) => self.hive_state(pos).map(|s| ((**h).clone(), s)),
            _ => None,
        };
        // playerWillDestroy and Level.removeBlock through the vanilla behaviour
        // library: the block entity's side effects (containers spill), neighbour
        // and shape updates, and the destroy event (particles, sound).
        let creative = self.creative;
        // TripWireBlock.playerWillDestroy: shears disarm the string first (flags
        // 260), so taking it down does not trip the hook.
        if &*facts.name == "minecraft:tripwire"
            && self.held().is_some_and(|s| &*s.item == "minecraft:shears")
            && let Some(disarmed) = self.with_props(state, &[("disarmed", "true")])
        {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, disarmed, 260));
            self.post_block_game_event("minecraft:shear", pos, self.sculk_actor(), None);
        }
        if creative && storage::is_shulker_box(&facts.name) {
            self.drop_creative_shulker(pos);
        }
        // BeehiveBlock.playerWillDestroy: a creative break still drops the hive
        // with its bees and honey level.
        if creative
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
            && let Some((h, (honey, _))) = &hive
            && (!h.bees.is_empty() || honey.is_some_and(|l| l > 0))
            && let Some(stack) = self.picked_stack(pos, true)
        {
            let at = pos.map(f64::from);
            self.spawn_item_own(stack, at, 10);
        }
        let mut correct_tool =
            !facts.requires_tool || self.res.items().correct_for_drops(self.held(), &facts.name);
        if !creative
            && self
                .res
                .behaviours()
                .data(state)
                .is(horizon_world::level::classes::Class::Door)
            && mod_api::__private::listened(mod_api::HookId::PlayerHarvestCheck)
        {
            self.native_player_harvest_check(
                self.own_entity_id() as u32,
                pos,
                state.0,
                &mut correct_tool,
            );
        }
        let source = self
            .lan_acting_uuid
            .map(horizon_world::level::LevelEventPlayer);
        let removed = self.with_level(|b, level, ticks| {
            b.player_destroy_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                creative,
                correct_tool,
                source,
            )
        });
        if !creative {
            let held = self.held().cloned();
            let mut harvest = !facts.requires_tool
                || self
                    .res
                    .items()
                    .correct_for_drops(held.as_ref(), &facts.name);
            if mod_api::__private::listened(mod_api::HookId::PlayerHarvestCheck) {
                self.native_player_harvest_check(
                    self.own_entity_id() as u32,
                    pos,
                    state.0,
                    &mut harvest,
                );
            }
            if removed {
                self.mine_block_with_held(&facts.name, facts.hardness);
            }
            if removed && harvest {
                // Block.playerDestroy: the mined statistic, the exhaustion and the drops.
                self.award_stat("mined", &facts.name, 1);
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(0.005);
                }
                let stacks = self.block_loot_at(pos, state, held.as_ref(), loot_entity);
                let silk = held
                    .as_ref()
                    .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
                // Block.dropResources: the item entities, then spawnAfterBreak's
                // experience, with the mods' say between the two.
                let breaker = Some(self.native_acting_id());
                let dropped = self.with_level(|_, level, _| {
                    level.block_dropped(
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        state,
                        breaker,
                        held.as_ref(),
                        stacks,
                        exp_to_drop,
                    )
                });
                let canceled = dropped.canceled;
                if let Some(kept) = self.break_collect.as_mut() {
                    kept.extend(dropped.items.into_iter().map(|item| item.stack));
                    if dropped.experience > 0 {
                        self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), dropped.experience);
                    }
                } else {
                    self.spawn_block_drops(pos, dropped);
                }
                // BeehiveBlock.playerDestroy: silk touch keeps the bees in, so
                // the trigger counts them; otherwise they are out already.
                if let Some((hive, (honey, facing))) = hive {
                    let bees = if silk { hive.bees.len() as i64 } else { 0 };
                    if !silk {
                        self.hive_broken_by_player(pos, hive, honey, facing);
                    }
                    self.trigger_bee_nest_destroyed(broken, held.as_ref(), bees);
                }
                // InfestedBlock.spawnAfterBreak: its silverfish comes out unless the
                // tool has silk touch (#prevents_infested_spawns) or blocks don't drop.
                if facts.name.starts_with("minecraft:infested_")
                    && !silk
                    && !canceled
                    && self
                        .ui
                        .rules
                        .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
                {
                    let at = [
                        f64::from(pos[0]) + 0.5,
                        f64::from(pos[1]),
                        f64::from(pos[2]) + 0.5,
                    ];
                    if let Some(id) = self.spawn_mob(
                        "minecraft:silverfish",
                        at,
                        false,
                        creatures::SpawnSite::Plain,
                    ) {
                        self.mob_poof(id);
                    }
                }
                // TurtleEggBlock.playerDestroy: after the drops, one egg of a
                // mined stack is taken and the rest stay.
                self.with_level(|b, level, ticks| {
                    b.turtle_egg_destroyed_by_player(
                        level,
                        ticks,
                        BlockPos::new(pos[0], pos[1], pos[2]),
                        broken,
                    );
                });
            }
            // IceBlock.playerDestroy: the ice melts back into water (the
            // #prevents_ice_melting tool tag holds only silk touch).
            let prevents_melting = held
                .as_ref()
                .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
            self.with_level(|b, level, ticks| {
                b.ice_destroyed_by_player(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    broken,
                    prevents_melting,
                );
            });
        }
        self.pending.clear(pos);
        self.process_level_events()?;
        Ok(removed)
    }
    /// The `ItemStack.interactLivingEntity` fallback in `Player.interactOn`,
    /// currently for native Mob entries after the entity's own action passed.
    fn item_interact_mob(&mut self, target: Option<InteractTarget>) -> bool {
        if self.spectator() {
            return false;
        }
        let Some(index) = self.mob_interaction_target(target) else {
            return false;
        };
        let mob = &self.mobs[index];
        if mob.removed || !mob.alive() {
            return false;
        }
        let (entity, kind) = (
            mob.id as i32,
            horizon_client::command::args::normalize(&mob.kind.id),
        );
        let Some(stack) = self.held().filter(|stack| stack.count > 0).cloned() else {
            return false;
        };
        let mut swing = None;
        self.native_item_interact_mob(entity, &kind, &stack, &mut swing);
        if swing == Some(true) {
            self.swing_arm();
        }
        swing.is_some()
    }
    /// `ItemStack.applyAfterUseComponentSideEffects`: a used stack with a
    /// `use_cooldown` component starts its entry (`UseCooldown.apply`). A
    /// server's guest also gets `ClientboundCooldownPacket`
    /// (`ServerItemCooldowns.onCooldownStarted`), queued for their turn's end.
    pub(super) fn apply_use_cooldown(&mut self, stack: &ItemStack) {
        let default = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.use_cooldown.clone());
        let Some(component) = horizon_client::cooldowns::component(stack, default.as_ref()) else {
            return;
        };
        let group = horizon_client::cooldowns::group(stack, default.as_ref());
        let mut cancel = false;
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_player_item_cooldown(
                player,
                &name,
                &stack.item,
                component.ticks(),
                &mut cancel,
            );
        }
        if !cancel {
            self.add_cooldown(group, component.ticks());
        } else if self.lan_acting.is_some() {
            // The guest's client started the cooldown itself; 0 ends it there.
            self.guest_cooldowns.push((group, 0));
        }
    }
    /// `ItemCooldowns.addCooldown` (a guest also gets the packet).
    pub(super) fn add_cooldown(&mut self, group: Arc<str>, ticks: i32) {
        self.cooldowns.add(Arc::clone(&group), ticks);
        if self.lan_acting.is_some() {
            self.guest_cooldowns.push((group, ticks));
        }
    }
    /// One hand's turn in `startUseItem`: `useItemOn` the block aimed at,
    /// then `useItem`. Whether the click ends here (it acted or failed).
    fn use_hand(&mut self, block_hit: Option<&Hit>) -> Result<bool> {
        self.use_failed = false;
        // The click's target as it was decided, before the adventure gate
        // below may drop the block for the item's own use.
        let aimed_at_block = block_hit.is_some();
        // isSecondaryUseActive with something in either hand keeps blocks
        // from reacting.
        let sneaking = self.keys.held(Mapping::Sneak);
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        let mut anchor_passed = false;
        // Native mods, where ServerPlayerGameMode.useItemOn starts
        // (RightClickBlock): a cancel answers for the block and the item, and
        // its PASS goes on to Item.use, which a guest's UseItem asks for.
        // A slow tick's line names the phase of a use that took it (#8669): the
        // mods' hook, the block's own use, then the item's.
        use mod_api::InteractionResult as Answer;
        use mod_api::TriState;
        let on_block = matches!(self.guest_use, Some(GuestUse::OnBlock(_)));
        let mut block_hit = block_hit;
        let (mut use_block, mut use_item) = (TriState::Default, TriState::Default);
        let block_phase = horizon_client::spans::guard("use_block");
        if let Some(hit) = block_hit {
            let held = self.held().cloned();
            let answer = horizon_client::spans::time("use_mods", || {
                self.right_click_block(hit, self.hand, held.as_ref())
            });
            match answer {
                native_hooks::BlockClickAnswer::Go {
                    use_block: block,
                    use_item: item,
                } => (use_block, use_item) = (block, item),
                native_hooks::BlockClickAnswer::Canceled(answer)
                    if answer.consumes_action() || answer == Answer::Fail =>
                {
                    if matches!(answer, Answer::Success | Answer::SuccessServer) {
                        self.swing_arm();
                    }
                    self.use_failed = answer == Answer::Fail;
                    return Ok(true);
                }
                native_hooks::BlockClickAnswer::Canceled(_) => {
                    (use_block, use_item) = (TriState::False, TriState::False);
                }
            }
        }
        // ServerPlayerGameMode.useItemOn: the held item's onItemUseFirst, when
        // RightClickBlock didn't deny the item, ahead of the sneak gate and
        // every block behaviour. Its answer ends the click for this hand.
        if use_item.or(true)
            && let Some(hit) = block_hit
            && let Some(held) = self.held().cloned()
            && let Some(answer) = self.item_use_first(hit, self.hand, &held)
        {
            if matches!(answer, Answer::Success | Answer::SuccessServer) {
                self.swing_arm();
            }
            self.use_failed = answer == Answer::Fail;
            return Ok(answer.consumes_action() || answer == Answer::Fail);
        }
        // ServerPlayerGameMode.useItemOn: after the spectator and sneak gates,
        // BlockState.useItemOn runs before every block-specific behaviour.
        if use_block.or(!(sneaking && hands_full))
            && let Some(hit) = block_hit
        {
            let held = self.held().cloned();
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let mut block_answer =
                self.use_item_on_block(hit, self.hand, held.as_ref(), mod_api::UsePhase::Block);
            if block_answer.is_none()
                && mod_api::__private::has_part_hook(mod_api::HookId::UseItemOn)
                && api_values::block_state::block_listens(mod_api::HookId::UseItemOn, state)
                && let Ok(snapshot) = held.try_into_api()
            {
                let click = self.block_click(hit, self.hand, snapshot.as_ref());
                self.native_use_item_on(state, &click, &mut block_answer);
            }
            let mut chains_sign = false;
            if block_answer.is_none() {
                if self.use_on_cauldron(hit.pos)? {
                    self.swing_arm();
                    return Ok(true);
                }
                if let Some(placed) = self.use_on_campfire(hit.pos)? {
                    if placed {
                        self.swing_arm();
                    }
                    return Ok(true);
                }
                if self.use_lectern(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_sign_applicator(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_golem_statue(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if let Some(swings) = self.use_shelf(hit) {
                    let used = held.unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                    self.trigger_block("minecraft:item_used_on_block", hit.pos, &used);
                    if swings {
                        self.swing_arm();
                    }
                    return Ok(true);
                }
                if self.use_composter(hit.pos, held.as_ref()) {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_vault(hit.pos, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_pumpkin(hit.pos, hit.face, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_beehive(hit.pos, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                if self.use_campfire_douse(hit, held.as_ref())? {
                    self.swing_arm();
                    return Ok(true);
                }
                block_answer = self.use_redstone_ore(hit.pos, held.as_ref())?;
                match self.use_respawn_anchor(hit.pos, held.as_ref())? {
                    respawn_anchor::AnchorUse::Charged => {
                        self.swing_arm();
                        return Ok(true);
                    }
                    // The main hand passes; its item is used, then the off hand's glowstone.
                    respawn_anchor::AnchorUse::Pass => anchor_passed = true,
                    respawn_anchor::AnchorUse::Other => {}
                }
                // CeilingHangingSignBlock.useItemOn: a hanging sign aimed at the
                // underside passes, so the next sign chains below instead of
                // opening the editor, unless the side has click commands to run
                // (`shouldTryToChainAnotherHangingSign`).
                chains_sign = hit.face == [0, -1, 0]
                // `instanceof HangingSignItem`, independent of the item's name.
                && held.as_ref().is_some_and(|s| {
                    self.res.behaviours().item_binding(&s.item)
                        .is_some_and(|i| i.kind == horizon_world::level::ItemKind::HangingSign)
                })
                && self
                    .res
                    .facts()
                    .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                    .has_class("CeilingHangingSignBlock")
                && matches!(
                    self.world
                        .block_entities
                        .get(&(hit.pos[0], hit.pos[1], hit.pos[2])),
                    Some(BlockEntity::Sign(_))
                )
                && !self.sign_can_execute_click_commands((hit.pos[0], hit.pos[1], hit.pos[2]));
            }
            if block_answer.is_none() {
                use horizon_world::level::UseBranch::ItemOn;
                block_answer = self
                    .use_jukebox(hit.pos, state, held.as_ref(), ItemOn)
                    .or_else(|| self.use_decorated_pot(hit.pos, state, held.as_ref(), ItemOn))
                    .or_else(|| self.use_chiseled_bookshelf(hit, state, held.as_ref(), ItemOn));
            }
            // TRY_WITH_EMPTY_HAND: only the main hand's turn uses the block
            // itself (useWithoutItem). The block's item behaviour hears both hands.
            if !anchor_passed && !chains_sign && (self.hand == Hand::Main || block_answer.is_some())
            {
                let face = match hit.face {
                    [0, 1, 0] => Direction::Up,
                    [0, -1, 0] => Direction::Down,
                    [0, 0, -1] => Direction::North,
                    [0, 0, 1] => Direction::South,
                    [-1, 0, 0] => Direction::West,
                    _ => Direction::East,
                };
                let player = self.use_context();
                let block_hit = horizon_core::geometry::BlockHit {
                    block: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                    direction: face,
                    location: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
                    inside: hit.inside,
                };
                let item = held.as_ref().map(|s| s.item.to_string());
                let adds_candle = self
                    .with_level(|b, _, _| b.cake_candle_state(state, item.as_deref()).is_some());
                let pots_flower = self
                    .with_level(|b, _, _| b.potted_flower_state(state, item.as_deref()).is_some());
                use horizon_world::level::{InteractionResult, UseBranch};
                let item_result = match block_answer {
                    Some(answer) => native_hooks::block_use_result(answer),
                    None => self.with_level(|b, level, ticks| {
                        b.block_use_item_on(level, ticks, state, block_hit, player, item.as_deref())
                    }),
                };
                let mut empty_template_ran = false;
                let (result, branch) = if let Some(result) = item_result {
                    (result, UseBranch::ItemOn)
                } else if self.hand == Hand::Main {
                    let mut empty_answer = None;
                    if mod_api::__private::has_part_hook(mod_api::HookId::UseWithoutItem)
                        && api_values::block_state::block_listens(
                            mod_api::HookId::UseWithoutItem,
                            state,
                        )
                        && let Ok(snapshot) = self.held().try_into_api()
                    {
                        let click = self.block_click(hit, self.hand, snapshot.as_ref());
                        self.native_use_without_item(state, &click, &mut empty_answer);
                    }
                    if empty_answer.is_none()
                        && self
                            .res
                            .behaviours()
                            .data(state)
                            .is(horizon_world::level::classes::Class::Fence)
                        && self.use_knot(hit.pos, false)
                    {
                        self.trigger_default_block_use(hit.pos);
                        return Ok(true);
                    }
                    let answer = empty_answer.or_else(|| {
                        self.use_jukebox(hit.pos, state, None, UseBranch::WithoutItem)
                            .or_else(|| {
                                self.use_decorated_pot(hit.pos, state, None, UseBranch::WithoutItem)
                            })
                            .or_else(|| {
                                self.use_chiseled_bookshelf(
                                    hit,
                                    state,
                                    None,
                                    UseBranch::WithoutItem,
                                )
                            })
                    });
                    let result = if let Some(answer) = answer {
                        native_hooks::block_use_result(answer).unwrap_or(InteractionResult::Pass)
                    } else {
                        empty_template_ran = true;
                        let player = self.use_context();
                        self.with_level(|b, level, ticks| {
                            b.use_without_item(level, ticks, state, block_hit, player)
                        })
                    };
                    (result, UseBranch::WithoutItem)
                } else {
                    (InteractionResult::Pass, UseBranch::ItemOn)
                };
                self.process_level_events()?;
                // CakeBlock.useItemOn: the candle is used up and counted.
                if block_answer.is_none()
                    && adds_candle
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.award_stat("used", &used.item, 1);
                    self.consume_held();
                }
                // FlowerPotBlock.useItemOn: the plant is potted, used up, and counted.
                if block_answer.is_none()
                    && pots_flower
                    && result == horizon_world::level::InteractionResult::Success
                    && let Some(used) = held.as_ref()
                {
                    self.award_stat("used", &used.item, 1);
                    self.award_custom_stat("pot_flower", 1);
                    self.consume_held();
                    if let (Some(player), Some(name)) =
                        (self.lan_acting_uuid, self.lan_acting_name.clone())
                    {
                        self.native_player_flower_pot_manipulate(
                            player, &name, hit.pos, &used.item,
                        );
                    }
                }
                // These effects belong to the vanilla method, not its Part replacement.
                if empty_template_ran && result == horizon_world::level::InteractionResult::Success
                {
                    let name = self
                        .res
                        .facts()
                        .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]))
                        .name
                        .clone();
                    match &*name {
                        "minecraft:note_block" => self.award_custom_stat("tune_noteblock", 1),
                        "minecraft:bell" => self.award_custom_stat("bell_ring", 1),
                        "minecraft:lever" => self.lever_puff(hit.pos),
                        _ => {}
                    }
                }
                if result.consumes_action() {
                    // ServerPlayerGameMode.useItemOn: ITEM_USED_ON_BLOCK with
                    // the stack before the use, or DEFAULT_BLOCK_USE.
                    match branch {
                        horizon_world::level::UseBranch::ItemOn => {
                            let used = held.unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
                            self.trigger_block("minecraft:item_used_on_block", hit.pos, &used);
                        }
                        horizon_world::level::UseBranch::WithoutItem => {
                            self.trigger_default_block_use(hit.pos);
                        }
                    }
                    if result.should_swing() {
                        // SUCCESS swings on the client too; the beds, light,
                        // respawn anchor and sign's own uses are SUCCESS_SERVER.
                        if server_only_block_use(self.res.facts().get(state), branch) {
                            self.swing_arm();
                        } else {
                            self.swing_arm_predicted();
                        }
                    }
                    return Ok(true);
                }
            }
        }
        // RightClickBlock's useItem FALSE: no ItemStack.useOn, so only
        // Item.use is left (a guest's UseItem asks for it).
        if !use_item.or(true) {
            if on_block {
                return Ok(false);
            }
            block_hit = None;
        }
        drop(block_phase);
        let _item_phase = horizon_client::spans::guard("use_item");
        let Some(stack) = self.held().cloned() else {
            return Ok(false);
        };
        let Some(info) = self.items().get(&stack.item).cloned() else {
            return Ok(false);
        };
        // ServerPlayerGameMode.useItem and useItemOn: an item on cooldown is
        // not used (PASS, so the other hand may still act), unless
        // RightClickBlock's useItem TRUE forces a block click's useOn.
        let forced = on_block && use_item == TriState::True;
        if !forced
            && self
                .cooldowns
                .is_on_cooldown(&horizon_client::cooldowns::group(
                    &stack,
                    info.use_cooldown.as_ref(),
                ))
        {
            return Ok(false);
        }
        // ItemStack.useOn: adventure's predicate selects Item.useOn before
        // its event and Part, not just before the eventual block placement.
        if let Some(hit) = block_hit
            && !self.may_build()
            && !self.can_place_on_in_adventure(hit.pos)
        {
            if on_block {
                return Ok(false);
            }
            block_hit = None;
        }
        // Native mods, where ItemStack.useOn and Item.use start: the block
        // aimed at had its turn, and the item is off cooldown. A guest's
        // UseItemOn reaches only useOn, its UseItem only use.
        let mut claimed = false;
        if let Some(hit) = block_hit {
            if let Some(answer) = self.use_item_on_block(
                hit,
                self.hand,
                Some(&stack),
                mod_api::UsePhase::ItemAfterBlock,
            ) {
                if answer.consumes_action() {
                    self.trigger_block("minecraft:item_used_on_block", hit.pos, &stack);
                    if matches!(answer, Answer::Success | Answer::SuccessServer) {
                        self.swing_arm();
                    }
                    return Ok(true);
                }
                if answer == Answer::Fail {
                    self.use_failed = true;
                    return Ok(true);
                }
                if on_block {
                    return Ok(false);
                }
                block_hit = None;
            } else if mod_api::__private::has_part_hook(mod_api::HookId::UseOn)
                && let Ok(snapshot) = stack.try_into_api()
            {
                let (player, player_entity_id) = self.native_clicker();
                let (hand, at) = (self.hand, hit.native());
                self.native_use_on(mod_api::__private::UseOnCall {
                    stack: &snapshot,
                    pos: hit.pos,
                    face: hit.face,
                    hand,
                    hit: at,
                    player,
                    player_entity_id,
                    claimed: &mut claimed,
                });
            }
        }
        if !claimed && !on_block {
            if let Some(answer) = self.right_click_item(self.hand, &stack, aimed_at_block) {
                if matches!(answer, Answer::Success | Answer::SuccessServer) {
                    self.swing_arm();
                }
                return Ok(answer.consumes_action());
            }
            self.native_use_item(&stack, &mut claimed);
        }
        if claimed {
            self.swing_arm();
            return Ok(true);
        }
        // LeadItem.useOn follows the block and the item's override, including
        // when sneaking bypassed the block's useWithoutItem.
        if block_hit.is_some() && &*stack.item == "minecraft:lead" && self.interact_knot() {
            return Ok(true);
        }
        // ItemStack.useOn: after the block's own useItemOn/useWithoutItem
        // passed. DebugStickItem.useOn cycles the selected property.
        if let Some(hit) = block_hit
            && let Some(success) = self.debug_stick_use_on(hit.pos)?
        {
            // `Minecraft.startUseItem`: the stick's client-side `useOn` is
            // SUCCESS, so the actor's hand swings; a remote guest swings in
            // its own client, not on the server.
            if self.lan_acting.is_none() {
                self.swing_arm();
            }
            if success {
                // `ServerPlayerGameMode.useItemOn`: a consuming `useOn`
                // triggers ITEM_USED_ON_BLOCK with the stack before the use.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, &stack);
            } else {
                // DebugStickItem.useOn's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        }
        // TntBlock.useItemOn is the block's own: a sneak with something in hand skips it, and
        // the flint and steel or fire charge lights a fire on the TNT instead.
        if block_hit.is_some()
            && (self.use_bone_meal(&stack)?
                || (!(sneaking && hands_full) && self.ignite_tnt(&stack)?))
        {
            return Ok(true);
        }
        // ShearsItem.useOn: shears crop a growing plant's head to its top age.
        if let Some(hit) = block_hit
            && self.shears_crop(hit.pos, &stack)?
        {
            return Ok(true);
        }
        // A guest's UseItemOn stops at ItemStack.useOn: what Item.use does
        // (throw, drink, eat, raise, equip) waits for its UseItem.
        let spawn_egg = stack.item.ends_with("_spawn_egg");
        use horizon_client::first_person::UseResult;
        let swings = self.swings_started;
        match self.use_tool(&stack, block_hit.is_some())? {
            UseResult::Success { swing } => {
                if (UseResult::Success { swing }).should_swing(swings, self.swings_started) {
                    self.swing_arm();
                }
                return Ok(true);
            }
            UseResult::Fail => {
                self.use_failed = true;
                return Ok(true);
            }
            UseResult::Pass => {}
        }
        // PotionItem.useOn comes before PotionItem.use's drink.
        if let Some(hit) = block_hit
            && self.use_water_bottle_on_block(hit.pos, hit.face, &stack)?
        {
            return Ok(true);
        }
        // a mod's item used over time: its Item.use, once a block item's
        // useOn had its turn.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_mod_use(&stack)
        {
            return Ok(true);
        }
        if (!on_block && self.use_potion_item(&stack)?)
            || ((!on_block || (spawn_egg && block_hit.is_some())) && self.throw_item(&stack)?)
        {
            return Ok(true);
        }
        // BrushItem.useOn: brushing starts on the block aimed at (CONSUME).
        if block_hit.is_some() && &*stack.item == "minecraft:brush" {
            self.start_using_item();
            return Ok(true);
        }
        if !on_block
            && (self.raise_shield(self.held_slot())
                || self.use_fishing_rod(&stack)
            || self.use_crossbow(&stack)
            || self.use_trident(&stack)
            || self.use_spear(&stack)
            || self.boost_firework(&stack)
            || self.use_empty_map(&stack)
                || self.use_book(&stack)
                // KnowledgeBookItem.use: the recipes it names become known.
                || self.use_knowledge_book(&stack)
                // Block items (heads, carved pumpkins) aimed at a block are
                // placed first (useItemOn before useItem).
                || (!(info.block.is_some() && block_hit.is_some())
                    && self.equip_from_hand(&stack)))
        {
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:spyglass" {
            self.use_spyglass();
            return Ok(true);
        }
        // FoodOnAStickItem.use with no boost (not riding its mount, or the
        // mount refused): PASS, but counted as a use of the stick.
        if !on_block
            && matches!(
                &*stack.item,
                "minecraft:carrot_on_a_stick" | "minecraft:warped_fungus_on_a_stick"
            )
        {
            self.award_stat("used", &stack.item, 1);
        }
        // BundleItem.use: startUsingItem, then onUseTick empties it.
        if !on_block && info.bundle {
            self.start_using_item();
            self.swing_arm();
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:goat_horn" {
            self.use_instrument(&stack);
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:bow" {
            // BowItem.use: ArrowNockEvent, then FAIL without ammunition.
            let found_projectile = self.get_projectile(&stack).is_some();
            let has_ammo = found_projectile || self.creative;
            let player = horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid());
            let name = self.acting_player_name().to_owned();
            let (hand, mut cancel) = (self.hand, false);
            self.native_arrow_nock(player, &name, &stack, hand, has_ammo, &mut cancel);
            if has_ammo && !cancel {
                self.start_using_item();
            }
            return Ok(true);
        }
        // Block foods attempt placement before starting consumption.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_eating(&info) {
            return Ok(true);
        }
        // BoatItem.use also performs its own fluid/block raycast.
        if !on_block && self.place_boat(&stack) {
            return Ok(true);
        }
        // ItemStack.useOn passes in adventure mode unless the item's
        // can_place_on matches the clicked block; Item.use then runs as usual.
        if let Some(hit) = block_hit
            && (self.may_build() || self.can_place_on_in_adventure(hit.pos))
            && (self.launch_firework(hit, &stack)
                || self.place_minecart(hit, &stack)
                || self.place_painting(hit, &stack)
                || self.place_item_frame(hit, &stack)
                || self.use_lodestone_compass(hit, &stack)
                || self.use_map_on_banner(hit, &stack)
                || self.place_armor_stand(hit, &stack)
                || self.place_cushion(hit, &stack)
                || self.place_end_crystal(hit, &stack))
        {
            return Ok(true);
        }
        if self.use_ender_eye(&stack, block_hit.is_some())? {
            return Ok(true);
        }
        // PlaceOnWaterBlockItem.use: the lily pad and frogspawn aim at the block
        // or water source the SOURCE_ONLY ray meets and are placed on the
        // position above it.
        let water_placement = matches!(&*stack.item, "minecraft:lily_pad" | "minecraft:frogspawn");
        let water_hit;
        let hit = if water_placement {
            let Some(h) = self.pick_fluid() else {
                return Ok(false);
            };
            water_hit = Hit {
                pos: [h.pos[0], h.pos[1] + 1, h.pos[2]],
                ..h
            };
            &water_hit
        } else {
            let Some(hit) = block_hit else {
                return Ok(false);
            };
            hit
        };
        if info.block.is_none() {
            return Ok(false);
        }
        if water_placement && !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
            // The fluid-ray placement has its own target, selected by Item.use.
            return Ok(!on_block && self.start_eating(&info));
        }
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        // BlockItem.place through the vanilla placement rules.
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face,
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: false,
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.keys.held(Mapping::Sneak),
            item: stack.item.to_string(),
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        let outcome = if self.player_place_canceled(&ctx) {
            PlaceOutcome::Failed
        } else {
            self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx))
        };
        let PlaceOutcome::Placed { pos, state } = outcome else {
            self.process_level_events()?;
            // BlockItem.useOn: a consumable that could not be placed is used,
            // which ItemStack.useOn counts as a use of the item.
            if self.start_eating(&info) {
                self.award_stat("used", &stack.item, 1);
            } else {
                // BlockItem.place's FAIL.
                self.use_failed = true;
            }
            return Ok(true);
        };
        let target = [pos.x, pos.y, pos.z];
        let state = self.apply_item_block_state(target, state, &stack)?;
        let key = (target[0], target[1], target[2]);
        if !self.world.block_entities.stateful_entity(state.0)
            && let Some(entity) = self.new_block_entity_for_state(state)
        {
            self.world.block_entities.insert(key, entity);
        }
        if self.world.block_entities.get(&key).is_some() {
            let game_master = self.can_use_game_master_blocks();
            self.apply_placed_item(target, &stack, game_master);
            // Block.setPlacedBy.
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => {
                    let data = stack.data().is_some_and(|d| {
                        d.other(horizon_client::block_entity_item::BLOCK_ENTITY_DATA)
                            .is_some()
                    });
                    self.command_block_placed(key, !data);
                }
                Some(BlockEntity::StructureBlock(_)) => self.structure_block_placed(key),
                Some(BlockEntity::JigsawBlock(_)) => self.jigsaw_block_placed(key),
                Some(BlockEntity::TestBlock(_)) => self.test_block_placed(key),
                Some(BlockEntity::Jukebox(_)) => self.jukebox_placed(target, &stack),
                Some(BlockEntity::Sign(sign)) if !sign.waxed => self.open_sign_editor(key, true),
                _ => {}
            }
        }
        // BlockItem.place's SUCCESS: the client swings with its own prediction.
        self.swing_arm_predicted();
        // BlockItem.place's sound is among the level events;
        // SolidBucketItem.getPlaceSound replaces it with the bucket's own.
        let solid_bucket = &*stack.item == "minecraft:powder_snow_bucket";
        if solid_bucket {
            for event in &mut self.level_events {
                if let LevelEvent::Sound { pos: at, sound, .. } = event
                    && *at == pos
                {
                    *sound = "minecraft:item.bucket.empty_powder_snow".into();
                }
            }
        }
        self.process_level_events()?;
        // BlockItem.place: the used statistic and the placed_block trigger.
        self.award_stat("used", &stack.item, 1);
        self.trigger_block("minecraft:placed_block", target, &stack);
        self.check_wither_spawn(target)?;
        self.check_golem_spawn(target)?;
        self.consume_held();
        // SolidBucketItem.useOn: BucketItem.getEmptySuccessItem in hand.
        if solid_bucket && !self.creative {
            *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
        }
        // ItemStack.useOn applies the pre-use stack's component side effects
        // after the successful placement and its hand replacement.
        self.apply_use_cooldown(&stack);
        Ok(true)
    }
    fn apply_consume_effect(
        &mut self,
        consumed: &horizon_client::items::ItemStack,
        effect: &horizon_client::items::ConsumeEffect,
        mob: Option<mod_api::LivingRef>,
        allow_native: bool,
    ) -> Option<bool> {
        use horizon_client::items::ConsumeEffect;
        Some(match effect {
            ConsumeEffect::ApplyEffects {
                effects,
                probability,
            } => {
                // ApplyStatusEffectsConsumeEffect draws from the user's random,
                // even at probability 1. The level random stands in for a player's.
                let draw = match mob {
                    Some(living) => {
                        let commands::Target::Mob(id) = self.native_living_target(living)? else {
                            return None;
                        };
                        self.mobs
                            .iter_mut()
                            .find(|m| m.id == id)?
                            .random
                            .next_float()
                    }
                    None => self.world.random.next_float(),
                };
                if draw >= *probability {
                    return Some(false);
                }
                let mut changed = false;
                for eff in effects {
                    let Some(def) = horizon_client::brewing::effect(&eff.id) else {
                        continue;
                    };
                    if let Some(living) = mob {
                        let effect = mod_api::ActiveEffect::new(
                            mod_api::__private::effect_key_by_index(def.index)
                                .expect("effects resolved at load"),
                            eff.duration,
                            eff.amplifier,
                            eff.ambient,
                            eff.show_particles,
                            eff.show_icon,
                        );
                        changed |= self.living_add_effect(living, &effect);
                    } else {
                        let mut instance = horizon_client::effects::EffectInstance::new(
                            def.id,
                            eff.duration,
                            eff.amplifier,
                        );
                        instance.ambient = eff.ambient;
                        instance.visible = eff.show_particles;
                        instance.show_icon = eff.show_icon;
                        changed |= self.add_player_effect(instance);
                    }
                }
                changed
            }
            ConsumeEffect::ClearAllEffects => match mob {
                Some(living) => self.living_remove_all_effects(living),
                None => self.remove_player_effects(None),
            },
            ConsumeEffect::RemoveEffects(effects) => {
                let mut changed = false;
                for effect in effects {
                    changed |= match mob {
                        Some(living) => horizon_client::brewing::effect(effect)
                            .and_then(|def| mod_api::__private::effect_key_by_index(def.index))
                            .is_some_and(|key| self.living_remove_effect(living, key)),
                        None => horizon_client::brewing::effect(effect)
                            .is_some_and(|def| self.remove_player_effects(Some(def.index))),
                    };
                }
                changed
            }
            ConsumeEffect::TeleportRandomly(teleport) => {
                if mob.is_some() {
                    return None;
                }
                self.teleport_randomly(teleport, consumed)
            }
            ConsumeEffect::PlaySound(sound) => {
                let at = match mob {
                    Some(living) => {
                        let commands::Target::Mob(id) = self.native_living_target(living)? else {
                            return None;
                        };
                        self.mobs.iter().find(|m| m.id == id).map(|m| m.position)?
                    }
                    None => self.player.as_ref()?.position,
                };
                let b = horizon_core::position::BlockPos::containing(at);
                let at = [b.x, b.y, b.z].map(|v| f64::from(v) + 0.5);
                let level = self.level(self.dim.id())?;
                let source = match mob {
                    Some(living) => self.entity_sound_source(living.entity())?,
                    None => mod_api::SoundSource::Players,
                };
                self.native_queue_sound(level, sound, at, source, 1.0, 1.0, None)?;
                true
            }
            ConsumeEffect::Native { kind, data } => {
                if !allow_native {
                    return None;
                }
                self.native_consume_effect(kind, data);
                return None;
            }
        })
    }
    /// `BundleItem.dropContent`: `removeOneItemFromBundle` and `Player.drop`,
    /// then the drop sound and the used statistic.
    fn drop_bundle_content(&mut self, slot: usize) {
        let items = self.items();
        let Some(mut bundle) = self.stack_in(slot).cloned() else {
            return;
        };
        let Some(mut contents) = horizon_client::bundle::Contents::of(items, &bundle) else {
            return;
        };
        let Some(removed) = contents.remove_one(items) else {
            return;
        };
        contents.store(&mut bundle);
        let item = Arc::clone(&bundle.item);
        *self.stack_in_mut(slot) = Some(bundle);
        let Some(position) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let at = [position.x, position.y, position.z];
        // playRemoveOneSound, then Player.drop and playDropContentsSound at
        // the player's block.
        let pitch = 0.8 + self.world.random.next_float() * 0.4;
        self.audio
            .play(horizon_client::bundle::REMOVE_ONE, Some(at), 0.8, pitch);
        self.player_drop(removed);
        let block = BlockPos::containing(position);
        let pitch = 0.8 + self.world.random.next_float() * 0.4;
        self.audio.play(
            horizon_client::bundle::DROP_CONTENTS,
            Some([block.x, block.y, block.z].map(|v| f64::from(v) + 0.5)),
            0.8,
            pitch,
        );
        self.award_stat("used", &item, 1);
    }
    /// `ServerPlayerGameMode.tick` while a player digs: level event 2020 every
    /// fourth tick since the START (the hit sound), else 2019 (the crack only),
    /// for everyone around, the digger included (`levelEvent(null, ...)`).
    fn destroy_progress_event(&mut self, pos: [i32; 3], face: [i32; 3], ticks: u32) {
        let id = if ticks.is_multiple_of(4) { 2020 } else { 2019 };
        let direction = horizon_core::geometry::Direction::ALL
            .iter()
            .position(|d| d.normal() == face)
            .unwrap_or(1);
        self.level_event_at(id, pos, direction as i32);
    }
    fn tick_mining(&mut self) -> Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
        }
        if !self.left || self.screen != Screen::Playing || self.dialog_open() {
            self.mining = None;
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return Ok(());
        };
        // Player.blockActionRestricted: spectators, and adventure players
        // without a matching can_break item.
        if self.block_action_restricted(hit.pos)
            || !self
                .res
                .items()
                .can_destroy_block(self.held(), self.creative)
        {
            self.mining = None;
            return Ok(());
        }
        if self.creative {
            if self.destroy_delay == 0 {
                self.break_block(hit.pos)?;
                self.destroy_delay = 5;
            }
            return Ok(());
        }
        if self.destroy_delay > 0 {
            return Ok(());
        }
        if self.mining.as_ref().is_none_or(|m| m.pos != hit.pos) {
            self.attack_block(hit.pos)?;
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                per_tick: 0.0,
                ticks: 0,
            });
        }
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if self.res.facts().get(state).hardness < 0.0 {
            // The server still counts its destroy ticks on an unbreakable block.
            let m = self.mining.as_mut().ok_or("mining")?;
            m.per_tick = 0.0;
            m.ticks += 1;
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
            return Ok(());
        }
        let progress = self.destroy_progress(hit.pos);
        self.swing_arm();
        let m = self.mining.as_mut().ok_or("mining")?;
        m.per_tick = progress;
        m.progress += progress;
        m.ticks += 1;
        // An instant break at START sends no destroy events.
        if m.progress < 1.0 || m.ticks > 1 {
            let (pos, ticks) = (m.pos, m.ticks);
            self.destroy_progress_event(pos, hit.face, ticks);
        }
        let m = self.mining.as_mut().ok_or("mining")?;
        // ServerPlayerGameMode breaks a player's block at once only when it is instant (START's
        // insta-mine); otherwise it waits for the client's STOP_DESTROY_BLOCK
        // (`guest_stop_digging`), never breaking it on its own count. Breaking it here would
        // take the block from under the player's crack before the player's client predicts the
        // break, so it gets neither particles nor sound.
        if m.progress >= 1.0 && (self.lan_acting.is_none() || m.ticks == 1) {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        Ok(())
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        if self.dim == Dim::Nether {
            // Below the bedrock ceiling: a solid floor with two air blocks above.
            for y in (32..120).rev() {
                let floor = self.res.facts().get(self.world.block(x, y - 1, z));
                if floor.full_collision
                    && floor.fluid == Fluid::None
                    && self.res.facts().is_air(self.world.block(x, y, z))
                    && self.res.facts().is_air(self.world.block(x, y + 1, z))
                {
                    return Some(y);
                }
            }
            return None;
        }
        let mut y = self.world.top_y() - 1;
        while y > self.world.bottom_y() {
            let s = self.world.block(x, y, z);
            if !self.res.facts().is_air(s) {
                let f = self.res.facts().get(s);
                let ok = f.full_collision
                    && f.fluid == Fluid::None
                    && !f.name.contains("leaves")
                    && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    /// Landing rules (farmland trampling, turtle eggs) wrote or removed the block
    /// raw: finish `Level.setBlock`'s neighbour updates, so a crop on trampled
    /// farmland breaks. The local player's and every server player's landings.
    /// The level events they send go out too: a turtle egg's break particles
    /// and sound (`decreaseEggs`' 2001, `Level.destroyBlock`'s for the last).
    ///
    /// A mod hears the change first (`native_entity_change_block_pre`, Bukkit's
    /// `EntityChangeBlockEvent`): cancelling puts the block back and skips the
    /// updates and the events, so nothing the change would have caused happens -
    /// the crop on trampled farmland stays, because breaking it is the
    /// neighbour update below and not a write of its own.
    ///
    /// The game event the rule posts comes back for the caller to post with the
    /// landing player in place (`post_landing_vibration`).
    fn landing_block_updates(
        &mut self,
        player: Option<u128>,
        event: &horizon_client::landing::FallEvent,
    ) -> Option<LandingVibration> {
        use horizon_client::landing::FallEvent;
        use horizon_world::block_landing::LandingEvent;
        let mut vibration = None;
        // The block the landing changed, what it replaced, and what it became.
        let (position, from, to, flags) = match *event {
            FallEvent::Write {
                position,
                from,
                state,
                flags,
            } => (position, from, state, flags as i32),
            FallEvent::Destroy {
                position, state, ..
            } => {
                // `Level.destroyBlock`: BLOCK_DESTROY with no source, after the
                // write (`TurtleEggBlock.decreaseEggs`' last egg).
                vibration = Some(LandingVibration {
                    event: "minecraft:block_destroy",
                    position,
                    state,
                    from_entity: false,
                });
                (position, state, self.air_state(), level_flags::UPDATE_ALL)
            }
            // `FarmlandBlock.turnToBaseBlock`'s BLOCK_CHANGE from the entity that
            // trampled it, and `TurtleEggBlock.decreaseEggs`' BLOCK_DESTROY
            // (`Context.of(state)`, no source): the write is already applied.
            FallEvent::BlockEvent {
                kind,
                position,
                state,
                with_entity,
            } => {
                // A cancelled change put the block back: nothing happened.
                let current = self.world.block(position.x, position.y, position.z);
                let cancelled = match kind {
                    LandingEvent::BlockChange => current != state,
                    LandingEvent::BlockDestroy => current == state,
                };
                if cancelled {
                    return None;
                }
                return Some(LandingVibration {
                    event: match kind {
                        LandingEvent::BlockChange => "minecraft:block_change",
                        LandingEvent::BlockDestroy => "minecraft:block_destroy",
                    },
                    position,
                    state,
                    from_entity: matches!(kind, LandingEvent::BlockChange) && with_entity,
                });
            }
            FallEvent::LevelEvent {
                event,
                position,
                data,
            } => {
                self.level_event_at(event, [position.x, position.y, position.z], data);
                return None;
            }
            _ => return None,
        };
        if self.landing_change_cancelled(player, position, from, to) {
            self.world
                .set(position.x, position.y, position.z, from)
                .ok();
            return None;
        }
        if matches!(event, FallEvent::Destroy { .. }) {
            self.level_event_at(2001, [position.x, position.y, position.z], from.0 as i32);
        }
        self.with_level(|b, level, ticks| b.notify_written(level, ticks, position, flags));
        vibration
    }
    /// Post a landing's block game event (`FarmlandBlock.turnToBaseBlock`'s
    /// BLOCK_CHANGE, `TurtleEggBlock.decreaseEggs`' BLOCK_DESTROY) as the player
    /// whose landing it was, who must be in place (`as_guest`). A sculk sensor, a
    /// calibrated sensor and a warden hear it.
    fn post_landing_vibration(&mut self, vibration: LandingVibration) {
        let at = [
            vibration.position.x,
            vibration.position.y,
            vibration.position.z,
        ];
        let who = vibration
            .from_entity
            .then_some(horizon_client::mobs::Who::Player);
        self.post_block_game_event(vibration.event, at, who, Some(vibration.state));
    }
    /// `Blocks.AIR.defaultBlockState`: what a landing's destroy leaves.
    fn air_state(&self) -> BlockStateId {
        self.pack
            .assets
            .registry
            .default_state("minecraft:air")
            .unwrap_or(BlockStateId(0))
    }
    /// Whether a mod cancelled a landing's block change
    /// (`native_entity_change_block_pre`): asked with the two blocks' names and
    /// the player whose landing it was (`None` when no player did it).
    fn landing_change_cancelled(
        &mut self,
        player: Option<u128>,
        position: BlockPos,
        from: BlockStateId,
        to: BlockStateId,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::EntityChangeBlockPre) {
            return false;
        }
        let mut cancel = false;
        self.native_entity_change_block_pre(native_hooks::EntityChangeBlockPreCall {
            player,
            entity: None,
            kind: "minecraft:player",
            pos: [position.x, position.y, position.z],
            from,
            to,
            cancel: &mut cancel,
        });
        cancel
    }
    /// The clock of the animated textures: `Minecraft.runTick` ticks the `TextureManager`
    /// only while the level runs normally. Under `HORIZON_STILL` it is the world's ticks.
    pub(super) fn texture_clock(&self) -> u64 {
        if self.clock.still() {
            self.visual_ticks()
        } else {
            self.texture_ticks
        }
    }
    fn tick(&mut self) -> Result<()> {
        if self.agent.hold_server {
            return Ok(());
        }
        self.abort_open_transactions();
        // `Player.entityInteractionRange`/`blockInteractionRange` for the tick's
        // picks: the attribute carries creative's and a server's modifiers.
        self.entity_reach = self.interaction_range("minecraft:entity_interaction_range", 3.0, 5.0);
        self.block_reach = self.interaction_range("minecraft:block_interaction_range", 4.5, 5.0);
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                if let Some(c) = self.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.set_screen(Screen::Inventory);
            }
        }
        if self.remote.is_some() {
            return self.tick_remote();
        }
        // Only a server ticks its own world from here: its players are guests, swapped in for
        // their turns (`as_guest`), and the window's player is a client of it.
        self.ticks += 1;
        self.repro_tick();
        self.track_experience_display();
        self.tick_enchanting_book();
        // `/tick freeze` stops the world but not the player.
        let running = match self.level_pass.running {
            Some(running) => running,
            None => self.clock.world_runs(),
        };
        // GameRenderer.tick: a wither's bar darkens the world, then fades;
        // the activated item counts down.
        self.boss_overlay.tick_darken(running);
        self.tick_item_activation();
        self.native_inventory_ticks();
        if running {
            // `MinecraftServer.tickChildren`: `commandFunctions` runs before
            // any level ticks, so a tick function reads the game time and the
            // day clock from before this tick. Vanilla's SavedData is loaded
            // long before this point; the lazy scoreboard/storage read has to
            // be done by then too, or a load tag writes over what it restores.
            self.load_scoreboard();
            self.tick_functions();
            self.world.game_time += 1;
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ADVANCE_WEATHER)
            {
                self.world.weather.tick_timers(&mut self.world.random);
            }
            self.world.weather.tick_levels();
        } else if self.clock.still() {
            // Under `HORIZON_STILL` a frozen world's rain and thunder sit at their levels, as a
            // shader pack's smoothed values do, so `/weather rain` shows full rain without
            // stepping the world (and its mobs) through the fade.
            self.world.weather.settle_levels();
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.tick_weather_effects();
        let _ = self.finish_player_autosaves(false);
        // MinecraftServer.tickServer's countdown, then autoSave: three
        // hundred seconds of ticks at the rate the server runs at now.
        self.ticks_until_autosave -= 1;
        self.autosaving = self.ticks_until_autosave <= 0;
        if let Some(serving) = &self.serving {
            serving.set_saving(self.autosaving);
        }
        if self.autosaving {
            self.save_game_with(false)?;
            self.ticks_until_autosave = self.next_autosave_interval();
            if let Some(serving) = &self.serving {
                serving.set_saving(false);
            }
        }
        if running {
            self.advance_clock();
            // `ServerLevel.tickTime`, once the clock has moved on.
            self.run_scheduled_functions();
        }
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        self.tick_totem();
        self.update_totem_ready();
        self.tick_command_extras();
        self.tick_world(running)
    }
    /// What the tick runs after the local player's own update: effects, the
    /// world's entities and blocks, and the network hub.
    fn tick_world(&mut self, running: bool) -> Result<()> {
        // Entities added since the last tick join the level before it ticks.
        self.native_joins_flush(false, 0);
        self.share_spawn_conditions();
        horizon_client::mobs::sulfur_cube::set_tnt_explodes(
            self.ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::TNT_EXPLODES),
        );
        self.update_chunk_tickets();
        self.score_destroyed_entities();
        horizon_client::spans::time("tick_effects", || self.tick_effects(running));
        self.tick_sign_editor();
        // Player.tick's ItemCooldowns.tick for the window's player (a
        // server's players count theirs down in `tick_guests`).
        self.shield_cooldown = self.shield_cooldown.saturating_sub(1);
        self.cooldowns.tick();
        horizon_client::spans::time("tick_using", || self.tick_using());
        self.tick_bundle_hover();
        horizon_client::spans::time("tick_hud", || self.tick_hud());
        // The player's digging comes in with its packets, before the level
        // ticks; ServerLevel.tick then runs the scheduled and random block
        // ticks before any entity, so what they drop or spawn (dispensed
        // items, popped crops) moves on the tick it appears, as in vanilla.
        // On the server thread the window is a hub guest: drain its packets
        // here too, before falling-block schedules run (#4510).
        horizon_client::spans::time("tick_local_hand", || self.tick_local_hand())?;
        horizon_client::spans::time("postprocess_chunks", || self.postprocess_chunks())?;
        let mut sent = None;
        if running {
            horizon_client::spans::time("tick_lan_packets", || self.tick_lan_packets());
            sent = horizon_client::spans::time("tick_blocks", || self.tick_blocks())?;
            // What the blocks spawned (a dispensed minecart, a lit TNT, a
            // placed boat) joins before the entities tick.
            self.native_joins_flush(false, 0);
            self.lan_mark_synced();
            // `EntityTickList.forEach`: what joined before the entity pass
            // (function summons, dropped or dispensed items) ticks in it;
            // what joins during it waits for the next tick
            // (`ensureActiveIsNotIterated`). Primed TNT and falling blocks
            // keep their own packet cutoff (`ordered.announced`, #10063).
            self.ticked_entities_through = self.next_entity;
        }
        // ChunkMap still ticks while /tick freeze holds block/entity ticks.
        if !running {
            self.save_chunks_eagerly();
        }
        self.begin_ordered_ticks();
        if running {
            horizon_client::spans::time("tick_paintings", || self.tick_paintings());
            horizon_client::spans::time("tick_item_frames", || self.tick_item_frames());
            self.tick_explicit_knots();
            self.tick_summoned_eyes();
            self.ordered_before(ordered_tick::Batch::ArmorStands)?;
            horizon_client::spans::time("tick_armor_stands", || self.tick_armor_stands())?;
            horizon_client::spans::time("tick_cushions", || self.tick_cushions());
            // ServerLevel.tick: the arrivals of this server tick are in the level
            // (`addDuringTeleport`), and each vehicle handles its portal as its
            // own tick opens.
            self.land_portal_arrivals();
            self.handle_vehicle_portals();
            self.ordered_before(ordered_tick::Batch::Boats)?;
            horizon_client::spans::time("tick_boats", || self.tick_boats());
            self.ordered_before(ordered_tick::Batch::Carts)?;
            horizon_client::spans::time("tick_carts", || self.tick_carts());
            self.tick_vehicle_watch();
            let from = self.lan_synced_from();
            horizon_client::spans::time("tick_thunder", || self.tick_thunder());
            self.lan_synced_since(from);
            horizon_client::spans::time("tick_lecterns", || self.tick_lecterns());
            horizon_client::spans::time("tick_maps", || self.tick_maps());
            self.tick_lodestone_compasses();
            horizon_client::spans::time("tick_bolts", || self.tick_bolts());
            horizon_client::spans::time("tick_fishing", || self.tick_fishing());
            self.ordered_before(ordered_tick::Batch::Tridents)?;
            horizon_client::spans::time("tick_tridents", || self.tick_tridents());
            self.ordered_before(ordered_tick::Batch::Rockets)?;
            horizon_client::spans::time("tick_rockets", || self.tick_rockets());
            horizon_client::spans::time("tick_ominous_item_spawners", || {
                self.tick_ominous_item_spawners()
            });
        }
        if running {
            horizon_client::spans::time("entity_portals", || {
                self.handle_entity_portals();
                self.handle_end_gateway_portals();
            });
            self.ordered_before(ordered_tick::Batch::Items)?;
            horizon_client::spans::time("tick_items", || self.tick_items());
            if self.remote.is_none() {
                self.hoppers_catch_items();
            }
            // Merged, burnt, despawned or picked up.
            self.native_leaves_flush();
            self.native_joins_flush(false, 0);
            horizon_client::spans::time("touch_arrows", || self.touch_arrows());
        }
        horizon_client::spans::time("spawn_loaded_entities", || self.spawn_loaded_entities());
        // The ride the player's file names, once the vehicle is in the world.
        self.restore_root_vehicle();
        if running {
            self.ordered_before(ordered_tick::Batch::Mobs)?;
            horizon_client::spans::time("tick_mobs", || self.tick_mobs())?;
            // Dead, despawned or gone into a hive.
            self.native_leaves_flush();
            horizon_client::spans::time("recheck_golem_openers", || self.recheck_golem_openers());
            self.tick_mod_container_viewers();
            // Orbs, primed TNT and falling blocks still to tick, in join order.
            self.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)?;
            horizon_client::spans::time("tick_experience", || self.tick_experience());
            // Merged, despawned or picked up.
            self.native_leaves_flush();
            horizon_client::spans::time("entity_portals", || {
                self.find_entity_portals();
                self.resolve_arrivals();
            });
        }
        // The rest of the tick's spawns join, and what left is told of.
        self.native_joins_flush(false, 0);
        self.native_leaves_flush();
        // ServerLevel.tick broadcasts the changed blocks after the scheduled and
        // random ticks (`tick_blocks`), before the block events, the entities
        // and the block entities: what those change goes out with the next
        // tick's. A piston's moved block reaches the clients once their own
        // copy of it finished moving, so a slow tick rate shows it slide
        // (#8430).
        if let (Some(sent), Some(log)) = (sent, self.world.block_log.as_mut()) {
            self.world.block_log_held = log.split_off(sent.min(log.len()));
        }
        horizon_client::spans::time("tick_lan", || self.tick_lan(false));
        let held = std::mem::take(&mut self.world.block_log_held);
        if let Some(log) = self.world.block_log.as_mut() {
            log.extend(held);
        }
        self.tick_friends();
        horizon_client::spans::time("tick_hand", || self.tick_hand());
        horizon_client::spans::time("tick_sleep", || self.tick_sleep());
        self.announce_sleep_status();
        self.free_left_beds();
        if running {
            horizon_client::spans::time("tick_end", || self.tick_end())?;
            horizon_client::spans::time("end_gateways", || {
                self.find_end_gateway_portals();
                self.resolve_end_gateway_arrivals()
            })?;
            self.keep_gateway_destinations();
        }
        if running {
            // Primed TNT and falling blocks that joined since the spawn packets.
            self.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, true)?;
            self.apply_late_wind_pushes();
        }
        // Everything that has joined has had its spawn packet and, if it joined
        // before the entity passes (TNT a piston or repeater primed), its first
        // tick in the late pass above (#10063).
        self.ordered.announced = horizon_client::join_seq::JoinSeq::upcoming();
        if running {
            horizon_client::spans::time("tick_moving_pistons", || self.tick_moving_pistons())?;
            horizon_client::spans::time("tick_block_entities", || self.tick_block_entities())?;
            horizon_client::spans::time("tick_sculk_entities", || self.tick_sculk_entities());
            horizon_client::spans::time("tick_sculk_blocks", || self.tick_sculk_blocks())?;
        }
        if running {
            // ClientLevel.tickBlockEntities (not while paused or frozen):
            // chest lids, books, shulker boxes, bells, conduits, heads.
            horizon_client::spans::time("tick_block_entity_anims", || {
                self.tick_block_entity_anims()
            });
            // A conduit's hurt teleports an enderman now (`Enderman.hurtServer`).
            self.settle_hurt_teleports()?;
            horizon_client::spans::time("tick_particles", || self.tick_particles());
            self.tick_client_ambient();
        }
        // ServerLevel.tick's `entityManager.tick()`, frozen or not, after every
        // block entity ticker (the bells, conduits and gateways above too): the
        // chunks loaded before this pass have their entities loaded from the next.
        self.world.entities_pending.clear();
        self.tick_frame_visuals(running);
        if running {
            // MinecraftServer.tickServer: GameTestTicker.SINGLETON.tick().
            self.tick_game_tests();
        }
        // ServerBossEvent broadcasts, after this level's entities have ticked.
        self.publish_boss_bars();
        // Minecraft.handleKeybinds: held use repeats only while no item is in
        // use (a raised shield or a drawn bow places and uses nothing).
        if self.right
            && self.use_delay == 0
            && self.using.is_none()
            && self.screen == Screen::Playing
            && !self.dialog_open()
            && self.sleeping.is_none()
        {
            self.use_with_hand()?;
            self.use_delay = 4;
        }
        if running {
            horizon_client::spans::time("lan_note_joined", || self.lan_note_joined());
        }
        Ok(())
    }
    /// The title remains the no-level sentinel while debug options covers it.
    /// Only UI dispatch ignores it; world loading and simulation still see it.
    fn title_input_active(&self) -> bool {
        self.title.is_some() && self.screen != Screen::DebugOptions
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        // Off a world (title, options, Win, a prompt before joining) the HUD shows nothing to
        // keep clear of; `hud_layers` fills it in once the world's HUD draws.
        let gui = HudBatch::new(size, self.settings.gui_scale_inputs());
        self.ui.room = horizon_client::hud_kit::room::Room::new(gui.width, gui.height);
        // The pack prompt replaces the connecting screen (setScreen), but
        // in game it opens over the world.
        let mut batch = if (self.server_packs.prompt.is_some()
            || self.code_of_conduct.is_some()
            || self.dialog_open())
            && self.player.is_none()
        {
            HudBatch::new(size, self.settings.gui_scale_inputs())
        } else {
            self.hud_layers(size)
        };
        // Vanilla's confirmation replaces the dialog as the screen.
        if self.command_prompt.is_none() {
            self.draw_dialog(&mut batch);
        }
        self.draw_pack_prompt(&mut batch);
        self.draw_code_of_conduct(&mut batch);
        self.draw_link_prompt(&mut batch);
        self.draw_command_prompt(&mut batch);
        // The ToastManager draws over every screen, the title and the
        // options included: the narrator's toast follows the player there.
        self.draw_narrator_toast(&mut batch);
        // Gui.extractRenderState: the debug overlay comes after the screen
        // and the toasts, so it is over everything, menus included.
        self.draw_debug_overlay(&mut batch, size);
        self.frame_graph
            .draw(self.settings.horizon.overlay, &mut batch, &self.gui.atlas);
        self.draw_pad_cursor(&mut batch);
        batch
    }
    fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        self.pad.guide_shown.clear();
        let mut batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        batch.glint_colour = self.settings.extras.pvp.tweaks.glint_texel();
        // GlobalSettingsUniform's GameTime, for a resource pack's text shader.
        batch.game_time = ((self.world.game_time.rem_euclid(24000)) as f32
            + self.clock.fraction() as f32)
            / 24000.0;
        let (w, h) = (batch.width, batch.height);
        // HDR calibration ends when its page does, whatever moved the screen
        // off Options (a death screen opened by a server, say).
        if self.screen != Screen::Options && self.ui.options.calibration_fullscreen.is_some() {
            self.show_calibration(h);
        }
        if self.screen == Screen::Win {
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn {
                self.draw_win_screen(&mut batch);
            }
            // The early return skips the match below: the hooks still see the
            // screen, after its own drawing.
            self.draw_screen_with_post_hook(&mut batch, drawn);
            return batch;
        }
        if self.screen == Screen::Options {
            if self.title.is_none() {
                self.flush_skin_slots();
                self.update_remote_skins();
            }
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn {
                self.draw_options(&mut batch);
            }
            self.draw_screen_with_post_hook(&mut batch, drawn);
            self.draw_split_hud(&mut batch, true);
            return batch;
        }
        self.settle_title_toasts();
        if let Some(title) = &mut self.title {
            if std::mem::take(&mut title.onboarding_finished) {
                // Options.onboardingAccessibilityFinished.
                self.settings.file.set(
                    "onboardAccessibility",
                    horizon_client::options_file::V::Bool(false),
                );
                self.settings.save(&Self::options_path());
            }
            if let Some(sound) = title.update_toast(self.settings.notification_display_time) {
                self.audio.play(sound, None, 1.0, 1.0);
            }
        }
        if let Some(title) = &self.title {
            for found in title.lan_found_narrations((w, h)) {
                self.say_system_queued(&found);
            }
        }
        if self.screen == Screen::DebugOptions && self.title.is_some() {
            // DebugOptionsScreen replaces the title; no level HUD is behind it.
            let drawn = self.draw_screen_with_pre_hook(&mut batch);
            if drawn {
                self.draw_debug_options(&mut batch);
            }
            self.draw_screen_with_post_hook(&mut batch, drawn);
            self.draw_toasts(&mut batch, false);
            self.draw_tutorial_toasts(&mut batch, false);
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        if self.title.is_some() {
            // TitleScreen.extractRenderState, then ToastManager over it: vanilla
            // draws the toast manager on every screen, so a pack download or a
            // resource-reload failure still shows on the title (#4428).
            let drawn = self.draw_title_screen_with_pre_hook(&mut batch);
            if drawn && let Some(title) = &self.title {
                title.draw_with_items(
                    &self.gui.atlas,
                    Some(&self.pack.item_renderer),
                    &mut batch,
                    self.cursor,
                    self.settings.file.bool("hideSplashTexts"),
                    self.settings.file.bool("hideServerAddress"),
                );
            }
            // ScreenEvent.Render.Post over the title's own screens: a mod draws
            // over them here (a pack browser over Create New World, say), and the
            // toasts still come after, as over any screen.
            self.draw_title_screen_with_post_hook(&mut batch, drawn);
            self.draw_toasts(&mut batch, false);
            self.draw_tutorial_toasts(&mut batch, false);
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        let atlas_ptr: *const HudAtlas = &self.gui.atlas;
        let _ = atlas_ptr;
        // The ToastManager draws over every screen: a server's packs
        // download while the connection screen shows.
        if self.remote_overlay(&mut batch) {
            self.draw_pack_toasts(&mut batch, false);
            self.draw_music_toast(&mut batch, false);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.spectator();
        let (wi, hi) = (w as i32, h as i32);
        let mut gui_canceled = false;
        if !self.ui.hide_gui {
            self.native_render_gui_pre(&mut gui_canceled);
        }
        // RenderGuiEvent.Pre: a mod's cancel hides the HUD as F1 does.
        let show_hud = !self.ui.hide_gui && !gui_canceled && self.studio_replay.is_none();
        // The mods' layers (`register_gui_layers`) draw among vanilla's while the HUD shows.
        self.hud_layer_next = show_hud.then_some(0);
        self.ui.room = self.hud_room(w, h, show_hud);
        let mut guide_from = 0;
        if show_hud {
            self.draw_spyglass(&mut batch);
            guide_from = batch.vertices.len();
            // Hud.extractCrosshair / extractItemHotbar / extractPlayerHealth /
            // extractVehicleHealth and the experience bar, laid out by hud_layout.
            // Combat feel Legacy hides the attack indicator unless the player keeps it.
            let combat = self.settings.extras.pvp.combat;
            let indicator = if combat.legacy && !combat.indicator {
                0
            } else {
                self.settings.attack_indicator
            };
            let delay = horizon_client::attack_strength::delay(self.main_hand_attack_speed());
            let strength = self.hand_strength.scale(0.0, self.main_hand_attack_speed());
            // The 3D crosshair (F3) takes the 2D one's place.
            // Hud.extractCrosshair: a spectator only sees it over something to open.
            if self.camera_type.is_first_person()
                && (!spectator || self.spectator_crosshair())
                && !self.debug_crosshair()
            {
                let attack = (indicator == 1).then_some(strength);
                let full = attack.is_some_and(|a| {
                    hud_layout::full_attack_indicator(a, delay, self.crosshair_living())
                });
                let draws = hud_layout::crosshair(wi, hi, attack, full);
                self.gui_layer("minecraft:crosshair", &mut batch, |g, batch| {
                    // The custom crosshair takes the sprite's place (the first draw); the
                    // attack indicator stays.
                    if g.custom_crosshair() {
                        g.draw_custom_crosshair(batch, wi, hi);
                        hud_layout::paint(batch, &g.gui.atlas, &draws[1..]);
                    } else {
                        hud_layout::paint(batch, &g.gui.atlas, &draws);
                    }
                });
            }
            self.gui_layer("minecraft:hotbar", &mut batch, |g, batch| {
                if spectator {
                    g.draw_spectator_hotbar(batch);
                } else {
                    let partial = g.clock.fraction() as f32;
                    let inputs = hud_layout::HotbarInputs {
                        selected: g.selected as i32,
                        right_handed: !g.settings.main_hand_left,
                        offhand: g.offhand.is_some(),
                        slots: std::array::from_fn(|i| {
                            g.inventory[i]
                                .as_ref()
                                .map(|_| (f32::from(g.hotbar_pop[i]) - partial).max(0.0))
                        }),
                        offhand_pop: (f32::from(g.hotbar_pop[9]) - partial).max(0.0),
                        attack: (indicator == 2).then_some(strength),
                    };
                    let draws = hud_layout::hotbar(wi, hi, &inputs);
                    hud_layout::paint(batch, &g.gui.atlas, &draws);
                    for d in &draws {
                        if let hud_layout::GuiDraw::Item { slot, x, y, .. } = *d {
                            let stack = if slot == 40 {
                                g.offhand.as_ref()
                            } else {
                                g.inventory[slot as usize].as_ref()
                            };
                            if let Some(stack) = stack {
                                g.draw_item(batch, stack, x as f32, y as f32);
                            }
                        }
                    }
                }
            });
            self.gui_layer("minecraft:effects", &mut batch, |g, batch| {
                // The PvP & HUD effects widget takes the icons' place.
                if !g.pvp_effects_shown() {
                    g.draw_effect_icons(batch);
                }
            });
            // Hud: the action bar and titles draw over the hotbar (nextStratum).
            self.gui_layer("minecraft:boss_overlay", &mut batch, |g, batch| {
                g.draw_boss_bars(batch);
            });
            self.draw_extras(&mut batch);
        }
        if show_hud {
            // Hud.nextContextualInfoState: the jump bar, the experience bar or
            // the locator bar above the hotbar.
            let has_experience = !self.creative && !spectator;
            let contextual = horizon_client::locator_bar::ContextualBar::next(
                self.waypoints.has_waypoints(),
                !spectator && self.jumpable_mount(),
                self.mount_jumping(),
                has_experience,
                self.experience_recent(),
            );
            use horizon_client::locator_bar::ContextualBar;
            // The mount's hearts.
            let vehicle_hearts = if spectator {
                0
            } else {
                self.draw_mount_hud(&mut batch, wi, hi)
            };
            let (visual_ticks, visual_millis) = (self.visual_ticks(), self.visual_millis());
            let kind = (!self.creative && !spectator)
                .then(|| self.player_heart_kind())
                .flatten();
            if let Some(kind) = kind
                && let Some(p) = &mut self.player
            {
                let effects = &self.potions.effects;
                // Hud.extractAirBubbles: `isEyeInFluidMatching`, so the eyes are below
                // the surface, not merely in the water block.
                let under_water = p.is_eye_in_fluid(horizon_client::motion::WATER);
                let inputs = hud_layout::HealthInputs {
                    tick: visual_ticks as i32,
                    millis: visual_millis,
                    health: p.health.health(),
                    max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
                    absorption: p.health.absorption(),
                    armor: p
                        .attribute("minecraft:armor")
                        .or_else(|_| p.attribute("armor"))
                        .unwrap_or(0.0) as i32,
                    food: p.health.food.food,
                    saturation: p.health.food.saturation,
                    kind,
                    regeneration: effects.get("regeneration").is_some(),
                    hunger: effects.get("hunger").is_some(),
                    hardcore: self.level.hardcore,
                    damage_cooldown: p.health.hurt.cooldown > 0,
                    air: p.air,
                    max_air: 300,
                    under_water,
                    vehicle_hearts,
                };
                let at = [p.position.x, p.position.y, p.position.z];
                // The native mods' health HUD (`native_health_hud`): a mod
                // that answers draws every row instead.
                let mut native_health_hud = None;
                self.native_health_hud(&inputs, wi, hi, &mut native_health_hud);
                let draws = match native_health_hud {
                    Some(draws) => draws,
                    None => self.health_hud.layout(&inputs, wi, hi),
                };
                // The rows draw in one pass here, so a mod's `player_health`
                // layer stands for the armor, food and air rows too.
                self.gui_layer("minecraft:player_health", &mut batch, |g, batch| {
                    hud_layout::paint(batch, &g.gui.atlas, &draws);
                });
                // Hud.playAirBubblePoppedSound: Player.playSound (PLAYERS), heard
                // by this client only.
                if let Some((volume, pitch)) = self.health_hud.bubble_pop_sound.take() {
                    let players = horizon_client::settings::SOUND_SOURCES
                        .iter()
                        .position(|s| *s == "player")
                        .unwrap_or(0);
                    let capture = self.audio.capture.take();
                    self.audio
                        .play_on(players, "ui.hud.bubble_pop", Some(at), volume, pitch);
                    self.audio.capture = capture;
                }
            }
            // After the health rows (Hud.extractHotbarAndDecorations): jump, locator or XP bar.
            self.gui_layer("minecraft:contextual_info_bar", &mut batch, |g, batch| {
                if contextual == ContextualBar::JumpableVehicle {
                    g.draw_jump_bar(batch, wi, hi);
                }
                if contextual == ContextualBar::Locator {
                    let (sprite, [x, y, w, h]) = horizon_client::locator_bar::background(wi, hi);
                    horizon_client::ui::nine_slice(
                        batch,
                        &g.gui.atlas,
                        sprite,
                        x as f32,
                        y as f32,
                        w as f32,
                        h as f32,
                        u32::MAX,
                    );
                }
                if has_experience && g.player.is_some() {
                    let atlas = &g.gui.atlas;
                    let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                    let draws = hud_layout::experience_colored(
                        g.pack.polytone_colors.xp_bar(),
                        wi,
                        hi,
                        g.experience.progress,
                        g.experience.level,
                        contextual == ContextualBar::Experience,
                        &width,
                    );
                    hud_layout::paint(batch, atlas, &draws);
                }
                if contextual == ContextualBar::Locator {
                    let draws = g.locator_bar_draws(wi, hi);
                    hud_layout::paint(batch, &g.gui.atlas, &draws);
                }
            });
            if !spectator {
                self.gui_layer("minecraft:selected_item_name", &mut batch, |g, batch| {
                    g.draw_selected_name(batch, !g.creative);
                });
            }
        }
        // Hud.extractSleepOverlay: over all health/food bars, under titles and chat.
        let draws = hud_overlays::sleep_overlay(wi, hi, self.sleep_timer as i32);
        self.gui_layer("minecraft:sleep_overlay", &mut batch, |g, batch| {
            hud_layout::paint(batch, &g.gui.atlas, &draws);
        });
        if show_hud {
            self.gui_layer("minecraft:overlay_message", &mut batch, |g, batch| {
                g.draw_overlay_message(batch);
            });
            self.gui_layer("minecraft:title", &mut batch, |g, batch| {
                g.draw_title(batch);
            });
            self.gui_layer("minecraft:scoreboard_sidebar", &mut batch, |g, batch| {
                g.draw_scoreboard(batch);
            });
            self.gui_layer("minecraft:subtitle_overlay", &mut batch, |g, batch| {
                g.draw_subtitles(batch);
            });
        }
        let atlas = &self.gui.atlas;
        if show_hud
            && let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        // The chat screen draws the focused chat itself; F1 hides the rest.
        if self.screen == Screen::Chat {
            // The mod layers under the chat stay under the focused chat.
            self.draw_mod_gui_layers(Some("minecraft:chat"), &mut batch);
            self.draw_chat(&mut batch);
        } else if show_hud {
            self.gui_layer("minecraft:chat", &mut batch, |g, batch| g.draw_chat(batch));
            self.gui_layer("minecraft:tab_list", &mut batch, |g, batch| {
                g.draw_tab_list(batch);
            });
        }
        // The mod layers above the last vanilla layer drawn.
        self.draw_mod_gui_layers(None, &mut batch);
        self.hud_layer_next = None;
        // Screen.extractBlurredBackground blurs the world and HUD, while the
        // screen's background, widgets, toasts and debug overlay stay sharp.
        batch.blur_at = batch.vertices.len() as u32;
        // PackConfirmScreen takes the open screen's place (it comes back once
        // the prompt is answered): the screen under the prompt isn't drawn.
        let screen = if self.server_packs.prompt.is_some() || self.book.is_some() {
            Screen::Playing
        } else {
            self.screen
        };
        let drawn = self.draw_screen_with_pre_hook(&mut batch);
        match screen {
            _ if !drawn => {}
            Screen::Paused if self.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                let title = self.translate_with_fallback("menu.paused", "Game Paused");
                horizon_client::widgets::begin_screen(|| {
                    horizon_client::widgets::ScreenInfo::new("paused", title.clone())
                });
                let tw = HudBatch::text_width(&self.gui.atlas, &title);
                batch.text(
                    &self.gui.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
            }
            Screen::GameModeSwitcher => self.draw_gamemode_switcher(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::Mod => {}
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Friends => self.draw_friends_screen(&mut batch),
            Screen::Social => self.draw_social_screen(&mut batch),
            Screen::ServerLinks => self.draw_server_links(&mut batch),
            Screen::ConfirmLink => self.draw_confirm_link(&mut batch),
            Screen::DialogWarning => self.draw_dialog_warning(&mut batch),
            Screen::CommandBlock => self.draw_command_block(&mut batch),
            Screen::BlockEditor => self.draw_block_editor(&mut batch),
            Screen::DebugOptions => self.draw_debug_options(&mut batch),
            Screen::HudEditor => self.draw_hud_editor(&mut batch),
            Screen::Studio => {}
            Screen::Inventory => {
                let from = batch.vertices.len();
                self.draw_inventory(&mut batch);
                self.draw_mod_browser(&mut batch);
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Creative => {
                let from = batch.vertices.len();
                self.draw_creative(&mut batch);
                self.draw_mod_browser(&mut batch);
                self.draw_pad_menu_guide(&mut batch, from);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Options | Screen::Win => {}
        }
        // ScreenEvent.Render.Post: the open screen has drawn itself and its
        // tooltips, so a native mod draws over it here. Never `Playing`: the
        // prompts, books and sign editors drawn over gameplay keep it, draw
        // after this point and so over what a mod drew.
        self.draw_screen_with_post_hook(&mut batch, drawn);
        self.draw_saving_indicator(&mut batch);
        // ToastManager.extractRenderState: F1 hides the toasts too.
        self.draw_toasts(&mut batch, !show_hud);
        self.draw_tutorial_toasts(&mut batch, !show_hud);
        self.draw_pack_toasts(&mut batch, !show_hud);
        self.draw_music_toast(&mut batch, !show_hud);
        if self.server_packs.prompt.is_none() {
            self.remote_hud(&mut batch);
            self.draw_sign_editor(&mut batch);
            self.draw_book(&mut batch);
        }
        if !self.ui.hide_gui
            && self.screen == Screen::Playing
            && (self.studio_replay.is_some() || self.studio_freecam.active)
        {
            self.studio_topbar.render(
                &mut batch,
                &self.gui.atlas,
                self.cursor,
                self.studio_freecam.locked_to_path,
            );
            self.studio_timeline.render(
                &mut batch,
                &self.gui.atlas,
                self.cursor,
                &self.studio_freecam,
            );
        }
        if show_hud {
            self.draw_pad_guide(&mut batch, guide_from);
        }
        batch
    }
    /// Slot ids: 0..36 player inventory, 100..109 crafting grid, 200 result.
    fn container_kind(&self) -> Option<&BlockEntity> {
        self.container
            .and_then(|k| self.world.block_entities.get(&k))
    }
    /// 300.. are the open container's slots.
    fn slots(&self) -> Vec<usize> {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().collect();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().collect();
        }
        if let Some(v) = self.station_slots().or_else(|| self.mount_slots()) {
            return v;
        }
        let mut v: Vec<usize> = (0..36).collect();
        if let Some(shown) = self.native_screen_slots() {
            v.extend(shown);
            return v;
        }
        match self.container_kind() {
            Some(BlockEntity::Chest(_)) => {
                v.extend((0..self.open_container_size()).map(|i| 300 + i))
            }
            Some(BlockEntity::Furnace(_)) => v.extend(300..303),
            Some(BlockEntity::Brewing(_)) => v.extend(300..305),
            Some(BlockEntity::Crafter(_)) => v.extend(300..310),
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CopperGolemStatue(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Bell
                | BlockEntity::Conduit(_)
                | BlockEntity::EndPortal
                | BlockEntity::EnchantingTable
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => {}
            None => {
                v.extend((0..self.craft_size * self.craft_size).map(|i| 100 + i));
                v.push(200);
                if self.craft_size == 2 {
                    v.extend(500..505);
                }
            }
        }
        v
    }
    /// How many slots `slots` lists, counted without building the list: the
    /// screen render hook asks once a frame and must not allocate. Same
    /// branches in the same order, so the two stay in step.
    fn slot_count(&self) -> usize {
        if self.creative_inventory_open() {
            return Creative::inventory_slots().count();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().count();
        }
        if let Some(n) = self
            .station_slot_count()
            .or_else(|| self.mount_slot_count())
        {
            return n;
        }
        let inventory = 36;
        if let Some(shown) = self.native_screen_slot_count() {
            return inventory + shown;
        }
        let container = match self.container_kind() {
            Some(BlockEntity::Chest(_)) => self.open_container_size(),
            Some(BlockEntity::Furnace(_)) => 3,
            Some(BlockEntity::Brewing(_)) => 5,
            Some(BlockEntity::Crafter(_)) => 10,
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CopperGolemStatue(_)
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_)
                | BlockEntity::EndGateway(_)
                | BlockEntity::Skull(_)
                | BlockEntity::DaylightDetector
                | BlockEntity::Bell
                | BlockEntity::Conduit(_)
                | BlockEntity::EndPortal
                | BlockEntity::EnchantingTable
                | BlockEntity::Campfire(_)
                | BlockEntity::ChiseledBookshelf(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::Shelf(_)
                | BlockEntity::Mod(_)
                | BlockEntity::PotentSulfur(_)
                | BlockEntity::StructureBlock(_)
                | BlockEntity::JigsawBlock(_)
                | BlockEntity::TestBlock(_)
                | BlockEntity::TestInstanceBlock(_),
            ) => 0,
            None => {
                self.craft_size * self.craft_size + 1 + if self.craft_size == 2 { 5 } else { 0 }
            }
        };
        inventory + container
    }

    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        screen_info::begin_container("container");
        if self.draw_station(batch) {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.gui.atlas;
            self.render_transparent_background("inventory", batch);
            let part = |batch: &mut HudBatch,
                        name: &str,
                        sx: f32,
                        sy: f32,
                        w: f32,
                        h: f32,
                        dx: f32,
                        dy: f32| {
                if let Some(mut r) = atlas.sprites.get(name).copied() {
                    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
                    r.u0 += sx * du;
                    r.v0 += sy * dv;
                    r.u1 = r.u0 + w * du;
                    r.v1 = r.v0 + h * dv;
                    batch.rect_uv(dx, dy, w, h, r, u32::MAX);
                }
            };
            let grey = rgba(64, 64, 64, 255);
            match self
                .container
                .and_then(|k| self.world.block_entities.get(&k))
            {
                Some(_) if self.draw_native_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) if self.draw_machine_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) => self.draw_chest_screen(batch, ox, oy),
                Some(BlockEntity::Furnace(f)) => {
                    let (tex, key, fallback) = match f.kind {
                        horizon_client::containers::FurnaceKind::Furnace => {
                            ("gui/container/furnace", "container.furnace", "Furnace")
                        }
                        horizon_client::containers::FurnaceKind::BlastFurnace => (
                            "gui/container/blast_furnace",
                            "container.blast_furnace",
                            "Blast Furnace",
                        ),
                        horizon_client::containers::FurnaceKind::Smoker => {
                            ("gui/container/smoker", "container.smoker", "Smoker")
                        }
                    };
                    part(batch, tex, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    // A server's furnace menu has the title it was given.
                    let title = self.remote_menu_title().unwrap_or_else(|| {
                        horizon_client::text::Text::literal(
                            self.container_title(self.translate_with_fallback(key, fallback)),
                        )
                        .resolve_legacy()
                    });
                    screen_info::name_container(|| title.plain());
                    let tw = atlas.font.width_text(&title);
                    batch.text_styled(
                        atlas,
                        &title,
                        ox + (176.0 - tw) / 2.0,
                        oy + 6.0,
                        grey,
                        false,
                    );
                    let inventory =
                        self.translate_with_fallback("container.inventory", "Inventory");
                    batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    if f.lit_remaining > 0 {
                        // AbstractFurnaceScreen: flame height ceil(lit * 13) + 1.
                        let total = if f.lit_total == 0 { 200 } else { f.lit_total };
                        let h = ((f.lit_remaining as f32 / total as f32) * 13.0).ceil() + 1.0;
                        part(
                            batch,
                            "gui/sprites/container/furnace/lit_progress",
                            0.0,
                            14.0 - h,
                            14.0,
                            h,
                            ox + 56.0,
                            oy + 36.0 + 14.0 - h,
                        );
                    }
                    if f.cooking_timer > 0 && f.cooking_total > 0 {
                        let w = ((f.cooking_timer as f32 / f.cooking_total as f32) * 24.0).ceil();
                        part(
                            batch,
                            "gui/sprites/container/furnace/burn_progress",
                            0.0,
                            0.0,
                            w,
                            16.0,
                            ox + 79.0,
                            oy + 34.0,
                        );
                    }
                }
                Some(BlockEntity::Brewing(_)) => {}
                Some(BlockEntity::Crafter(_)) => self.draw_crafter_screen(batch, ox, oy),
                Some(
                    BlockEntity::Sign(_)
                    | BlockEntity::Jukebox(_)
                    | BlockEntity::Beacon(_)
                    | BlockEntity::Lectern(_)
                    | BlockEntity::Sculk(_)
                    | BlockEntity::Banner(_)
                    | BlockEntity::CopperGolemStatue(_)
                    | BlockEntity::CreakingHeart(_)
                    | BlockEntity::Spawner(_)
                    | BlockEntity::TrialSpawner(_)
                    | BlockEntity::Vault(_)
                    | BlockEntity::Beehive(_)
                    | BlockEntity::CommandBlock(_)
                    | BlockEntity::EndGateway(_)
                    | BlockEntity::Skull(_)
                    | BlockEntity::DaylightDetector
                    | BlockEntity::Bell
                    | BlockEntity::Conduit(_)
                    | BlockEntity::EndPortal
                    | BlockEntity::EnchantingTable
                    | BlockEntity::Campfire(_)
                    | BlockEntity::ChiseledBookshelf(_)
                    | BlockEntity::Brushable(_)
                    | BlockEntity::Shelf(_)
                    | BlockEntity::Mod(_)
                    | BlockEntity::PotentSulfur(_)
                    | BlockEntity::StructureBlock(_)
                    | BlockEntity::JigsawBlock(_)
                    | BlockEntity::TestBlock(_)
                    | BlockEntity::TestInstanceBlock(_),
                ) => {}
                None if self.mount_active() => self.draw_mount_background(batch, ox, oy),
                None => {
                    let texture = if self.craft_size == 3 {
                        "gui/container/crafting_table"
                    } else {
                        "gui/container/inventory"
                    };
                    part(batch, texture, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let title = self.translate_with_fallback("container.crafting", "Crafting");
                    screen_info::name_container(|| title.clone());
                    let tx = if self.craft_size == 3 { 29.0 } else { 97.0 };
                    batch.text(atlas, &title, ox + tx, oy + 6.0, grey, false);
                    if self.craft_size == 3 {
                        let inventory =
                            self.translate_with_fallback("container.inventory", "Inventory");
                        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        if self.container.is_none() && self.craft_size != 3 && !self.mount_active() {
            self.draw_inventory_player(batch, ox, oy);
        }
        self.draw_hovered_slot_highlight(batch, false, ox, oy);
        self.draw_inventory_slots(batch, ox, oy);
        self.draw_hovered_slot_highlight(batch, true, ox, oy);
        self.draw_sort_buttons(batch, ox, oy);
        self.draw_recipe_book(batch);
        let effect_tip = if self.container.is_none() && self.craft_size == 2 && !self.mount_active()
        {
            self.draw_inventory_effects(batch, (ox, oy), 176)
        } else {
            None
        };
        self.draw_carried(batch);
        self.draw_slot_tooltip(batch);
        self.draw_native_screen_tooltip(batch);
        if let Some(lines) = effect_tip {
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// `AbstractContainerScreen` keeps `isQuickCrafting` and the drag it
    /// belongs to on the screen. Horizon keeps both on `Game` (`inv_clicks`,
    /// `inv_drag`, `last_inv_click`), so they are dropped when the pointer
    /// changes hands: a screen that opens or closes starts with its own
    /// state, and a press whose release never reached the screen -- the
    /// pointer left the window, or the screen closed first -- would
    /// otherwise leave every later press on every later screen swallowed.
    pub(super) fn reset_inventory_clicks(&mut self) {
        self.inv_clicks.cancel_drag();
        self.inv_drag = None;
        self.last_inv_click = None;
    }
    pub(super) fn close_container(&mut self) {
        // `Gui.setScreen` clears the old screen's focus before it goes: the
        // key that closed the container (Tab bound to the inventory) has
        // already moved the focus, and it must not outlive the screen.
        self.pad_clear_focus();
        self.mod_browser_closed();
        self.reset_inventory_clicks();
        self.close_station();
        self.mount_closed();
        self.recipe_ui.reset_screen();
        if self.remote.is_some() {
            return self.remote_close_container();
        }
        let mut returned: Vec<ItemStack> = self.carried.take().into_iter().collect();
        returned.extend(self.craft_grid.iter_mut().filter_map(Option::take));
        for stack in returned {
            if let Some(rest) = self.add_stack_to_inventory(stack)
                && let Some(eye) = self.eye_position()
            {
                // AbstractContainerMenu.dropOrPlaceInInventory: `drop(stack, false)`.
                self.throw_from_eyes(rest, eye, false, false);
            }
        }
        self.craft_size = 2;
        self.craft_table = None;
        self.open_container_slots = None;
        if let Some(key) = self.container.take() {
            // A cart's or chest boat's items go back to it.
            self.close_cart_container(key);
            let last = self.container_opener_event(key, false);
            if last {
                self.post_block_game_event(
                    "minecraft:container_close",
                    [key.0, key.1, key.2],
                    Some(horizon_client::mobs::Who::Player),
                    None,
                );
                self.container_edge_sound(key, false);
            }
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        self.close_ender_chest();
    }
    /// `quickMoveStack` of the open plain menu for a shift-click.
    fn inventory_quick_move(&mut self, slot: usize) -> bool {
        self.click_open_inventory(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
    /// `AbstractContainerMenu.clicked` on the open inventory, crafting table,
    /// chest, hopper, dispenser or furnace menu, run by the verified menu
    /// engine (`horizon_client::menu`). `slot` None is outside the window.
    /// False when the open screen is not one the engine models.
    fn click_open_inventory(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: horizon_client::menu::Input,
    ) -> bool {
        if let Some(slot) = slot
            && slot >= Creative::DISPLAY
            && self.creative_tab_open()
        {
            self.creative_display_click(slot, button, input);
            return true;
        }
        if let Some(slot) = slot
            && input == horizon_client::menu::Input::Throw
            && self.carried.is_some()
            && self.remote.is_some()
            && ((slot < 9 && self.creative_tab_open()) || self.creative_inventory_open())
        {
            self.creative_throw_carrying(slot, button);
            return true;
        }
        self.menu_ops(&[(slot, button, input)])
    }
    /// `CreativeModeInventoryScreen.slotClicked`: THROW while a stack is carried, on an item
    /// tab's hotbar slot or any slot of the Survival Inventory tab. The menu's own THROW needs an
    /// empty hand, so the screen drops from the slot itself: one item, or with the control key
    /// the whole stack (on the inventory tab `Slot.remove(getMaxStackSize())`), through
    /// `handleCreativeModeItemDrop`, then sends the slot (`broadcastChanges`,
    /// `handleCreativeModeItemAdd`); the carried stack stays. Only the server drops it: this is
    /// the client of a server (`remote`).
    fn creative_throw_carrying(&mut self, slot: usize, button: i32) {
        let Some(old) = self.slot_mut(slot).clone() else {
            return;
        };
        let count = if button == 0 {
            1
        } else if self.creative_inventory_open() {
            let max = world_items!(self)
                .get(&old.item)
                .map_or(64, |i| i.max_stack);
            old.count.min(max)
        } else {
            old.count
        };
        let before = self.player_equipment();
        let left = old.count - count;
        *self.slot_mut(slot) = (left > 0).then(|| ItemStack {
            count: left,
            ..old.clone()
        });
        self.player_equipment_changed(&before);
        self.apply_equipment();
        self.remote_creative_throw(ItemStack { count, ..old });
        self.send_creative_slots();
    }
    /// A click on an item tab's display slot: the screen's own handling
    /// (`creative::click_display`), never the menu's.
    fn creative_display_click(
        &mut self,
        slot: usize,
        button: i32,
        input: horizon_client::menu::Input,
    ) {
        use horizon_client::creative::{DisplayClick, click_display};
        if self.creative_screen.tab().search {
            // CreativeModeInventoryScreen.slotClicked: moveCursorToEnd, then
            // setHighlightPos(0), so typing replaces the whole query.
            let search = &mut self.creative_screen.search;
            search.move_cursor_to_end(false);
            search.highlight = 0;
        }
        let first = self.creative_screen.first_row(&self.pack.names);
        let shown = self
            .creative_screen
            .shown(&self.pack.names, first * 9 + slot - Creative::DISPLAY);
        let catalog = world_items!(self);
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        match click_display(&mut self.carried, shown.as_ref(), max, button, input) {
            DisplayClick::Hotbar(i, stack) if i < self.inventory.len() => {
                self.inventory[i] = Some(stack);
            }
            DisplayClick::Offhand(stack) => self.offhand = Some(stack),
            // handleCreativeModeItemDrop: on a server the stack goes to it (SetCreativeModeSlot
            // to slot -1), which drops it; a local drop here would never reach the server.
            DisplayClick::Drop(stack) if self.remote.is_some() => self.remote_creative_throw(stack),
            DisplayClick::Drop(stack) => {
                self.player_drop(stack);
            }
            _ => {}
        }
        // handleCreativeModeItemAdd / broadcastChanges: a swapped-in hotbar slot goes out now.
        if self.remote.is_some() {
            self.send_creative_slots();
        }
    }
    /// Several clicks on one menu instance (a quick craft drag is three).
    fn menu_ops(&mut self, ops: &[(Option<usize>, i32, horizon_client::menu::Input)]) -> bool {
        use horizon_client::menu::Input;
        // BundleMouseActions.onSlotClicked: a Shift click or a swap clears the bundle's selection.
        for &(slot, _, input) in ops {
            if let (Some(slot), Input::QuickMove | Input::Swap) = (slot, input) {
                self.unselect_bundle(slot);
            }
        }
        if self.remote.is_some() && self.screen == Screen::Inventory {
            // On a server the clicks are the server's: predicted and sent.
            return self.remote_menu_ops(ops);
        }
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let ops = ops.to_vec();
        let done = self.menu_session(move |menu, env, to_menu| {
            let mut clicks = Vec::new();
            for &(slot, button, input) in &ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        match to_menu(g) {
                            Some(i) => i as i32,
                            None => return None,
                        }
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            for (menu_slot, button, input) in clicks {
                if menu.click(env, menu_slot, button, input).is_err() {
                    // Vanilla's server throws: nothing of this click is kept.
                    return Some(false);
                }
            }
            Some(true)
        });
        // CreativeModeInventoryScreen.slotClicked: the changed slots go out with the click.
        if done && self.remote.is_some() && self.screen == Screen::Creative {
            self.send_creative_slots();
        }
        done
    }
    /// A sound a menu click made the player play (a bundle's insert or
    /// remove): `Entity.playSound` at the player.
    pub(super) fn player_menu_sound(&mut self, sound: &str) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let (volume, pitch) = if sound == horizon_client::bundle::INSERT_FAIL {
            (1.0, 1.0)
        } else {
            (0.8, 0.8 + self.world.random.next_float() * 0.4)
        };
        if self.is_logical_server() {
            self.audio.play(sound, Some(at), volume, pitch);
        } else {
            self.play_client_player_sound(sound, volume, pitch);
        }
    }
    /// The open menu as the verified engine (`horizon_client::menu`): the
    /// game's slots are copied in, `run` acts on it, and the slots, carried
    /// stack, menu data and drops are copied back when it returns Some(true)
    /// (Some(false): handled, nothing kept; None: not handled). False when the
    /// open screen is not one the engine models or `run` did not handle it.
    fn menu_session(
        &mut self,
        run: impl FnOnce(
            &mut horizon_client::menu::Menu,
            &horizon_client::menu::Env,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        use horizon_client::menu::{Env, Kind, Menu};
        #[derive(Clone, Copy)]
        struct Map {
            container: usize,
            table: bool,
            cells: usize,
            plain: bool,
        }
        impl Map {
            fn main(&self) -> usize {
                match (self.plain, self.table) {
                    (false, _) => self.container,
                    (true, true) => 10,
                    (true, false) => 9,
                }
            }
            fn to_menu(self, g: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match g {
                        200 => Some(0),
                        100..109 if g - 100 < self.cells => Some(1 + g - 100),
                        500..504 if !self.table => Some(5 + g - 500),
                        504 if !self.table => Some(45),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                } else {
                    match g {
                        300.. if g < 300 + self.container => Some(g - 300),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                }
            }
            fn to_game(self, i: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match i {
                        0 => None,
                        1..=9 if i - 1 < self.cells => Some(100 + i - 1),
                        5..9 if !self.table => Some(500 + i - 5),
                        45 if !self.table => Some(504),
                        _ if i >= main && i < main + 27 => Some(9 + i - main),
                        _ if i >= main + 27 && i < main + 36 => Some(i - main - 27),
                        _ => None,
                    }
                } else if i < self.container {
                    Some(300 + i)
                } else if i < self.container + 27 {
                    Some(9 + i - self.container)
                } else {
                    Some(i - self.container - 27)
                }
            }
        }
        // Menu slot -> game slot, for every menu slot.
        let mut plain_map = None;
        let crafter_open = matches!(self.container_kind(), Some(BlockEntity::Crafter(_)));
        let loom_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Loom);
        let merchant_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Merchant);
        let stonecutter_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Stonecutter);
        let station_kind = self.station.as_ref().map(|s| s.kind);
        let grindstone_open = station_kind == Some(stations::Kind::Grindstone);
        let cartography_open = station_kind == Some(stations::Kind::Cartography);
        let anvil_open = station_kind == Some(stations::Kind::Anvil);
        let smithing_open = station_kind == Some(stations::Kind::Smithing);
        let enchanting_open = station_kind == Some(stations::Kind::Enchanting);
        let beacon_open = station_kind == Some(stations::Kind::Beacon);
        let (kind, game_of): (Kind, Vec<Option<usize>>) = if self.creative_tab_open() {
            // ItemPickerMenu: the display slots (clicked by the screen), then the hotbar.
            let mut v = vec![None; 45];
            v.extend((0..9).map(Some));
            (Kind::ItemPicker, v)
        } else if let Some(columns) = self
            .mount_menu_columns()
            .filter(|_| self.remote.is_some() && self.mount_active())
        {
            // A server's HorseInventoryMenu: the saddle, the body armour,
            // the chest, then the inventory.
            let mut v = vec![Some(mount_menu::SADDLE), Some(mount_menu::ARMOR)];
            v.extend((0..columns * 3).map(|i| Some(mount_menu::CHEST + i)));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Mount(columns), v)
        } else if merchant_open {
            // MerchantMenu: two payment slots, the result, then the inventory.
            let mut v: Vec<Option<usize>> = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Merchant, v)
        } else if crafter_open {
            // CrafterMenu: the grid, the inventory, then the result.
            let mut v: Vec<Option<usize>> = (300..309).map(Some).collect();
            v.extend((9..36).chain(0..9).map(Some));
            v.push(None);
            (Kind::CrafterBlock, v)
        } else if loom_open {
            // LoomMenu: banner, dye, pattern, result, then the inventory.
            let mut v: Vec<Option<usize>> = (0..3).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Loom, v)
        } else if stonecutter_open {
            // StonecutterMenu: the input, the result, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::RESULT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Stonecutter, v)
        } else if grindstone_open || cartography_open {
            // GrindstoneMenu and CartographyTableMenu: the two inputs, the
            // result, then the inventory.
            let mut v = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            let kind = if grindstone_open {
                Kind::Grindstone
            } else {
                Kind::Cartography
            };
            (kind, v)
        } else if enchanting_open {
            // EnchantmentMenu: the item, the lapis, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::INPUT + 1)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Enchanting, v)
        } else if beacon_open {
            // BeaconMenu: the payment, then the inventory.
            let mut v = vec![Some(stations::INPUT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Beacon, v)
        } else if anvil_open || smithing_open {
            // ItemCombinerMenu: the inputs, the result, then the inventory.
            let inputs = if anvil_open { 2 } else { 3 };
            let mut v: Vec<Option<usize>> =
                (0..inputs).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (
                if anvil_open {
                    Kind::Anvil
                } else {
                    Kind::Smithing
                },
                v,
            )
        } else {
            let (kind, map) = if self.container.is_some() {
                let size = self.open_container_size();
                let (kind, count) = match self.container_kind() {
                    Some(BlockEntity::Chest(_)) => {
                        let shulker = self.container.is_some_and(|k| {
                            let id = self.world.block(k.0, k.1, k.2);
                            storage::is_shulker_box(&self.res.facts().get(id).name)
                        });
                        let kind = match size {
                            5 => Kind::Hopper,
                            9 => Kind::Dispenser,
                            27 if shulker => Kind::Shulker,
                            n => Kind::Chest((n / 9).clamp(1, 6)),
                        };
                        (kind, size)
                    }
                    Some(BlockEntity::Brewing(_)) => (Kind::Brewing, 5),
                    Some(BlockEntity::Mod(_)) => {
                        match self
                            .remote_menu_type()
                            .and_then(horizon_client::containers::mod_menu_type)
                        {
                            Some(t) => (Kind::Mod(t), t.slots),
                            None => return false,
                        }
                    }
                    Some(BlockEntity::Furnace(f)) => (
                        Kind::Furnace(
                            match f.kind.recipe_type().trim_start_matches("minecraft:") {
                                "blasting" => "blasting",
                                "smoking" => "smoking",
                                _ => "smelting",
                            },
                        ),
                        3,
                    ),
                    _ => return false,
                };
                let map = Map {
                    container: count,
                    table: false,
                    cells: 0,
                    plain: false,
                };
                (kind, map)
            } else if self.station.is_none() && !self.mount_active() {
                let table = self.craft_size == 3;
                let map = Map {
                    container: 0,
                    table,
                    cells: self.craft_size * self.craft_size,
                    plain: true,
                };
                (
                    if table {
                        Kind::Crafting
                    } else {
                        Kind::Inventory
                    },
                    map,
                )
            } else {
                return false;
            };
            let total = Menu::new(kind).slot_count();
            plain_map = Some(map);
            (kind, (0..total).map(|i| map.to_game(i)).collect())
        };
        // The plain map also takes the crafting result slot (a shift-click on it).
        let to_menu = |g: usize| match plain_map {
            Some(map) => map.to_menu(g),
            None => game_of.iter().position(|x| *x == Some(g)),
        };
        let mut menu = Menu::new(kind);
        let menu_id = if matches!(kind, Kind::Mount(_)) {
            Some("horizon:mount_inventory")
        } else if matches!(kind, Kind::Inventory | Kind::ItemPicker) {
            None
        } else {
            self.remote_menu_type()
        };
        menu.set_registry_id(menu_id);
        menu.brewed_potion_hook = !mod_api::__private::mods().is_empty();
        if matches!(kind, Kind::Mount(_)) {
            menu.mount = self.mount_menu_entity();
        }
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        let carried = self.carried.clone();
        let creative = self.creative;
        // CreativeModeInventoryScreen clicks the menu itself and sends every
        // changed slot, which the server sets by player.
        let equipment_before = (self.screen == Screen::Creative).then(|| self.player_equipment());
        let station_menu = self.station.as_ref().map(|s| {
            (
                s.loom.selected.map_or(-1, |i| i as i32),
                s.loom.patterns.clone(),
            )
        });
        let stone = self
            .station
            .as_ref()
            .map(|s| (s.selected.map_or(-1, |i| i as i32), s.stone.clone()));
        let crafter_data = match self.container_kind() {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.triggered)),
            _ => None,
        };
        // The merchant's session, lent to the menu for this click.
        let mut session = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map(|m| (m.session.clone(), m.mob));
        if let Some((s, mob)) = session.as_mut() {
            self.merchant_sync_in(s, *mob);
        }
        // Crafting and cartography read and derive the level's maps; a
        // server's (the client menu's ContainerLevelAccess.NULL) doesn't.
        let level_maps = (matches!(kind, Kind::Cartography | Kind::Crafting | Kind::Inventory)
            && self.remote.is_none())
        .then(|| {
            before
                .iter()
                .flatten()
                .chain(self.inventory.iter().flatten())
                .chain(self.carried.iter())
                .cloned()
                .collect::<Vec<_>>()
        })
        .filter(|held| {
            kind == Kind::Cartography
                || held
                    .iter()
                    .filter_map(maps::map_id)
                    .any(|id| self.map(id).is_some())
        })
        .map(|held| self.level_maps(&held));
        let next_map = if merchant_open { self.peek_map_id() } else { 0 };
        let tag = |t: &str| self.structure_tag(t);
        let maps = trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = horizon_client::merchant::TradeEnv {
            trades: &self.stations().trades,
            enchantments: &self.stations().enchantments,
            maps: Some(&maps),
        };
        let env = Env {
            player: self
                .lan_acting_uuid
                .or_else(|| self.remote.as_ref().map(|r| r.profile())),
            player_entity_id: Some(self.own_entity_id()),
            loop_context: &self.loop_context,
            items: self.items(),
            recipes: self.recipes(),
            brewing: &self.stations().brewing,
            banners: &self.stations().banners,
            station_recipes: &self.stations().recipes,
            enchantments: &self.stations().enchantments,
            names: &self.pack.names,
            trade: Some(&trade_env),
            maps: level_maps
                .as_ref()
                .map(|m| m as &dyn horizon_client::menu::Maps),
            limited_crafting: self.limited_crafting(),
            known_recipes: Some(&self.level.recipe_book),
        };
        menu.creative = creative;
        menu.selected = self.selected;
        menu.levels = self.experience.level;
        if anvil_open {
            // The rename box as AnvilScreen.onNameChanged gives it to the menu.
            menu.anvil.name = self.station.as_ref().map(|s| self.anvil_item_name(s));
        }
        for (i, stack) in before.iter().enumerate() {
            menu.set_slot(&env, i, stack.clone());
        }
        // The off hand is part of the inventory even when the menu has no slot for it.
        let offhand_mapped = game_of.contains(&Some(504));
        if !offhand_mapped {
            menu.set_player_item(40, self.offhand.clone());
        }
        menu.settle(&env);
        if kind == Kind::Loom
            && let Some((selected, patterns)) = station_menu
        {
            // The menu's state between clicks: what set_slot recomputed is replaced.
            menu.data[0] = selected;
            menu.loom_patterns = patterns;
            menu.set_slot(&env, 3, before[3].clone());
        }
        if kind == Kind::Stonecutter
            && let Some((selected, state)) = stone
        {
            // The same: the list and selection between clicks, then the result.
            menu.data[0] = selected;
            menu.stone = state;
            menu.set_slot(&env, 1, before[1].clone());
        }
        if anvil_open && let Some(s) = self.station.as_ref() {
            // The same: the cost, material cost and rename flag between clicks, as
            // the station's createResult and the mods' anvil_update left them (the
            // result came with the slots), so a take is predicted as the server makes it.
            menu.data[0] = s.anvil.cost;
            menu.anvil.repair_count = s.anvil.repair_item_count_cost;
            menu.anvil.only_renaming = s.anvil.only_renaming;
        }
        if kind == Kind::Enchanting
            && let Some(s) = self.station.as_ref()
        {
            // The offers between clicks (what copying the slots in flagged is not a change).
            menu.enchant_changed = false;
            for i in 0..3 {
                menu.data[i] = s.offers.costs[i];
                menu.data[7 + i] = s.offers.clues[i].as_ref().map_or(-1, |(_, l)| *l as i32);
            }
            menu.data[3] = self.level.xp_seed;
        }
        if let Some((disabled, triggered)) = crafter_data {
            for (i, d) in disabled.iter().enumerate() {
                menu.data[i] = i32::from(*d);
            }
            menu.data[9] = i32::from(triggered);
        }
        menu.carried = carried;
        menu.drops.clear();
        if kind == Kind::Merchant {
            // Attached after the slots were set: setting them is not a click.
            menu.merchant = session.map(|(s, _)| s);
        }
        // `FurnaceResultSlot.checkTakeAchievements` pays out on what left the output slot.
        let furnace_before = if matches!(kind, Kind::Furnace(_)) {
            menu.slot(2).cloned().map_or(0, |s| s.count)
        } else {
            0
        };
        let outcome = run(&mut menu, &env, &to_menu);
        let made = maps.made.take();
        match outcome {
            None => return false,
            Some(false) => return true,
            Some(true) => {}
        }
        // On a server the clicks only predict: the server drops, crafts and
        // makes maps itself.
        let predicting = self.remote.is_some();
        if !predicting {
            self.register_exploration_maps(made);
            for (item, count) in std::mem::take(&mut menu.crafted_counts) {
                self.award_stat("crafted", &item, count);
            }
        }
        if let Some(s) = menu.merchant.take() {
            self.merchant_sync_out(s);
        }
        let after: Vec<Option<ItemStack>> = (0..total).map(|i| menu.slot(i).cloned()).collect();
        // Only a furnace has a result slot to compare: an enchanting table has two slots.
        let furnace_after = if matches!(kind, Kind::Furnace(_)) {
            after[2].as_ref().map_or(0, |s| s.count)
        } else {
            0
        };
        let carried = menu.carried.take();
        let dropped = std::mem::take(&mut menu.dropped);
        let sounds = std::mem::take(&mut menu.sounds);
        // A bundle's sounds are the player's own (`Entity.playSound`): the
        // client plays them as it clicks, predicting or not.
        for sound in std::mem::take(&mut menu.player_sounds) {
            self.player_menu_sound(sound);
        }
        let crafted = std::mem::take(&mut menu.crafted);
        let brewed = std::mem::take(&mut menu.brewed);
        let brewed_potion_takes = std::mem::take(&mut menu.brewed_potion_takes);
        let equipped = std::mem::take(&mut menu.equipped);
        let item_crafted = std::mem::take(&mut menu.item_crafted);
        for (_, stack) in &item_crafted {
            self.tutorial_get_item(Some(stack));
        }
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        self.carried = carried;
        if !offhand_mapped {
            self.offhand = menu.player_item(40).cloned();
        }
        if !predicting {
            if !brewed_potion_takes.is_empty() {
                let (player, entity_id) = (self.native_acting_id(), self.own_entity_id());
                for stack in &brewed_potion_takes {
                    self.native_player_brewed_potion(player, entity_id, stack);
                }
            }
            for stack in dropped {
                self.player_drop(stack);
            }
            self.award_crafted(&crafted);
            for stack in &brewed {
                self.trigger_brewed_potion(stack);
            }
            if furnace_before > furnace_after
                && self.remote.is_none()
                && let (Some(pos), Some(p)) = (self.container, self.player.as_ref())
            {
                let at = [p.position.x, p.position.y, p.position.z];
                self.award_furnace_recipes(pos, at);
            }
        } else if self.screen == Screen::Creative {
            // A THROW in the creative screen with an empty hand: on an item tab's hotbar
            // `AbstractContainerMenu.doClick` calls `Player.handleCreativeModeItemDrop`, on the
            // Survival Inventory tab the screen does. The stack goes to the server, the slot
            // follows with the others (SetCreativeModeSlot). Only the server can drop it; the
            // slot has already given it up.
            for stack in dropped {
                self.remote_creative_throw(stack);
            }
        }
        if kind == Kind::Loom {
            if let Some(s) = self.station.as_mut() {
                s.loom.selected = usize::try_from(menu.data[0]).ok();
                s.loom.patterns = std::mem::take(&mut menu.loom_patterns);
            }
            // A server's loom plays its own sounds (the client's menu has no level).
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
            self.loom_screen_changed();
        } else if kind == Kind::Cartography {
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        } else if kind == Kind::Stonecutter {
            if let Some(s) = self.station.as_mut() {
                s.selected = usize::try_from(menu.data[0]).ok();
                s.stone = std::mem::take(&mut menu.stone);
                s.stonecutter_changed();
            }
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        }
        if let Some(amount) = menu.ground.take().filter(|_| !predicting)
            && let Some(pos) = self.station.as_ref().map(|s| s.pos)
        {
            self.grindstone_taken(pos, amount);
        }
        if kind == Kind::Anvil || kind == Kind::Smithing {
            self.combiner_session_end(&menu, predicting);
        }
        if kind == Kind::Enchanting && (menu.enchant_changed || before[0] != menu.slot(0).cloned())
        {
            // EnchantmentMenu.slotsChanged: new offers for the item.
            self.station_changed();
        }
        if kind == Kind::CrafterBlock
            && let Some(BlockEntity::Crafter(c)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            for (i, d) in c.disabled.iter_mut().enumerate() {
                *d = menu.data[i] == 1;
            }
        }
        if matches!(kind, Kind::Furnace(_))
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
            f.input_clicked(&self.loop_context, world_recipes!(self), before[0].clone());
        }
        match equipment_before {
            Some(before) => self.player_equipment_changed(&before),
            None => {
                for (slot, stack) in equipped {
                    self.player_equipped(slot, stack.as_ref());
                }
            }
        }
        self.apply_equipment();
        if let Some(maps) = level_maps {
            self.register_made_maps(maps);
        }
        true
    }

    fn click_inventory(&mut self, right: bool) {
        if self.mod_browser_press(right) || self.mount_click(right) {
            return;
        }
        if self.recipe_book_click(right) {
            return;
        }
        if self.station_click(right) {
            return;
        }
        if self.remote.is_some() {
            return self.remote_click(right);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size, self.settings.gui_scale_inputs());
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let held_shift = self.has_shift_down();
        if self.crafter_click(slot, held_shift, right) {
            return;
        }
        if held_shift && self.container.is_some() && self.container_quick_move(slot) {
            if let Some(key) = self.container {
                self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            }
            return;
        }
        if let Some(c) = &self.carried
            && self.refuses(slot, c)
        {
            return;
        }
        let max = |items: &Items, s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let shift = held_shift;
        if slot >= 300
            && let Some(key) = self.container
        {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        let is_furnace = matches!(self.container_kind(), Some(BlockEntity::Furnace(_)));
        if is_furnace && slot == 302 {
            // FurnaceResultSlot: take only.
            let Some(result) = self.slot_mut(302).take() else {
                return;
            };
            match &mut self.carried {
                None => self.carried = Some(result),
                Some(c)
                    if c.item == result.item
                        && c.count + result.count <= max(world_items!(self), c) =>
                {
                    c.count += result.count
                }
                _ => *self.slot_mut(302) = Some(result),
            }
            return;
        }
        // Changing the input restarts cooking (AbstractFurnaceBlockEntity.setItem).
        let furnace_input = (is_furnace && slot == 300).then(|| self.slot_mut(300).clone());
        if slot == 200 {
            // ResultSlot: take the crafted stack and consume one of each ingredient.
            let mut crafted = 0;
            while let Some((recipe, result)) = self.craft_recipe() {
                let made = (result.item.clone(), result.count);
                let taken = result.clone();
                let cells = self.craft_size * self.craft_size;
                let ingredients = self.craft_grid[..cells].to_vec();
                let remainders = world_recipes!(self).remainders(
                    &recipe,
                    &self.craft_grid[..cells],
                    self.craft_size,
                );
                if shift {
                    if self.add_stack_to_inventory(result).is_some() {
                        break;
                    }
                } else {
                    match &mut self.carried {
                        None => self.carried = Some(result),
                        Some(c)
                            if c.item == result.item
                                && c.count + result.count <= max(world_items!(self), c) =>
                        {
                            c.count += result.count
                        }
                        _ => return,
                    }
                }
                for cell in self.craft_grid.iter_mut() {
                    if let Some(s) = cell {
                        s.count -= 1;
                        if s.count == 0 {
                            *cell = None;
                        }
                    }
                }
                for (i, rest) in remainders.into_iter().enumerate() {
                    let Some(rest) = rest else { continue };
                    match &mut self.craft_grid[i] {
                        cell @ None => *cell = Some(rest),
                        Some(c) if c.same_item_same_components(&rest) => c.count += rest.count,
                        _ => self.give_or_drop(rest),
                    }
                }
                crafted += 1;
                // `ResultSlot.onTake`: the recipe and the stack taken
                // (native_item_crafted).
                let player = self.native_acting_id();
                self.native_item_crafted(player, &format!("minecraft:{recipe}"), &taken);
                self.tutorial_get_item(Some(&taken));
                self.award_stat("crafted", &made.0, made.1 as i32);
                self.award_crafted(&[(format!("minecraft:{recipe}"), ingredients)]);
                if !shift || crafted >= 64 {
                    break;
                }
            }
            return;
        }
        if shift
            && (slot < 36 || (500..504).contains(&slot) || (100..109).contains(&slot))
            && self.inventory_quick_move(slot)
        {
            return;
        }
        if (500..504).contains(&slot) {
            let wanted = ["head", "chest", "legs", "feet"][slot - 500];
            let fits = self.carried.as_ref().is_none_or(|c| {
                self.res
                    .items()
                    .get(&c.item)
                    .and_then(|i| i.equip_slot.as_deref())
                    == Some(wanted)
            });
            if !fits {
                return;
            }
            std::mem::swap(&mut self.carried, &mut self.armor[slot - 500]);
            self.apply_equipment();
            return;
        }
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let items = self.items();
        let (new_slot, new_carried) = match (carried, current) {
            (None, Some(mut s)) if right => {
                let half = s.count.div_ceil(2);
                s.count -= half;
                let taken = ItemStack {
                    count: half,
                    ..s.clone()
                };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (None, s) => (None, s),
            (Some(mut c), None) if right => {
                let one = ItemStack {
                    count: 1,
                    ..c.clone()
                };
                c.count -= 1;
                (Some(one), (c.count > 0).then_some(c))
            }
            (Some(c), None) => (Some(c), None),
            (Some(mut c), Some(mut s)) if c.same_item_same_components(&s) => {
                let room = max(items, &s).saturating_sub(s.count);
                let moved = if right {
                    1.min(room).min(c.count)
                } else {
                    room.min(c.count)
                };
                s.count += moved;
                c.count -= moved;
                (Some(s), (c.count > 0).then_some(c))
            }
            (Some(c), Some(s)) => (Some(c), Some(s)),
        };
        self.carried = new_carried;
        *self.slot_mut(slot) = new_slot;
        if let Some(before) = furnace_input
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            f.input_clicked(&self.loop_context, world_recipes!(self), before);
        }
    }
    /// `Camera.setRotation`: the camera's orientation, turned round in
    /// third-person front view.
    pub(super) fn camera_orientation(&self) -> glam::Quat {
        if self.studio_freecam.active {
            return camera_rotation(
                horizon_client::camera::CameraType::FirstPerson,
                self.studio_freecam.yaw,
                self.studio_freecam.pitch,
            ) * glam::Quat::from_rotation_z(self.studio_freecam.roll.to_radians());
        }
        match self.posed_rotation {
            // The frame's camera, already turned round in front view.
            Some((yaw, pitch)) => {
                camera_rotation(horizon_client::camera::CameraType::FirstPerson, yaw, pitch)
            }
            None => camera_rotation(self.camera_type, self.yaw, self.pitch),
        }
    }
    /// `mark_motion` ids of entities that aren't mobs or dropped items (boats, minecarts, armor
    /// stands, item frames, paintings, primed TNT, falling blocks, rockets, tridents, thrown
    /// potions): their entity id under this tag.
    pub(super) const MOTION_OBJECT: u64 = 3 << 32;
    /// An item frame's held item, apart from the frame (one id may mark one run).
    pub(super) const MOTION_FRAME_ITEM: u64 = 4 << 32;
    /// A block entity, by its position (`motion_block_entity`).
    pub(super) const MOTION_BLOCK_ENTITY: u64 = 5 << 32;
    /// A block a piston moves, by the cell it moves to (`motion_at`).
    pub(super) const MOTION_MOVING_BLOCK: u64 = 6 << 32;
    /// The player's own model (third person, and a shader pack's shadow pass).
    pub(super) const MOTION_OWN_PLAYER: u64 = 7 << 32;
    /// The entity whose geometry starts at vertex `start` (0: none), for frame
    /// generation to follow its own motion. Each run lasts until the next mark.
    pub(super) fn mark_motion(&self, start: usize, id: u64) {
        self.entity_motion.borrow_mut().push((start as u32, id));
        // Where its GPU-posed vertices start (`entity_skin`): a shader pack's `at_velocity`
        // follows those too.
        let posed = self.skin_frame.borrow().vertices;
        self.entity_motion_posed.borrow_mut().push(posed);
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        billboard_frustum: Option<horizon_client::gfx::Frustum>,
        mut out: Vec<Vertex>,
    ) -> (
        Vec<Vertex>,
        Arc<[StaticSectionSnapshot]>,
        Vec<[i32; 3]>,
        StaticCacheFrameStats,
    ) {
        // Its steps as spans (HORIZON_SPIKES): each lap names the step that just ended.
        let mut laps = horizon_client::spans::laps();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.entity_spans.borrow_mut().clear();
        self.entity_motion.borrow_mut().clear();
        self.entity_motion_posed.borrow_mut().clear();
        self.maps.slots.next_frame();
        // LevelExtractor.extract: the transient blocks queued since the last frame.
        self.drain_transient_block_queue();
        self.prepare_entity_culling();
        self.prepare_occlusion_culling(camera);
        self.prepare_trims();
        self.begin_skinning();
        self.mark_entity(out.len(), "minecraft:entity_flame");
        // One scan for both passes that draw them (block entities below and the
        // overlay near the end of this function).
        let cracks = self.cracks(camera);
        let fire_yaw = if self.camera_type == horizon_client::camera::CameraType::ThirdPersonFront {
            self.yaw + 180.0
        } else {
            self.yaw
        };
        // Packs may move vertices or draw from another camera. The guarded
        // billboard frustum covers TAA's jitter without changing model culling.
        let fire_view = billboard_frustum
            .as_ref()
            .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
        let fire_visible = |position, width, height| {
            fire_view.is_none_or(|view| {
                horizon_client::fire_render::in_view(view, camera, position, width, height)
            })
        };
        laps.lap("eg_prep");
        for m in &self.mobs {
            // Blaze.isOnFire: a charged blaze burns.
            let blazing = m.species() == horizon_client::mobs::Species::Blaze && m.extra.flag;
            if (m.is_on_fire() || blazing) && !m.removed {
                let at = m.previous.lerp(m.position, alpha);
                if !fire_visible([at.x, at.y, at.z], m.width(), m.height()) {
                    continue;
                }
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    m.width(),
                    m.height(),
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        if self.own_body_in_view()
            && let Some(p) = &self.player
            && p.fire_ticks > 0
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            if fire_visible(
                [feet.x, feet.y, feet.z],
                p.dimensions.width,
                p.dimensions.height,
            ) {
                horizon_client::fire_render::flames(
                    [feet.x, feet.y, feet.z],
                    p.dimensions.width,
                    p.dimensions.height,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        feet.x.floor() as i32,
                        feet.y.floor() as i32,
                        feet.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_mobs");
        for item in &self.dropped {
            if item.fire_ticks > 0
                && !self
                    .res
                    .items()
                    .resists_fire(&item.stack, "minecraft:in_fire")
            {
                let at = item.previous.lerp(item.position, alpha);
                if !fire_visible([at.x, at.y, at.z], 0.25, 0.25) {
                    continue;
                }
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.25,
                    0.25,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_items");
        for stand in &self.armor_stands {
            if stand.fire_ticks > 0 && !stand.marker {
                let scale = if stand.small { 0.5 } else { 1.0 };
                let at = stand.render_position(alpha);
                if !fire_visible([at.x, at.y, at.z], 0.5 * scale, 1.975 * scale) {
                    continue;
                }
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.5 * scale,
                    1.975 * scale,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        laps.lap("eg_fire_stands");
        // `ItemEntityRenderer`.
        self.mark_entity(out.len(), "minecraft:item");
        self.dropped_item_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        // `VaultRenderer`.
        self.mark_entity(out.len(), "");
        self.vault_geometry(camera, alpha, &mut out);
        // `CampfireRenderer`.
        self.mark_entity(out.len(), "");
        self.campfire_geometry(camera, &mut out);
        self.brushable_geometry(camera, &mut out);
        self.shelf_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:ominous_item_spawner");
        self.ominous_item_spawner_geometry(camera, alpha, &mut out);
        laps.lap("eg_items_misc");
        // Mobs and their projectiles (a span each).
        self.mob_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        laps.lap("eg_mobs");
        // In first person only a shader pack's shadow pass draws the player.
        self.mark_player(out.len(), !self.own_body_in_view());
        let player_start = out.len();
        self.mark_motion(player_start, Self::MOTION_OWN_PLAYER);
        let player_alpha = if self.studio_replay.is_some() {
            (self.studio_virtual_tick_ms as f64 / 50.0).clamp(0.0, 1.0)
        } else {
            self.clock.camera_fraction()
        };
        self.player_model_geometry(camera, player_alpha, &mut out);
        // A native pack's shadow pass alone gets the first-person model (no per-entity draws).
        if self.pack_ids.is_none() && !self.own_body_in_view() {
            self.shadow_only_vertices
                .borrow_mut()
                .extend(out.drain(player_start..));
        }
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:evoker_fangs");
        self.fangs_geometry(camera, alpha, &mut out);
        self.block_item_display_geometry(camera, alpha, &mut out);
        laps.lap("eg_players");
        let mut see_through = std::mem::take(&mut self.see_through_vertices);
        see_through.clear();
        self.name_tag_geometry(camera, alpha, &mut out, &mut see_through);
        self.test_instance_marker_geometry(camera, &mut out, &mut see_through);
        self.debug_renderer_geometry(camera, &mut out);
        self.text_display_geometry(camera, alpha, &mut out, &mut see_through);
        self.see_through_vertices = see_through;
        laps.lap("eg_name_tags");
        // Entity shadows (EntityRenderer.extractShadow); a native pack's sun shadows replace them.
        self.mark_translucent_entity(out.len(), "minecraft:entity_shadow");
        let pack_shadows = self.renderer.as_ref().is_some_and(|r| {
            r.pack_entity_shadows() || (self.dim == Dim::Overworld && r.native_shadow())
        });
        let shadows = self.settings.entity_shadows && self.pack.polytone_colors.entity_shadows();
        let shadow_sprite = (shadows && !pack_shadows)
            .then(|| self.gui.atlas.sprites.get("misc/shadow").copied())
            .flatten();
        if let Some(sprite) = shadow_sprite {
            // DimensionType.ambientLight: the_nether 0.1, the_end 0.25.
            let ambient = match self.dim {
                Dim::Nether => 0.1,
                Dim::End => 0.25,
                Dim::Overworld | Dim::Extra(_) => 0.0,
            };
            let darken = self.sky_darken();
            let a = alpha;
            let lerp = |p: Vec3, q: Vec3| {
                glam::DVec3::new(
                    p.x + (q.x - p.x) * a,
                    p.y + (q.y - p.y) * a,
                    p.z + (q.z - p.z) * a,
                )
            };
            let mut casters: Vec<(glam::DVec3, f32, f32)> =
                horizon_client::shadows::mob_casters(&self.mobs, a).collect();
            let item = horizon_client::shadows::shadow_params("minecraft:item");
            casters.extend(
                self.dropped
                    .iter()
                    .map(|i| (lerp(i.previous, i.position), item.0, item.1)),
            );
            let orb = horizon_client::shadows::shadow_params("minecraft:experience_orb");
            casters.extend(
                self.orbs
                    .iter()
                    .map(|o| (lerp(o.previous, o.position), orb.0, orb.1)),
            );
            // Text, block and item displays (DisplayRenderer's shadow radius).
            casters.extend(self.text_displays.values().filter_map(|d| d.shadow(a)));
            // Packs may move vertices or draw from another view.
            let shadow_view = self
                .view_frustum
                .as_ref()
                .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
            for (pos, radius, strength) in casters {
                if let Some(view) = shadow_view
                    && !horizon_client::shadows::in_view(view, camera, pos, radius, strength)
                {
                    continue;
                }
                horizon_client::shadows::shadow(
                    &self.world,
                    self.res.facts(),
                    &|state| {
                        self.res
                            .collision()
                            .cached_shapes()
                            .cached_shape(state)
                            .ok()
                            .flatten()
                            .and_then(|shape| shape.bounds())
                    },
                    sprite,
                    camera,
                    pos,
                    radius,
                    strength,
                    ambient,
                    darken,
                    &mut out,
                );
            }
        }
        laps.lap("eg_shadows");
        // Block entities (BlockEntityRenderers): chests, shulker boxes, banners, skulls...
        if let Some(r) = &self.renderer {
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| {
                let name = &*self.res.facts().get(BlockStateId(state)).name;
                matches!(
                    name,
                    "minecraft:beacon"
                        | "minecraft:end_gateway"
                        | "minecraft:structure_block"
                        | "minecraft:test_instance_block"
                )
            };
            let long_range = |state: u32| {
                &*self.res.facts().get(BlockStateId(state)).name == "minecraft:end_gateway"
            };
            let in_view = |p: &[i32; 3]| {
                view.is_none_or(|v| {
                    let o = (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                        - camera)
                        .as_vec3();
                    v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                })
            };
            let camera_list: Vec<&([i32; 3], u32)> = r
                .visible_block_entities_in_frustum(
                    self.settings.chunk_fade,
                    camera,
                    view,
                    &off_screen,
                    long_range,
                )
                .filter(|(block, section_inside)| {
                    let (pos, state) = **block;
                    self.res.block_entity_render().renders(state)
                        && (off_screen(state) || *section_inside || in_view(&pos))
                })
                .map(|(block, _)| block)
                .collect();
            // A shader pack's shadow pass draws the block entities of every section it draws,
            // in the view or not (`Renderer::shadow_pass_block_entities`, #8464): those the
            // view leaves out go in hidden from the camera.
            let shadow_only: Vec<&([i32; 3], u32)> =
                if self.pack_ids.is_some() && self.pack_shadow_casters.0 {
                    let listed: HashSet<[i32; 3]> = camera_list.iter().map(|b| b.0).collect();
                    r.shadow_pass_block_entities(camera)
                        .filter(|(pos, state)| {
                            !listed.contains(pos) && self.res.block_entity_render().renders(*state)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
            let shadow_only_at: HashSet<[i32; 3]> = shadow_only.iter().map(|b| b.0).collect();
            let dynamic_visible = |pos: [i32; 3], state: u32, lo: glam::DVec3, hi: glam::DVec3| {
                if off_screen(state) || shadow_only_at.contains(&pos) {
                    return true;
                }
                if let Some(view) = view
                    && !view.intersects(
                        (lo - camera - glam::DVec3::splat(0.5)).as_vec3(),
                        (hi - camera + glam::DVec3::splat(0.5)).as_vec3(),
                    )
                {
                    return false;
                }
                let name = &*self.res.facts().get(BlockStateId(state)).name;
                if self.settings.horizon.entity_culling == EntityCullingMode::Off
                    || self.pack_shadow_casters.0
                    || name == "minecraft:end_portal"
                    || name == "minecraft:end_gateway"
                    || hi.y - lo.y > 2.0
                {
                    return true;
                }
                // Culler::aabb_bounds adds 0.05 on each side. Keep its final
                // query just inside the conservative rendered-model envelope.
                let bounds = Aabb::new(
                    Vec3::new(lo.x + 0.06, lo.y + 0.06, lo.z + 0.06),
                    Vec3::new(hi.x - 0.06, hi.y - 0.06, hi.z - 0.06),
                );
                !self.block_entity_occluded(bounds)
            };
            let lid = |p: [i32; 3]| self.chest_lid(p, alpha as f32);
            let anim = |p: [i32; 3]| self.block_anim(p, alpha as f32);
            let scene = BlockEntityScene {
                camera,
                orientation: self.camera_orientation(),
                anim: &anim,
                texts: None,
                partial: alpha as f32,
                game_time: self.world.game_time,
                max_y: self.world.shape.max_y() - 1,
                world: &self.world,
                sprites: &self.gui.atlas.sprites,
                emissive: &self.gui.atlas.emissive,
                translucent_sprites: &self.gui.atlas.translucent,
                atlas_revision: self.gui.atlas.revision,
                nether: self.dim == Dim::Nether,
                lid: &lid,
                sign: &|p| match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(BlockEntity::Sign(sign)) => Some(sign),
                    _ => None,
                },
                banner: &|p| self.banner_layers_at(p),
                skull: &|p| self.skull_skin_at(p),
                glyph: &|c| self.gui.atlas.glyph_cell(c),
                glyphs: self.gui.atlas.font.epoch(),
                font: Some(&self.gui.atlas.font),
                dynamic_visible: Some(&dynamic_visible),
            };
            let visible = camera_list.into_iter().chain(shadow_only.iter().copied());
            let block_entity_cache = CacheOptions {
                shulker_boxes: self.settings.horizon.cache_shulker_boxes,
                signs: self.settings.horizon.cache_signs,
                decorated_pots: self.settings.horizon.cache_decorated_pots,
                bells: self.settings.horizon.cache_bells,
                chests: self.settings.horizon.cache_chests,
                banners: self.settings.horizon.cache_banners,
                copper_golem_statues: self.settings.horizon.cache_copper_golem_statues,
            };
            // Entity occlusion is for ordinary entity meshes. Block entities
            // already have section and per-block frustum culling; routing them
            // through the per-vertex occlusion callback both repeats that work
            // and disables their per-section GPU cache. Shader packs still need
            // per-entity span marks, so they keep the callback path.
            if self.pack_ids.is_some() {
                let section_revision = |pos| r.block_entity_section_revision(pos);
                static_block_entity_stats = self
                    .res
                    .block_entity_render()
                    .render_with_cache_options_and_callbacks(
                        &scene,
                        visible,
                        block_entity_cache,
                        r.block_entity_revision(),
                        Some(&section_revision),
                        &mut out,
                        |pos, state, start| {
                            self.mark_block_entity(start, state);
                            self.mark_motion(start, motion_block_entity(pos));
                        },
                        |pos, _, start, out| {
                            if shadow_only_at.contains(&pos) {
                                self.hide_spans_from(start, out.len());
                            }
                        },
                    );
            } else {
                let cache_candidates = r
                    .visible_block_entities_in_frustum(
                        self.settings.chunk_fade,
                        camera,
                        None,
                        |_| false,
                        |_| false,
                    )
                    .filter(|(block, _)| self.res.block_entity_render().renders(block.1))
                    .map(|(block, _)| block);
                (
                    static_block_entities,
                    visible_static_block_entity_sections,
                    static_block_entity_stats,
                ) = self
                    .res
                    .block_entity_render()
                    .render_with_gpu_cache_options(
                        &scene,
                        visible,
                        cache_candidates,
                        block_entity_cache,
                        r.block_entity_revision(),
                        |pos| r.block_entity_section_revision(pos),
                        &mut out,
                    );
            }
            // Mod block entities: their mods draw them (`native_block_entity_submit`),
            // culled by the bounds they give. A world without any (`has_mods`) has none
            // to find among its other unrendered block entities (furnaces, hoppers...).
            let mods = self
                .world
                .block_entities
                .has_mods()
                .then(|| {
                    r.visible_block_entities(self.settings.chunk_fade)
                        .filter(|(_, state)| !self.res.block_entity_render().renders(*state))
                })
                .into_iter()
                .flatten();
            let untemplated = self.untemplated_mod_block_entities(camera);
            self.mod_block_entity_geometry(
                mods.chain(untemplated.iter()),
                camera,
                alpha as f32,
                &mut out,
            );
            self.mark_stage(out.len(), "DESTROY");
            for &(pos, stage) in &cracks {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                self.res
                    .block_entity_render()
                    .render_breaking(&scene, pos, state.0, stage, &mut out);
            }
        }
        laps.lap("eg_block_entities");
        self.mark_entity(out.len(), "minecraft:experience_orb");
        let live = self.orbs.iter().map(|o| {
            let at = [
                o.previous.x + (o.position.x - o.previous.x) * alpha,
                o.previous.y + (o.position.y - o.previous.y) * alpha,
                o.previous.z + (o.position.z - o.previous.z) * alpha,
            ];
            (o, at, alpha as f32)
        });
        self.orb_geometry(camera, live, &mut out);
        laps.lap("eg_orbs");
        // Falling blocks use their baked model, including non-cubic shapes.
        self.mark_entity(out.len(), "minecraft:falling_block");
        // Packs can move vertices or draw another view; otherwise only depth
        // planes are used, which TAA's X/Y projection jitter cannot change.
        let falling_view = self
            .view_frustum
            .as_ref()
            .filter(|_| self.pack_ids.is_none() && self.native_shaders.picked.is_empty());
        let tables = &self.pack.context.tables;
        let falling_forward = self.camera_orientation() * -glam::Vec3::Z;
        for f in &self.falling {
            let p = f.model_origin(alpha, |state, x, z| tables.offset(state, x, z)) - camera;
            let o = p.as_vec3();
            // Only rear-origin models pay for the envelope/depth check. The
            // full posed-camera test still decides rejection, including models
            // whose origin is behind but whose geometry reaches into view.
            if falling_forward.dot(o) < 0.0
                && falling_view.is_some_and(|view| {
                    self.pack
                        .render
                        .model_catalog
                        .render_bounds(f.state)
                        .is_some_and(|[lo, hi]| {
                            let lo = glam::Vec3::from_array(lo) + o;
                            let hi = glam::Vec3::from_array(hi) + o;
                            let magnitude = o
                                .abs()
                                .max_element()
                                .max(lo.abs().max_element())
                                .max(hi.abs().max_element());
                            let padding =
                                glam::Vec3::splat(1.0e-4 + magnitude * f32::EPSILON * 4.0);
                            !view.intersects_depth(lo - padding, hi + padding)
                        })
                })
            {
                continue;
            }
            // `FallingBlockEntity.blockPosition()`: the cell its feet are in.
            let (x, y, z) = (
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            );
            if !falling_block_visible(f.state, self.world.block(x, y, z)) {
                continue;
            }
            let Some((_, tint)) = self.falling_block_face_sprite(f.state) else {
                continue;
            };
            let light = self.world.light(x, y + 1, z);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let start = out.len();
            self.mark_motion(start, Self::MOTION_OBJECT | u64::from(f.id));
            self.block_model_vertices(
                f.state,
                horizon_core::random::position_seed(f.start_pos),
                glam::Mat4::from_translation(o),
                packed,
                false,
                &mut out,
            );
            let tint = horizon_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = horizon_client::hud_layout::multiply(vertex.color, tint);
            }
        }
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "");
        self.moving_piston_geometry(camera, alpha, &mut out);
        // LevelRenderer.submitTransientBlocks.
        self.transient_block_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:tnt");
        self.tnt_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:painting");
        self.painting_geometry(camera, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:item_frame");
        self.frame_geometry(camera, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:armor_stand");
        self.armor_stand_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.cushion_geometry(camera, &mut out);
        self.mark_entity(out.len(), "minecraft:oak_boat");
        self.boat_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:minecart");
        self.cart_geometry(camera, alpha, &mut out);
        self.mark_motion(out.len(), 0);
        // Beams: `gbuffers_beaconbeam`, not a block entity's program (5.4 has no stage for them).
        self.mark_stage(out.len(), "NONE");
        self.beacon_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:fishing_bobber");
        self.fishing_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:leash_knot");
        self.leash_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:trident");
        self.trident_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:firework_rocket");
        self.rocket_geometry(camera, alpha, &mut out);
        self.mark_entity(out.len(), "minecraft:splash_potion");
        self.thrown_potion_geometry(camera, alpha, &mut out);
        laps.lap("eg_objects");
        // Breaking progress overlays (destroy_stage_0..9): ours and the other players'.
        self.mark_stage(out.len(), "DESTROY");
        for &(pos, stage) in &cracks {
            let o = (glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - camera)
                .as_vec3();
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if self.res.facts().get(state).render_shape
                == horizon_client::blocks::RenderShape::Model
                && let Ok(models) = self
                    .pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(pos[0], pos[1], pos[2]))
            {
                let offset = self
                    .pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3]);
                let origin = o + glam::DVec3::from(offset).as_vec3();
                let translucent = models
                    .iter()
                    .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                for (_, model) in models {
                    horizon_client::breaking::geometry(model, origin, translucent, stage, &mut out);
                }
            }
        }
        laps.lap("eg_cracks");
        self.finish_async_occlusion_culling();
        laps.lap("eg_culling_finish");
        (
            out,
            static_block_entities,
            visible_static_block_entity_sections,
            static_block_entity_stats,
        )
    }

    fn orb_geometry<'a>(
        &self,
        camera: glam::DVec3,
        orbs: impl Iterator<Item = (&'a horizon_client::entities::ExperienceOrb, [f64; 3], f32)>,
        out: &mut Vec<Vertex>,
    ) {
        if let Some(r) = self
            .gui
            .atlas
            .sprites
            .get("entity/experience/experience_orb")
            .copied()
        {
            let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
            let forward = glam::Vec3::new(
                -yaw.sin() * pitch.cos(),
                -pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
            let up = right.cross(forward).normalize_or_zero();
            for (o, at, partial) in orbs {
                let p = glam::DVec3::new(at[0], at[1] + 0.1, at[2]) - camera;
                let icon = o.icon();
                let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
                let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
                let t = (o.age as f32 + partial) / 2.0;
                let red = ((t.sin() + 1.0) * 0.5 * 255.0) as u32;
                let blue = (((t + 4.188_79).sin() + 1.0) * 0.1 * 255.0) as u32;
                let color = 0x8000_0000 | blue << 16 | 255 << 8 | red;
                // `ExperienceOrbRenderer.getBlockLightLevel` (the block light
                // plus 7, clamped) at the block the orb's eye is in.
                let light = self.world.light(
                    o.position.x.floor() as i32,
                    o.light_probe_y().floor() as i32,
                    o.position.z.floor() as i32,
                );
                let packed = o.render_light(i32::from(light));
                let c = p.as_vec3();
                // `ExperienceOrbRenderer.submit`: the quad runs from
                // (-0.5, -0.25) to (0.5, 0.75) and is scaled by 0.3, so the
                // sprite hangs a quarter of its height below the orb's centre.
                let s = horizon_client::entities::ORB_QUAD_SCALE;
                for (k, uv) in horizon_client::entities::ORB_QUAD.iter().zip([
                    [u0, v0 + dv],
                    [u0 + du, v0 + dv],
                    [u0 + du, v0],
                    [u0, v0],
                ]) {
                    let v = c + right * (k[0] * s) + up * (k[1] * s);
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: v.to_array(),
                        uv,
                        // ExperienceOrbRenderer: `RenderTypes.entityTranslucent`; in HDR
                        // output the orbs shine (entity.wgsl fs_hdr_mark).
                        layer: horizon_client::gfx::OIT_ENTITY
                            | u32::from(horizon_client::mesher::VERY_BRIGHT) << 16,
                        color,
                        light: packed,

                        glint_uv: [0.0, 0.0],
                    });
                }
            }
        }
    }

    /// Extract every particle group after the frame's entity, screen and hand callbacks.
    /// Item model callbacks can clear during extraction too: roll back only this tail of
    /// geometry (including GPU jobs and outlines), never the frame's ordinary entities.
    fn client_particle_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        fov_degrees: f32,
        out: &mut Vec<Vertex>,
    ) {
        self.apply_mod_particle_clear();
        if self.client_particle_count() == 0 {
            return;
        }
        let first = out.len();
        let skin = self.skin_frame.borrow().mark();
        let outline_jobs = self.skin_frame.borrow().outline_jobs.len();
        let outline_vertices = self.skin_frame.borrow().outline_vertices;
        let spans = self.entity_spans.borrow().len();
        let motion = self.entity_motion.borrow().len();
        let glowing = self.glowing_vertices.borrow().len();
        self.mark_motion(out.len(), 0);
        // Particles (ParticleEngine.extract): camera-facing quads.
        self.mark_stage(out.len(), "PARTICLES");
        let world = &self.world;
        let particle_mode = self.settings.horizon.particle_culling;
        if particle_mode == horizon_client::graphics::ParticleCullingMode::Off {
            self.particles.render(
                camera,
                self.yaw,
                self.pitch,
                alpha as f32,
                &|b| world.light(b[0], b[1], b[2]),
                out,
            );
        } else {
            let frustum = self.view_frustum;
            let pixel_focal_length = self.renderer.as_ref().map_or(0.0, |renderer| {
                let tangent = (f64::from(fov_degrees).to_radians() * 0.5).tan();
                if tangent.is_finite() && tangent > 0.0 {
                    (f64::from(renderer.config.height.max(1)) / (2.0 * tangent)) as f32
                } else {
                    0.0
                }
            });
            {
                let mut particle_culling = self.particle_occlusion_context(camera);
                self.particles.render_culled(
                    camera,
                    self.yaw,
                    self.pitch,
                    alpha as f32,
                    pixel_focal_length,
                    true,
                    frustum.as_ref(),
                    &mut |min, max| particle_culling.hidden(min, max),
                    &|b| world.light(b[0], b[1], b[2]),
                    out,
                );
            }
            self.finish_async_particle_culling();
        }
        self.mark_entity(out.len(), "minecraft:item");
        self.pickup_item_geometry(camera, alpha, out);
        self.mark_motion(out.len(), 0);
        self.mark_entity(out.len(), "minecraft:arrow");
        self.pickup_arrow_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:trident");
        self.pickup_trident_geometry(camera, alpha, out);
        self.mark_entity(out.len(), "minecraft:experience_orb");
        // ItemPickupParticle: the render state extracted at partial tick 1.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            pickups::Taken::Orb(o) => Some((&**o, pickups::position(p, alpha), 1.0)),
            _ => None,
        });
        self.orb_geometry(camera, taken, out);
        self.mark_entity(out.len(), "minecraft:elder_guardian");
        self.elder_apparition_geometry(alpha, out);
        if self.mod_clear_particles.get() {
            self.apply_mod_particle_clear();
            out.truncate(first);
            {
                let mut frame = self.skin_frame.borrow_mut();
                frame.truncate(skin);
                frame.outline_jobs.truncate(outline_jobs);
                frame.outline_vertices = outline_vertices;
            }
            self.entity_spans.borrow_mut().truncate(spans);
            self.entity_motion.borrow_mut().truncate(motion);
            self.glowing_vertices.borrow_mut().truncate(glowing);
            return;
        }
        self.mark_motion(out.len(), 0);
        self.mark_stage(out.len(), "NONE");
    }

    /// `LevelRenderer.destroyBlockProgress`: a player's crack on a block (0..=9), or none
    /// (any other progress). Cracks not updated for 400 ticks are dropped.
    pub(crate) fn destroy_block_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        let now = self.world.game_time;
        self.destroying
            .retain(|_, c| now.saturating_sub(c.2) <= 400);
        if (0..10).contains(&progress) {
            self.destroying.insert(breaker, (pos, progress as u8, now));
        } else {
            self.destroying.remove(&breaker);
        }
    }

    /// `ServerLevel.destroyBlockProgress`: breaker `breaker`'s crack on `pos`
    /// (0..=9, anything else takes it away), for every player within 64 blocks.
    pub(super) fn server_destroy_block_progress(
        &mut self,
        breaker: i32,
        pos: [i32; 3],
        progress: i32,
    ) {
        self.destroy_block_progress(breaker, pos, progress);
        if let Some(host) = self.lan.as_ref() {
            host.hub.broadcast_near(
                [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                64.0,
                &horizon_protocol::packets::play::Clientbound::BlockDestruction(
                    horizon_protocol::packets::play::BlockDestruction {
                        id: breaker,
                        pos,
                        progress: if (0..10).contains(&progress) {
                            progress as u8
                        } else {
                            255
                        },
                    },
                ),
                None,
            );
        }
    }

    /// The cracks to draw and their stage sprite's atlas origin: our own mining and the
    /// other players' (`renderBlockDestroyAnimation`: within 32 blocks of the camera, the
    /// furthest progress on each block).
    fn cracks(&self, camera: glam::DVec3) -> Vec<([i32; 3], [f32; 2])> {
        // `destroy_stage_0` .. `destroy_stage_9`: the sprite name by stage, so the
        // lookup does not format a string per crack.
        const DESTROY_STAGES: [&str; 10] = [
            "block/destroy_stage_0",
            "block/destroy_stage_1",
            "block/destroy_stage_2",
            "block/destroy_stage_3",
            "block/destroy_stage_4",
            "block/destroy_stage_5",
            "block/destroy_stage_6",
            "block/destroy_stage_7",
            "block/destroy_stage_8",
            "block/destroy_stage_9",
        ];
        // `ModelBakery.DESTROY_STAGES_OIT`: with improved transparency the cracks are
        // ordinary translucent quads (`OIT_CRUMBLING`) and use their own sprites.
        const DESTROY_STAGES_OIT: [&str; 10] = [
            "block/destroy_oit_stage_0",
            "block/destroy_oit_stage_1",
            "block/destroy_oit_stage_2",
            "block/destroy_oit_stage_3",
            "block/destroy_oit_stage_4",
            "block/destroy_oit_stage_5",
            "block/destroy_oit_stage_6",
            "block/destroy_oit_stage_7",
            "block/destroy_oit_stage_8",
            "block/destroy_oit_stage_9",
        ];
        let names = if self.settings.improved_transparency {
            &DESTROY_STAGES_OIT
        } else {
            &DESTROY_STAGES
        };
        let mut stages: Vec<([i32; 3], u32)> = Vec::new();
        let mut add = |pos: [i32; 3], stage: u32| match stages.iter_mut().find(|c| c.0 == pos) {
            Some(c) => c.1 = c.1.max(stage),
            None => stages.push((pos, stage)),
        };
        if let Some(m) = &self.mining
            && let Some(stage) = m.stage()
        {
            add(m.pos, u32::from(stage));
        }
        let now = self.world.game_time;
        for (pos, stage, at) in self.destroying.values() {
            if now.saturating_sub(*at) <= 400 {
                add(*pos, u32::from(*stage));
            }
        }
        stages
            .into_iter()
            .filter(|(pos, _)| {
                let d = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                    + 0.5
                    - camera;
                d.length_squared() <= 1024.0
            })
            .filter(|(pos, _)| {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                self.res.facts().get(state).destroy_overlay
            })
            .filter_map(|(pos, stage)| {
                let name = names.get(stage as usize)?;
                let r = self.gui.atlas.sprites.get(*name)?;
                Some((pos, [r.u0, r.v0]))
            })
            .collect()
    }

    fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        if self.dim == Dim::End {
            let sky = self
                .res
                .end_environment()
                .sample(&horizon_client::environment::Sample {
                    day_time: self.world.day_time,
                    ..Default::default()
                });
            // Level.updateSkyBrightness truncates (`(int)`), see `environment::sky_darken`.
            return (15.0 - 15.0 * sky.sky_light_level) as i32;
        }
        let (rain, thunder) = self.weather_levels(1.0);
        // Sampled once per (dimension, time, weather): every mob asks each tick.
        let key = (
            self.dim,
            self.world.day_time,
            rain.to_bits(),
            thunder.to_bits(),
        );
        if let Some((k, darken)) = self.sky_darken_memo.get()
            && k == key
        {
            return darken;
        }
        let darken = self.sky_darken_in(&self.world, self.dim);
        self.sky_darken_memo.set(Some((key, darken)));
        darken
    }
    /// The mob hit along the view, the distance to it and the point the ray
    /// met its box at, measured from the mob's feet (`EntityHitResult
    /// .getLocation() - entity.position()`, the interact packet's location).
    fn mob_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        if let Some(hit) = self.tick_mob {
            return hit;
        }
        self.mob_hit_within(self.entity_reach)
    }
    /// [`Game::mob_hit`] out to `reach` instead of the entity interaction range
    /// (`ProjectileUtil.getHitResultOnViewVector` with another range).
    pub(super) fn mob_hit_within(&self, reach: f64) -> Option<(usize, f64, [f64; 3])> {
        let (eye, dir) = self.aim()?;
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, m) in self.mobs.iter().enumerate() {
            // ProjectileUtil.getEntityHitResult: the mob the player rides
            // shares its root vehicle and is never the hit, nor is the
            // camera entity the pick starts from. EntitySelector.CAN_BE_PICKED
            // is isPickable — `!isRemoved`, not alive: a health-0 fake player
            // still answers a click.
            if m.removed || Some(m.id) == self.riding || Some(m.id) == self.remote_camera_id() {
                continue;
            }
            if let Some(t) = ray_box(eye, dir, reach, m.bounds())
                && best.is_none_or(|b| t < b.1)
            {
                let point = Vec3::new(
                    eye[0] + dir[0] * t,
                    eye[1] + dir[1] * t,
                    eye[2] + dir[2] * t,
                );
                best = Some((i, t, interact_location(point, m.position)));
            }
        }
        best
    }
    /// `Player.attack`'s damage against `target_kind` (with the attack
    /// strength and whether it is a critical hit): the weapon and effects
    /// scaled by the attack strength, a mace's smash, the critical hit, then
    /// enchantments.
    pub(super) fn melee_damage(
        &mut self,
        target_kind: &str,
        living: bool,
        victim: Option<u32>,
    ) -> (f32, f32, bool, bool) {
        if self.legacy_rules() {
            return self.legacy_melee_damage(target_kind, living);
        }
        let hand_damage = self.attack_damage();
        // Riptide's spin replaces the attack damage attribute.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 { spin } else { hand_damage };
        let scale = self.attack_strength_scale();
        // getEnchantedDamage(target, damage, source) - damage, taken before the
        // attack strength scales the base damage.
        let enchant_bonus = self.enchanted_damage(target_kind, damage, victim) - damage;
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        // Player.attack decides the critical hit only for a hit with base or
        // enchantment damage: checked before the mace's smash bonus is added.
        let damaging = amount > 0.0 || enchant_bonus * scale > 0.0;
        // Item.getAttackDamageBonus (a mace's smash), before the critical hit.
        amount += self.smash_damage_bonus(target_kind);
        let vanilla_crit = living && self.critical_attack(scale);
        let mut crit = vanilla_crit;
        let mut multiplier = if vanilla_crit { 1.5 } else { 1.0 };
        if living && damaging {
            self.native_critical_hit(target_kind, vanilla_crit, &mut crit, &mut multiplier);
        }
        if crit {
            amount *= multiplier;
        }
        // Enchantment damage (sharpness, smite, bane) scaled by the attack strength.
        amount += enchant_bonus * scale;
        (amount, scale, crit, enchant_bonus * scale > 0.0)
    }
    /// `Player.isSweepAttack` with the player's speed as it is, then
    /// `native_sweep_attack`, on either side: whether the hit on `target` sweeps.
    pub(super) fn sweep_decision(&mut self, scale: f32, crit: bool, target: &str) -> bool {
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let sword = self.held().is_some_and(|s| {
            self.res
                .items()
                .item_tags
                .get("minecraft:swords")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        let vanilla = p.is_sweep_attack(scale, crit, sword);
        let (mut sweeping, mut canceled) = (vanilla, false);
        // `setCanceled` skips later mods and keeps the current `sweeping`.
        self.native_sweep_attack(target, vanilla, &mut sweeping, &mut canceled);
        sweeping
    }
    /// `Player.attack`'s `isSweepAttack`, decided before the hit as vanilla does
    /// (`crit`: the hit on a living target is critical): where the sweep plays,
    /// none without one. Reads the charge, so before the hit resets it.
    pub(super) fn sweep_before_hit(&mut self, target: &str, crit: bool) -> Option<[f64; 3]> {
        let scale = self.attack_strength_scale();
        self.sweep_attack_position(scale, crit, target)
    }
    /// `Player.attack`'s sounds when a vehicle takes the hit (it always does, and
    /// can't be hit critically): knockback when sprinting at full strength, then
    /// strong or weak, or the `sweep` decided before the hit around the vehicle's
    /// box `area`. Before the hit resets the charge (`Player.onAttack`).
    pub(super) fn vehicle_attack_sounds(
        &mut self,
        area: Aabb,
        sweep: Option<[f64; 3]>,
        magic: Option<i32>,
    ) {
        self.attack_effects(area, sweep, None, false, mace::Spared::Nothing, magic);
    }
    /// `Player.attack` on a mob: the damage, knockback, sweep, particles and
    /// sounds.
    fn attack_mob(&mut self, index: usize) {
        let attack = self.attack_of(self.player.as_ref().map_or(0, |p| p.entity_id));
        let target_kind = self.mobs[index].kind.id.clone();
        let target_kind = format!("minecraft:{}", target_kind.trim_start_matches("minecraft:"));
        let (amount, scale, crit, magic) =
            self.melee_damage(&target_kind, true, Some(self.mobs[index].id));
        let sweep = self.sweep_attack_position(scale, crit, &target_kind);
        let (knockback, knockback_attack) = self.extra_knockback(&target_kind, scale);
        self.attack_ticks = 0;
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if knockback_attack {
            self.audio
                .play("minecraft:entity.player.attack.knockback", from, 1.0, 1.0);
        }
        // LivingEntity.setLastHurtMob (tamed wolves join in).
        self.combat.last_hurt_mob = Some((self.mobs[index].id, self.world.game_time));
        let old_health = self.mobs[index].health;
        let source = match from {
            Some(f) => self.attack_source(f),
            None => horizon_client::mobs::Damage::of(horizon_client::mobs::DamageKind::Generic),
        };
        let landed = self.hurt_by_player(index, amount, source);
        if landed && old_health > 0.0 && !self.mobs[index].alive() {
            self.native_kill_event(index);
        }
        if landed {
            let victim = self.mob_entity(&self.mobs[index]);
            self.player_hit(
                &victim,
                creatures::damage_type(source.kind, true),
                None,
                amount,
            );
        }
        if let Some(p) = self.player.as_mut() {
            if p.attack_log.len() >= 64 {
                p.attack_log.remove(0);
            }
            p.attack_log.push(horizon_client::health::AttackRecord {
                game_time: p.health.game_time(),
                target: target_kind.clone(),
                id: self.mobs[index].id,
                killed: landed && !self.mobs[index].alive(),
                amount,
                scale,
                crit,
                landed,
                dealt: old_health - self.mobs[index].health,
            });
        }
        if landed {
            if knockback > 0.0 {
                let (xd, zd) = self.extra_knockback_direction();
                if self.legacy_rules() {
                    let m = &mut self.mobs[index];
                    let v = [m.velocity.x, m.velocity.y, m.velocity.z];
                    let [x, y, z] = horizon_client::legacy_combat::extra_knockback(
                        v,
                        f64::from(knockback),
                        (xd, zd),
                    );
                    m.velocity = Vec3::new(x, y, z);
                    m.needs_sync = true;
                } else {
                    let level = crate::game::entity::damage_level_of!(self, &attack);
                    self.mobs[index].knockback_effect(
                        level,
                        f64::from(knockback),
                        xd,
                        zd,
                        &source,
                        amount,
                    );
                }
                self.stop_sprint_after_extra_knockback();
            }
            let mob = &self.mobs[index];
            // Player.damageStatsAndHearts: the damage the hit actually did.
            let actual = old_health - mob.health;
            // Player.attack: crit emitter and damage indicator hearts.
            let (feet, width, height) = (
                [mob.position.x, mob.position.y, mob.position.z],
                mob.kind.width,
                mob.kind.height,
            );
            self.award_custom_stat("damage_dealt", (actual * 10.0).round() as i32);
            if crit {
                self.critical_hit(
                    self.lan_acting,
                    self.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    false,
                );
            }
            // Player.attackVisualEffects: magicCrit when the enchantments added damage.
            if magic {
                self.critical_hit(
                    self.lan_acting,
                    self.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    true,
                );
            }
            if actual > 2.0 {
                let indicator = horizon_client::particles::ParticleOptions::simple(
                    horizon_client::particles::ParticleType::DamageIndicator,
                );
                self.particles.send_particles(
                    &indicator,
                    [feet[0], feet[1] + height * 0.5, feet[2]],
                    [0.1, 0.0, 0.1],
                    0.2,
                    (actual * 0.5) as u32,
                );
                // The players a server serves are sent them too.
                self.lan_send_particles(lan_particles::QueuedParticle {
                    dimension: self.dim.id(),
                    options: indicator,
                    state: None,
                    at: [feet[0], feet[1] + height * 0.5, feet[2]],
                    spread: [0.1, 0.0, 0.1],
                    speed: [0.2; 3],
                    count: (actual * 0.5) as u32 as i32,
                    randomization: 0,
                });
            }
            self.remember_mob_player_credit(self.mobs[index].id);
            let sound = if crit {
                "minecraft:entity.player.attack.crit"
            } else if scale > 0.9 {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            // Player.playServerSideSound: at the player. A critical hit a mod
            // let sweep has both sounds (attackVisualEffects).
            if let Some(at) = sweep {
                let target = (
                    self.mobs[index].bounds(),
                    mace::Spared::Mob(self.mobs[index].id),
                );
                self.sweep_attack(at, from, Some(target), scale);
            }
            if crit || sweep.is_none() {
                self.audio.play(sound, from, 1.0, 1.0);
            }
            // The hurt or death sound the mob raised (getHurtSound / getDeathSound,
            // its voice pitch from its own random).
            let queued = std::mem::take(&mut self.mobs[index].outbox);
            let mut infested = Vec::new();
            for event in queued {
                match event {
                    horizon_client::mobs::MobEvent::Sound(s, at, volume, pitch) => {
                        self.audio
                            .play(&format!("minecraft:{s}"), Some(at), volume, pitch);
                    }
                    horizon_client::mobs::MobEvent::Infested { at, velocities } => {
                        infested.push((at, velocities))
                    }
                    other => {
                        // Left for the end of the tick.
                        self.mobs[index].outbox.push(other);
                    }
                }
            }
            for (at, velocities) in infested {
                self.spawn_infested(at, velocities);
            }
            self.weapon_hit(index);
            // Player.attack: causeFoodExhaustion(0.1F) only when the hit hurt
            // (inside `if (wasHurt)`), after damageStatsAndHearts.
            if let Some(p) = self.player.as_mut()
                && !self.creative
            {
                p.cause_food_exhaustion(0.1);
            }
        } else {
            self.audio
                .play("minecraft:entity.player.attack.nodamage", from, 1.0, 1.0);
        }
    }
    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    /// A mob explosion (`ServerLevel.explode` with `ExplosionInteraction.MOB`:
    /// blocks are kept without mobGriefing, else the mob_explosion_drop_decay
    /// rule picks decay). Mods are asked through `can_entity_grief` for
    /// `source`, the exploding entity (the rule alone without one).
    fn explode(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: explosion::ExplosionSource,
    ) -> Result<()> {
        self.explode_with(source, center, radius, false, blast)
    }
    /// `ServerLevel.explode` with `ExplosionInteraction.MOB` and the caller's fire flag.
    fn explode_mob(
        &mut self,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: explosion::ExplosionSource,
    ) -> Result<()> {
        self.explode_with(None, center, radius, fire, blast)
    }
    fn explode_with(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        blast: explosion::ExplosionSource,
    ) -> Result<()> {
        let interaction = if !self.can_entity_grief(source) {
            explosion::BlockInteraction::Keep
        } else {
            self.destroy_type(horizon_client::gamerules::vanilla::MOB_EXPLOSION_DROP_DECAY)
        };
        self.server_explode_from(center, radius, interaction, fire, blast)
    }
    /// `SulfurCube.tickFuse`: `ExplosionInteraction.TNT` (the
    /// tnt_explosion_drop_decay rule) with mobGriefing, `NONE` without.
    fn explode_sulfur_cube(
        &mut self,
        source: Option<u32>,
        center: [f64; 3],
        radius: f32,
        blast: explosion::ExplosionSource,
    ) -> Result<()> {
        let interaction = if !self.can_entity_grief(source) {
            explosion::BlockInteraction::Keep
        } else {
            self.destroy_type(horizon_client::gamerules::vanilla::TNT_EXPLOSION_DROP_DECAY)
        };
        self.server_explode_from(center, radius, interaction, false, blast)
    }
    /// Copies of a save batch carrying its live block entities and save data.
    fn with_entities<'a>(
        &self,
        chunks: impl IntoIterator<Item = &'a Arc<horizon_client::chunk::Chunk>>,
    ) -> Vec<Arc<horizon_client::chunk::Chunk>> {
        horizon_client::save::chunks_with_block_entities(
            chunks,
            &self.world.block_entities,
            &self.world.block_names,
        )
        .into_iter()
        .map(|mut c| {
            self.attach_save_data(&mut c);
            Arc::new(c)
        })
        .collect()
    }
    fn tick_mobs(&mut self) -> Result<()> {
        self.sync_mob_death_protection();
        self.tick_creatures()?;
        if !self.entity_attachments.is_empty() {
            self.prune_entity_attachments();
        }
        self.native_mobs_equipment();
        self.tick_equipment_attributes();
        self.position_rider();
        self.tick_player_stat_counters();
        self.tick_scoreboard();
        self.load_bossbars();
        self.tick_worldborder();
        self.tick_subtitles();
        self.spawn_phantoms();
        self.tick_advancements();
        self.tick_player_model();
        Ok(())
    }

    fn finish_open_world(&mut self, prepared: world_loading::PreparedWorld) -> Result<()> {
        self.reset_tutorial();
        // Another world: its generator, not the last one's Density line.
        self.debug_density.borrow_mut().clear();
        let world_loading::PreparedWorld {
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
        } = prepared;
        self.server_loot = loot;
        let dim = Dim::from_id(&level.dimension);
        self.level_shapes = shapes;
        self.world_root = save_root.clone();
        // The root moves before the prepared level is installed. Until then a
        // final save must not run: the lock below only marks tag tables.
        self.world_adopted = false;
        self.owner_migration = owner_migration;
        // The world's packs over the game's biomes (`Biomes::with_packs`),
        // the table its chunks were built with.
        self.world_biomes = self.packed_biomes();
        self.world_biomes_base = self.world_biomes.clone();
        horizon_client::crash::set_detail(
            "World",
            format!("{} ({})", level.name, save_root.display()),
        );
        self.end.set_locator(locator);
        self.share_spawn_conditions();
        self.score = Default::default();
        let stack = self.reload_datapacks(false, &level.enabled_features)?;
        let root = self.options.resources.clone();
        (self.world_tables, self.health_resources) =
            self.build_world_tables(&stack, false, &root, self.mods.data_switched)?;
        self.damage_tag_generation = self.damage_tag_generation.wrapping_add(1);
        self.resolve_parts()?;
        // A code-mod switch can have rebuilt the root since `Game::new`.
        self.structure_spawns = horizon_client::mobs::spawn_selection::StructureSpawns::load(&root);
        // Native tag reads use the lock to tell that the tag tables are ready.
        self.session_lock = Some(session_lock);
        // Keep the no-pack preload; a world's packs replace it atomically.
        // A later /reload keeps this session's synchronized registries.
        lan::open_world_sync_data(
            &self.options.resources,
            &self.res.core.item_data,
            &pack_files(&self.world_tables),
            &self.health_resources.damage,
        )
        .map_err(|e| format!("world registries: {e}"))?;
        // Warm the loot data with this world's packs before chunks decode
        // (`save::roll_loaded_loot` reads the cached copy).
        self.load_server_loot();
        // MinecraftServer's constructor reads CustomBossEvents before any
        // server event.
        self.bosses = Default::default();
        self.load_bossbars();
        // ServerLevel's constructor reads its Raids saved data before any
        // server event; their randoms are seeded on the first raid tick.
        if self.remote.is_none() {
            self.read_raids();
        }
        self.native_tags_updated(mod_api::TagsUpdateCause::ServerDataLoad { reload: false });
        self.announce_data_maps_updated(mod_api::DataMapsUpdateCause::ServerReload);
        self.sync_recipe_content();
        reset_world_progress(&mut self.adv, &mut self.stats);
        self.prefetch_advancement_tree(&stack);
        self.restore_player_state = true;
        self.rand_sequences = rand_sequences;
        // ServerLevel.getWorldBorder: the level in place reads its own saved
        // border when it is made, before any command (the parked levels read
        // theirs in `open_level`).
        self.border = self.load_border(dim);
        self.set_mode_id(level.mode());
        self.previous_mode = level.previous_game_mode.map(game_mode::mode_from_id);
        let mut inventory = self.load_saved_inventory(&level);
        if self.creative && level.inventory.is_empty() && level.position.is_none() {
            for (i, name) in [
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:oak_planks",
                "minecraft:glass",
                "minecraft:torch",
                "minecraft:oak_log",
                "minecraft:chest",
                "minecraft:furnace",
                "minecraft:diamond_pickaxe",
            ]
            .iter()
            .enumerate()
            {
                inventory[i] = Some(ItemStack {
                    item: Arc::from(*name),
                    count: 1,
                    damage: 0,
                    extra: None,
                });
            }
        }
        let (yaw, pitch) = self
            .options
            .look
            .unwrap_or((level.rotation[0], level.rotation[1]));
        self.yaw = yaw;
        self.pitch = pitch;
        self.selected = level.selected_slot.clamp(0, 8) as usize;
        self.streaming = streaming;
        self.world = world;
        self.inventory = inventory;
        self.save = save;
        // MinecraftServer.prepareLevels: the saved tickets come back.
        self.tickets = if self.options.execution_role.is_client() {
            Default::default()
        } else {
            horizon_client::ticket_levels::ChunkTickets::with_saved_portal(
                self.save.load_portal_tickets(),
            )
        };
        // The ride the player's file names, once the vehicle is in the world.
        self.pending_vehicle = level.root_vehicle;
        self.level = level;
        self.world_adopted = true;
        // ServerRecipeBook.loadUntrusted: recipes the manager does not have are dropped.
        let index = &world_stations!(self).recipe_index;
        let book = &mut self.level.recipe_book;
        book.recipes.retain(|r| index.contains(r));
        book.to_be_displayed.retain(|r| index.contains(r));
        self.dim = dim;
        self.audio.set_world(dim.id());
        // LevelStorageAccess.getIconFile: this save's icon.png is what the
        // Select World list shows.
        self.world_icon = Some(WorldIcon {
            path: save_root.join("icon.png"),
            last_attempt: None,
        });
        self.world_flat = horizon_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == horizon_client::save::WorldPreset::Flat);
        self.world_debug = horizon_client::save::WorldGen::read(&save_root)
            .is_some_and(|w| w.preset == horizon_client::save::WorldPreset::Debug);
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.clear_entities();
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.spawn = None;
        self.respawn_config = self
            .level
            .respawn_elsewhere
            .as_ref()
            .map(respawn_anchor::RespawnConfig::from_saved);
        self.spawn_origin = None;
        self.spawn_hook_fired = false;
        self.spawn_hook_cancelled = false;
        self.spawn_spiral = Default::default();
        self.player_spawn = None;
        self.creatures.reset_trader();
        self.creatures.cat_tick = 0;
        self.creatures.custom_spawners.clear();
        self.portals = Default::default();
        self.experience = Experience {
            level: self.level.xp_level,
            progress: self.level.xp_progress,
            total: self.level.xp_total,
        };
        self.restore_effects();
        self.mining = None;
        self.container = None;
        let kinds = std::mem::take(&mut self.be_anims.kinds);
        self.be_anims = horizon_client::block_entity_anim::BlockEntityAnims {
            kinds,
            ..Default::default()
        };
        self.carried = None;
        self.using = None;
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.streaming.sky_light = dim != Dim::Nether;
        self.streaming.nether_shading = dim == Dim::Nether;
        self.load_world_state();
        // The level the world opens in, before any of its chunks.
        self.native_level_opened(dim);
        self.open_saved_portal_levels()
    }
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    fn portal_shape(&self, start: [i32; 3], x_axis: bool) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| {
            self.res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                .clone()
        };
        let empty = |p: [i32; 3]| {
            let n = name(p);
            matches!(
                &*n,
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:nether_portal"
            )
        };
        let frame = |p: [i32; 3]| &*name(p) == "minecraft:obsidian";
        let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
        let mut bottom = start;
        let mut n = 0;
        while empty([bottom[0], bottom[1] - 1, bottom[2]]) && n < 21 {
            bottom[1] -= 1;
            n += 1;
        }
        if !frame([bottom[0], bottom[1] - 1, bottom[2]]) {
            return None;
        }
        let mut left = bottom;
        n = 0;
        while empty([left[0] - d[0], left[1], left[2] - d[2]]) && n < 21 {
            left = [left[0] - d[0], left[1], left[2] - d[2]];
            n += 1;
        }
        if !frame([left[0] - d[0], left[1], left[2] - d[2]]) {
            return None;
        }
        let mut width = 0;
        while width <= 21 {
            let p = [left[0] + d[0] * width, left[1], left[2] + d[2] * width];
            if !empty(p) || !frame([p[0], p[1] - 1, p[2]]) {
                break;
            }
            width += 1;
        }
        if !(2..=21).contains(&width)
            || !frame([left[0] + d[0] * width, left[1], left[2] + d[2] * width])
        {
            return None;
        }
        let mut height = 0;
        'rows: while height <= 21 {
            let y = left[1] + height;
            if !frame([left[0] - d[0], y, left[2] - d[2]])
                || !frame([left[0] + d[0] * width, y, left[2] + d[2] * width])
            {
                break;
            }
            for i in 0..width {
                if !empty([left[0] + d[0] * i, y, left[2] + d[2] * i]) {
                    break 'rows;
                }
            }
            height += 1;
        }
        let top_ok =
            (0..width).all(|i| frame([left[0] + d[0] * i, left[1] + height, left[2] + d[2] * i]));
        ((3..=21).contains(&height) && top_ok).then_some((left, width, height))
    }
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal,
    /// only in the Overworld and the Nether (`inPortalDimension`).
    fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        if !matches!(self.dim, Dim::Overworld | Dim::Nether) {
            return Ok(false);
        }
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.pack.assets.registry;
                let Some(default) = registry.default_state("minecraft:nether_portal") else {
                    return Ok(false);
                };
                let mut portal = registry.state(default).cloned().ok_or("portal state")?;
                portal
                    .properties
                    .insert("axis".into(), if x_axis { "x" } else { "z" }.into());
                let id = registry.resolve(&portal).unwrap_or(default);
                // PortalShape.createPortalBlocks uses BlockPos.betweenClosed:
                // X varies fastest, then Y, then Z.
                let first = BlockPos::new(origin[0], origin[1], origin[2]);
                let last = BlockPos::new(
                    origin[0] + d[0] * (width - 1),
                    origin[1] + height - 1,
                    origin[2] + d[2] * (width - 1),
                );
                for pos in BlockPos::between_closed(first, last).expect("portal size is bounded") {
                    self.level_set_with_flags(
                        [pos.x, pos.y, pos.z],
                        id,
                        level_flags::UPDATE_CLIENTS | level_flags::UPDATE_KNOWN_SHAPE,
                    )?;
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn finish_close_world(&mut self) -> Result<()> {
        self.ambience.clear(&mut self.audio);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        if self.remote.is_some() {
            self.world_icon = None;
            return self.leave_server(None);
        }
        // The save screen has finished every write before releasing storage.
        self.ui.dispatcher = Default::default();
        self.server_loot.set(None);
        self.server_items = None;
        self.session_lock = None;
        mod_api::__private::clear_game_parts();
        self.world_adopted = false;
        self.world_icon = None;
        horizon_client::crash::clear_detail("World");
        self.debug_charts.reset();
        self.boss_overlay.reset();
        // GameRenderer.resetData (Minecraft.clearClientLevel).
        self.item_activation.reset();
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.bosses = Default::default();
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        // `LocalPlayer`'s constructor: `KeyMapping.resetToggleKeys`, so a sneak or sprint toggled
        // in this world is not still on in the next one.
        self.reset_toggle_keys();
        self.sleeping = None;
        self.sleep_timer = 0;
        self.player_beds.clear();
        self.clear_entities();
        self.set_screen(Screen::Playing);
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        // Clear sync data (registries, dialog tags) so dialogs don't leak
        // between worlds.
        lan::forget_sync_data();
        lan::forget_server_sources();
        let mut title = Title::new(
            std::env::current_dir()?.join("saves"),
            &self.options.resources,
        )
        .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        self.show_title(title);
        self.capture(false);
        Ok(())
    }
    /// Let go of every entity of the level (saved first): the next world
    /// opened must not inherit them, nor save them as its own.
    pub(super) fn clear_entities(&mut self) {
        self.clear_pending_mob_actions();
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.creatures.projectiles.clear();
        self.falling.clear();
        self.transient_blocks.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.cushions.clear();
        self.item_frames.clear();
        self.paintings.clear();
        self.others.clear();
        self.dragon.discard();
        self.crystals.clear();
        self.breath_clouds.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.bolts.clear();
        self.clear_client_particles();
        self.riding = None;
        self.persist = Default::default();
        self.missing_professions.clear();
        self.entity_attachments.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.levels.clear();
        self.entity_vertices.clear();
        self.see_through_vertices.clear();
        self.glowing_vertices.get_mut().clear();
        self.shadow_only_vertices.get_mut().clear();
        self.entity_spans.borrow_mut().clear();
        self.entity_motion.borrow_mut().clear();
        self.entity_motion_posed.borrow_mut().clear();
        self.skin_models
            .borrow_mut()
            .restart(&mut self.skin_frame.borrow_mut());
        self.skin_frame.borrow_mut().clear_draws();
    }
    /// Equipment attribute modifiers (LivingEntity.collectEquipmentChanges).
    fn apply_equipment(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        for (attribute, id) in std::mem::take(&mut self.applied_modifiers) {
            let _ = player.set_modifier_unclamped(&attribute, &id, 0.0, "add_value", false);
        }
        let slots = ["head", "chest", "legs", "feet"];
        let mut equipped: Vec<(Option<&ItemStack>, &str)> = self
            .armor
            .iter()
            .zip(slots)
            .map(|(s, n)| (s.as_ref(), n))
            .collect();
        equipped.push((self.inventory[self.selected].as_ref(), "mainhand"));
        equipped.push((self.offhand.as_ref(), "offhand"));
        for (stack, slot) in equipped {
            let Some(stack) = stack else {
                continue;
            };
            for horizon_items::item_data::ModifierRow {
                attribute,
                id,
                amount,
                operation: op,
                slot: group,
            } in horizon_client::item_attributes::attribute_modifiers(stack, world_items!(self))
                .iter()
            {
                // The attack code still reads the held item's attack damage and
                // speed directly; the map copy is what `/attribute` reports.
                if !horizon_client::living_attributes::modifier_applies(group, slot) {
                    continue;
                }
                if player
                    .set_modifier_unclamped(attribute, id, *amount, op, true)
                    .is_ok()
                {
                    self.applied_modifiers.push((attribute.clone(), id.clone()));
                }
            }
        }
        let _ = player.clamp_health_to_max();
        self.apply_enchantment_attributes();
    }
    /// The armor hits among a tick's motion events (`Player.hurtArmor` from
    /// `getDamageAfterArmorAbsorb`), whatever the damage: a mob, an explosion,
    /// a cactus, lava, a campfire.
    fn wear_armor(&mut self, events: &[MotionEvent]) {
        for event in events {
            if let MotionEvent::Health(horizon_client::health::HealthEvent::ArmorHurt {
                source,
                amount,
            }) = event
            {
                self.hurt_armor(*source, *amount);
            }
        }
    }
    /// Run equipment continuations with the acting player installed on the real host.
    #[allow(clippy::too_many_arguments)]
    fn hurt_player_equipment(
        &mut self,
        source: horizon_world::damage_source::DamageTypeId,
        amount: f32,
        blocking: Option<horizon_client::health::Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        attack: &horizon_client::health::Attack,
    ) -> horizon_client::Result<horizon_client::health::BlockOutcome> {
        let rules = self.world.health_rules;
        let helmet = self.armor[0]
            .as_ref()
            .is_some_and(|s| s.count > 0 && &*s.item != "minecraft:air");
        let mut hit = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .start_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                source,
                amount,
                blocking,
                living_cause,
                causing,
                creative_source,
                helmet,
            )?;
        if let Some(damage) = hit.helmet_damage() {
            self.hurt_helmet(source, damage);
            self.refresh_armor_protection();
        }
        let armor_damage = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .select_equipment_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                &mut hit,
            )?;
        if let Some(damage) = armor_damage {
            self.hurt_armor(source, damage);
            self.refresh_armor_protection();
        }
        self.player
            .as_mut()
            .ok_or("missing damage target")?
            .finish_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.motion.health,
                rules,
                hit,
            )
    }
    /// `Player.hurtArmor`: every armor slot wears.
    fn hurt_armor(&mut self, source: horizon_world::damage_source::DamageTypeId, damage: f32) {
        self.hurt_equipment(source, damage, &[3, 2, 1, 0]);
    }
    /// `Player.hurtHelmet`: the head slot alone wears.
    fn hurt_helmet(&mut self, source: horizon_world::damage_source::DamageTypeId, damage: f32) {
        self.hurt_equipment(source, damage, &[0]);
    }
    /// `LivingEntity.doHurtEquipment` of the `slots` of `armor`: each worn piece that
    /// wears when its wearer is hurt loses max(1, damage / 4) durability, except
    /// to a fire source when the piece resists fire (`ItemStack.canBeHurtBy`).
    fn hurt_equipment(
        &mut self,
        source: horizon_world::damage_source::DamageTypeId,
        damage: f32,
        slots: &[usize],
    ) {
        if damage <= 0.0 {
            return;
        }
        let amount = if damage.is_nan() {
            0
        } else {
            (damage / 4.0).max(1.0) as i32
        };
        if amount <= 0 {
            return;
        }
        let amount = amount as u32;
        let name = self
            .health_resources
            .damage
            .definition(source)
            .map(|d| d.name.clone())
            .unwrap_or_default();
        // The pieces that may wear (`doHurtEquipment`'s checks).
        let mut eligible: [Option<ItemStack>; 4] = std::array::from_fn(|i| {
            let stack = self.armor[i].clone().filter(|_| slots.contains(&i))?;
            let info = world_items!(self).get(&stack.item)?;
            let component = |id| stack.data().and_then(|data| data.other(id));
            // ItemStack.isDamageableItem: component removal and
            // unbreakable are checked before enchantment RNG or hooks.
            if stack.count == 0
                || !info.equip_damage_on_hurt
                || component("minecraft:unbreakable").is_some()
                || component("!minecraft:max_damage").is_some()
                || component("!minecraft:damage").is_some()
                || !self.stack_can_be_hurt_by(&stack, &name).unwrap_or(true)
            {
                return None;
            }
            let max = match component("minecraft:max_damage") {
                Some(horizon_core::nbt::Tag::Int(value)) => u32::try_from(*value).ok()?,
                _ => info.max_damage,
            };
            if max == 0 {
                return None;
            }
            Some(stack)
        });
        // No equipment can wear: there is no armor damage event to dispatch.
        if eligible.iter().all(Option::is_none) {
            return;
        }
        // ArmorHurtEvent: once for the pieces that would wear, a server player's only.
        let mut amounts = [amount; 4];
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let (Some(uuid), Some(player)) = (self.lan_acting_uuid, self.lan_acting_entity)
        {
            let mut entries: Vec<mod_api::ArmorHurt> = eligible
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    Some(mod_api::ArmorHurt {
                        slot: armor_slot(i),
                        stack: e.as_ref()?.try_into_api().ok()?,
                        damage: durability_damage(damage),
                        new_damage: durability_damage(damage),
                    })
                })
                .collect();
            let hit = horizon_client::health::HurtSource {
                damage_type: &name,
                causing: 0,
                direct: 0,
                position: None,
            };
            let mut cancel = false;
            self.native_armor_hurt(
                Some(uuid),
                player,
                "minecraft:player",
                &hit,
                &mut entries,
                &mut cancel,
            );
            if cancel {
                return;
            }
            for (i, amount) in amounts.iter_mut().enumerate() {
                *amount = answered_wear(&entries, armor_slot(i), *amount);
            }
        }
        // Each break is applied before the next slot's hook and RNG draw.
        for &index in slots {
            let Some(before) = eligible[index].take() else {
                continue;
            };
            if amounts[index] == 0 || self.armor[index].as_ref() != Some(&before) {
                continue;
            }
            let (updated, broken) =
                self.damage_stack(&before, amounts[index].min(i32::MAX as u32) as i32, true);
            if updated.as_ref() != Some(&before) && self.armor[index].as_ref() == Some(&before) {
                self.armor[index] = updated;
                if let Some(stack) = broken {
                    self.player_equipment_broke(armor_slot(index), &stack);
                    self.award_stat("broken", &stack.item, 1);
                    self.apply_equipment();
                }
            }
        }
    }
    /// Voxel traversal that also stops at fluid source blocks (bucket pick).
    fn pick_fluid(&self) -> Option<Hit> {
        let (origin, dir) = self.aim()?;
        let mut t = 0.0;
        let mut last = origin.map(|v| v.floor() as i32);
        let mut first = true;
        while t <= self.reach() {
            let p = [
                origin[0] + dir[0] * t,
                origin[1] + dir[1] * t,
                origin[2] + dir[2] * t,
            ];
            let b = p.map(|v| v.floor() as i32);
            // `BlockGetter.traverseBlocks` visits the block the ray starts in
            // first: the eyes' own water, under water (#3787).
            if first || b != last {
                let state = self.world.block(b[0], b[1], b[2]);
                let f = self.res.facts().get(state);
                let face = [last[0] - b[0], last[1] - b[1], last[2] - b[2]];
                if f.fluid != Fluid::None
                    && f.fluid_level == 8
                    && !f.fluid_falling
                    && !f.waterlogged
                    && f.render_shape != horizon_client::blocks::RenderShape::Model
                {
                    if !first {
                        return Some(Hit {
                            pos: b,
                            face,
                            point: p,
                            inside: false,
                        });
                    }
                    let above = self.world.block(b[0], b[1] + 1, b[2]);
                    let same_above = self.res.facts().get(above).fluid == f.fluid;
                    if let Some((face, point)) =
                        clip_start_source(origin, dir, self.reach(), b, same_above)
                    {
                        return Some(Hit {
                            pos: b,
                            face,
                            point,
                            // A start in the shape hits at the eyes.
                            inside: point == origin,
                        });
                    }
                } else if !first && !self.res.facts().is_air(state) && f.fluid == Fluid::None {
                    // `ClipContext.Block.OUTLINE`: the ray stops at the block's
                    // outline, not its cell; it passes through the empty part
                    // of a torch, a fence or a flower to the water behind.
                    if let Some(hit) =
                        outline_stop(origin, dir, self.reach(), b, face, || self.outline_boxes(b))
                    {
                        return Some(hit);
                    }
                }
                last = b;
                first = false;
            }
            t += 0.05;
        }
        None
    }
    /// `getPlayerPOVHitResult(level, player, SOURCE_ONLY)` for an empty
    /// bucket: the block outline or a source fluid, whichever is nearer. A
    /// waterlogged block is a block hit (its outline is the block's, its
    /// fluid a source); flowing fluid has no shape to pick.
    pub(super) fn bucket_hit(&self) -> Option<Hit> {
        let origin = self.aim().map_or([0.0; 3], |(origin, _)| origin);
        let distance = |p: [f64; 3]| {
            (p[0] - origin[0]).powi(2) + (p[1] - origin[1]).powi(2) + (p[2] - origin[2]).powi(2)
        };
        match (self.pick(), self.pick_fluid()) {
            (Some(block), Some(fluid)) => Some(if distance(block.point) <= distance(fluid.point) {
                block
            } else {
                fluid
            }),
            (block, fluid) => block.or(fluid),
        }
    }
    /// `BucketItem.use`'s place position: a water bucket fills the clicked
    /// block itself when that block can hold it (`canBlockContainFluid`: a
    /// waterloggable stair, slab, fence, ..., but not a double slab, kelp or
    /// seagrass); every other bucket empties into the block the clicked face
    /// leads to. `SolidBucketItem` places like a block item, into a
    /// replaceable clicked block.
    pub(super) fn bucket_target(&self, content: BucketContent, hit: &Hit) -> [i32; 3] {
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let behaviours = self.res.behaviours();
        if content == BucketContent::Powder {
            if self.res.facts().get(clicked).replaceable {
                return hit.pos;
            }
        } else if content.is_water()
            && behaviours.is_liquid_container(clicked)
            && behaviours.can_place_liquid(
                clicked,
                horizon_world::fluid::FluidKind::Water,
                self.creative,
            )
        {
            return hit.pos;
        }
        hit.beside()
    }
    /// The cauldron at `pos`, and whether water is above it
    /// (`CauldronInteractions.isUnderWater`).
    pub(super) fn cauldron_at(
        &self,
        pos: [i32; 3],
    ) -> Option<(horizon_world::chunk::BlockState, bool)> {
        let registry = self.pack.assets.registry;
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let state = registry
            .state(id)
            .filter(|s| s.name.ends_with("cauldron"))
            .cloned()?;
        let above = self.world.block(pos[0], pos[1] + 1, pos[2]);
        let under_water = registry
            .state(above)
            .is_some_and(|s| s.name == "minecraft:water");
        Some((state, under_water))
    }

    /// What `BlockTransformer.transformBlock` would do with a held axe, hoe
    /// or shovel on the block at `pos`, without doing it: the new state, its
    /// sound, loot table, the durability it costs, its particle and its
    /// transform type.
    pub(super) fn block_transform(
        &self,
        stack: &ItemStack,
        hit: &Hit,
    ) -> Option<(BlockStateId, String, Option<String>, u32, String, String)> {
        let blocking = self
            .offhand
            .as_ref()
            .is_some_and(|s| self.blocks_attacks(s).is_some())
            && !self.keys.held(Mapping::Sneak);
        self.block_transform_in(
            stack,
            hit,
            TransformContext {
                world: &self.world,
                player: self.player.is_some().then(|| self.own_entity_id() as u32),
                hand: self.hand,
                blocking,
                simulated: false,
            },
        )
    }

    fn block_transform_in(
        &self,
        stack: &ItemStack,
        hit: &Hit,
        context: TransformContext<'_>,
    ) -> Option<(BlockStateId, String, Option<String>, u32, String, String)> {
        use horizon_client::block_transform::Around;
        let TransformContext {
            world,
            player,
            hand,
            blocking,
            simulated,
        } = context;
        let Hit { pos, face, .. } = *hit;
        struct Column<'w> {
            world: &'w World,
            registry: &'w horizon_world::registry::BlockRegistry,
            pos: [i32; 3],
        }
        impl Around for Column<'_> {
            fn state(&self, o: [i32; 3]) -> horizon_world::chunk::BlockState {
                let id =
                    self.world
                        .block(self.pos[0] + o[0], self.pos[1] + o[1], self.pos[2] + o[2]);
                self.registry.state(id).cloned().unwrap_or_else(|| {
                    horizon_world::chunk::BlockState {
                        name: "minecraft:air".into(),
                        properties: Default::default(),
                    }
                })
            }
        }
        let id = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.block_transformer.clone())?;
        // playerHasBlockingItemUseIntent: a shield in the off hand is raised
        // instead, unless sneaking.
        if blocking {
            return None;
        }
        let face_name = match face {
            [0, -1, 0] => "down",
            [0, 1, 0] => "up",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let registry = self.pack.assets.registry;
        let around = Column {
            world,
            registry,
            pos,
        };
        let vanilla = self
            .res
            .transformers()
            .apply(self.items(), registry, &id, face_name, &around)
            .and_then(|(_, t, state)| Some((registry.resolve(&state).ok()?, t)));
        let result = |new, t: &horizon_client::block_transform::Transform| {
            (
                new,
                t.sound.clone(),
                t.loot.clone(),
                t.damage,
                t.particle.clone(),
                t.transform_type.clone(),
            )
        };
        if mod_api::__private::mods().is_empty() {
            let (new, t) = vanilla?;
            return Some(result(new, t));
        }
        use mod_api::ToolAbility::*;
        let actions: &[_] = match id.as_str() {
            "minecraft:axe" => &[AxeStrip, AxeScrape, AxeWaxOff],
            "minecraft:shovel" => &[ShovelFlatten],
            "minecraft:hoe" => &[HoeTill],
            _ => return vanilla.map(|(new, t)| result(new, t)),
        };
        for &ability in actions {
            let mut state = vanilla
                .as_ref()
                .and_then(|(state, t)| vanilla_tool_state(&id, &t.particle, ability, *state));
            let matches_vanilla = state.is_some();
            let mut cancel = false;
            let tool = mod_api::ToolUse::from(mod_api::__private::built::ToolUse {
                player,
                hand,
                pos,
                face,
                click: hit.point,
                ability,
                simulated,
            });
            self.native_block_tool_modification(stack, &tool, &mut state, &mut cancel);
            if cancel {
                return None;
            }
            if let Some(new) = state {
                if matches_vanilla {
                    let (_, t) = vanilla?;
                    return Some(result(new, t));
                }
                let (sound, particle) = match ability {
                    AxeStrip => ("minecraft:item.axe.strip", "none"),
                    AxeScrape => ("minecraft:item.axe.scrape", "scrape"),
                    AxeWaxOff => ("minecraft:item.axe.wax_off", "wax_off"),
                    ShovelFlatten => ("minecraft:item.shovel.flatten", "none"),
                    HoeTill => ("minecraft:item.hoe.till", "none"),
                    ShovelDouse => unreachable!(),
                    _ => unreachable!("ToolAbility: unhandled variant"),
                };
                return Some((
                    new,
                    sound.to_owned(),
                    None,
                    1,
                    particle.to_owned(),
                    String::new(),
                ));
            }
        }
        None
    }
    /// `BlockTransformer.TransformParticle.send`: its level event.
    pub(super) fn transform_particle(&mut self, particle: &str, pos: [i32; 3]) {
        let source = self
            .lan_acting_uuid
            .map(horizon_world::level::LevelEventPlayer);
        match particle {
            "scrape" => self.level_event_from(source, 3005, pos, 0),
            "wax_on" => self.level_event_from(source, 3003, pos, 0),
            "wax_off" => self.level_event_from(source, 3004, pos, 0),
            _ => {}
        }
    }
    /// `BlockTransformer.transformBlock` for a held axe, hoe or shovel.
    fn use_block_transformer(&mut self, stack: &ItemStack, hit: &Hit) -> Result<bool> {
        let pos = hit.pos;
        let face = hit.face;
        let Some((new, sound, loot, damage, particle, transform_type)) =
            self.block_transform(stack, hit)
        else {
            return Ok(false);
        };
        // BlockTransformer.transformBlock: ITEM_USED_ON_BLOCK before the swap.
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        // A loot table drops on the clicked face, before the block changes.
        if loot.as_deref() == Some("minecraft:till/rooted_dirt") {
            self.pop_resource_from_face(pos, face, ItemStack::new("minecraft:hanging_roots", 1));
        }
        let old = self.world.block(pos[0], pos[1], pos[2]);
        self.level_set(pos, new)?;
        // BlockTransformer.TransformParticle.send.
        self.transform_particle(&particle, pos);
        // `Level.playSound(player, ...)`: the actor's client already played
        // the strip/scrape/till sound in `predict_item_on_block` (#4681).
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if self.lan_acting.is_some() {
            self.audio.play_unshared(&sound, Some(at), 1.0, 1.0);
            self.guest_player_sounds.push((sound.clone(), at, 1.0, 1.0));
        } else {
            self.audio.play(&sound, Some(at), 1.0, 1.0);
        }
        // ItemStack.useOn: a use that acted counts as one of the item.
        self.award_stat("used", &stack.item, 1);
        self.damage_held(damage);
        self.post_block_game_event("minecraft:block_change", pos, self.sculk_actor(), Some(new));
        if transform_type == "copper_chest" {
            self.transform_copper_chest_neighbour(pos, old, &particle);
        }
        Ok(true)
    }
    /// `HoneycombItem.useOn`'s `Level.playSound(player, ...)`: the actor's client
    /// already played it in `predict_item_on_block`.
    fn wax_on_sound(&mut self, at: [f64; 3]) {
        let sound = "minecraft:item.honeycomb.wax_on";
        if self.lan_acting.is_some() {
            self.audio.play_unshared(sound, Some(at), 1.0, 1.0);
            self.guest_block_sounds
                .push((sound.to_owned(), at, 1.0, 1.0));
        } else {
            self.audio.play(sound, Some(at), 1.0, 1.0);
        }
    }
    /// `BlockTransformer.transformBlock`'s `COPPER_CHEST` extra: a double
    /// chest's other half hears a block change and gets the same particle.
    fn transform_copper_chest_neighbour(
        &mut self,
        pos: [i32; 3],
        old: BlockStateId,
        particle: &str,
    ) {
        let old = self.pack.assets.registry.state(old).cloned();
        let Some(old) = old else { return };
        if !old.name.ends_with("copper_chest") {
            return;
        }
        let kind = old
            .properties
            .get("type")
            .map(String::as_str)
            .unwrap_or("single");
        let Some(facing) = old.properties.get("facing").map(String::as_str) else {
            return;
        };
        // `ChestBlock.getConnectedDirection`: LEFT is facing.getClockWise().
        let (dx, dz) = match (kind, facing) {
            ("left", "north") => (1, 0),
            ("left", "south") => (-1, 0),
            ("left", "west") => (0, -1),
            ("left", "east") => (0, 1),
            ("right", "north") => (-1, 0),
            ("right", "south") => (1, 0),
            ("right", "west") => (0, 1),
            ("right", "east") => (0, -1),
            _ => return,
        };
        let neighbour = [pos[0] + dx, pos[1], pos[2] + dz];
        let state = self.world.block(neighbour[0], neighbour[1], neighbour[2]);
        self.post_block_game_event(
            "minecraft:block_change",
            neighbour,
            self.sculk_actor(),
            Some(state),
        );
        self.transform_particle(particle, neighbour);
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.pack.assets.registry;
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let Some(default) = registry.default_state(name) else {
            return Ok(false);
        };
        let mut state = registry.state(default).cloned().ok_or("state")?;
        if let Some(previous) = registry.state(old) {
            for k in keep {
                if let Some(v) = previous.properties.get(*k) {
                    state.properties.insert((*k).to_owned(), v.clone());
                }
            }
        }
        let id = registry.resolve(&state).unwrap_or(default);
        self.level_set(pos, id)?;
        Ok(true)
    }
    /// `LayeredCauldronBlock.handleEntityOnFireInside` for a burning player
    /// in the cauldron at `pos` (`mayInteract`: not a spectator): the fill
    /// level drops (`lowerFillLevel`, a `block_change` with no source). A
    /// server's cauldron is the server's.
    pub(super) fn cauldron_extinguish(&mut self, pos: BlockPos) {
        if self.remote.is_some() {
            return;
        }
        let at = [pos.x, pos.y, pos.z];
        let registry = self.pack.assets.registry;
        let Some(old) = registry
            .state(self.world.block(at[0], at[1], at[2]))
            .filter(|s| {
                matches!(
                    s.name.as_str(),
                    "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron"
                )
            })
            .cloned()
        else {
            return;
        };
        let Ok(new) = registry.resolve(&horizon_client::cauldron::extinguished(&old)) else {
            return;
        };
        let level = |state: &horizon_world::chunk::BlockState| {
            if matches!(state.name.as_str(), "minecraft:cauldron") {
                0
            } else if state.name == "minecraft:lava_cauldron" {
                3
            } else {
                state
                    .properties
                    .get("level")
                    .and_then(|level| level.parse().ok())
                    .unwrap_or(1)
            }
        };
        let Some(new_state) = registry.state(new).cloned() else {
            return;
        };
        if !self.native_cauldron_level_change_allowed(
            pos,
            level(&old),
            level(&new_state),
            "extinguishing",
        ) {
            return;
        }
        if self.level_set(at, new).is_ok() {
            self.post_block_game_event("minecraft:block_change", at, None, Some(new));
        }
    }
    pub(super) fn damage_held(&mut self, amount: u32) {
        self.damage_held_in_slot(
            amount,
            horizon_core::equipment_slot::EquipmentSlot::MainHand,
        );
    }
    /// Hurt the acting hand's stack through ordinary wear/criterion/break handling,
    /// reporting its break in the caller's explicit equipment slot.
    pub(super) fn damage_held_in_slot(
        &mut self,
        amount: u32,
        break_slot: horizon_core::equipment_slot::EquipmentSlot,
    ) {
        let Some(stack) = self.held().cloned() else {
            return;
        };
        let (updated, broken) = self.damage_stack(&stack, amount.min(i32::MAX as u32) as i32, true);
        if self.held() != Some(&stack) {
            return;
        }
        *self.held_mut() = updated;
        if let Some(broken) = broken {
            self.award_stat("broken", &broken.item, 1);
            self.player_equipment_broke(break_slot, &broken);
        }
    }
    /// `ItemStack.hurtWithoutBreaking`: Unbreaking applies, but the stack stops at
    /// one point of durability left instead of breaking.
    fn hurt_held_without_breaking(&mut self, amount: u32) {
        if self.creative {
            return;
        }
        let Some(held) = self.held().cloned() else {
            return;
        };
        let amount = self.wear(&held, amount);
        let max = self.trident_max_damage(&held);
        if amount == 0 || max == 0 {
            return;
        }
        if let Some(stack) = self.held_mut().as_mut() {
            stack.damage = (stack.damage + amount).min(max - 1);
        }
    }
    /// Block tool use: distinguish success, failure and fall-through for hand feedback.
    fn use_tool(
        &mut self,
        stack: &ItemStack,
        allow_block_actions: bool,
    ) -> Result<horizon_client::first_person::UseResult> {
        use horizon_client::first_person::UseResult;
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        // BucketItem has no useOn: a guest's UseItemOn leaves it to UseItem.
        let bucket = horizon_client::items::bucket_contents(&stack.item);
        if matches!(self.guest_use, Some(GuestUse::OnBlock(_)))
            && (item == "bucket" || bucket.is_some())
        {
            return Ok(UseResult::Pass);
        }
        if item == "bucket" {
            let Some(hit) = self.bucket_hit() else {
                return Ok(UseResult::Pass);
            };
            // `BucketItem.use`: `level.mayInteract` and `mayUseItemAt`.
            if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
                return Ok(UseResult::Fail);
            }
            let mut canceled = false;
            self.native_fill_bucket(stack, hit.pos, &mut canceled);
            if canceled {
                return Ok(UseResult::Fail);
            }
            if self.is_powder_snow(hit.pos) {
                self.pick_up_powder_snow(hit.pos)?;
                return Ok(UseResult::Success { swing: true });
            }
            // `BucketPickup.pickupBlock`: a fluid source, a waterlogged block
            // or powder snow. A block that gives nothing fails the click.
            let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let creative = self.creative;
            let Some(filled) =
                self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, pos, creative))
            else {
                return Ok(UseResult::Fail);
            };
            self.process_level_events()?;
            // `player.awardStat(Stats.ITEM_USED.get(this))`,
            // `getPickupSound` and `level.gameEvent(player, FLUID_PICKUP)`.
            self.award_stat("used", "minecraft:bucket", 1);
            let sound = if filled == "minecraft:lava_bucket" {
                "minecraft:item.bucket.fill_lava"
            } else {
                "minecraft:item.bucket.fill"
            };
            self.audio.play(sound, Some(at(hit.pos)), 1.0, 1.0);
            self.post_block_game_event(
                "minecraft:fluid_pickup",
                hit.pos,
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            let filled = ItemStack::new(filled, 1);
            self.filled_result(filled.clone());
            self.trigger_item("minecraft:filled_bucket", &filled);
            return Ok(UseResult::Success { swing: true });
        }
        // BucketItem.use has its own ray, even when the crosshair hits an entity.
        if !allow_block_actions && bucket.is_none() {
            return Ok(UseResult::Pass);
        }
        let Some(hit) = self.pick() else {
            return Ok(UseResult::Pass);
        };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if let Some((content, sound, mob)) = bucket {
            if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
                return Ok(UseResult::Fail);
            }
            let target = self.bucket_target(content, &hit);
            let mut canceled = false;
            self.native_fill_bucket(stack, target, &mut canceled);
            // `mayUseItemAt(blockPos2)`: a block that refuses the fluid passes
            // it to the block beside, so a mod hears that block too (powder
            // snow places like a block and never goes there).
            let beside_too = content != BucketContent::Powder && hit.beside() != target;
            if !canceled && beside_too {
                self.native_fill_bucket(stack, hit.beside(), &mut canceled);
            }
            if canceled
                || self.bucket_empty_cancelled(stack, target)
                || beside_too && self.bucket_empty_cancelled(stack, hit.beside())
            {
                return Ok(UseResult::Fail);
            }
            let pos = BlockPos::new(target[0], target[1], target[2]);
            let [x, y, z] = hit.beside();
            let beside = Some(BlockPos::new(x, y, z));
            // `Player.isShiftKeyDown`: a block that refuses the fluid (any
            // non-air block under Shift) passes it to the block beside, where
            // Shift no longer counts.
            let sneaking = self.keys.held(Mapping::Sneak);
            let creative = self.creative;
            let emptied = self.with_level(|b, level, ticks| {
                b.bucket_empty(
                    level, ticks, pos, beside, content, sound, sneaking, creative,
                )
            });
            self.process_level_events()?;
            let Some(emptied) = emptied else {
                return Ok(UseResult::Fail);
            };
            if self.dimension_type().water_evaporates && content.is_water() {
                self.evaporation_smoke([emptied.x, emptied.y, emptied.z]);
            }
            // `BucketItem.emptyContents` uses `playSound(player, …)`: the
            // actor's client already played the empty / evaporate sound from
            // its DryLevel prediction (#4684).
            if self.lan_acting.is_some()
                && let Some(capture) = self.audio.capture.as_mut()
            {
                let want = sound.trim_start_matches("minecraft:");
                let mut keep = Vec::with_capacity(capture.len());
                for entry in capture.drain(..) {
                    let name = entry.event.trim_start_matches("minecraft:");
                    if name == want || name == "block.fire.extinguish" {
                        self.guest_player_sounds.push((
                            entry.event,
                            entry.position,
                            entry.volume,
                            entry.pitch,
                        ));
                    } else {
                        keep.push(entry);
                    }
                }
                *capture = keep;
            }
            if let Some(kind) = mob {
                if let Some(id) = self.spawn_mob_at(kind, (target[0], target[1], target[2]), true) {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        horizon_client::entity_nbt::apply_item_components(m, stack);
                    }
                    self.mob_placed(id)?;
                }
                self.post_block_game_event(
                    "minecraft:entity_place",
                    target,
                    self.sculk_actor(),
                    None,
                );
            }
            self.trigger_block("minecraft:placed_block", target, stack);
            if !self.creative {
                *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
            }
            return Ok(UseResult::Success { swing: true });
        }
        if self.use_block_transformer(stack, &hit)? {
            return Ok(UseResult::Success { swing: true });
        }
        if item == "honeycomb" {
            let registry = self.pack.assets.registry;
            let old = registry.state(clicked);
            if let Some(waxed) = old.and_then(|old| self.res.transformers().waxed(registry, old))
                && let Ok(state) = registry.resolve(&waxed)
            {
                let other = old.and_then(|old| storage::chest_connected_pos(old, hit.pos));
                // HoneycombItem.useOn: ITEM_USED_ON_BLOCK before the swap.
                self.trigger_block("minecraft:item_used_on_block", hit.pos, stack);
                self.level_set(hit.pos, state)?;
                let source = self
                    .lan_acting_uuid
                    .map(horizon_world::level::LevelEventPlayer);
                self.level_event_from(source, 3003, hit.pos, 0);
                self.wax_on_sound(at(hit.pos));
                // HoneycombItem.useOn's game event, and ItemStack.useOn's used statistic.
                self.post_block_game_event(
                    "minecraft:block_change",
                    hit.pos,
                    self.sculk_actor(),
                    Some(state),
                );
                // A double chest's other half: its game event and particles, and
                // a second sound from the clicked block.
                if let Some(other) = other {
                    let now = self.world.block(other[0], other[1], other[2]);
                    self.post_block_game_event(
                        "minecraft:block_change",
                        other,
                        self.sculk_actor(),
                        Some(now),
                    );
                    self.level_event_at(3003, other, 0);
                    self.wax_on_sound(at(hit.pos));
                }
                self.award_stat("used", &stack.item, 1);
                if !self.creative {
                    self.consume_held();
                }
                return Ok(UseResult::Success { swing: true });
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            if !self.may_build() && !self.can_place_on_in_adventure(hit.pos) {
                return Ok(UseResult::Fail);
            }
            let name = self.res.facts().get(clicked).name.clone();
            if self.can_light(clicked, &name) {
                let registry = self.pack.assets.registry;
                let mut state = registry
                    .state(clicked)
                    .cloned()
                    .ok_or("missing lit block state")?;
                state.properties.insert("lit".into(), "true".into());
                self.level_set(hit.pos, registry.resolve(&state)?)?;
                self.audio.play(
                    if item == "fire_charge" {
                        "minecraft:item.firecharge.use"
                    } else {
                        "minecraft:item.flintandsteel.use"
                    },
                    Some(at(hit.pos)),
                    1.0,
                    horizon_client::items::fire_starter_pitch(
                        item == "fire_charge",
                        &mut self.world.random,
                    ),
                );
                // FlintAndSteelItem / FireChargeItem.useOn: BLOCK_CHANGE at the lit block,
                // and ItemStack.useOn's used statistic.
                self.post_block_game_event(
                    "minecraft:block_change",
                    hit.pos,
                    self.sculk_actor(),
                    None,
                );
                self.award_stat("used", &stack.item, 1);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            if self.can_place_used_fire(target) {
                // With mods, the fire goes in and `BaseFireBlock.onPlace`
                // fills the frame, as vanilla's order is, so that
                // `portal_spawn` is asked once, where vanilla asks it.
                if !mod_api::__private::mods().is_empty() || !self.try_light_portal(target)? {
                    self.place_used_fire(target)?;
                }
                let sound = horizon_client::items::fire_starter_sound(item == "fire_charge");
                let pitch = horizon_client::items::fire_starter_pitch(
                    item == "fire_charge",
                    &mut self.world.random,
                );
                self.audio.play(sound, Some(at(target)), 1.0, pitch);
                // BLOCK_PLACE: flint and steel names the clicked block, a fire charge the
                // fire. Then ItemStack.useOn's used statistic.
                let placed_at = if item == "fire_charge" {
                    target
                } else {
                    hit.pos
                };
                self.post_block_game_event(
                    "minecraft:block_place",
                    placed_at,
                    self.sculk_actor(),
                    None,
                );
                self.award_stat("used", &stack.item, 1);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            return Ok(UseResult::Fail);
        }
        Ok(UseResult::Pass)
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    fn spawn_orbs(&mut self, at: [f64; 3], value: u32) {
        self.spawn_orbs_toward(at, [0.0; 3], value);
    }
    /// `ExperienceOrb.awardWithDirection`: `award` whose fresh orbs start
    /// `roughly` that way (`new ExperienceOrb(level, pos, roughly, value)`).
    fn spawn_orbs_toward(&mut self, at: [f64; 3], roughly: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            if !self.merge_new_orb(at, v, merge_id) {
                self.add_orb_toward(at, roughly, v);
            }
        }
    }
    /// `ExperienceOrb.tryMergeToExisting`: an orb of the same value within a
    /// 1 block box of `at`, whose id is a multiple of 40 from `merge_id`, takes
    /// the new one as one more in its count (and restarts its age); otherwise
    /// a fresh orb spawns.
    fn award_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) {
        if !self.merge_new_orb(at, value, merge_id) {
            self.add_orb(at, value);
        }
    }
    /// `tryMergeToExisting`: true when an orb took the new one.
    fn merge_new_orb(&mut self, at: [f64; 3], value: u32, merge_id: i32) -> bool {
        let centre = Vec3::new(at[0], at[1], at[2]);
        let half = Vec3::new(0.5, 0.5, 0.5);
        let area = Aabb::new(centre.subtract(half), centre.add_vector(half));
        // A server's orbs are its own; a LAN or server guest only shows them.
        let mut target = None;
        if self.remote.is_none() {
            for i in 0..self.orbs.len() {
                let o = &self.orbs[i];
                if !o.bounds().intersects(area) {
                    continue;
                }
                let vanilla = o.can_merge(merge_id as u32, value as i32);
                let (id, orb_value) = (o.id, o.points());
                if self.orb_merges(id, orb_value as i32, 0, value as i32, vanilla) {
                    target = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = target {
            self.orbs[i].count += 1;
            self.orbs[i].age = 0;
            true
        } else {
            false
        }
    }
    /// Whether orb `into` takes orb `from` (0: a spawning orb of `from_value`),
    /// given vanilla's `canMerge` answer: the mods may flip it, then cancel the merge.
    fn orb_merges(
        &mut self,
        into: u32,
        into_value: i32,
        from: u32,
        from_value: i32,
        vanilla: bool,
    ) -> bool {
        let dimension = self.dim.id();
        let mut can_merge = vanilla;
        self.native_experience_orb_can_merge(
            dimension,
            into,
            from,
            into_value,
            from_value,
            &mut can_merge,
        );
        if !can_merge {
            return false;
        }
        let mut canceled = false;
        self.native_experience_orb_merge(dimension, into, from, from_value, &mut canceled);
        !canceled
    }
    /// `ExperienceOrb.tick`'s scan before movement, once every twenty ticks.
    /// Hooks can change the resident list, so retain join identities, not indices.
    fn merge_orb(&mut self, i: usize) {
        if !self.orbs[i].ticks.is_multiple_of(20) {
            return;
        }
        let into_join = self.orbs[i].join;
        let area = self.orbs[i].bounds().inflate(Vec3::new(0.5, 0.5, 0.5));
        let candidates: Vec<_> = self
            .orbs
            .iter()
            .filter(|o| o.join != into_join && o.bounds().intersects(area))
            .map(|o| o.join)
            .collect();
        for from_join in candidates {
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let Some(j) = self.orbs.locate(from_join) else {
                continue;
            };
            let (a, b) = (&self.orbs[i], &self.orbs[j]);
            let (into, into_value, from, from_value, vanilla) =
                (a.id, a.value, b.id, b.value, b.can_merge(a.id, a.value));
            if !self.orb_merges(into, into_value, from, from_value, vanilla) {
                continue;
            }
            let Some(i) = self.orbs.locate(into_join) else {
                return;
            };
            let Some(j) = self.orbs.locate(from_join) else {
                continue;
            };
            // Refuse a stale decision rather than merging a changed pair.
            let (a, b) = (&self.orbs[i], &self.orbs[j]);
            if a.value != into_value
                || b.value != from_value
                || !b
                    .bounds()
                    .intersects(a.bounds().inflate(Vec3::new(0.5, 0.5, 0.5)))
            {
                continue;
            }
            let (count, age) = (b.count, b.age);
            self.orbs[i].count = self.orbs[i].count.saturating_add(count);
            self.orbs[i].age = self.orbs[i].age.min(age);
            self.orbs.remove(j);
        }
    }
    /// `Entity.unstuckIfPossible`, which `ExperienceOrb`'s constructor runs on an
    /// orb whose box is in a block: the free spot nearest its box's centre. An
    /// orb in open air keeps its position.
    fn unstuck_orb(&self, at: [f64; 3]) -> [f64; 3] {
        let b = self.res.behaviours();
        let border = self.border_bounds();
        ExperienceOrb::unstuck(Vec3::new(at[0], at[1], at[2]), &border, |p| {
            b.collision_shape(self.world.block(p.x, p.y, p.z))
                .map(|s| s.moved(Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z))))
        })
        .map_or(at, |p| [p.x, p.y, p.z])
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    fn add_orb(&mut self, at: [f64; 3], value: u32) {
        self.add_orb_toward(at, [0.0; 3], value);
    }
    /// `new ExperienceOrb(level, pos, roughly, value)`: a random motion that
    /// points against `roughly` is turned round, and the orb starts half its
    /// size (0.25) along `roughly`, unstuck if it collides there.
    fn add_orb_toward(&mut self, at: [f64; 3], roughly: [f64; 3], value: u32) {
        let len_sqr = roughly.iter().map(|v| v * v).sum::<f64>();
        // Vec3.normalize: a vector shorter than 1e-5 has no direction.
        let len = len_sqr.sqrt();
        let unit = if len < 1.0e-5 {
            [0.0; 3]
        } else {
            roughly.map(|v| v / len)
        };
        let at = self.unstuck_orb([0, 1, 2].map(|a| at[a] + unit[a] * 0.25));
        let mut r = random_source::entity();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        let mut velocity = Vec3::new(
            (r.next_double() * 0.2 - 0.1) * 2.0,
            r.next_double() * 0.2 * 2.0,
            (r.next_double() * 0.2 - 0.1) * 2.0,
        );
        let dot = roughly[0] * velocity.x + roughly[1] * velocity.y + roughly[2] * velocity.z;
        if len_sqr > 0.0 && dot < 0.0 {
            velocity = Vec3::new(-velocity.x, -velocity.y, -velocity.z);
        }
        self.next_entity += 1;
        self.orbs.push(ExperienceOrb {
            attachment_lifetime: Default::default(),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            id: self.next_entity,
            position: p,
            previous: p,
            velocity,
            // `getExperienceValue` sizes: 1 to 2477.
            value: value as i32,
            age: 0,
            on_ground: false,
            count: 1,
            ticks: 0,
            health: 5,
            following: None,
        });
        // ServerLevel.addFreshEntity.
        self.native_joins_flush(false, 0);
    }
    fn can_place_used_fire(&mut self, target: [i32; 3]) -> bool {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let direction = match ((self.yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
            0 => Direction::South,
            1 => Direction::West,
            2 => Direction::North,
            _ => Direction::East,
        };
        self.with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, pos, direction))
    }
    fn place_used_fire(&mut self, target: [i32; 3]) -> Result<()> {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let state = self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, pos));
        self.level_set(target, state)
    }
    /// `ExperienceOrb.followNearbyPlayer`'s candidates: each orb goes for the
    /// nearest living player within 8 blocks, the host's own or a guest. Each
    /// is the player's feet, the point the orb homes on and the player's UUID.
    pub(super) fn orb_followed(&self) -> Vec<xp_orb_targeting::OrbCandidate> {
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator && !self.lan_acting_fake);
        // ExperienceOrb.followNearbyPlayer: each orb goes for the nearest
        // living player within 8 blocks, the host's own or a guest.
        let uuid = self
            .level
            .player_uuid
            .map(horizon_client::lan::playerdata::ints_uuid);
        let mut followed: Vec<xp_orb_targeting::OrbCandidate> = player
            .map(|p| {
                let feet = [p.position.x, p.position.y, p.position.z];
                let eye = f64::from(p.dimensions.eye_height);
                (
                    feet,
                    [feet[0], feet[1] + eye / 2.0, feet[2]],
                    uuid,
                    p.entity_id as u32,
                )
            })
            .into_iter()
            .collect();
        if let Some(host) = self.lan.as_ref() {
            followed.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| {
                        g.placed
                            && g.participates_in_player_simulation()
                            && g.loaded
                            && !g.dead
                            && g.health > 0.0
                    })
                    .filter(|g| !g.spectator())
                    .map(|g| {
                        let p = g.position;
                        (
                            p,
                            [p[0], p[1] + g.eye_height() / 2.0, p[2]],
                            Some(g.uuid),
                            g.entity_id as u32,
                        )
                    }),
            );
        }
        followed
    }
    /// `Player.getVehicle().getBoundingBox()` for the local player's vehicle
    /// (`riding`): a mob, one of `object_target`'s kinds (boat, cart, armour
    /// stand, item frame, painting) or a tracked special — the same chain
    /// `guest_target_box` resolves a guest's vehicle through.
    fn riding_box(&self) -> Option<Aabb> {
        let id = self.riding?;
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(m.bounds());
        }
        if let Some((lo, hi, _)) = self.object_target(id as i32) {
            return Some(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ));
        }
        self.tracked_special_box(id as i32)
    }
    /// The orbs' tick, but for the movement: that is `tick_ordered_before`'s,
    /// in join order with the other kinds.
    fn tick_experience(&mut self) {
        if self.xp_delay > 0 {
            self.xp_delay -= 1;
        }
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        // Player.aiStep: standing, the pickup area is the player's box
        // inflated by (1, 0.5, 1); riding, the union with the vehicle's box
        // inflated by (1, 0, 1).
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            let own = Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ),
            );
            let area = match self.riding_box() {
                Some(vehicle) => own.union_bounds(vehicle).inflate(Vec3::new(1.0, 0.0, 1.0)),
                None => own.inflate(Vec3::new(1.0, 0.5, 1.0)),
            };
            (
                [
                    area.min_position().x,
                    area.min_position().y,
                    area.min_position().z,
                ],
                [
                    area.max_position().x,
                    area.max_position().y,
                    area.max_position().z,
                ],
            )
        });
        let mut orbs = self.orbs.lift();
        #[cfg(test)]
        entity::taken_pass_tests::after_take(self);
        let mut gained = 0;
        let mut taken = Vec::new();
        if let Some((min, max)) = bounds {
            orbs.retain_mut(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
                if touching && self.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks; a
                    // merged orb gives its value once per count.
                    self.xp_delay = 2;
                    gained += o.points();
                    taken.push(o.clone());
                    o.count -= 1;
                    return o.count > 0;
                }
                true
            });
        }
        self.orbs
            .restore(orbs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        if gained > 0 {
            gained = self.mending(gained);
        }
        for o in taken {
            // ClientPacketListener.handleTakeItemEntity: every orb taken sounds
            // at the orb, whether mending took its points or not.
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            let at = [o.position.x, o.position.y, o.position.z];
            self.audio.play(
                "minecraft:entity.experience_orb.pickup",
                Some(at),
                0.1,
                pitch,
            );
            self.add_pickup(pickups::Taken::Orb(Box::new(o)), pickups::Collector::Player);
        }
        if gained > 0 {
            self.give_experience_points(gained as i32);
        }
    }
    /// `FurnaceResultSlot.checkTakeAchievements`: the player took from the output of the
    /// furnace at `pos`, which pops the experience of what it made at them
    /// (`awardUsedRecipesAndPopExperience`) and forgets it.
    pub(super) fn award_furnace_recipes(&mut self, pos: (i32, i32, i32), at: [f64; 3]) {
        let used = match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Furnace(f)) => {
                let used = f.recipe_experience(world_recipes!(self));
                f.recipes_used.clear();
                used
            }
            _ => return,
        };
        for (count, experience) in used {
            let xp = horizon_client::containers::create_experience(
                count,
                experience,
                &mut self.world.random,
            );
            self.spawn_orbs(at, xp);
        }
    }
    /// `Player.giveExperiencePoints`: `add_experience_levels` runs once for each
    /// level gained or lost, and the mods hear of the level change once.
    pub(super) fn give_experience_points(&mut self, points: i32) {
        let Some(points) = self.xp_change(points) else {
            return;
        };
        let old_level = self.experience.level;
        self.experience.add_points(points);
        while self.experience.progress < 0.0 {
            let xp = &self.experience;
            let lost = xp.progress * horizon_client::entities::Experience::needed(xp.level) as f32;
            let had_level = xp.level > 0;
            self.add_experience_levels(-1);
            let xp = &mut self.experience;
            xp.progress = if had_level {
                1.0 + lost / horizon_client::entities::Experience::needed(xp.level) as f32
            } else {
                0.0
            };
        }
        while self.experience.progress >= 1.0 {
            let xp = &mut self.experience;
            xp.progress =
                (xp.progress - 1.0) * horizon_client::entities::Experience::needed(xp.level) as f32;
            self.add_experience_levels(1);
            let xp = &mut self.experience;
            xp.progress /= horizon_client::entities::Experience::needed(xp.level) as f32;
        }
        self.native_player_level_changed(old_level);
    }
    /// BowItem.releaseUsing: power from charge time, arrows at power * 3.
    fn release_bow(&mut self, stack: &ItemStack, ticks: u32) {
        let ammo = self.get_projectile(stack);
        let mut charge = i32::try_from(ticks).unwrap_or(i32::MAX);
        let mut canceled = false;
        self.native_arrow_loose(stack, ammo.is_some(), &mut charge, &mut canceled);
        let power = item_use::bow_release_power(ammo.is_some(), charge, canceled);
        let (Some(power), Some(ammo)) = (power, ammo) else {
            return;
        };
        let arrow_slot = ammo.slot;
        let weapon = Some(stack.clone());
        let Some(eye) = self.eye_position() else {
            return;
        };
        // BowItem.shootProjectile: shootFromRotation(power * 3, inaccuracy 1).
        let p = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let ammo = ammo.stack;
        let (power_bonus, infinity) = self.bow_enchantments(&ammo.item);
        let effect = dispenser::tipped_effect(&ammo).or_else(|| {
            (&*ammo.item == "minecraft:spectral_arrow").then_some(("minecraft:glowing", 200))
        });
        let kind = horizon_client::projectiles::ProjectileKind::Arrow {
            base_damage: 2.0 + power_bonus,
            critical: power >= 1.0,
            effect,
        };
        let mut arrow = horizon_client::projectiles::Projectile::new(
            kind,
            p,
            Some(horizon_client::mobs::Who::Player),
        );
        // ProjectileWeaponItem.useAmmo: creative and Infinity draw without
        // using the ammo, so the arrow is CREATIVE_ONLY; else setOwner(player)
        // lets it be picked up.
        if let Some(a) = arrow.arrow.as_mut() {
            a.set_item(&horizon_client::arrows::drawn_ammo(
                &ammo,
                self.creative || infinity,
            ));
            a.set_owner(Some(horizon_client::mobs::Who::Player));
            a.fired_from_weapon = weapon;
        }
        // Flame: the arrow is spawned burning (EnchantmentHelper.onProjectileSpawned).
        self.ignite_spawned_arrow(stack, &mut arrow);
        let (velocity, on_ground) = self.player.as_ref().map_or((Vec3::ZERO, true), |pl| {
            (self.known_player_movement(), pl.ground.on_ground)
        });
        arrow.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            power * 3.0,
            1.0,
            velocity,
            on_ground,
        );
        self.add_projectile(arrow);
        let (player, name, projectile) = (
            horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid()),
            self.acting_player_name().to_owned(),
            self.next_entity,
        );
        self.native_entity_shoot_bow(player, &name, projectile, power * 3.0);
        // ArrowState posts projectile_shoot on the arrow's first tick.
        let pitch = 1.0 / (self.world.random.next_float() * 0.4 + 1.2) + power * 0.5;
        // BowItem.releaseUsing: `playSound(null, getX(), getY(), getZ(), ...)`, at
        // the feet: an eye-height source a few cm from the listener pans hard.
        let feet = self
            .player
            .as_ref()
            .map_or(eye, |pl| [pl.position.x, pl.position.y, pl.position.z]);
        self.audio
            .play("minecraft:entity.arrow.shoot", Some(feet), 1.0, pitch);
        self.award_stat("used", "minecraft:bow", 1);
        if !self.creative
            && !infinity
            && let Some(i) = arrow_slot
        {
            let cell = self.stack_in_mut(i);
            if let Some(s) = cell.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *cell = None;
                }
            }
        }
        self.damage_held(1);
    }
    /// `renderPlayerHand` (`AvatarRenderer.renderHand`): the player's own
    /// `left` or right arm with its skin at pose `m`, in its rest pose
    /// turned 0.1 radians outwards. Mods are asked first (`RenderArmEvent`)
    /// and may draw at `m` or leave the arm out.
    fn first_person_arm(
        &self,
        left: bool,
        m: glam::Mat4,
        light: u32,
        partial: f32,
        out: &mut Vec<Vertex>,
    ) {
        if mod_api::__private::listened(mod_api::HookId::RenderArm) {
            let eye = self.eye_position().unwrap_or_default();
            let mut draw = BlockEntityDraw::posed(
                partial,
                self.world.game_time,
                light,
                eye.map(|c| c.floor() as i32),
                m,
                horizon_client::block_entity_render::diffuse_lights(self.dim == Dim::Nether),
                std::mem::take(out),
            );
            let mut canceled = false;
            self.native_render_arm(
                self.own_uuid(),
                self.own_entity_id(),
                left,
                &mut draw,
                &mut canceled,
            );
            *out = draw.into_vertices();
            if canceled {
                return;
            }
        }
        let slim = self.player_model.as_ref().is_some_and(|m| m.slim);
        let Some(arm) = self.res.player_arm(slim, left).as_ref() else {
            return;
        };
        // The player's own skin, as the inventory's model shows it.
        let sprites = &self.gui.atlas.sprites;
        let Some(r) = self
            .player_model
            .as_ref()
            .and_then(|m| m.skin.as_deref())
            .and_then(|skin| sprites.get(skin))
            .or_else(|| sprites.get("entity/player/wide/steve"))
            .copied()
        else {
            return;
        };
        let z_rot = if left { -0.1 } else { 0.1 };
        let pose = |p: &horizon_client::mobs::Part| {
            let mut pose = p.pose;
            pose[3] = 0.0;
            pose[4] = 0.0;
            pose[5] = if p.name == arm.name { z_rot } else { 0.0 };
            pose
        };
        horizon_client::mobs::render_model(arm, m, &pose, [r.u0, r.v0, r.u1, r.v1], light, 0, out);
    }
    /// FirstPersonHandsAndItemsRenderer: the right arm or the held items of
    /// both hands in view space, converted to camera-relative world space,
    /// and where the arm and each item start, with the `currentRenderedItemId`
    /// a shader pack reads for them.
    fn hand_geometry(&mut self, alpha: f64) -> (Vec<Vertex>, Vec<(u32, i32)>) {
        let mut out = Vec::new();
        let mut runs = Vec::new();
        // F1 hides both; a native HUD mode may keep the hands visible. The
        // replay player's hands remain visible in first person; freecam has
        // no player-view hand.
        if !fov::studio_draws_hand(
            self.studio_freecam.active,
            self.ui.hands_drawn(
                self.spectator(),
                self.sleeping.is_some(),
                self.camera_type.is_first_person(),
                self.player
                    .as_ref()
                    .is_some_and(|p| !p.health.dead_or_dying()),
                self.screen == Screen::Dead,
            ),
        ) {
            return (out, runs);
        }
        let a = alpha as f32;
        let interpolated_pitch = self.player_model.as_ref().map_or(self.pitch, |model| {
            model.previous_head_pitch + a * (self.pitch - model.previous_head_pitch)
        });
        // FirstPersonHandsAndItemsRenderer: the swing goes to the hand that
        // swings; each hand has its inverse arm height (the swap scale is 1
        // for every item but spears).
        let swing = self.swing.animation(a);
        let attacks = Hand::BOTH.map(|h| {
            if self.swing.attack_hand() == h {
                swing
            } else {
                0.0
            }
        });
        let heights = self.heights.interpolated(a);
        // The hand sway: both hands turn by a tenth of the bobs' lag.
        let to_world = self.sway.to_world(a, &self.view);
        let eye = self.eye_position().unwrap_or_default();
        let light = self.world.light(
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let shown = self.hands_to_render();
        let used_hand = self.used_hand();
        let spinning = self
            .player
            .as_ref()
            .is_some_and(|p| p.auto_spin_attack_ticks > 0);
        let items = self.visible_hand_items();
        let off_hand_empty = items[1].is_none();
        // Combat feel Legacy: a sword is held up while use is held, as 1.8.9's block looked.
        // Only the picture changes: the clicks and what they send stay 26.3's.
        let legacy_block = self.settings.extras.pvp.combat.legacy
            && self.right
            && self.screen == Screen::Playing
            && items[0]
                .as_ref()
                .is_some_and(|s| s.item.ends_with("_sword"))
            && match used_hand {
                None => true,
                Some(Hand::Off) => items[1]
                    .as_ref()
                    .is_some_and(|s| &*s.item == "minecraft:shield"),
                Some(Hand::Main) => false,
            };
        for (i, (hand, item)) in Hand::BOTH.into_iter().zip(items).enumerate() {
            if !shown[i] {
                continue;
            }
            let scale = item.as_ref().map_or(1.0, |stack| {
                self.pack
                    .item_renderer
                    .models
                    .swap_animation_scale(&horizon_client::item_render::stack_view(stack))
            });
            let attack = attacks[i];
            let inverse_height =
                horizon_client::first_person::equip_displacement(heights[i], scale);
            let snapshot = if mod_api::__private::listened(mod_api::HookId::RenderHand) {
                item.as_ref()
                    .filter(|stack| stack.count > 0)
                    .try_into_api()
                    .ok()
            } else {
                None
            };
            let mut render = mod_api::HandRender {
                player: self.own_uuid(),
                player_entity_id: self.own_entity_id(),
                hand,
                stack: snapshot.as_ref().and_then(Option::as_ref),
                partial: a,
                interpolated_pitch,
                swing: attack,
                equip: inverse_height,
                // LightCoordsUtil.pack(block, sky), as RenderHandEvent
                // gives it, not the vertex's encoding.
                packed_light: u32::from(light & 15) << 4 | u32::from(light >> 4) << 20,
                translation: [0.0; 3],
                rotation: [0.0; 3],
                scale: 1.0,
            };
            let mut canceled = false;
            if snapshot.is_some() {
                self.native_render_hand(&mut render, &mut canceled);
            }
            if canceled {
                continue;
            }
            // No mod moved the hand (always so without one): no matrices to build.
            let [x, y, z] = render.translation;
            let [pitch, yaw, roll] = render.rotation;
            let pose = [x, y, z, pitch, yaw, roll, render.scale];
            let to_world = if pose == [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
                to_world
            } else {
                to_world * native_hooks::hand_pose_matrix(pose)
            };
            // The main hand on its arm, the off hand on the other one.
            let left = self.settings.main_hand_left != (hand == Hand::Off);
            // A Legacy sword held up hides the off hand's shield.
            if hand == Hand::Off && legacy_block {
                continue;
            }
            let Some(stack) = item else {
                // renderArmWithItem: only the main hand's empty arm shows,
                // on the main arm (the left one with the main hand set to
                // left), and not while the player is invisible.
                if hand == Hand::Off || self.own_invisible() {
                    continue;
                }
                // renderPlayerArm
                let m = to_world
                    * horizon_client::first_person::player_arm_pose(left, inverse_height, attack);
                runs.push((out.len() as u32, 0));
                self.first_person_arm(left, m, packed, a, &mut out);
                continue;
            };
            let item_id = self.rendered_item_id(&stack.item);
            if let Some(id) = maps::map_id(&stack) {
                use horizon_client::first_person::{MapHold, map_arm_poses, one_handed_map_pose};
                let hold = MapHold::of(hand, off_hand_empty);
                // renderOneHandedMap and renderTwoHandedMap: the arms under
                // the map, unless the player is invisible.
                if !self.own_invisible() {
                    runs.push((out.len() as u32, 0));
                    for (arm, pose) in map_arm_poses(hold, left, self.pitch, inverse_height, attack)
                    {
                        self.first_person_arm(arm, to_world * pose, packed, a, &mut out);
                    }
                }
                runs.push((out.len() as u32, item_id));
                match hold {
                    MapHold::TwoHanded => self.two_handed_map_geometry(
                        to_world,
                        attack,
                        inverse_height,
                        id,
                        packed,
                        &mut out,
                    ),
                    MapHold::OneHanded => self.held_map(
                        to_world * one_handed_map_pose(left, inverse_height, attack),
                        id,
                        packed,
                        &mut out,
                    ),
                }
                continue;
            }
            let mut owner = self.player_owner(Some(&stack));
            // The other hand may hold an equal stack: only the used
            // hand's item is in use, only the main hand's selected.
            owner.using_this_item &= used_hand == Some(hand);
            if hand == Hand::Off {
                owner.is_selected = false;
                owner.is_carried = false;
            }
            // The hand renderer's copy of the main hand stack.
            owner.fishing_cast_with_this_item =
                self.main_hand_rod() && self.cast_rod() == Some(&stack);
            let using = owner.using_this_item && owner.use_item_remaining_ticks > 0;
            let ticks_used =
                owner.use_duration as f32 - (owner.use_item_remaining_ticks as f32 - a + 1.0);
            use horizon_client::first_person::ItemPose;
            use item_use::ItemUseState;
            let state = using.then(|| self.item_use_state(&stack));
            use horizon_core::use_animation::UseAnimation;
            let pose = match state {
                Some(ItemUseState::Spear(timing)) => ItemPose::Spear {
                    timing,
                    time_held: ticks_used,
                    ticks_since_hit: self.kinetic_hit_ticks(a),
                },
                Some(ItemUseState::Other(UseAnimation::Bow)) => ItemPose::Bow { ticks_used },
                Some(ItemUseState::Other(UseAnimation::Crossbow)) | None
                    if &*stack.item == "minecraft:crossbow"
                        || state.map(ItemUseState::animation) == Some(UseAnimation::Crossbow) =>
                {
                    let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
                    ItemPose::Crossbow {
                        ticks_used: (using && !charged).then_some(ticks_used),
                        charge_duration: owner.crossbow_charge_duration as f32,
                        // Only the main hand turns a charged crossbow aside.
                        charged: charged && hand == Hand::Main,
                    }
                }
                Some(ItemUseState::Other(UseAnimation::Trident)) => {
                    ItemPose::Trident { ticks_used }
                }
                // ItemUseAnimation.BRUSH: applyBrushTransform.
                Some(ItemUseState::Other(UseAnimation::Brush)) => ItemPose::Brush {
                    remaining: owner.use_item_remaining_ticks as f32,
                    partial: a,
                },
                Some(ItemUseState::Other(animation)) => item_use::use_pose(
                    animation,
                    owner.use_item_remaining_ticks as f32,
                    owner.use_duration as f32,
                    a,
                ),
                _ if spinning => ItemPose::SpinAttack,
                // ItemInHandRenderer: a STAB swing of this hand jabs.
                _ if self
                    .swing
                    .current
                    .is_some_and(|c| c.hand == hand && c.animation.kind == 2) =>
                {
                    ItemPose::Stab
                }
                _ => ItemPose::Swing,
            };
            let mut m = to_world
                * horizon_client::first_person::item_pose(left, inverse_height, attack, pose);
            // PvP & HUD's shield size (100% is vanilla's), toward the hand holding it.
            let shield_size = self.settings.extras.pvp.tweaks.shield_size;
            if shield_size != 100 && &*stack.item == "minecraft:shield" {
                m *= glam::Mat4::from_scale(glam::Vec3::splat(f32::from(shield_size) / 100.0));
            }
            // Seeded by the player id plus the context ordinal.
            let (context, seed) = if left {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonLeftHand,
                    3,
                )
            } else {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonRightHand,
                    4,
                )
            };
            let cit_hand = if hand == Hand::Off {
                horizon_client::cit::Hand::Off
            } else {
                horizon_client::cit::Hand::Main
            };
            let state = self.resolve_item_in_hand(&stack, context, Some(&owner), seed, cit_hand);
            // Combat feel Legacy: 1.8.9's pose for a flat item drawn with vanilla's display.
            if self.settings.extras.pvp.combat.legacy
                && !state.layers.is_empty()
                && state
                    .layers
                    .iter()
                    .all(|l| horizon_client::first_person::is_modern_flat_display(&l.matrix, left))
                && let Some(legacy) = horizon_client::first_person::legacy_item_pose(
                    left,
                    inverse_height,
                    attack,
                    pose,
                    hand == Hand::Main && legacy_block,
                )
            {
                m = to_world * legacy;
            }
            runs.push((out.len() as u32, item_id));
            let light = self.world.light(
                eye[0].floor() as i32,
                eye[1].floor() as i32,
                eye[2].floor() as i32,
            );
            self.submit_item_world(
                &state,
                m,
                block_entity_render::vertex_light(light),
                &mut out,
            );
        }
        // The hand has its own submit storage, which never uses improved
        // transparency (`LevelRenderer` alone calls `setUseImprovedTransparency`):
        // a held player head stays classic.
        for vertex in &mut out {
            vertex.layer &= !horizon_client::gfx::OIT_ENTITY;
        }
        (out, runs)
    }
    /// The renderer for the window, with the current block sprites, GUI
    /// atlas and world border texture.
    /// A new renderer's atlas holds only the pack's sprites: what was drawn into it at run time
    /// (skins, maps, the themed UI's glyphs and corners) has to be drawn again.
    pub(super) fn forget_atlas_slots(&mut self) {
        self.skin_slots.forget();
        self.maps.slots.forget();
        self.trim_slots.forget();
        self.map_patches.clear();
        self.kept_block_entities.get_mut().clear();
        horizon_client::ui_theme::reset_atlas();
    }

    fn new_renderer(&self, window: Arc<Window>) -> Result<Renderer> {
        self.renderer_for(window, &self.pack, &self.gui)
    }
    /// Rebuild GPU-owned state after the backend reports device loss.
    fn recover_device(&mut self) -> Result<()> {
        if !self.renderer.as_ref().is_some_and(Renderer::device_lost) {
            return Ok(());
        }
        // Split-screen compositors and guests share the device; rebuilding one
        // renderer alone would leave the other windows using invalid resources.
        if self.options.paced_by_split {
            return Err("the graphics device was lost during split-screen play".into());
        }
        let now = Instant::now();
        if self
            .last_device_recovery
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
        {
            return Err("the graphics device was lost again after recovery".into());
        }
        self.last_device_recovery = Some(now);
        let window = self
            .window
            .clone()
            .ok_or("no window for graphics recovery")?;
        log::warn!("Horizon: rebuilding the renderer after graphics device loss");
        // Drop the old surface and device before opening another on this window.
        let mut old = self
            .renderer
            .take()
            .ok_or("no renderer for graphics recovery")?;
        // A GPU-culled frame may be what lost it: the renderers after this one don't
        // offer GPU culling, until Horizon restarts (`graphics::lose_gpu_culling`).
        if old.gpu_frame() {
            log::warn!(
                "Horizon: GPU culling off until the next start: the graphics device was lost in one of its frames"
            );
            horizon_client::graphics::lose_gpu_culling();
        }
        let (screenshot, screenshot_result) = (old.screenshot.take(), old.screenshot_result.take());
        let display = old.display_reading();
        drop(old);
        let mut renderer = build_renderer(
            window,
            self.options.offscreen,
            &self.pack,
            &self.gui,
            None,
            None,
            self.settings.horizon.api,
            self.settings.horizon.translucent_batching,
            &self.settings.horizon,
            shader_packs::picked_pack_shape(&self.settings),
            display,
            (
                self.settings.file.int("textureFiltering"),
                self.settings.file.int("maxAnisotropyBit"),
            ),
        )?;
        (renderer.screenshot, renderer.screenshot_result) = (screenshot, screenshot_result);
        self.adopt_renderer(renderer);
        log::info!("Horizon: graphics device recovered; rebuilding world meshes");
        Ok(())
    }
    /// A renderer made again from the resources loaded takes over: what named the old one's
    /// atlas slots is forgotten, the world is meshed again, and the shader pack and the
    /// settings are applied again.
    pub(super) fn adopt_renderer(&mut self, mut renderer: Renderer) {
        // A new renderer starts with the vanilla column and without the world border's wall.
        renderer.clear_sections(self.world.layout());
        self.border_renderer.invalidate();
        self.renderer = Some(renderer);
        self.forget_atlas_slots();
        self.clear_lod();
        self.pending_uploads.clear();
        let context = self.worker_context();
        self.streaming.reload_resources(&mut self.world, context);
        self.native_shaders = native_shaders::NativeShaders::picking(&std::mem::take(
            &mut self.native_shaders.picked,
        ));
        self.apply_ui_settings();
    }
    /// `LevelExtractor.allChanged`: displayed sections and mesh work are
    /// discarded, then every loaded section is rebuilt.
    fn all_sections_changed(&mut self) {
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.clear_sections(self.world.layout());
        }
        self.streaming.reload_sections(&mut self.world);
    }
    /// Whether precipitation at a column is snow (Biome.coldEnoughToSnow:
    /// height adjusted temperature) or none (no precipitation).
    fn precipitation(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        self.res
            .biomes()
            .get(chunk.biome(x, y, z))
            .snows_at(x, y, z)
    }
    /// `Level.isRainingAt`: it rains, the sky light is 15, nothing that stops
    /// motion is above, and the biome's precipitation there is rain.
    fn raining_at(&self, p: BlockPos) -> bool {
        self.weather_levels(1.0).0 > 0.2
            && self.world.light(p.x, p.y, p.z) >> 4 == 15
            && self.rain_height(p.x, p.z) <= p.y
            && self.precipitation(p.x, p.y, p.z) == Some(false)
    }
    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block in `blocks_motion_in_heightmap` or holding a fluid.
    fn rain_height(&self, x: i32, z: i32) -> i32 {
        render_tick::motion_blocking_height(&self.world, &self.pack.assets.properties, x, z)
    }
    fn tick_weather_effects(&mut self) {
        if self.flash > 0 {
            self.flash -= 1;
        }
        // The rain sounds come with the rain splashes (`tick_particles`).
    }
    /// A translated message for the player the game acts for: `overlay` is
    /// `sendOverlayMessage` (the action bar), else `sendSystemMessage` (chat).
    /// A guest gets a packet, so its own client translates it.
    fn tell_sleeper(&mut self, key: &str, fallback: &str, overlay: bool) {
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content: horizon_protocol::text::translatable(key, &[]),
                        overlay,
                    },
                ));
        } else if overlay {
            self.message = Some((self.translate_with_fallback(key, fallback), Instant::now()));
        } else {
            self.system_message(horizon_client::text::Text::translate(key, &[]));
        }
    }
    /// ServerPlayer.startSleepInBed: in reach, not obstructed, night or thunder,
    /// and (outside creative) no monsters within 8 blocks; the bed becomes the
    /// respawn point either way.
    fn try_sleep(&mut self, pos: [i32; 3]) {
        if self.fake_blocks(mod_api::FakeEffect::Sleep) {
            return;
        }
        let Some((position, yaw)) = self.player.as_ref().map(|p| (p.position, p.yaw)) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        // The head and the foot: the block faces from the foot to the head.
        let d = facing_offset(props.get("facing").map_or("north", String::as_str));
        let (head, foot) = if props.get("part").map(String::as_str) == Some("foot") {
            ([pos[0] + d[0], pos[1], pos[2] + d[2]], pos)
        } else {
            (pos, [pos[0] - d[0], pos[1], pos[2] - d[2]])
        };
        // AbstractBedBlock.useWithoutItem: in an occupied bed a villager
        // asleep there is woken (kickVillagerOutOfBed), else someone else
        // sleeps in it; the player stays up either way.
        let head_state = self.world.block(head[0], head[1], head[2]);
        let occupied = self
            .pack
            .assets
            .registry
            .state(head_state)
            .is_some_and(|s| s.properties.get("occupied").is_some_and(|v| v == "true"));
        if occupied {
            if !self.kick_villager_out_of_bed(head) {
                self.tell_sleeper("block.minecraft.bed.occupied", "This bed is occupied", true);
            }
            return;
        }
        // ServerPlayer.bedInRange / isReachableBedBlock: within 3 blocks
        // sideways and 2 vertically of the head or the foot.
        let reachable = |bed: [i32; 3]| {
            (position.x - f64::from(bed[0]) - 0.5).abs() <= 3.0
                && (position.y - f64::from(bed[1])).abs() <= 2.0
                && (position.z - f64::from(bed[2]) - 0.5).abs() <= 3.0
        };
        let mut problem: Option<&'static str> = if !reachable(head) && !reachable(foot) {
            Some("too_far_away")
        } else {
            None
        };
        // ServerPlayer.bedBlocked: Player.freeAt (not suffocating) for the
        // blocks above the head and the foot, as `in_wall` reads suffocating.
        let blocked = problem.is_none()
            && [head, foot].iter().any(|bed| {
                let f = self
                    .res
                    .facts()
                    .get(self.world.block(bed[0], bed[1] + 1, bed[2]));
                f.full_collision && f.solid_render
            });
        if blocked {
            problem = Some("obstructed");
        }
        // In reach (bedInRange), day or night: setRespawnPosition(
        // RespawnConfig(the head, the player's yaw), not forced). It replaces
        // an anchor or a spawnpoint; respawning checks the bed is still there
        // and finds where to stand up (findRespawnAndUseSpawnBlock).
        // Straw beds never set it (`straw_bed_rule`).
        if problem.is_none() && self.bed_sets_spawn(state) {
            let config = respawn_anchor::RespawnConfig::wrapped(self.dim, head, yaw, false);
            // ServerPlayer.setRespawnPosition: the chat message only when the
            // point moved (`isSamePosition`).
            let moved = self
                .respawn_config
                .is_none_or(|c| (c.dim, c.pos) != (config.dim, config.pos));
            // `canSetSpawn` calls the setter every time, the same bed too (it
            // refreshes the angle).
            let cancel = self.spawn_point_change(self.dim, Some(head), false);
            if !cancel {
                self.respawn_config = Some(config);
                if moved {
                    self.tell_sleeper("block.minecraft.set_spawn", "Respawn point set", false);
                }
                // The fallback becomes the world spawn: a guest's is none of its
                // own, so it follows `setworldspawn`.
                self.spawn = if self.lan_acting.is_some() {
                    None
                } else {
                    self.level
                        .world_spawn
                        .map(|[x, y, z]| {
                            Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                        })
                        .or(self.spawn)
                };
            }
        }
        if problem.is_none() && !self.bed_can_sleep_now() {
            problem = Some("not_possible_now");
        }
        // Creative players skip the monster check (`!this.isCreative()`).
        if problem.is_none()
            && !self.creative
            && self.mobs.iter().any(|m| {
                // `Monster.class` entities that `isPreventingPlayerRest`: a
                // zombified piglin only while it is angry.
                horizon_client::mobs::ai::is_monster(m.species())
                    && (m.species() != horizon_client::mobs::Species::ZombifiedPiglin
                        || (m.extra.anger > 0 && m.ai.target.is_some()))
                    && m.alive()
                    && (m.position.x - position.x).abs() < 8.0
                    && (m.position.y - position.y).abs() < 5.0
                    && (m.position.z - position.z).abs() < 8.0
            })
        {
            problem = Some("not_safe");
        }
        // NeoForge's CanPlayerSleepEvent: a mod may clear or replace the
        // problem vanilla found, or add one.
        let mut problem = problem.map(str::to_owned);
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_can_player_sleep(player, &name, head, &mut problem);
        }
        if let Some(problem) = problem {
            match problem.as_str() {
                "too_far_away" => self.tell_sleeper(
                    "block.minecraft.bed.too_far_away",
                    "You may not rest now; the bed is too far away",
                    true,
                ),
                "obstructed" => self.tell_sleeper(
                    "block.minecraft.bed.obstructed",
                    "This bed is obstructed",
                    true,
                ),
                "not_safe" => self.tell_sleeper(
                    "block.minecraft.bed.not_safe",
                    "You may not rest now; there are monsters nearby",
                    true,
                ),
                "not_possible_now" => self.tell_sleeper(
                    "block.minecraft.bed.no_sleep",
                    "You can sleep only at night or during thunderstorms",
                    true,
                ),
                // `other_problem` and any name a mod invents: refused without a message.
                _ => {}
            }
            return;
        }
        // ServerPlayer.startSleepInBed: a percentage above 100 cannot skip the night.
        if self
            .ui
            .rules
            .int_at(horizon_client::gamerules::vanilla::PLAYERS_SLEEPING_PERCENTAGE)
            > 100
        {
            self.tell_sleeper(
                "sleep.not_possible",
                "No amount of rest can pass this night",
                true,
            );
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            let mut cancel = false;
            self.native_player_bed_enter(player, &name, head, &mut cancel);
            if cancel {
                return;
            }
        }
        if self.lie_in_bed(head) {
            self.award_custom_stat("sleep_in_bed", 1);
            self.trigger_player("minecraft:slept_in_bed");
            self.stats.reset("custom", "minecraft:time_since_rest");
        }
    }
    /// `LivingEntity.startSleeping`: the player takes the bed whose head
    /// `head` names, moved onto it and asleep from now on. False when the
    /// bed gives no shape to lie on.
    fn lie_in_bed(&mut self, head: [i32; 3]) -> bool {
        // The shape the bed gives the player to lie on (`getSleepHeight`).
        // A straw bed's head answers with its foot's shape, and with no foot
        // of its own there is none, so the player stays up as
        // `startSleeping` returning false leaves them.
        let Some(at) = self.sleeping_position(head) else {
            return false;
        };
        // LivingEntity.startSleeping: stopRiding once the bed is valid.
        self.stop_riding();
        self.stop_mining();
        self.left = false;
        self.right = false;
        self.using = None;
        self.sleeping = Some(head);
        self.sleep_timer = 0;
        // LivingEntity.startSleeping and ServerPlayer.startSleeping: setPosToBed
        // and connection.teleport.
        if let Some(p) = self.player.as_mut() {
            p.position = at;

            p.velocity = Vec3::ZERO;
            let _ = p.set_sleeping(true);
            p.pitch = 0.0;
            p.teleported = true;
            p.teleported_delta = [false; 3];
        }
        self.pitch = 0.0;
        if self.lan_acting.is_none() {
            self.previous_position = at;
            self.snap_player_model_pitch();
        }
        self.occupy_bed(head);
        true
    }
    fn wake_up(&mut self) {
        if self.remote.is_some() {
            // InBedChatScreen.sendWakeUp: the server wakes the player.
            self.remote_stop_sleeping();
            return;
        }
        let Some(head) = self.sleeping.take() else {
            return;
        };
        self.sleep_timer = 100;
        // The active guest's live body wakes here; the hub owns its wake
        // animation and sleeping metadata, not the suspended outer actor.
        if let Some(i) = self.lan_acting.and_then(|conn| self.guest_index(conn))
            && let Some(host) = self.lan.as_mut()
        {
            // The live turn may have changed its bed since entry.
            {
                let value = Some(head);
                host.hub.guests.update(i, |guest| guest.bed = value);
            }
            host.hub.wake(i);
            // This live body applies the stand-up pose below, not the later
            // wake_woken_guests pass over stored guest bodies.
            {
                let value = None;
                host.hub.guests.update(i, |guest| guest.woke_from = value);
            }
        }
        let stand = self.bed_wake_pose(head, self.yaw);
        if let Some(p) = self.player.as_mut() {
            if let Some((at, yaw)) = stand {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.yaw = yaw;
                p.pitch = 0.0;
                p.teleported = true;
                p.teleported_delta = [false; 3];
            }
            let _ = p.set_sleeping(false);
        }
        if let Some((at, yaw)) = stand {
            self.yaw = yaw;
            self.pitch = 0.0;
            if self.lan_acting.is_none() {
                self.previous_position = at;
                self.snap_player_model_pitch();
            }
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_player_bed_leave(player, &name, head);
        }
    }
    fn tick_sleep(&mut self) {
        self.sleep_timer = tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        if let Some(head) = self.sleeping {
            // LivingEntity.tick's checkBedExists: a sleeper whose bed is
            // gone gets up.
            let bed = self.world.block(head[0], head[1], head[2]);
            let bed_gone = !self.res.facts().get(bed).name.ends_with("_bed");
            // Vanilla has no Shift wake-up: the chat screen's capitals must
            // not get the sleeper up.
            if (self.screen != Screen::Chat && self.keys.held(Mapping::Sneak))
                || bed_gone
                || self
                    .player
                    .as_ref()
                    .is_none_or(|p| p.health.dead_or_dying())
            {
                self.wake_up();
                return;
            }
            // setPosToBed: the bed's top (`getSleepHeight`) and 0.125 above
            // it. A sleeper whose bed no longer gives a shape to lie on stays
            // where it is: vanilla only ever places a sleeper when it starts.
            let at = self.sleeping_position(head);
            if let (Some(p), Some(at)) = (self.player.as_mut(), at) {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.pitch = 0.0;
            }
            // LivingEntity.tick: a sleeper's xRot is 0.
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // ServerLevel.tick: once enough of the players have slept 100 ticks,
        // skip to morning if `advance_time` is on, wake everyone, and
        // resetWeatherCycle if `advance_weather` is on and it is raining.
        if self.enough_slept() {
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ADVANCE_TIME)
            {
                let t = self.world.day_time;
                let mut target = t + 24000 - t.rem_euclid(24000);
                // SleepFinishedTimeEvent: the mods may move the wake-up time,
                // never before now (`setTimeAddition` refuses a lower one).
                let mut asked = target;
                let mut cancel = false;
                self.native_sleep_finished_time(self.dim.id(), t, &mut asked, &mut cancel);
                if asked >= t {
                    target = asked;
                }
                // A canceled event leaves the clock alone, so `native_time_skip` is not asked.
                if !cancel {
                    self.native_time_skip(self.dim.id(), t, target, "sleep", &mut cancel);
                }
                if !cancel {
                    self.world.day_time = target;
                }
            }
            // Level.isRaining: the rain level is above 0.2.
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ADVANCE_WEATHER)
                && self.weather_levels(1.0).0 > 0.2
            {
                self.world.weather.raining = false;
                self.world.weather.thundering = false;
                self.world.weather.rain_time = 0;
                self.world.weather.thunder_time = 0;
            }
            if self.sleeping.is_some() {
                self.wake_up();
            }
            self.wake_guests();
            // SleepStatus.removeAllSleepers: waking everyone is not announced.
            self.sleep_seen.1 = 0;
        }
    }
    /// Copy the live player and world state into the `Level` that gets saved.
    pub(super) fn sync_level(&mut self) {
        self.level.player_impulse_grace = self.player.as_ref().map(|p| p.impulse.reset_grace_time);
        self.level.player_fall_particles = self
            .player
            .as_ref()
            .map(|p| p.spawn_extra_particles_on_fall);
        // Level.write saves the game rules (game_rules.dat) with the rest.
        self.level.game_rules = self.ui.rules.by_name();
        // PrimaryLevelData.setDifficulty: /difficulty and the options button change it.
        self.level.difficulty =
            horizon_client::title::DIFFICULTIES[usize::from(self.ui.difficulty) & 3].into();
        self.level.time = self.world.game_time;
        self.level.weather = self.world.weather;
        // level.dat holds the Overworld clock's total, whichever level is in place.
        self.level.day_time = self.command_clock_time("minecraft:overworld");
        self.level.end_clock = Some(self.world.end_clock);
        // A server has no player of its own; `enter_guest` puts one in
        // `self.player` only while a command reads it.
        if self.player.is_none() && self.options.dedicated.is_some() {
            return;
        }
        self.level.selected_slot = self.selected as i32;
        let mode = self.mode_id();
        // The vehicle the player rides (`ServerPlayer.saveParentVehicle`).
        let others = self
            .lan
            .as_ref()
            .map(|h| self.ridden_by_others(&h.hub.guests, self.dim.id(), None, false))
            .unwrap_or_default();
        let vehicle = self.parent_vehicle_tag(self.riding, &others);
        // Only the local player's file is this one's to rewrite.
        let local = self.serving.is_none() && self.options.dedicated.is_none();
        let level = &mut self.level;
        level.creative = self.creative;
        level.game_mode = mode;
        level.previous_game_mode = self.previous_mode.map(mod_api::GameMode::id);
        level.spawn = self.spawn.map(|s| [s.x, s.y, s.z]);
        level.respawn_elsewhere = self.respawn_config.map(|c| c.saved());
        level.rotation = [self.yaw, self.pitch];
        level.xp_level = self.experience.level;
        level.xp_progress = self.experience.progress;
        level.xp_total = self.experience.total;
        level.root_vehicle = vehicle.as_ref().map(|v| v.2).or(self.pending_vehicle);
        // The ride's tree, or none once the file's own came back.
        match vehicle {
            Some((tag, ..)) => {
                let extra = level
                    .player_extra
                    .get_or_insert_with(|| horizon_core::nbt::Tag::Compound(Default::default()));
                if let horizon_core::nbt::Tag::Compound(map) = extra {
                    map.insert("RootVehicle".into(), tag);
                }
            }
            None if local && self.pending_vehicle.is_none() => {
                if let Some(horizon_core::nbt::Tag::Compound(map)) = &mut level.player_extra {
                    map.remove(&"RootVehicle".into());
                }
            }
            None => {}
        }
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.absorption = p.health.absorption();
            let extra = level
                .player_extra
                .get_or_insert_with(|| horizon_core::nbt::Tag::Compound(Default::default()));
            if let horizon_core::nbt::Tag::Compound(map) = extra {
                map.insert(
                    "Fire".into(),
                    horizon_core::nbt::Tag::Short(
                        p.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                    ),
                );
                map.insert("Air".into(), horizon_core::nbt::Tag::Short(p.air as i16));
                // Entity.saveWithoutId: the `Invulnerable` flag always, its
                // `invulnerable_time` window only while open, like a mob's
                // save (#916).
                map.insert(
                    "Invulnerable".into(),
                    horizon_core::nbt::Tag::Byte(i8::from(p.health.permanently_invulnerable)),
                );
                if p.health.invulnerable_time > 0 {
                    map.insert(
                        "invulnerable_time".into(),
                        horizon_core::nbt::Tag::Int(p.health.invulnerable_time),
                    );
                } else {
                    map.remove(&"invulnerable_time".into());
                }
                // LivingEntity.addAdditionalSaveData: AttributeMap.pack.
                map.insert("attributes".into(), p.saved_attributes());
            }
            level.fall_distance = p.fall_distance;

            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
            level.flying = p.flying;
        }
        level.inventory = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref()
                    .map(|s| (i as u8, s.item.to_string(), s.count, s.damage))
            })
            .collect();
        level.inventory_components = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i as u8, s.as_ref()?.extra.clone()?)))
            .collect();
        if let Some(s) = &self.offhand {
            level
                .inventory
                .push((150, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = s.extra.clone() {
                level.inventory_components.insert(150, extra);
            }
        }
        for (equipment_slot, s) in self.equipment.iter().enumerate() {
            if let (Some(s), Some(slot)) = (s, horizon_client::save::saved_slot(equipment_slot)) {
                level
                    .inventory
                    .push((slot, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(slot, extra);
                }
            }
        }
        // Vanilla stores armor in slots 100 (feet) to 103 (head).
        for (i, s) in self.armor.iter().enumerate() {
            if let Some(s) = s {
                level
                    .inventory
                    .push((103 - i as u8, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(103 - i as u8, extra);
                }
            }
        }
        level.dimension = self.dim.id().to_owned();
        // An open ender chest holds the live copy.
        if let Some(key) = self.ender_open
            && let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key)
        {
            self.ender_items = items.clone();
        }
        level.ender_items = self.ender_items.clone();
        level.active_effects = self.potions.effects.saved();
    }
    /// The saved player's hands, armor, body, saddle and ender chest go in
    /// their fields; the 36 inventory slots come back.
    fn load_saved_inventory(&mut self, level: &Level) -> Vec<Option<ItemStack>> {
        let mut inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.offhand = None;
        self.equipment = Default::default();
        self.ender_items = if level.ender_items.len() == 27 {
            level.ender_items.clone()
        } else {
            vec![None; 27]
        };
        for (slot, id, count, damage) in &level.inventory {
            let stack = ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra: level.inventory_components.get(slot).cloned(),
            };
            if (100..104).contains(slot) {
                self.armor[usize::from(103 - slot)] = Some(stack);
            } else if *slot == 150 {
                self.offhand = Some(stack);
            } else if let Some(equipment_slot) = horizon_client::save::equipment_slot(*slot) {
                self.equipment.set(equipment_slot, Some(stack));
            } else if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
                *cell = Some(stack);
            }
        }
        inventory
    }
    /// Save the level in place: its changed chunks, its entities and its raids.
    fn save_level(&mut self, wait: bool) -> Result<()> {
        self.save_border();
        self.save_chunk_tickets()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.streaming.prepare_save(&mut self.world, wait, deadline) {
                horizon_client::streaming::SaveStep::Full(chunk) => {
                    self.native_late_full_chunk_snapshot(&chunk);
                    self.streaming.save_finished(chunk);
                }
                horizon_client::streaming::SaveStep::Protos(protos) => {
                    self.native_proto_snapshots(&protos);
                    self.streaming.finish_save(protos, wait);
                    break;
                }
                horizon_client::streaming::SaveStep::Pending => unreachable!(),
            }
        }
        let keys: Vec<_> = self.world.modified.iter().copied().collect();
        self.save.save_attachments(&self.world.attachments)?;
        for key in &keys {
            self.world.modified.remove(key);
        }
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.native_full_chunk_snapshots(&chunks);
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        if !wait {
            self.world.modified.autosaved(&keys, Instant::now());
        }
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        if mod_api::__private::listened(mod_api::HookId::LevelSave) {
            self.native_level_save(self.dim.id());
        }
        Ok(())
    }
    /// `ChunkMap.saveChunksEagerly`: snapshot only while the tick and writer
    /// have room. NBT encoding, compression and durable writes stay on workers.
    fn save_chunks_eagerly(&mut self) {
        // No world open (`world_root` is set by `finish_open_world`): the save
        // is `Game::new`'s placeholder under the working directory.
        if self.remote.is_some()
            || self.world_root.as_os_str().is_empty()
            || self.agent.hold_server
            || self.clock.sprinting()
            || self
                .server_lists
                .as_ref()
                .is_some_and(|lists| !lists.lock().unwrap().autosave)
        {
            return;
        }
        let deadline = self
            .server_tick_deadline
            .unwrap_or_else(|| Instant::now() + self.clock.tick_duration());
        let mut modified = std::mem::take(&mut self.world.modified);
        modified.save_eagerly(
            Instant::now(),
            || Instant::now() < deadline && self.save.active_chunk_writes() < 128,
            |key| self.world.chunks.contains_key(&key),
            |key| {
                // `saveChunkIfNeeded` saves only a LevelChunk, so a lit one; a
                // column still waiting for its light keeps its queue entry.
                let Some(chunk) = self.world.chunks.get(&key).filter(|c| c.lit) else {
                    return false;
                };
                let chunks = self.with_entities([chunk]);
                match self
                    .save
                    .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))
                {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!("Horizon: failed to save changed chunks: {e}");
                        false
                    }
                }
            },
        );
        self.world.modified = modified;
    }
    /// `GameRenderer.tryTakeScreenshotIfNeeded`: while a local world without a
    /// saved icon is on the screen, grab the rendered frame once a second until
    /// the save has one. The Select World list shows it as the world's preview.
    fn tick_world_icon(&mut self) {
        // Its grab would take a sequence's frame.
        if self.screenshot_sequence.is_some() {
            return;
        }
        let Some(icon) = self.world_icon.as_ref() else {
            return;
        };
        if icon
            .last_attempt
            .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        if icon.path.is_file() {
            // tryTakeScreenshotIfNeeded's hasWorldScreenshot: it has one, stop asking.
            self.world_icon = None;
            return;
        }
        // takeAutoScreenshot only saves with every loaded section rendered, or
        // the icon would picture the holes the streamer has not filled yet.
        if self.streaming.pending() != 0
            || !self.pending_uploads.is_empty()
            || self.remote_backlog() != 0
        {
            return;
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        // countRenderedSections() > 10: a client that joined its server
        // thread has nothing pending before the first chunks arrive, and the
        // icon would be only sky.
        if renderer.drawn_sections <= 10 {
            return;
        }
        if renderer.screenshot.is_some() {
            // F2's or an agent's screenshot owns the frame this time; ours
            // comes around again with the next attempt.
            return;
        }
        let path = icon.path.clone();
        if let Some(icon) = self.world_icon.as_mut() {
            icon.last_attempt = Some(Instant::now());
        }
        renderer.screenshot = Some(horizon_client::gfx::ScreenshotRequest {
            path,
            size: Some([64, 64]),
            scaled: false,
        });
    }
    fn render(&mut self) -> Result<bool> {
        if self.screenshot_sequence.is_some() {
            return self.render_sequence_frame();
        }
        self.render_frame()
    }
    /// Render the frame, then its two moments after that: `render_frame_post` whatever
    /// became of it, `flip_frame` once it was presented.
    fn render_and_flip(&mut self, elapsed: Duration) -> Result<bool> {
        let rendered = match self.render() {
            // The callback can fire during this frame; rebuild on
            // the next redraw rather than exiting on its error.
            Err(_) if self.renderer.as_ref().is_some_and(Renderer::device_lost) => false,
            result => result?,
        };
        self.native_render_frame_post(elapsed);
        if rendered {
            // The submit thread may still hold the frame: a listener hears it after the
            // present, and with none the next frame keeps overlapping it.
            if mod_api::__private::listened(mod_api::HookId::FlipFrame)
                && let Some(renderer) = &mut self.renderer
            {
                renderer.wait_submissions();
            }
            self.native_flip_frame();
        }
        Ok(rendered)
    }
    fn render_frame(&mut self) -> Result<bool> {
        self.render_with(|renderer, frame| {
            horizon_client::spans::time("renderer", || renderer.render(frame))
        })
    }

    /// Build the complete frame before handing its borrowed geometry and lighting to
    /// the consumer. Tests use the same boundary as GPU submission.
    fn render_with(
        &mut self,
        consume: impl FnOnce(&mut Renderer, &Frame<'_>) -> Result<bool>,
    ) -> Result<bool> {
        self.prepared_model_draws.borrow_mut().clear();
        let prepare = spans::guard("prepare");
        let mut pr = spans::laps();
        // The window's GPU holds the atlas: its copy in memory is kept
        // compressed (a no-op once it is).
        if self.renderer.is_some() {
            self.gui.atlas.pack_pixels();
            self.pack.render.pixels.pack();
        }
        // Before any text of this frame: a full glyph region starts over.
        self.gui.atlas.font.begin_frame();
        if self.clock.still() {
            // Obfuscated text picks the same glyphs in a frame of the same world tick.
            self.gui.atlas.font.reseed(self.visual_ticks() as i64);
        }
        self.glint_frame = self.capture_glint_frame();
        if self.renderer.as_mut().is_some_and(|r| r.display_changed()) {
            self.apply_ui_settings();
        }
        self.follow_frame();
        self.observe_screen();
        // `Gui.setScreen` clears the focus of the screen it leaves: a menu
        // that comes back starts with no widget highlighted.
        self.drop_left_screen_focus();
        // Minecraft.doWorldLoad: the LevelLoadingScreen while single player's
        // server starts, with the server's chunk map and progress.
        if let Some(starting) = &self.integrated_starting {
            // WorldOpenFlows.upgradeAndOpenWorld: the server's file fixers run on
            // FileFixerProgressScreen, before the LevelLoadingScreen has progress.
            let upgrade = starting.handle.upgrade();
            if upgrade.is_file_fixing() {
                horizon_client::widgets::begin_screen(|| {
                    horizon_client::widgets::ScreenInfo::new(
                        "file_fixer",
                        screen_info::tr("upgradeWorld.title", &[]),
                    )
                });
                if let Some(renderer) = &mut self.renderer {
                    let mut hud = HudBatch::new(
                        [renderer.config.width, renderer.config.height],
                        self.settings.gui_scale_inputs(),
                    );
                    horizon_client::level_loading::draw_file_fixer(
                        &mut hud,
                        &self.gui.atlas,
                        upgrade,
                        self.cursor,
                    );
                    pr.lap("pr_loading");
                    drop(prepare);
                    return renderer.render(&loading_frame(&hud));
                }
                return Ok(false);
            }
            // The server's progress once it has some (`hasProgress`).
            let progress = Some(starting.handle.load_view())
                .filter(|v| v.has_progress())
                .map(|v| v.server_progress());
            horizon_client::widgets::begin_screen(|| {
                screen_info::level_loading(progress, &screen_info::tr)
            });
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                );
                let (w, h) = (hud.width, hud.height);
                horizon_client::ui::menu_background(
                    &mut hud,
                    &self.gui.atlas,
                    0.0,
                    0.0,
                    w,
                    h,
                    false,
                );
                // The chunk map and progress are the server thread's.
                let view = self
                    .integrated_starting
                    .as_ref()
                    .map(|s| s.handle.load_view().clone());
                let status = |x: i32, z: i32| view.as_ref().and_then(|v| v.status(x, z));
                let progress = view
                    .as_ref()
                    .is_some_and(|v| v.has_progress())
                    .then_some(self.loading_progress);
                horizon_client::level_loading::draw(
                    &mut hud,
                    &self.gui.atlas,
                    Some(&status),
                    progress,
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        if self.pending_entry.is_some() {
            if let (Some(renderer), Some(loader)) = (&mut self.renderer, &self.rest_loader) {
                horizon_client::widgets::begin_screen(|| {
                    screen_info::level_loading(Some(loader.progress.fraction()), &screen_info::tr)
                });
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                );
                horizon_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    loader.progress.line(),
                    loader.progress.fraction(),
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        // A pack reload covers the frame with its progress
        // (`LoadingOverlay`) until `drive_pack_reload` applies it.
        if let Some(reload) = &self.pack_reload {
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new(
                    [renderer.config.width, renderer.config.height],
                    self.settings.gui_scale_inputs(),
                );
                horizon_client::level_loading::draw_loading_overlay(
                    &mut hud,
                    &self.gui.atlas,
                    reload.progress.line(),
                    reload.progress.fraction(),
                    self.settings.file.bool("darkMojangStudiosBackground"),
                );
                pr.lap("pr_loading");
                drop(prepare);
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        pr.lap("pr_setup");
        let alpha = self.clock.fraction();
        // Camera.getCameraEntityPartialTicks: the view, the hand and the
        // player's own model stay smooth while the level is frozen.
        let camera_alpha = self.clock.camera_fraction();
        self.apply_requested_camera_type();
        let vanilla = self.camera_view(camera_alpha as f32);
        let view = vanilla.map(|view| {
            if self.shows_panorama() || self.studio_freecam.active {
                view
            } else {
                self.native_camera_pose_now(camera_alpha as f32, view)
            }
        });
        // What faces the camera (particles, item sprites, block entity text) faces a mod's.
        // A spectated entity's view is not the local player's yaw and pitch
        // (`Camera.setup` reads the camera entity), so it is kept too.
        let spectating = self.camera_mob().is_some();
        self.posed_rotation = view
            .filter(|posed| spectating || Some(*posed) != vanilla)
            .map(|(_, yaw, pitch)| (yaw, pitch));
        let eye = view.map(|_| {
            (
                glam::DVec3::from_array(self.view.eye),
                self.view.eye_rotation,
            )
        });
        let (camera, underwater, view_yaw, view_pitch) = match view {
            Some((c, yaw, pitch)) => {
                let s =
                    self.world
                        .block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts().get(s).fluid == Fluid::Water, yaw, pitch)
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.yaw,
                self.pitch,
            ),
        };
        pr.lap("pr_camera");
        self.audio
            .set_listener([camera.x, camera.y, camera.z], view_yaw, view_pitch);
        pr.lap("pr_listener");
        let block = [
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        ];
        // The camera inside an opaque block sees nothing through it: disable.
        let cave_culling = self.settings.horizon.cave_culling
            && !self
                .res
                .facts()
                .get(self.world.block(block[0], block[1], block[2]))
                .solid_render;
        // The cave culling search runs on its worker while the frame is prepared.
        if cave_culling
            && self.title.is_none()
            && let Some(renderer) = self.renderer.as_mut()
        {
            horizon_client::spans::time("occlusion", || {
                renderer.start_occlusion(camera, (self.streaming.radius * 16) as f32)
            });
        }
        pr.lap("pr_occlusion");
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let environment = self.view_environment();
        // EnvironmentAttributeProbe: the spatially interpolated biome colours,
        // Gaussian-weighted over the noise biomes around the camera (plains
        // where no chunk is loaded, as ClientLevel.getUncachedNoiseBiome).
        let biomes = world_biomes!(self);
        let weights = {
            let chunks = &self.world.chunks;
            let plains = biomes.id("minecraft:plains");
            horizon_client::biome::gaussian_weights(camera.to_array(), |qx, qy, qz| {
                chunks
                    .get(&(qx >> 2, qz >> 2))
                    .map_or(plains, |c| c.biome(qx * 4, qy * 4, qz * 4))
            })
        };
        // visual/water_fog_color: no dimension sets it, its default is #050533.
        let water_fog = horizon_client::biome::blend_rgb(&weights, |b| {
            horizon_client::biome::rgb(biomes.get(b).water_fog)
        });
        // The attribute stack, with the camera's biome mix in the probe's order; the biome
        // maps as the packs left them.
        let attributes_of = |b: u16| {
            let info = biomes.get(b);
            environment.biome(&info.name).unwrap_or(&info.attributes)
        };
        let mut blend = horizon_world::environment_system::BiomeBlend::default();
        for &(b, weight) in &weights {
            blend.accumulate(weight, attributes_of(b));
        }
        // The packs' biome post-process group, mixed the same way where they set one.
        let empty = horizon_world::environment_attributes::AttributeMap::default();
        let post_of = |b: u16| {
            environment
                .biome_post(&biomes.get(b).name)
                .unwrap_or(&empty)
        };
        let mut post_blend = horizon_world::environment_system::BiomeBlend::default();
        if environment.has_biome_post() {
            for &(b, weight) in &weights {
                post_blend.accumulate(weight, post_of(b));
            }
        }
        let (rain, thunder) = self.view_weather(alpha as f32);
        // What the packs' attribute expressions read (only a pack's stack has any).
        let mut polytone_frame = None;
        if self.pack.environments.is_some()
            || self.pack.polytone_particles.is_some()
            || self.pack.polytone_post.is_some()
        {
            let light = self.world.light(block[0], block[1], block[2]);
            let frame = horizon_client::environment::Frame {
                rain,
                thunder,
                game_time: self.world.game_time,
                day_time: self.view_day_time(),
                render_distance: self.streaming.radius,
                sky_type: match self.dim {
                    Dim::Overworld | Dim::Extra(_) => 1,
                    Dim::End => 2,
                    Dim::Nether => 0,
                },
                dimension: self.dim.id().to_owned(),
                player: self.player.as_ref().map_or(camera.to_array(), |p| {
                    [p.position.x, p.position.y, p.position.z]
                }),
                player_velocity: self
                    .player
                    .as_ref()
                    .map_or([0.0; 3], |p| [p.velocity.x, p.velocity.y, p.velocity.z]),
                player_on_ground: self.player.as_ref().is_some_and(|p| p.ground.on_ground),
                in_water: underwater,
                camera: camera.to_array(),
                sky_light: i32::from(light >> 4),
                block_light: i32::from(light & 15),
                biome: biomes.get(biome).name.clone(),
                temperature: biomes.get(biome).temperature,
                downfall: biomes.get(biome).downfall,
            };
            if let Some(particles) = &self.pack.polytone_particles
                && let Ok(mut f) = particles.frame.write()
            {
                f.clone_from(&frame);
            }
            polytone_frame = Some(frame.clone());
            environment.set_frame(frame);
        }
        let mut sky = environment.sample(&horizon_client::environment::Sample {
            day_time: self.view_day_time(),
            rain,
            thunder,
            blend: Some(&blend),
            biome: Some(attributes_of(biome)),
            post_blend: Some(&post_blend),
            post_biome: Some(post_of(biome)),
        });
        self.end_flash_sky(&mut sky, alpha);
        if self.flash > 0 && !self.settings.file.bool("hideLightningFlashes") {
            // ClientLevel's flash layer while skyFlashTime > 0: the sky colour
            // moves 22% of the way to (0.8, 0.8, 1.0).
            let flash = [0.8, 0.8, 1.0];
            sky.sky_color =
                std::array::from_fn(|i| sky.sky_color[i] + (flash[i] - sky.sky_color[i]) * 0.22);
            sky.sky_light_factor = 1.0;
        }
        pr.lap("pr_environment");
        // FishingHookRenderer's line, then DebugRenderer: chunk borders and
        // hitboxes (F3+G, F3+B).
        let mut lines = self.fishing_line(alpha, size);
        let mut g = self.debug_gizmos(alpha as f32, camera.to_array());
        if self.studio_freecam.active || self.studio_replay.is_some() {
            horizon_client::studio::render_camera_path(
                &self.studio_freecam.track,
                &mut g,
                None,
                100,
            );
        }
        lines.extend(g.lines);
        let on_top = g.on_top;
        pr.lap("pr_debug");
        let visuals = if self.title.is_none() {
            self.native_frame_visuals(alpha, &mut lines)
        } else {
            native_visuals::FrameVisuals::default()
        };
        let emissive_layers = if self.title.is_none() {
            let mut requests = Vec::new();
            self.native_emissive(&mut requests);
            // Native mods' overlays first, then the packs' OptiFine emissive blocks.
            horizon_client::native_emissive::table(
                &self.pack.render.sprite_layers,
                requests,
                &self.pack.render.emissive,
            )
        } else {
            Vec::new()
        };
        pr.lap("pr_native_visuals");
        let view = if self.studio_freecam.active {
            glam::Mat4::from_quat(self.camera_orientation().conjugate())
        } else {
            horizon_client::camera::view_matrix(view_yaw, view_pitch)
        };
        let at = camera.to_array();
        // GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown.
        let mut crosshair = horizon_client::gizmos::Gizmos::default();
        if debug_crosshair_drawn(
            self.debug_crosshair(),
            self.camera_type.is_first_person(),
            self.ui.hide_gui,
            self.shows_panorama(),
        ) {
            let scale = HudBatch::new(size, self.settings.gui_scale_inputs()).scale;
            crosshair.debug_crosshair(at, view_pitch, view_yaw, scale);
        }
        let debug_lines = horizon_client::gizmos::camera_relative(&lines, at, view);
        let debug_lines_on_top = horizon_client::gizmos::camera_relative(&on_top, at, view);
        let debug_crosshair = horizon_client::gizmos::camera_relative(&crosshair.on_top, at, view);
        // LevelExtractor.extractBlockOutline: model translucency controls draw order;
        // no outline when an entity is targeted (Minecraft.hitResult an EntityHitResult).
        let mut outline_after_terrain = false;
        // GameRenderer.shouldRenderBlockOutline: not with the HUD hidden (F1), and only for a
        // player who may build. Adventure players only get one with a CAN_BREAK or
        // CAN_PLACE_ON item matching the block, and spectators with a menu block; Horizon
        // does not open menus for spectators, so those never show it. An open
        // screen does not hide it: vanilla checks the HUD and the game mode, not the screen.
        let adventure = self.mode == horizon_client::command::args::GameMode::Adventure;
        let can_build = self.may_build() || adventure;
        let outline = if block_outline_drawn(
            self.ui.hide_gui,
            self.entity_targeted(),
            can_build,
            self.shows_panorama(),
        ) {
            self.pick()
                .filter(|h| {
                    self.may_build()
                        || !self.block_action_restricted(h.pos)
                        || self.can_place_on_in_adventure(h.pos)
                })
                .filter(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let mut canceled = false;
                    self.native_block_highlight(h.pos, state, alpha as f32, &mut canceled);
                    !canceled
                })
                .map(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let f = self.res.facts().get(state);
                    outline_after_terrain = self
                        .pack
                        .render
                        .model_catalog
                        .collect(state, BlockPos::new(h.pos[0], h.pos[1], h.pos[2]))
                        .is_ok_and(|models| {
                            models
                                .iter()
                                .any(|(_, model)| model.faces.iter().any(|f| f.translucent))
                        });
                    let o = if f.outline_offset {
                        self.pack
                            .context
                            .tables
                            .offset(state, h.pos[0], h.pos[2])
                            .unwrap_or([0.0; 3])
                    } else {
                        [0.0; 3]
                    };
                    let p = h.pos.map(f64::from);
                    if let Some(edges) = self
                        .collision
                        .cached_shapes()
                        .context_outline_edges(state, self.outline_context_kind())
                    {
                        return edges
                            .iter()
                            .map(|e| std::array::from_fn(|i| p[i % 3] + o[i % 3] + e[i]))
                            .collect();
                    }
                    if let Some(b) = self.context_outline(h.pos, state, true, &self.be_anims) {
                        return horizon_client::blocks::cube_edges()
                            .iter()
                            .map(|e| {
                                std::array::from_fn(|i| {
                                    let k = i % 3;
                                    p[k] + b[k] + e[i] * (b[k + 3] - b[k])
                                })
                            })
                            .collect();
                    }
                    f.outline_edges
                        .iter()
                        .map(|e| std::array::from_fn(|i| p[i % 3] + o[i % 3] + e[i]))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        pr.lap("pr_outline");
        let title = self.shows_panorama();
        let (yaw, pitch) = if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (
                self.panorama_rotation.update(
                    self.started.elapsed().as_secs_f32(),
                    self.settings.file.double("panoramaScrollSpeed") as f32,
                ),
                10.0,
            )
        } else {
            (view_yaw, view_pitch)
        };
        let fov_degrees = if title {
            85.0
        } else {
            self.fov_degrees(camera_alpha as f32, camera.to_array())
        };
        let camera_effects = fov::camera_effects_enabled(title, self.studio_freecam.active);
        let bob = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(camera_alpha as f32)
        };
        let spin = if !camera_effects {
            glam::Mat4::IDENTITY
        } else {
            self.spinning_matrix(camera_alpha as f32)
        };
        let terrain_distance = self.terrain_distance();
        let distance = (terrain_distance * 16) as f32;
        let depth_far =
            horizon_client::camera::depth_far(terrain_distance, self.settings.cloud_range);
        // Only native mods read it: no matrices or allocation per frame without one.
        let mods_read_camera = !mod_api::__private::mods().is_empty();
        self.screen_blur_requested.set(false);
        self.native_camera_frame = (mods_read_camera && !title).then(|| {
            let partial = camera_alpha as f32;
            mod_api::Camera {
                position: camera.to_array(),
                eye_height: self.view.eye_height_old
                    + (self.view.eye_height - self.view.eye_height_old) * partial,
                partial_tick: partial,
                yaw,
                pitch,
                fov_degrees,
                near: 0.05,
                far: depth_far,
                view: (bob * spin * horizon_client::camera::view_matrix(yaw, pitch))
                    .to_cols_array(),
                projection: glam::Mat4::perspective_rh(
                    fov_degrees.to_radians(),
                    size[0] as f32 / size[1].max(1) as f32,
                    depth_far,
                    0.05,
                )
                .to_cols_array(),
                dimension: self.dim.id().to_string(),
            }
        });
        pr.lap("pr_projection");
        // GUI hooks project against this frame's camera, including its bob/spin and FOV.
        let mut hud = horizon_client::spans::time("hud", || self.hud(size));
        self.flush_ime(&mut hud);
        self.native_gui_render_post(&mut hud, self.ui.hide_gui, self.focused);
        self.resolve_inline_profiles(&hud.inline_profiles);
        pr.lap("pr_hud");
        self.locator_camera =
            (!title).then_some((camera.to_array(), view_yaw, view_pitch, fov_degrees));
        let mut requested_post_effects = self.requested_post_effects();
        // The packs' Polytone post chains, after vanilla's (#11486).
        let game_ticks = self.world.game_time as f64 + alpha;
        let delta_ticks = (game_ticks - self.polytone_post_ticks).clamp(0.0, 20.0) as f32;
        self.polytone_post_ticks = game_ticks;
        let polytone_extras = match (&self.pack.polytone_post, &polytone_frame) {
            (Some(post), Some(frame)) => {
                let view = bob * spin * horizon_client::camera::view_matrix(yaw, pitch);
                let globals = horizon_client::polytone_post::PolyGlobals {
                    // 26.3's reverse-Z projection, as the world's (and its depth) is.
                    projection: glam::Mat4::perspective_rh(
                        fov_degrees.to_radians(),
                        size[0] as f32 / size[1].max(1) as f32,
                        depth_far,
                        0.05,
                    ),
                    model_view: view,
                    sun_angle: sky.sun_angle.to_radians() - std::f32::consts::FRAC_PI_2,
                    day_time: self.view_day_time().rem_euclid(24000) as f32,
                    delta_time: delta_ticks,
                    player: self.player.as_ref().map_or(camera.to_array(), |p| {
                        [p.position.x, p.position.y, p.position.z]
                    }),
                };
                let (effects, extras) = post.frame(frame, &globals);
                requested_post_effects.extend(effects);
                Some(extras)
            }
            _ => None,
        };
        self.view_frustum = self
            .renderer
            .as_ref()
            .map(|r| r.camera_frustum(fov_degrees, yaw, pitch, bob * spin, depth_far));
        let billboard_frustum = if self.settings.horizon.taa {
            self.renderer.as_ref().map(|r| {
                r.camera_frustum_with_jitter(fov_degrees, yaw, pitch, bob * spin, depth_far)
            })
        } else {
            self.view_frustum
        };
        // Keep the buffer local while builders need mutable access to the game.
        let mut entities = std::mem::take(&mut self.entity_vertices);
        entities.clear();
        let mut static_block_entities = std::sync::Arc::from([]);
        let mut visible_static_block_entity_sections = Vec::new();
        let mut static_block_entity_stats = StaticCacheFrameStats::default();
        self.see_through_vertices.clear();
        self.glowing_vertices.get_mut().clear();
        self.shadow_only_vertices.get_mut().clear();
        pr.lap("pr_entity_setup");
        // The pack is chosen before entity geometry, so a pack that starts this frame
        // keeps block-entity models in the CPU stream instead of dropping them.
        let mut pack_inputs = if title {
            Default::default()
        } else {
            self.shader_pack_inputs(camera, alpha as f32, &sky)
        };
        if !title {
            let (built, static_sections, visible_static_sections, static_stats) =
                spans::time("entity_geometry", || {
                    self.entity_geometry(camera, alpha, billboard_frustum, entities)
                });
            entities = built;
            static_block_entities = static_sections;
            visible_static_block_entity_sections = visible_static_sections;
            static_block_entity_stats = static_stats;
        } else {
            // No level behind the panorama, so no entity either: the GPU path's
            // draws are started by `begin_skinning`, which only `entity_geometry`
            // runs, and would otherwise stay those of the last world frame and
            // be drawn over the title (#8192). Its models stay stored.
            self.skin_frame.get_mut().clear_draws();
        }
        pr.lap("pr_entities");
        let bolts = if title {
            Vec::new()
        } else {
            self.bolt_instances(camera)
        };
        // Particles blended additively (resource packs' Polytone ones, #11486), from the
        // render that built `entities`.
        let mut additive = self.particles.take_additive();
        // The dragon is drawn in every dimension, as any entity: a dragon
        // summoned outside the End (the server's AddEntity) is shown too.
        if !title {
            let mut dragon = Vec::new();
            self.dragon_geometry(camera, alpha, &mut dragon, &mut additive);
            self.dragon_fireball_geometry(camera, alpha, &mut dragon);
            entities.extend(dragon);
        }
        pr.lap("pr_dragon_bolts");
        let (celestial, stars) = if title {
            (Vec::new(), None)
        } else if self.dim == Dim::Overworld
            && self.potions.effects.get("darkness").is_none()
            && self.potions.effects.get("blindness").is_none()
        {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky, rain)
        } else if self.dim == Dim::End && self.settings.horizon.sky {
            (self.end_flash_geometry(), None)
        } else {
            (Vec::new(), None)
        };
        self.custom_sky_frame([camera.x, camera.y, camera.z], &sky, (rain, thunder), title);
        pr.lap("pr_celestial");
        let (mut hand, hand_items) = if title || self.scoping() {
            Default::default()
        } else {
            self.hand_geometry(camera_alpha)
        };
        // GameRenderer.render3dHud: the screen effects after the hand, in
        // its pass (the hand pass draws with `bob * view`).
        if !title {
            let view = bob * horizon_client::camera::view_matrix(yaw, pitch);
            let aspect = size[0] as f32 / size[1].max(1) as f32;
            self.item_activation_geometry(camera_alpha as f32, view, aspect, &mut hand);
        }
        pr.lap("pr_hand");
        let hud_fov = self.hud_fov_degrees(camera_alpha as f32, camera.to_array());
        let screen = if title {
            Vec::new()
        } else {
            let scale = self
                .player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:scale").ok())
                .unwrap_or(1.0) as f32;
            let hand_projection = horizon_client::camera::hand_projection(
                hud_fov,
                size[0] as f32,
                size[1].max(1) as f32,
                depth_far,
            );
            self.screen_quads(alpha as f32, size, scale, hand_projection)
        };
        if !title {
            self.end_geometry(camera, &mut entities);
        }
        self.outline_fill(&outline, camera, &mut entities);
        pr.lap("pr_screen_end");
        let weather = if !title && self.can_have_weather() && self.settings.horizon.weather {
            horizon_client::spans::time("weather", || self.weather_geometry(camera, alpha))
        } else {
            Vec::new()
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        let see_through = std::mem::take(&mut self.see_through_vertices);
        pr.lap("pr_weather_border");
        if title {
            self.dynamic_lights.clear();
        } else {
            self.gather_dynamic_lights(camera, alpha);
        }
        pr.lap("pr_lights");
        pack_inputs.frame_timing = self
            .screenshot_sequence
            .as_ref()
            .map(screenshot_sequence::Sequence::timing);
        pr.lap("pr_shader_inputs");
        // Fog hooks may clear particles too. They must run before the final clear
        // boundary, not while constructing Frame after geometry has been extracted.
        let fog = self.frame_fog(camera, &sky, biome, water_fog);
        if !title {
            self.client_particle_geometry(camera, alpha, fov_degrees, &mut entities);
        }
        let mut world_geometry = self.world_geometry.take().expect("view storage");
        self.fill_world_geometry(
            title,
            camera.to_array(),
            view,
            alpha,
            &mut world_geometry,
            &mut entities,
        );
        let colored_lights = if self.title.is_none() {
            visuals.finish(self, camera)
        } else {
            Vec::new()
        };
        // Map textures and font glyphs changed while building this frame's
        // geometry (game/blocks/maps/mod.rs, world text) upload with this batch; draining
        // map_patches any earlier leaves them a frame behind.
        hud.patches.append(&mut self.map_patches);
        let entity_spans = std::mem::take(self.entity_spans.get_mut());
        let entity_motion = std::mem::take(self.entity_motion.get_mut());
        let entity_motion_posed = std::mem::take(self.entity_motion_posed.get_mut());
        let glowing = std::mem::take(self.glowing_vertices.get_mut());
        let shadow_only = std::mem::take(self.shadow_only_vertices.get_mut());
        let skin = std::mem::take(self.skin_frame.get_mut());
        let frame = Frame {
            weather: &weather,
            weather_time: horizon_client::weather::Time::new(self.world.game_time, alpha as f32),
            nether_lighting: self.dim == Dim::Nether,
            camera,
            yaw,
            pitch,
            fov_degrees,
            eye: eye.filter(|_| !title).map_or(camera, |e| e.0),
            eye_rotation: eye.filter(|_| !title).map_or([yaw, pitch], |e| e.1),
            first_person: eye.is_none() || title || !self.view.detached,
            bob,
            spin,
            sky,
            shadow_scene_revision: (self.world.revision, self.world.chunks.membership_revision()),
            shadow_cache: self.settings.horizon.shadow_cache,
            distance,
            detail_distance: (self.streaming.radius * 16) as f32,
            depth_far,
            underwater,
            water_plane: if title
                || underwater
                || !self
                    .renderer
                    .as_ref()
                    .is_some_and(|r| r.wants_water_plane())
            {
                None
            } else {
                self.water_plane(camera)
            },
            title,
            sky_blocked: horizon_client::fog::sky_pass_skipped(
                horizon_client::fog::fog_type(
                    &self.world,
                    self.res.facts(),
                    [camera.x, camera.y, camera.z],
                    &self.near_plane(),
                ),
                self.potions.effects.get("blindness").is_some()
                    || self.potions.effects.get("darkness").is_some(),
                self.boss_overlay.should_create_world_fog(),
            ),
            skybox: self.dim == Dim::Overworld && self.settings.horizon.sky,
            end_sky: self.dim == Dim::End && self.settings.horizon.sky,
            end_sky_sprite: self
                .gui
                .atlas
                .sprites
                .get("environment/end_sky")
                .map(|r| [r.u0, r.v0, r.u1, r.v1]),
            boss_world_fog: self.boss_overlay.should_create_world_fog(),
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            cave_culling,
            water_fog,
            fog: Some(fog),
            dark_disc: self.dark_disc(camera),
            outline,
            outline_after_terrain,
            outline_high_contrast: self.settings.file.bool("highContrastBlockOutline"),
            pvp: self.settings.extras.pvp.frame(),
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            debug_crosshair: &debug_crosshair,
            hud: &hud,
            glint_alpha: self.glint_frame.alpha,
            glint_ticks: self.glint_frame.scroll_ticks(),
            clouds: (!title
                && self.dim == Dim::Overworld
                && horizon_client::clouds::visible(&self.settings.clouds, sky.cloud_color[3]))
            .then(|| self.gui.atlas.sprites.get("horizon/white"))
            .flatten()
            .map(|white| horizon_client::clouds::Input {
                texture: &self.pack.clouds,
                uv: [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0],
                camera,
                time: self.world.game_time,
                partial_tick: alpha,
                range: self.settings.cloud_range,
                fast: self.settings.clouds == "fast",
                color: sky.cloud_color,
            }),
            entities: &entities,
            static_block_entities: &static_block_entities,
            visible_static_block_entity_sections: &visible_static_block_entity_sections,
            static_block_entity_stats,
            entity_motion: &entity_motion,
            entity_motion_posed: &entity_motion_posed,
            entity_spans: &entity_spans,
            skin: skin.pending().then_some(&skin),
            glowing: &glowing,
            dynamic_lights: &self.dynamic_lights,
            colored_lights: &colored_lights,
            emissive_layers: &emissive_layers,
            lightning: horizon_client::gfx::lightning::Input {
                bolts: &bolts,
                uv: horizon_client::gfx::lightning::white_uv(&self.gui.atlas.font.white()),
            },
            additive: &additive,
            see_through: &see_through,
            world_primitives: world_geometry.batch(),
            shadow_only: &shadow_only,
            celestial: if self.settings.horizon.sky || title {
                &celestial
            } else {
                &[]
            },
            stars: stars.filter(|_| self.settings.horizon.sky || title),
            hand: &hand,
            hand_items: &hand_items,
            hand_fov_degrees: hud_fov,
            screen: &screen,
            pack: pack_inputs,
            border,
            game_time: self.world.game_time as f32,
            shader_game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
            crumbling_sprite: self
                .gui
                .atlas
                .sprites
                .get("block/destroy_stage_0")
                .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0]),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.gui
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.texture_clock(),
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(alpha as f32),
            block_flicker: self.fog.block_light_flicker,
            boss_darken: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            menu_blur: self.menu_blur(),
            post_effects: horizon_client::post_chain::Request {
                effects: &requested_post_effects,
                root: &self.pack.source,
                extras: polytone_extras.as_ref(),
                menu_blur_radius: self.settings.menu_blur as f32,
                // GlobalSettingsUniform.update.
                globals: horizon_client::post_chain::Globals {
                    camera_block_pos: [
                        camera.x.floor() as i32,
                        camera.y.floor() as i32,
                        camera.z.floor() as i32,
                    ],
                    camera_offset: [
                        (camera.x.floor() - camera.x) as f32,
                        (camera.y.floor() - camera.y) as f32,
                        (camera.z.floor() - camera.z) as f32,
                    ],
                    game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
                    ..Default::default()
                },
            },
            chunk_fade: self.settings.chunk_fade,
        };
        pr.lap("pr_frame");
        // Renderer encoding and GPU waiting belong to render, not prepare.
        drop(prepare);
        let debug_passes = self.debug_lists_gpu_passes();
        let mut prepared_draws = std::mem::take(&mut *self.prepared_model_draws.borrow_mut());
        let result = match self.renderer.as_mut() {
            Some(renderer) => {
                renderer.set_pass_timing(debug_passes || self.gpu_passes);
                renderer.set_pass_timing_gap(if self.gpu_passes {
                    Duration::ZERO
                } else {
                    horizon_client::debug_overlay::GPU_TIMING_GAP
                });
                renderer.stage_prepared_draws(&prepared_draws);
                consume(renderer, &frame)
            }
            None => Err("renderer".into()),
        };
        prepared_draws.clear();
        *self.prepared_model_draws.borrow_mut() = prepared_draws;
        let _cleanup = spans::guard("pr_cleanup");
        // A frame the surface refused (occluded, lost, timed out) is `Ok(false)`.
        self.frame_drawn = matches!(result, Ok(true));
        // Restore storage even when rendering fails (for example, a lost surface).
        self.entity_vertices = entities;
        self.world_geometry = Some(world_geometry);
        *self.entity_spans.get_mut() = entity_spans;
        *self.entity_motion.get_mut() = entity_motion;
        *self.entity_motion_posed.get_mut() = entity_motion_posed;
        *self.glowing_vertices.get_mut() = glowing;
        *self.shadow_only_vertices.get_mut() = shadow_only;
        let mut skin = skin;
        // The renderer took the new models when its store grew by them; otherwise they stay
        // pending, or the stores restart (`SkinModels::begin`).
        if self
            .renderer
            .as_ref()
            .and_then(|r| r.skinned_models())
            .is_some_and(|stored| stored == skin.base + skin.new_model.len() as u32)
        {
            skin.handed();
        }
        *self.skin_frame.get_mut() = skin;
        self.see_through_vertices = see_through;
        result
    }

    fn clear_lod(&mut self) {
        // The samplers waiting on the server stop first: dropping the far
        // terrain joins them.
        if let Some(remote) = &self.remote {
            remote.session.cancel_far_terrain();
        }
        self.lod = None;
        self.lod_source_attempted = false;
        self.lod_observe.clear();
        self.lod_held.clear();
        self.lod_blue_map = None;
        self.far_beacons.clear();
        self.lod_covered_at = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_lods();
        }
    }
    fn prepare_lod(&mut self) {
        let enabled = self.far_terrain_choice().0 > 0
            && self.renderer.is_some()
            && self.player.is_some()
            && self.title.is_none();
        if !enabled {
            if self.lod.is_some() || self.lod_source_attempted {
                self.clear_lod();
            }
            return;
        }
        // Another level: its own kept chunks. (Another world at the same
        // address, a proxy's next server, comes through a new login; a seed
        // changed at a respawn of the same level is a plugin hiding it.)
        if self
            .lod_level
            .as_ref()
            .is_some_and(|level| self.remote.as_ref().is_some_and(|r| r.level != *level))
        {
            self.clear_lod();
        }
        if self.lod_source_attempted {
            return;
        }
        self.lod_source_attempted = true;
        self.lod_level = None;
        let dim = self.dim;
        let source = match (&self.integrated, self.server_far_terrain(), &self.remote) {
            (Some(server), ..) => server.call(move |game| game.lod_source_in(dim)).flatten(),
            (None, Some(offer), Some(remote)) => {
                let states = self.res.facts().states.len() as u32;
                let biomes = self.res.biomes().index.clone();
                let source: Arc<dyn horizon_client::streaming::LodSource> =
                    remote.session.far_terrain(offer, biomes, states);
                Some(source)
            }
            // A friend's world (a new code at every share) or a splitscreen
            // guest's (the player's own world, its other guests writing the
            // same folder): nothing to keep.
            (None, None, Some(remote))
                if self.options.split_guest
                    || remote
                        .session
                        .address
                        .starts_with(horizon_client::net::FRIEND) =>
            {
                None
            }
            // No offer from a server: draw what it sent, on this visit and before.
            (None, None, Some(remote)) => std::env::current_dir().ok().map(|home| {
                use horizon_client::server_terrain as terrain;
                let address = &remote.session.address;
                let root = home.join("server-terrain");
                let dir = terrain::level_dir(&root, address, remote.hashed_seed, &remote.level);
                self.lod_level = Some(remote.level.clone());
                // Past them, the server's BlueMap, if the player named it or
                // lets Horizon look for it (on the BlueMap worker).
                let maps = root.join(terrain::server_folder(address)).join("BlueMap");
                let settings = &self.settings.horizon.blue_map;
                let blue_map = horizon_client::bluemap_detect::source(
                    settings,
                    address,
                    &maps,
                    std::time::SystemTime::now(),
                )
                .map(|source| {
                    use horizon_client::{bluemap, bluemap_detect::Source};
                    // The map's address only at debug: logs land in crash reports.
                    log::info!("Horizon: far terrain also draws {}'s BlueMap", remote.level);
                    if let Source::Url { url, .. } = &source {
                        log::debug!("Horizon: BlueMap at {url}");
                    }
                    let map = Arc::new(bluemap::BlueMap::from_source(
                        source,
                        &remote.level,
                        maps,
                        settings.radius_blocks(),
                        bluemap::Palette::from_game(self.pack.assets.registry, self.res.biomes()),
                    ));
                    map.start();
                    map
                });
                self.lod_blue_map = blue_map.clone();
                // And the player's Bobby cache of the level, only read.
                let bobby = terrain::bobby_level_dir(
                    &terrain::minecraft_dir().join(".bobby"),
                    address,
                    remote.hashed_seed,
                    &remote.level,
                )
                .map(|bobby| {
                    log::info!(
                        "Horizon: far terrain also reads Bobby's chunks of {}",
                        remote.level
                    );
                    let registry = self.pack.assets.registry;
                    let biomes = self.res.biomes().clone();
                    let dimension = &remote.dimension;
                    let layout = horizon_client::chunk::ColumnLayout::of_level(
                        dimension.min_y,
                        dimension.height,
                    );
                    let sky_light = dimension.has_skylight;
                    let decode: terrain::DecodeChunk =
                        Box::new(move |root: &horizon_core::nbt::NamedTag| {
                            horizon_client::save::decode_lit(
                                root, registry, &biomes, layout, sky_light,
                            )
                        });
                    terrain::Bobby::new(bobby, decode, self.res.facts())
                });
                terrain::ServerTerrain::with_bobby(dir, blue_map, bobby) as _
            }),
            _ => Some(self.streaming.lod_source()),
        };
        if let Some(source) = source {
            self.lod = Some(horizon_client::lod::Lod::new(source, self.worker_context()));
            self.lod_observe.extend(self.world.chunks.keys().copied());
        }
    }
    fn update_world(&mut self) {
        if let Err(e) = horizon_client::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Horizon: {e}");
        }
        if let Err(e) = self.advance_studio_replay() {
            log::warn!("Horizon Studio replay: {e}");
        }
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went) and dropped every section and far tile: meshed again, as for a new one.
        if self
            .renderer
            .as_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            self.clear_lod();
            self.all_sections_changed();
        }
        self.prepare_lod();
        // The columns the single-player server changed in bulk without sending them, which far terrain still drew
        // as this window last saw them: summarized again (taken every tick, far terrain on or off, so none waits).
        if self.remote.is_some() {
            let changed = horizon_client::lan::handoff::take_changed();
            if let Some(lod) = &mut self.lod {
                let renderer = self.renderer.as_ref();
                for chunk in changed {
                    let column = (chunk.x, chunk.z);
                    if self.world.chunks.contains_key(&column) {
                        continue;
                    }
                    // Inside a mod's hold, far terrain shows it with the sections (`native_world_hold`).
                    if renderer.is_some_and(|r| r.world_holds_column(column))
                        && self.lod_held.len() < HELD_FAR_COLUMNS
                    {
                        self.lod_held.insert(column, chunk);
                    } else {
                        lod.observe(chunk, self.res.facts());
                    }
                }
            }
        }
        if self.lod.is_some() {
            for &((x, _, z), _, _) in &self.world.light_edits {
                self.lod_observe.insert((x >> 4, z >> 4));
            }
        }
        if let (Some(map), Some(p), Some(lod)) = (&self.lod_blue_map, &self.player, &mut self.lod) {
            map.set_center(p.position.x.floor() as i32, p.position.z.floor() as i32);
            for change in map.take_changed() {
                lod.refresh(change.area, change.widths);
            }
        }
        if let Some(p) = &self.player {
            self.streaming.center = (
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            );
        }
        self.tick_pregen();
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.options.headless {
            self.streaming.mesh = self.renderer.is_some();
        }
        let areas = self.streaming.extra.len();
        let waiting = std::mem::take(&mut self.spawn_wait);
        self.streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
        let plan = horizon_client::spans::time("streaming_plan", || {
            let mut step = self.streaming.begin_plan_update(&mut self.world);
            loop {
                match step {
                    horizon_client::streaming::PlanStep::Ready(plan) => break plan,
                    horizon_client::streaming::PlanStep::Save { chunk, resume } => {
                        self.native_late_full_chunk_snapshot(&chunk);
                        self.streaming.save_finished(chunk);
                        step = self.streaming.resume_plan_update(&mut self.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self.streaming.prepare_commit(&mut self.world, plan);
        self.native_proto_snapshots(&protos);
        self.streaming.submit_protos(protos);
        let (uploads, unloaded) = horizon_client::spans::time("streaming", || {
            self.streaming.complete_commit(&mut self.world, commit)
        });
        self.streaming.extra.truncate(areas);
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = horizon_client::spans::time("with_entities", || self.with_entities(&dirty));
        self.native_full_chunk_snapshots(&modified);
        if !modified.is_empty()
            && let Err(e) = horizon_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.pack.assets.registry, world_biomes!(self))
            })
        {
            log::warn!("Horizon: failed to save unloaded chunks: {e}");
        }
        self.spawn_fresh_chunks();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        self.native_chunks_loaded();
        self.native_chunks_unloaded(&unloaded);
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        if let Some(lod) = &mut self.lod {
            self.lod_observe
                .extend(uploads.iter().map(|(key, _)| (key.0, key.2)));
            for chunk in &unloaded {
                self.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then a few resident ones (an edit copies a chunk the worker
            // still holds). The worker summarizes them.
            horizon_client::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.res.facts());
                }
                while lod.observing() < horizon_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.lod_observe.iter().next()
                {
                    self.lod_observe.remove(&key);
                    if let Some(chunk) = self.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.res.facts());
                    }
                }
            });
        }
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        horizon_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            self.world
                .block_entities
                .unload(|p| gone.contains(&(p.0 >> 4, p.2 >> 4)));
            self.world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        // LevelChunk.registerTickContainerInLevel: a chunk that became
        // resident has its tick containers before a command can schedule into
        // it, even in a frozen level that runs no world tick. The unloaded
        // chunks above were saved with their ticks first.
        if self.remote.is_none() {
            self.sync_tick_chunks();
        }
        let (far_distance, far_reduced) = self.far_terrain_choice();
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.streaming.radius.min(r.view_distance + 1),
            None => self.streaming.radius,
        };
        let view = (self.streaming.center, radius);
        let has = |column: (i32, i32)| self.world.chunks.contains_key(&column);
        let now = Instant::now();
        let arriving = self.view_arrival.arriving(view, now, has) || self.remote_backlog() > 0;
        // Which far tiles a zoom magnifies (`lod::ZoomAim`), read before
        // `self.renderer.as_mut()` below borrows self mutably for the rest of
        // this function. Cheap: trigonometry on one angle, or None when the
        // view is not zoomed.
        let lod_aim = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .and_then(|camera| self.lod_aim(camera));
        if let Some(renderer) = self.renderer.as_mut() {
            // Far terrain takes in the columns the hold let go of, as the sections show.
            if !self.lod_held.is_empty() {
                let let_go: Vec<_> = self
                    .lod_held
                    .extract_if(|&column, _| !renderer.world_holds_column(column))
                    .collect();
                if let Some(lod) = &mut self.lod {
                    for (column, chunk) in let_go {
                        if !self.world.chunks.contains_key(&column) {
                            lod.observe(chunk, self.res.facts());
                        }
                    }
                }
            }
            let g = &self.settings.horizon;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            // The meshes, once copied, are freed by a worker (`Streaming::free`).
            if !budgeted && self.pending_uploads.is_empty() && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                horizon_client::spans::time("upload", || {
                    for (key, mesh) in &uploads {
                        renderer.upload(*key, mesh);
                    }
                });
                self.streaming.free(uploads);
            } else {
                // Horizon's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let fresh: horizon_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|(k, _)| *k).collect();
                let chunks = &self.world.chunks;
                let loaded = |k: &(i32, i32, i32)| chunks.contains_key(&(k.0, k.2));
                let mut spent: Vec<_> = pending
                    .extract_if(.., |(k, _)| fresh.contains(k) || !loaded(k))
                    .collect();
                for upload in uploads {
                    if loaded(&upload.0) {
                        pending.push(upload);
                    } else {
                        spent.push(upload);
                    }
                }
                let centre = self.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.world.bottom_y());
                let cam_y = self
                    .player
                    .as_ref()
                    .map_or(4, |p| ((p.position.y - bottom) / 16.0).floor() as i32);
                let far = |k: &(i32, i32, i32)| {
                    (k.0 - centre.0).pow(2) + (k.1 - cam_y).pow(2) + (k.2 - centre.1).pow(2)
                };
                pending.sort_unstable_by_key(|(k, _)| std::cmp::Reverse(far(k)));
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took
                // AUTO_UPLOAD_TIME of this frame (at least one goes): a burst of
                // finished meshes spreads over frames, nearest first.
                let auto = !budgeted;
                let auto_time = if arriving {
                    AUTO_UPLOAD_TIME_ARRIVING
                } else {
                    AUTO_UPLOAD_TIME
                };
                let started = Instant::now();
                while let Some((_, mesh)) = pending.last() {
                    if auto && count > 0 && started.elapsed() >= auto_time {
                        break;
                    }
                    let size = ((mesh.solid.len() + mesh.translucent.len())
                        * std::mem::size_of::<horizon_client::mesher::TerrainVertex>())
                        as u64;
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let (key, mesh) = pending.pop().expect("checked above");
                    renderer.upload(key, &mesh);
                    spent.push((key, mesh));
                    count += 1;
                    bytes += size;
                }
                self.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.streaming.busy() || !self.pending_uploads.is_empty();
            let sampling = self.lod.as_ref().is_some_and(|lod| lod.sampling());
            renderer.set_terrain_loading(
                arriving || (meshing && self.view_arrival.recent(now)) || sampling,
            );
            if let (Some(lod), Some(player)) = (&mut self.lod, &self.player) {
                let lod_distance =
                    horizon_client::graphics::lod_distance_capped(far_distance, &renderer.info);
                lod.set_near_loading(self.streaming.terrain_loading());
                lod.set_reduced_detail(far_reduced);
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.streaming.center,
                    self.streaming.radius,
                );
                let covered = (self.lod_covered_at != Some(at)).then(|| {
                    self.lod_covered_at = Some(at);
                    horizon_client::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                horizon_client::streaming::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                // Far terrain's finest tiles stand where full-detail terrain
                // ends: on a server, where it stops sending chunks, often well
                // inside the render distance. Thousands of chunk-sized tiles
                // between the two filled in long after the coarser ones.
                let detail = self.remote.as_ref().map_or(self.streaming.radius, |r| {
                    self.streaming.radius.min(r.sent_radius())
                });
                let camera = [player.position.x, player.position.y, player.position.z];
                let meshes = horizon_client::spans::time("lod_update", || {
                    lod.update(camera, detail, lod_distance, covered, lod_aim)
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                horizon_client::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
            }
        }
    }
}
/// `computeNextAutosaveInterval`'s arithmetic: `Math.max(100, (int)(rate * 300.0F))`.
fn autosave_interval(rate: f32) -> i64 {
    i64::from((rate * 300.0) as i32).max(100)
}

/// clouds.png as filled cells (`CloudRenderer.prepare`: alpha 10 or more).
fn load_clouds(root: &horizon_assets::resource_source::ResourceSource) -> (u32, u32, Vec<bool>) {
    root.read("assets/minecraft/textures/environment/clouds.png")
        .map_err(Into::into)
        .and_then(|bytes| horizon_client::hud::decode_bytes(&bytes))
        .map(|(w, h, px)| {
            (
                w,
                h,
                px.as_chunks::<4>().0.iter().map(|p| p[3] >= 10).collect(),
            )
        })
        .unwrap_or((1, 1, vec![false]))
}
/// misc/forcefield.png for the world border wall (square, power of two).
fn load_forcefield(root: &horizon_assets::resource_source::ResourceSource) -> (u32, u32, Vec<u8>) {
    root.read("assets/minecraft/textures/misc/forcefield.png")
        .map_err(Into::into)
        .and_then(|bytes| horizon_client::hud::decode_bytes(&bytes))
        .ok()
        .filter(|(w, h, _)| w == h && w.is_power_of_two())
        .unwrap_or((1, 1, vec![0; 4]))
}
impl<'a> Game<'a> {
    /// A game with no world open, over the loaded resources: the window's,
    /// or an integrated server's on its own thread.
    fn new(
        res: &'a Arc<Resources>,
        pack: PackResources,
        gui: UiResources,
        options: Options,
        settings: Settings,
        audio: Audio,
        prepared: Option<GameCpuResources>,
    ) -> Result<Self> {
        let phase = match mod_api::__private::current_phase() {
            mod_api::__private::ModPhase::Packages if cfg!(test) => {
                mod_api::__private::ModPhase::World
            }
            mod_api::__private::ModPhase::Packages => mod_api::__private::ModPhase::Setup,
            phase => phase,
        };
        let mod_phase = mod_api::__private::PhaseOwner::new(phase);
        // A scoped hook on this thread asks this game's parts.
        mod_api::__private::mark_game_thread();
        let standalone_geometry = native_block_entity_draw::StandaloneGeometry::new(&pack, &gui);
        let loop_side = options.execution_role.loop_side();
        let root = options.resources.clone();
        let mod_sources = options.mods.clone();
        let registry = pack.assets.registry;
        // Until a world is opened the streamer is idle.
        let mut streaming = Streaming::new(
            Arc::new(TestTerrain::new(registry, res.biomes())?),
            pack.context.clone(),
            1,
        );
        streaming.paused = true;
        let level = Level::default();
        let save = Arc::new(SaveDir::new(
            std::env::current_dir()?
                .join("saves")
                .join(".horizon-title"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        let prepared = prepared.map(Ok).unwrap_or_else(|| {
            GameCpuResources::load(
                &horizon_assets::resource_source::ResourceRoot::new(
                    root.clone(),
                    packs::cache_dir(),
                    pack.source.clone(),
                ),
                None,
            )
        })?;
        let world = World::new(res.facts().air, 0);
        let tutorial = tutorial::Tutorial::new(tutorial::Step::from_option(
            settings.file.int("tutorialStep"),
        ));
        let creative = options.creative;
        let inventory = vec![None; 36];
        let now = Instant::now();
        let look = options.look;
        let particles = horizon_client::particles::ParticleEngine::new(
            &pack.source,
            &gui.atlas.sprites,
            &gui.atlas.translucent,
        );
        let resource_source = pack.source.clone();
        let game = Game {
            structure_spawns: horizon_client::mobs::spawn_selection::StructureSpawns::load(
                &options.resources,
            ),
            world_biomes: None,
            world_biomes_base: None,
            res: res.clone(),
            rest_loader: None,
            pending_entry: None,
            debug_density: RefCell::default(),
            pending_pack_reload: false,
            frame_attempted: render_settings::FrameAttempt::default(),
            pack_folder: None,
            pack_reload: None,
            server_packs: Default::default(),
            link_prompt: None,
            command_prompt: None,
            code_of_conduct: None,
            dialogs: Default::default(),
            integrated_starting: None,
            world_renderer_ready: false,
            collision: res.collision(),
            motion: res.motion(),
            health_resources: res.motion().health.clone(),
            damage_tag_generation: 1,
            data_map_registries: Vec::new(),
            pack,
            gui,
            instance_groups: Default::default(),
            standalone_geometry,
            fog: Default::default(),
            screen_fx: Default::default(),
            item_activation: Default::default(),
            weather_parameters: Default::default(),
            panorama_rotation: Default::default(),
            world,
            streaming,
            lod: None,
            lod_source_attempted: false,
            far_beacons: HashMap::new(),
            lod_observe: Default::default(),
            lod_held: Default::default(),
            lod_level: None,
            lod_blue_map: None,
            lod_covered_at: None,
            view_arrival: Default::default(),
            options,
            player: None,
            spawn: None,
            clock: FixedStep::default(),
            previous_position: Vec3::ZERO,
            rope_rot_old: (0.0, 10.0),
            eye: 1.62,
            previous_eye: 1.62,
            yaw: look.map_or(level.rotation[0], |l| l.0),
            pitch: look.map_or(level.rotation[1], |l| l.1),
            inventory,
            selected: 0,
            carried: None,
            craft_grid: vec![None; 9],
            craft_size: 2,
            craft_table: None,
            inv_drag: None,
            tweaks: Default::default(),
            bundle_mouse: Default::default(),
            inv_clicks: Default::default(),
            last_inv_click: None,
            last_quick_moved: None,
            container: None,
            open_container_slots: None,
            station: None,
            potions: Default::default(),
            scratch_slot: None,
            mining: None,
            destroying: HashMap::new(),
            destroy_delay: 0,
            use_delay: 0,
            creative,
            mode: if creative {
                horizon_client::command::args::GameMode::Creative
            } else {
                horizon_client::command::args::GameMode::Survival
            },
            keys: Default::default(),
            physical_keys: HashSet::new(),
            rejected_keys: HashSet::new(),
            physical_buttons: HashSet::new(),
            drag_button: None,
            right_click_emulation: Default::default(),
            left: false,
            right: false,
            cursor: (0.0, 0.0),
            cursor_icon: horizon_client::widgets::Cursor::Arrow,
            captured: false,
            capture_mode: None,
            regrab_on_refocus: false,
            keyboard_mouse: true,
            split_window: false,
            view_at: [0, 0],
            split_hud: Default::default(),
            split_auth: None,
            focused: true,
            unclaimed_window: false,
            focus_after_run: None,
            ime_allowed: false,
            ime_area: None,
            focus_lost: None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            fullscreen: horizon_client::window::FullscreenState::new(false),
            video_modes: Default::default(),
            mouse: MouseLook::default(),
            agent_pointer: false,
            smooth_camera: false,
            toggles: Default::default(),
            was_playing: false,
            pause: false,
            debug: false,
            gpu_passes: false,
            screen: Screen::Playing,
            observed_screen: None,
            effects_layout: std::cell::Cell::new(None),
            window: None,
            resize_state: None,
            renderer: None,
            last_device_recovery: None,
            entity_vertices: Vec::new(),
            dynamic_lights: Vec::new(),
            luminance: Default::default(),
            see_through_vertices: Vec::new(),
            shadow_only_vertices: Default::default(),
            entity_spans: Default::default(),
            entity_motion: Default::default(),
            entity_motion_posed: Default::default(),
            skin_models: Default::default(),
            skin_frame: Default::default(),
            glowing_vertices: Default::default(),
            mod_sounds: Default::default(),
            mod_clear_particles: Default::default(),
            pack_ids: None,
            health_hud: hud_layout::HealthHud::default(),
            started: now,
            spectator_gui: Default::default(),
            previous: now,
            frame_end: now,
            last_input: now,
            pad: Default::default(),
            frames: 0,
            glint_frame: Default::default(),
            kept_block_entities: Default::default(),
            prepared_model_draws: Default::default(),
            world_geometry: Some(Default::default()),
            debug_renderers: Vec::new(),
            debug_renderers_version: None,
            hud_layer_next: None,
            mod_view_distances: Default::default(),
            item_level_cache: Default::default(),
            world_frames: 0,
            fps: 0.0,
            fps_frames: 0,
            fps_at: now,
            ticks: 0,
            texture_ticks: 0,
            error: None,
            message: None,
            save,
            level,
            player_autosaves: Default::default(),
            session_lock: None,
            world_adopted: false,
            audio,
            loop_context: Arc::new(res.core.item_data.context().loop_context(loop_side, [])),
            setting_overrides: Default::default(),
            damage_hooks: native_hooks::DAMAGE_HOOKS,
            music: {
                // The manager starts with the saved option (MusicManager's constructor).
                let mut music = horizon_client::music::MusicManager::default();
                music.frequency = horizon_client::music::MusicFrequency::parse(
                    &settings.file.name("musicFrequency"),
                );
                music
            },
            music_layers: Default::default(),
            // ToastManager.initializeMusicToast: Pause and Pause and Toast make the toast.
            now_playing: (settings.file.int("musicToast").clamp(0, 2) != 0).then(Default::default),
            music_toast_state: settings.file.int("musicToast").clamp(0, 2) as u8,
            music_notes: Default::default(),
            ambience: Default::default(),
            sound_paused: false,
            cart_sounds: HashSet::new(),
            riding_cart_sounds: None,
            riding_sound: None,
            was_fall_flying: false,
            bee_sounds: HashMap::new(),
            guardian_sounds: HashSet::new(),
            guardian_look_scratch: Vec::new(),
            dropped: Default::default(),
            next_entity: 0,
            ticked_entities_through: u32::MAX,
            contact_drops: 0..0,
            particles,
            pickups: Default::default(),
            sign_edit: None,
            book: None,
            book_last_click: None,
            lectern_pulses: Vec::new(),
            loading_progress: 0.0,
            creation_job: None,
            loading_progress_at: None,
            moving_pistons_stale: false,
            spawn_origin: None,
            spawn_hook_fired: false,
            spawn_hook_cancelled: false,
            spawn_spiral: Default::default(),
            player_spawn: None,
            spawn_wait: Vec::new(),
            mobs: Default::default(),
            mob_index: Default::default(),
            creatures: Default::default(),
            sculk: Default::default(),
            raiding: Default::default(),
            attack_ticks: 100,
            attack_item: None,
            last_hurt_by_player: HashMap::new(),
            lead_killers: HashMap::new(),
            love_cause: HashMap::new(),
            died: Default::default(),
            be_anims: {
                let mut anims = horizon_client::block_entity_anim::BlockEntityAnims::default();
                anims.classify(registry);
                anims
            },
            using: None,
            use_duration: 0,
            use_ticks_set: false,
            crossbow_sounds: [false; 2],
            view: Default::default(),
            limiter: horizon_client::window::FramerateLimiter::new(0),
            frame_clock: Default::default(),
            scope_scale: 0.5,
            frame_ticks: 0.0,
            title: None,
            armor: [None, None, None, None],
            offhand: None,
            equipment: Default::default(),
            hotbar_pop: [0; 10],
            hand: Hand::Main,
            shield_cooldown: 0,
            cooldowns: Default::default(),
            command_source: None,
            native_command_filter: false,
            titles: Default::default(),
            tool_highlight: Default::default(),
            saving_indicator: Default::default(),
            autosaving: false,
            // `computeNextAutosaveInterval` at the clock's 20 ticks a second.
            ticks_until_autosave: 6000,
            applied_modifiers: Vec::new(),
            experience: Experience::default(),
            orbs: Default::default(),
            xp_delay: 0,
            ordered: Default::default(),
            level_up_sound: Default::default(),
            swing: Default::default(),
            swings_started: 0,
            predicted_swings: 0,
            heights: Default::default(),
            sway: Default::default(),
            visible_hands: [(0, None), (0, None)],
            hands_replaced: [false; 2],
            clicks: Default::default(),
            attack_clicks: 0,
            use_clicks: 0,
            hand_strength: Default::default(),
            settings,
            tutorial,
            tutorial_toasts: Vec::new(),
            narrator_toast: None,
            narration: Default::default(),
            options_from_title: false,
            dragging: false,
            avatar: Default::default(),
            flash: 0,
            custom_sky: Default::default(),
            rain_sound_time: 0,
            lightning_seen: [0; 2],
            sleeping: None,
            sleep_timer: 0,
            player_beds: Vec::new(),
            sleep_seen: (0, 0),
            dim: Dim::Overworld,
            world_root: PathBuf::new(),
            owner_migration: None,
            world_flat: false,
            world_debug: false,
            server_trips: Vec::new(),
            level_teleports: Vec::new(),
            teleport_notes: Vec::new(),
            levels: Default::default(),
            level_pass: Default::default(),
            portals: Default::default(),
            respawn_config: None,
            end_portal_travel: false,
            win: None,
            end: Default::default(),
            block_ticks: Ticks::new(0),
            tick_scratch: TickScratch::default(),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            falling: Default::default(),
            transient_blocks: Default::default(),
            paintings: Default::default(),
            painting_variants: prepared.painting_variants,
            item_frames: Default::default(),
            frame_model: prepared.frame_model,
            glow_frame_model: prepared.glow_frame_model,
            map_frame_model: prepared.map_frame_model,
            glow_map_frame_model: prepared.glow_map_frame_model,
            armor_stands: Default::default(),
            location_armor: Default::default(),
            cushions: Default::default(),
            others: Default::default(),
            armor_stand_assets: prepared.armor_stand_assets,
            waypoint_styles: prepared.waypoint_styles,
            waypoints: Default::default(),
            locator_camera: None,
            native_camera_frame: None,
            posed_rotation: None,
            post_effects: Vec::new(),
            polytone_post_ticks: 0.0,
            spectated_post_effect: None,
            experience_display: Default::default(),
            boats: Default::default(),
            leash_boats: Vec::new(),
            boat_models: prepared.boat_models,
            crystal_model: prepared.crystal_model,
            cushion_model: prepared.cushion_model,
            entity_models: prepared.entity_models,
            carts: Default::default(),
            watched_vehicles: Vec::new(),
            jukebox_sounds: HashMap::new(),
            bolts: Vec::new(),
            maps: maps::Maps::default(),
            fight: dragon::Fight::default(),
            dragon: Default::default(),
            crystals: Default::default(),
            breath_clouds: Default::default(),
            dragon_model: prepared.dragon_model,
            map_patches: Vec::new(),
            name_tag_layouts: Default::default(),
            text_displays: Default::default(),
            interactions: Default::default(),
            remote_knots: Default::default(),
            knot_ids: Default::default(),
            text_display_vertices: 0,
            skin_downloads: RefCell::new({
                let mut downloads = horizon_client::skins::Downloader::new(
                    std::env::current_dir()
                        .unwrap_or_default()
                        .join("assets/skins"),
                );
                downloads.set_resource_root(resource_source.clone());
                downloads
            }),
            skin_profiles: RefCell::new(horizon_client::skins::Resolver::new()),
            skin_slots: horizon_client::skins::SkinSlots::new(horizon_client::hud::SKIN_SLOTS),
            skin_3d: Default::default(),
            trim_slots: Default::default(),
            cart_models: prepared.cart_models,
            riding: None,
            started_riding: Vec::new(),
            rotation_forced: false,
            posed_rider: None,
            pending_vehicle: None,
            ender_items: vec![None; 27],
            ender_open: None,
            pending: Default::default(),
            spill: Default::default(),
            recipe_ui: Default::default(),
            mount_menu: Default::default(),
            hook: None,
            remote_hooks: Vec::new(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            creaking: Default::default(),
            spawners: Default::default(),
            spawn_scratch: Default::default(),
            trident_model: prepared.trident_model,
            arrow_model: prepared.arrow_model,
            bee_stinger_model: prepared.bee_stinger_model,
            tnts: Default::default(),
            hoppers: HashMap::new(),
            transactions: Vec::new(),
            contents_changing: Vec::new(),
            mod_screen: None,
            mod_container_viewers: HashMap::new(),
            item_columns: None,
            pickup_columns: None,
            pass_mobs: None,
            spawner_tick_positions: Vec::new(),
            trial_spawner_tick_positions: Vec::new(),
            creative_screen: prepared.creative_screen,
            creative_ignore_text: false,
            control: None,
            agent_control_on: false,
            agent_last: None,
            frame_stats: Default::default(),
            pending_screenshot: None,
            screenshot_sequence: None,
            pending_render_capture: None,
            pending_bench: None,
            flight: None,
            ui_wait: Default::default(),
            events: Default::default(),
            input_trace: Default::default(),
            repro: Default::default(),
            tick_script: tick_script::TickScript::from_env(),
            agent: agent::Agent::default(),
            view_frustum: None,
            entity_section_visibility_valid: false,
            entity_section_camera: None,
            sync_occlusion_culler: None,
            async_occlusion_culler: None,
            async_occlusion_culling_failed: false,
            async_particle_culler: RefCell::new(Default::default()),
            pack_shadow_casters: (false, false),
            pending_uploads: Vec::new(),
            frame_graph: Default::default(),
            extras: Default::default(),
            pvp: Default::default(),
            mods: mod_sources,
            mod_jobs: Default::default(),
            recipe_tables: Default::default(),
            panel: Default::default(),
            native_shaders: Default::default(),
            native_input: Default::default(),
            native_lifecycle: Default::default(),
            lent_mobs: Default::default(),
            held_out: Vec::with_capacity(4),
            native_configs: Default::default(),
            mod_phase,
            native_net: Default::default(),
            native_mod_screen: None,
            sky_darken_memo: Default::default(),
            water_plane_memo: Default::default(),
            water_plane_scan: Default::default(),
            vehicle_mob_reach: Default::default(),
            persist: Default::default(),
            missing_professions: Default::default(),
            entity_attachments: Default::default(),
            entity_initialization: Default::default(),
            attachment_indexes: Default::default(),
            remote: None,
            studio_replay: None,
            studio_freecam: Default::default(),
            studio_timeline: Default::default(),
            studio_topbar: Default::default(),
            studio_rmb_down: false,
            studio_camera_track_dirty: false,
            studio_replay_previous_camera_type: None,
            studio_replay_standalone: false,
            last_replay_tick: None,
            studio_virtual_tick_ms: 0.0,
            lan: None,
            integrated: None,
            level_shapes: Default::default(),
            serving: None,
            world_options: None,
            options_to_world_options: false,
            options_to_game: false,
            command_block_screen: None,
            block_editor: None,
            structure_templates: HashMap::new(),
            test_defs: std::cell::RefCell::new(HashMap::new()),
            test_defs_snapshot: false,
            test_highlights: HashMap::new(),
            game_tests: game_tests::GameTests::default(),
            #[cfg(feature = "friends")]
            friends: None,
            social: Default::default(),
            debug_keys: Default::default(),
            debug_charts: Default::default(),
            debug_entries: horizon_client::debug_entries::DebugEntries::load(
                &std::env::current_dir().unwrap_or_default(),
            ),
            debug_options: None,
            memory_meter: Default::default(),
            debug_paint: Default::default(),
            debug_refresh: Default::default(),
            gpu_utilization: 0.0,
            gpu_culling_auto: horizon_client::autotune::GpuCullingAuto::new(),
            rendered_mobs: Default::default(),
            screen_blur_requested: Default::default(),
            spawn_counts: HashMap::new(),
            spawn_policies: HashMap::new(),
            pause_menu: true,
            pause_focus: None,
            death_focus: None,
            focus_screen: 0,
            previous_mode: None,
            gamemode_switcher: None,
            clipboard: Default::default(),
            user_input: Default::default(),
            text_selection: Default::default(),
            saved_for_exit: false,
            pending_save: None,
            server_tick_deadline: None,
            frame_drawn: true,
            world_icon: None,
            forced_hit: None,
            tick_hit: None,
            tick_mob: None,
            hotbar_presses: Vec::new(),
            entity_reach: 3.0,
            block_reach: 4.5,
            guest_use: None,
            use_failed: false,
            lan_events: Vec::new(),
            lan_songs: Vec::new(),
            lan_cushion_breaks: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_kinetic_hits: Vec::new(),
            stabbed: HashMap::new(),
            stab_players: Vec::new(),
            player_stabs: Vec::new(),
            known_movement: Vec3::ZERO,
            kinetic_feedback: HashMap::new(),
            lan_global_events: Vec::new(),
            lid_openers: HashMap::new(),
            lan_acting: None,
            acting_abilities: None,
            native_commands_refresh: false,
            lan_building_boxes: Default::default(),
            save_after_guest_turn: false,
            lan_guest_swaps: Vec::new(),
            player_attachments: Default::default(),
            lan_break_pos: None,
            break_collect: None,
            guest_packets: Vec::new(),
            guest_cooldowns: Vec::new(),
            guest_entity_events: Vec::new(),
            guest_equipment_breaks: Vec::new(),
            guest_block_sounds: Vec::new(),
            guest_player_sounds: Vec::new(),
            bound_sounds: Vec::new(),
            lan_acting_name: None,
            lan_acting_uuid: None,
            tnt_refused: None,
            lan_acting_permission: None,
            lan_acting_spawn_protection: None,
            lan_acting_entity: None,
            lan_acting_fake: false,
            fake_action: Default::default(),
            chat_capture: None,
            admin_feedback: Vec::new(),
            console_broadcast: None,
            rcon_source: false,
            server_lists: None,
            world_tables: None,
            server_loot: Default::default(),
            server_items: None,
            cmds: Default::default(),
            reload_resources: Default::default(),
            ui: chat::UiState::new(),
            combat: Default::default(),
            guest_combat: Default::default(),
            mount: Default::default(),
            stats: Default::default(),
            score: Default::default(),
            fns: Default::default(),
            bosses: Default::default(),
            boss_overlay: Default::default(),
            exec: Default::default(),
            rand_sequences: Default::default(),
            brushing: Default::default(),
            border: Default::default(),
            border_renderer: Default::default(),
            border_section: None,
            border_state: Default::default(),
            tab_list: Default::default(),
            subtitles: Default::default(),
            adv: Default::default(),
            restore_player_state: true,
            stats_ui: Default::default(),
            adv_ui: Default::default(),
            links_ui: Default::default(),
            camera_type: Default::default(),
            requested_camera_type: Default::default(),
            fangs: Default::default(),
            player_model: None,
        };
        Ok(game)
    }
}

/// CPU-only startup data, owned by one loader generation. No file reads or
/// model baking are needed when the event loop installs this bundle.
struct GameCpuResources {
    painting_variants: paintings::PaintingVariants,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    boat_models: boats::BoatModels,
    cart_models: minecarts::CartModels,
    crystal_model: Option<horizon_client::mobs::Part>,
    cushion_model: Option<horizon_client::mobs::Part>,
    dragon_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    bee_stinger_model: Option<horizon_client::mobs::Part>,
    trident_model: Option<horizon_client::mobs::Part>,
    entity_models: EntityModels,
    creative_screen: Creative,
}
/// Baked layers of entities that may first show up deep into a session. They come from the
/// document parsed once at startup: parsing entity-models.json (2.4 MB) when the first one
/// appeared stalled that frame by 40 ms of CPU and the file read.
#[derive(Default)]
struct EntityModels {
    /// LeashKnotModel.
    leash_knot: Option<horizon_client::mobs::Part>,
    /// WindChargeModel.
    wind_charge: Option<horizon_client::mobs::Part>,
    /// ShulkerBulletModel.
    shulker_bullet: Option<horizon_client::mobs::Part>,
    /// SkullModel of the wither skull.
    wither_skull: Option<horizon_client::mobs::Part>,
    /// EvokerFangsModel.
    evoker_fangs: Option<Arc<horizon_client::mobs::Part>>,
    /// BannerModel's pole and flag (a raid captain's banner).
    banner: Option<(horizon_client::mobs::Part, horizon_client::mobs::Part)>,
}
impl EntityModels {
    fn load(document: Option<&horizon_client::mobs::EntityModelDocument>) -> Self {
        let model = |key| document.and_then(|document| document.model(key));
        Self {
            leash_knot: model("minecraft:leash_knot#main"),
            wind_charge: model("minecraft:wind_charge#main"),
            shulker_bullet: model("minecraft:shulker_bullet#main"),
            wither_skull: model("minecraft:wither_skull#main"),
            evoker_fangs: model("minecraft:evoker_fangs#main").map(Arc::new),
            banner: model("minecraft:standing_banner#main")
                .zip(model("minecraft:standing_banner#flag")),
        }
    }
}
impl GameCpuResources {
    fn load(
        resources: &horizon_assets::resource_source::ResourceRoot,
        document: Option<&horizon_client::mobs::EntityModelDocument>,
    ) -> Result<Self> {
        let root = resources.prepared();
        let owned_document;
        let document = match document {
            Some(document) => Some(document),
            None => match horizon_client::mobs::EntityModelDocument::load(root) {
                Ok(document) => {
                    owned_document = document;
                    Some(&owned_document)
                }
                Err(_) => None,
            },
        };
        let model = |key| document.and_then(|document| document.model(key));
        let (
            painting_variants,
            frames,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            creative_screen,
        ) = std::thread::scope(|scope| {
            let painting_variants = named_scope(scope, "game_cpu.paintings", move || {
                paintings::PaintingVariants::load(root)
            });
            let frames = named_scope(scope, "game_cpu.frames", move || {
                frames::bake_frames(resources.source())
            });
            let armor_stand_assets = named_scope(scope, "game_cpu.armor_stands", move || {
                armor_stands::ArmorStandAssets::load(resources.source(), document)
            });
            let waypoint_styles = named_scope(scope, "game_cpu.waypoints", move || {
                horizon_client::locator_bar::WaypointStyles::load(resources.source())
            });
            let boat_models = named_scope(scope, "game_cpu.boats", move || {
                boats::BoatModels::load(document)
            });
            let cart_models = named_scope(scope, "game_cpu.carts", move || {
                minecarts::CartModels::load(document)
            });
            let creative_screen = named_scope(scope, "game_cpu.creative", move || {
                Creative::load(root).map_err(|error| error.to_string())
            });
            (
                painting_variants
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                frames
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                armor_stand_assets
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                waypoint_styles
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                boat_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                cart_models
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
                creative_screen
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            )
        });
        let [
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
        ] = frames;
        Ok(Self {
            painting_variants,
            frame_model,
            glow_frame_model,
            map_frame_model,
            glow_map_frame_model,
            armor_stand_assets,
            waypoint_styles,
            boat_models,
            cart_models,
            crystal_model: model("minecraft:end_crystal#main"),
            cushion_model: model("minecraft:cushion#main"),
            dragon_model: model("minecraft:ender_dragon#main"),
            arrow_model: model("minecraft:arrow#main"),
            bee_stinger_model: model("minecraft:bee_stinger#main"),
            trident_model: model("minecraft:trident#main"),
            entity_models: EntityModels::load(document),
            creative_screen: creative_screen
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
        })
    }
}

/// The data the background loader produces for `Game::new`.
struct Loaded {
    game_cpu: Option<GameCpuResources>,
    /// The one parsed `entity-models.json`, for the handoff `Game::new` does
    /// itself on the synchronous launch.
    entity_models: Option<Arc<horizon_client::mobs::EntityModelDocument>>,
    renderer_cpu: Option<horizon_client::gfx::PreparedTextures>,
    root: PathBuf,
    pack_error: Option<String>,
    resources: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    /// The client reload listeners' prepared applies, staged by the loader
    /// thread; `start_game` runs them once the game exists.
    client_applies: Vec<client_reload::PreparedClientReload>,
    pack: PackResources,
    /// The menu bundle, for the synchronous launch; the loading screen receives
    /// it earlier through its own channel.
    gui: Option<UiResources>,
}

/// What the background loader publishes: the menu bundle as soon as it is
/// ready, then the core bundle with a handle to the rest worker.
enum LoaderMsg {
    Ui(UiResources, PathBuf),
    Done(std::result::Result<Loaded, String>),
}

enum PendingWorldEntry {
    Local {
        folder: String,
        name: Option<String>,
        seed: Option<i64>,
        creative: bool,
        create: Option<Box<horizon_client::title::WorldOptions>>,
    },
    Server {
        address: String,
        name: String,
        lan: bool,
        /// A transfer's carried-over server cookies (`TransferState`).
        cookies: Option<horizon_client::net::Cookies>,
    },
    StudioReplay(std::path::PathBuf),
}

struct WorldRestLoader {
    progress: Progress,
    handle: JoinHandle<std::result::Result<(), String>>,
}

impl WorldRestLoader {
    fn ready(&self) -> Result<bool> {
        // No time limit: the title stays usable while this loads, and a slow
        // disk or a debug build can take minutes. A failure ends the thread
        // and is reported by `finish`.
        Ok(self.handle.is_finished())
    }

    fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| "world resource loader panicked")?
            .map_err(Into::into)
    }
}

/// `key.hotbar.1` to `key.hotbar.9` in a menu.
const HOTBAR_DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
/// The other keys `InputConstants.Key.getNumericKeyValue` gives a digit.
const NUMERIC_KEYS: [KeyCode; 11] = [
    KeyCode::Digit0,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
];

fn load_world_rest(
    resource_root: &horizon_assets::resource_source::ResourceRoot,
    resources: &Resources,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[horizon_client::blocks::GameplayProjection],
    entity_models: Option<&horizon_client::mobs::EntityModelDocument>,
    item_defaults: &horizon_world::item_defaults::ItemDefaults,
    visibility: horizon_world::block_render::FaceVisibility,
    progress: &Progress,
    started: Instant,
    // False on a dedicated server, which draws nothing: no block entity
    // renderer or animate tables (client-side display ticks).
    draws: bool,
) -> Result<WorldRest> {
    let root = resource_root.prepared();
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry = assets.registry;
    let content = horizon_client::datapack::DataPack::from_source(
        "horizon-content".into(),
        resource_root.data_changes().clone(),
    );
    let files = horizon_client::datapack::DataStore::load(&[&content])
        .files
        .into_iter()
        .collect::<Vec<_>>();
    let inventory = horizon_mod_package::enabled::EnabledMods::read(root)?;
    let conditions = horizon_world::conditions::LoadConditions::new(inventory.server_ids());
    let overlay = &horizon_world::loot::PackOverlay {
        conditions: &conditions,
        files: &files,
    };
    progress.set(6);
    // Independent tables load together, within the worker budget the settings
    // allow. Only the recipes and the stations read another table: the items.
    let load_items = || -> Result<_> {
        let items = startup::time("rest.items", || {
            Items::load(root, registry, facts, overlay, resources.item_defaults())
        })?;
        mod_api::__private::resolve_items(items.registry_order.iter().map(|id| &**id)).map_err(
            |errors| {
                errors
                    .into_iter()
                    .map(|error| {
                        format!("mod {} requested unknown item {}", error.mod_id, error.id)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
        )?;
        log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
        progress.set(7);
        // The recipe tables, the stations and the recipe book read one folder:
        // hold its files so it is read once.
        let _recipe_files = horizon_client::json_folder::read(&root.join("data/minecraft/recipe"));
        let sources = startup::time("rest.recipe_sources", || {
            lan::load_sources(
                root,
                &resources.core.item_data,
                resources.core.item_data.context().origin(),
                overlay,
            )
        })?;
        let recipes = startup::time("rest.recipes", || {
            Recipes::from_resolved(root, &items, overlay, &sources)
        })?;
        let stations = startup::time("rest.stations", || {
            horizon_client::stations::StationData::from_sources(
                root,
                &items,
                overlay,
                resources.item_defaults(),
                &sources,
            )
        });
        Ok((items, recipes, stations))
    };
    let load_behaviours = || {
        startup::time("rest.behaviours", || {
            Behaviours::load_with_gameplay(registry, root, behaviour_json, gameplay_projection)
        })
    };
    let load_block_entity_render = || -> Result<_> {
        let _step = startup::step("rest.block_entity_render");
        if !draws {
            return Ok(Default::default());
        }
        let mut renderer = BlockEntityRenderer::load_with_visibility(
            resource_root,
            registry,
            visibility,
            resources.item_defaults(),
        )?;
        renderer.add_mod_signs(facts, registry);
        Ok(renderer)
    };
    let load_animate = || -> Result<_> {
        let _step = startup::step("rest.animate_fog");
        let animate = if draws {
            horizon_client::animate_tick::AnimateTables::load(root, registry, biomes)?
        } else {
            Default::default()
        };
        Ok((animate, horizon_client::fog::FogTables::load(root, biomes)))
    };
    let load_mobs = || -> Result<_> {
        let _step = startup::step("rest.mobs");
        let mut types = if draws {
            match entity_models {
                Some(document) => MobTypes::load_with_document_and_defaults(
                    root,
                    document,
                    item_defaults.entries(),
                )?,
                None => MobTypes::load_with_defaults(root, item_defaults.entries())?,
            }
        } else {
            MobTypes::load_for_server(root)?
        };
        // #845: the declared sets take their types' rows — the merged rows
        // the live attribute map's catalog took in core (the template's with
        // the declared set on top) — as their attribute map and spawn
        // fields, so a mob of the type spawns with what the map answers.
        for declaration in mod_api::__private::entity_attribute_declarations() {
            let row = resources
                .motion()
                .attributes
                .entity_row(&declaration.id)
                .map(std::sync::Arc::new);
            match (types.types.get_mut(&declaration.id), row) {
                (Some(kind), Some(row)) => {
                    kind.take_attribute_row(row, &resources.motion().attributes)
                }
                (Some(_), None) => log::warn!(
                    "Horizon: mod entity attributes of {} never reached the catalog; ignored",
                    declaration.id
                ),
                (None, _) => log::warn!(
                    "Horizon: mod entity attributes of {} name no loaded entity type; ignored",
                    declaration.id
                ),
            }
        }
        // #846: the types mods added attributes to take their grown rows the
        // same way, so a new mob of the type spawns with them.
        resource_load::take_modified_attribute_rows(
            &mut types.types,
            &resources.motion().attributes,
            mod_api::__private::entity_attribute_modifications(),
        );
        let mob_types: HashMap<_, _> = types
            .types
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect();
        let mut mob_data = horizon_client::mobs::MobData::load(resource_root, facts);
        // #847: the mods' spawn rules (`RegisterSpawnPlacementsEvent`) take
        // their types' placement, heightmap and predicate here, once, so a
        // spawn check reads one table with no pass over the mods.
        for declaration in mod_api::__private::spawn_placement_declarations() {
            if !mob_types.contains_key(&declaration.id) {
                return Err(format!(
                    "mod spawn placement of {} names no loaded entity type",
                    declaration.id
                )
                .into());
            }
            mob_data.register_mod_spawn(&declaration.id, &declaration.rule);
        }
        mob_data.cem = std::sync::RwLock::new(Arc::new(if draws {
            horizon_client::mobs::cem::CemModels::load(resource_root.source(), &mob_types)
        } else {
            horizon_client::mobs::cem::CemModels::default()
        }));
        horizon_client::mobs::trial_spawner::load_registry(root);
        Ok((mob_types, mob_data))
    };
    let load_transformers = || -> Result<_> {
        let _step = startup::step("rest.transformers_models");
        let transformers = horizon_client::block_transform::Transformers::load(root, registry)?;
        let player_type = match entity_models {
            Some(document) => perspective::player_type_with_document(root, document),
            None => perspective::player_type(root),
        };
        let models = (
            player_type,
            player_arms(root, "minecraft:player#main", entity_models),
            player_arms(root, "minecraft:player_slim#main", entity_models),
            match entity_models {
                Some(document) => document.model("minecraft:spin_attack#main"),
                None => horizon_client::mobs::load_model(root, "minecraft:spin_attack#main"),
            },
        );
        Ok((transformers, models))
    };
    // The errors cross threads as text.
    fn text<T, E: std::fmt::Display>(
        result: std::result::Result<T, E>,
    ) -> std::result::Result<T, String> {
        result.map_err(|e| e.to_string())
    }
    let (
        items,
        behaviours,
        block_entity_render,
        animate,
        mobs,
        transformers,
        providers,
        music,
        ambient,
        nether_environment,
        end_environment,
    ) = if threads > 1 {
        std::thread::scope(|scope| {
            let items = named_scope(scope, "items", || text(load_items()));
            let behaviours = named_scope(scope, "behaviours", || text(load_behaviours()));
            let block_entity_render = named_scope(scope, "block_entity_render", || {
                text(load_block_entity_render())
            });
            let animate = named_scope(scope, "animate", || text(load_animate()));
            let mobs = named_scope(scope, "mobs", || text(load_mobs()));
            let transformers = named_scope(scope, "transformers", || text(load_transformers()));
            let providers = named_scope(scope, "providers", || {
                startup::time("rest.providers", || Providers::load(root))
            });
            let music = named_scope(scope, "music", || {
                startup::time("rest.music", || {
                    horizon_client::music::MusicTables::load(root)
                })
            });
            let ambient = named_scope(scope, "ambient", || {
                startup::time("rest.ambient", || {
                    horizon_client::ambience::AmbientTables::load(root)
                })
            });
            let nether_environment = named_scope(scope, "nether_environment", || {
                startup::time("rest.nether_environment", || {
                    text(Environment::load_dimension(
                        &horizon_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_nether",
                    ))
                })
            });
            let end_environment = named_scope(scope, "end_environment", || {
                startup::time("rest.end_environment", || {
                    text(Environment::load_dimension(
                        &horizon_assets::resource_source::ResourceSource::directory(
                            root.to_path_buf(),
                        ),
                        "the_end",
                    ))
                })
            });
            (
                items.join().unwrap(),
                behaviours.join().unwrap(),
                block_entity_render.join().unwrap(),
                animate.join().unwrap(),
                mobs.join().unwrap(),
                transformers.join().unwrap(),
                providers.join().unwrap(),
                music.join().unwrap(),
                ambient.join().unwrap(),
                nether_environment.join().unwrap(),
                end_environment.join().unwrap(),
            )
        })
    } else {
        (
            text(load_items()),
            text(load_behaviours()),
            text(load_block_entity_render()),
            text(load_animate()),
            text(load_mobs()),
            text(load_transformers()),
            Providers::load(root),
            startup::time("rest.music", || {
                horizon_client::music::MusicTables::load(root)
            }),
            startup::time("rest.ambient", || {
                horizon_client::ambience::AmbientTables::load(root)
            }),
            startup::time("rest.nether_environment", || {
                text(Environment::load_dimension(
                    &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_nether",
                ))
            }),
            startup::time("rest.end_environment", || {
                text(Environment::load_dimension(
                    &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
                    "the_end",
                ))
            }),
        )
    };
    let (items, recipes, stations) = items?;
    let (animate, fog) = animate?;
    let (mob_types, mob_data) = mobs?;
    let (transformers, (player_type, player_arm, player_arm_slim, spin_attack)) = transformers?;
    let nether_environment = nether_environment?;
    let end_environment = end_environment?;
    log::info!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    Ok(WorldRest {
        stations,
        items,
        recipes,
        transformers,
        providers,
        block_entity_render: block_entity_render?,
        animate,
        fog,
        music,
        ambient,
        nether_environment,
        end_environment,
        behaviours: behaviours?,
        mob_types,
        mob_data,
        player_type,
        player_arm,
        player_arm_slim,
        spin_attack,
    })
}

/// A title-shaped frame: black clear and panorama, only the HUD is drawn.
/// The window's `HWND`, for the boot screen's GDI paint (`horizon_threads`):
/// the raw handle read is safe, the paint is not.
#[cfg(target_os = "windows")]
fn hwnd(window: &Window) -> Option<isize> {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    match window.window_handle() {
        Ok(raw) => match raw.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        },
        Err(_) => None,
    }
}

fn write_report(game: &Game<'_>) -> Result<()> {
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.frames,
            "elapsed_seconds": game.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.streaming.generated_total,
            "meshed_sections": game.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error.as_ref().map(|e| e.to_string()),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

/// The ordinary launch: load the resources on a worker while the window shows
/// the loading screen, then swap in the game on the main thread.
fn run_loading(
    options: Options,
    settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
) -> Result<()> {
    let progress = Progress::new();
    let (receiver, loader) =
        startup_menu::spawn_loader(&options, &settings, &vanilla_root, started, &progress)?;
    let menu = startup_menu::StartupMenu::new(&options, pack_error.as_deref())?;
    let loading = Loading {
        window: None,
        renderer: None,
        boot: horizon_client::hud::HudAtlas::boot(),
        gui: None,
        settings,
        fullscreen: horizon_client::window::FullscreenState::new(false),
        video_modes: Default::default(),
        options: Some(options),
        vanilla_root,
        pack_error,
        started,
        panorama_rotation: Default::default(),
        progress,
        receiver,
        loaded: None,
        delivered: false,
        handle: Some(loader),
        shown: false,
        ui_at: None,
        title_drawn: false,
        menu,
        error: None,
    };
    let mut app = App::Loading(Box::new(loading));
    let result = EventLoop::new()?.run_app(&mut app);
    narrator::stop();
    match &mut app {
        // Ready: splitscreen around P1's game.
        App::Ready(split) => leave_fullscreen_after(&result, &mut split.host.settings),
        App::Loading(loading) => leave_fullscreen_after(&result, &mut loading.settings),
    }
    result?;
    match app {
        App::Ready(mut split) => {
            write_report(&split.host)?;
            if let Some(e) = split.host.error.take() {
                return Err(e);
            }
        }
        App::Loading(loading) => {
            if let Some(e) = loading.error {
                return Err(e);
            }
        }
    }
    Ok(())
}

pub use launch_api::{UsageError, exit_code};

pub fn run_with(
    launch: LaunchArguments,
    native: horizon_mod_package::runtime::Runtime,
) -> Result<()> {
    let Some(mut options) = Options::read(launch)? else {
        return Ok(());
    };
    options.mods.native = Arc::new(native);
    if options.execution_role == execution_role::ExecutionRole::DedicatedServer
        && !dedicated::preflight(&mut options)?
    {
        return Ok(());
    }
    let _ = horizon_client::block_contact::HOOKS.set(native_hooks::BLOCK_CONTACT_HOOKS);
    // Before any worker starts: on Linux they go behind the server thread
    // and the render thread.
    horizon_threads::put_helpers_behind();
    if options.execution_role.is_client() {
        // The render thread, like the server thread, ahead of the workers on
        // macOS and Windows. While terrain loads, the workers
        // (horizon_threads::jobs) and other programs outnumber the cores, and a frame that gave its core up (in a GPU submit) waited
        // 10 to 200 ms to get one back.
        horizon_threads::favour_current();
    }
    // A dedicated server too: vanilla's "Server Shutdown Thread" (`DedicatedServer.initServer`'s
    // shutdown hook) halts the server and saves its players and levels on SIGTERM, SIGINT or SIGHUP.
    if options.execution_role.is_client()
        || options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        horizon_threads::catch_stop_signals();
    }
    let started = Instant::now();
    // A dedicated server writes its slow ticks to the log with the spans
    // they spent their time in (`HORIZON_SPIKES`'s line; the variable still
    // sets another threshold), so a lag spike players felt can be traced
    // from the log alone. Timing the spans costs microseconds a tick.
    if options.spikes.is_none()
        && options.execution_role == execution_role::ExecutionRole::DedicatedServer
    {
        options.spikes = Some(dedicated::SLOW_TICK_MS);
    }
    horizon_client::spans::enable(options.spikes.is_some());
    let vanilla_root = options.resources.clone();
    // Saves from before 1.13 upgrade with vanilla's tables, prepared from the player's jar.
    horizon_world::datafix::set_resources(
        &std::path::absolute(&vanilla_root).unwrap_or_else(|_| vanilla_root.clone()),
    );
    if let Some(error) = missing_resources(&vanilla_root) {
        // The game build started by hand (the release's zip), not by the
        // launcher: a window says where to get it. Scripts and servers get
        // the error.
        if !options.offscreen && !synchronous_startup(&options) {
            log::error!("Horizon: {error}");
            let settings = Settings::load(&Game::options_path());
            return no_resources::show(&vanilla_root, &settings);
        }
        return Err(error.into());
    }
    let mut early_settings = Settings::load(&Game::options_path());
    options
        .mods
        .capture_code_switches(&early_settings.mods.disabled);
    options
        .mods
        .select(&std::env::current_dir()?, &early_settings.mods.enabled);
    let mod_phase = mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::Packages);
    api_values::install_packages(options.mods.native.packages(), &options.mods.selected);
    mods::install_registration_data(&vanilla_root)?;
    {
        use mod_api::__private::game::RegistrationContextNew;
        let context = mod_api::RegistrationContext::new();
        let inventory: Vec<_> = context.mods().iter().cloned().collect();
        horizon_mod_wasm::initialize(&inventory);
    }
    mod_points::__private::seal();
    mod_phase.advance(mod_api::__private::ModPhase::Registration)?;
    if let Err(refused) = crate::game::api_values::install_parts(mod_api::__private::mods()) {
        panic!("Mod parts refused:\n{}", refused.join("\n"));
    }
    // The structure types those mods registered, handed to the engine before
    // any world opens, as its StructureRegistry reads them.
    if horizon_world::worldgen::structure::mod_types::install_mod_structure_types(
        native_structure_types::installed_structure_types(),
    )
    .is_err()
    {
        log::warn!("Mod structure types already installed; keeping the first table");
    }
    // The levels they registered, below the data packs of every world opened.
    if horizon_world::worldgen::overrides::install_mod_dimensions(
        native_dimensions::installed_dimensions(),
    )
    .is_err()
    {
        log::warn!("Mod dimensions already installed; keeping the first table");
    }
    // Their chunk generators, which those levels' and the packs' stems name.
    if horizon_world::worldgen::mod_generators::install_mod_chunk_generators(
        native_chunk_generator::installed_chunk_generators(),
    )
    .is_err()
    {
        log::warn!("Mod chunk generators already installed; keeping the first table");
    }
    mods::register_components(&vanilla_root)?;
    mods::register_standalone_models(options.execution_role);
    mods::register_layer_definitions(options.execution_role);
    mods::register_debug_entries(options.execution_role);
    mods::register_gui_layers(options.execution_role);
    mods::register_map_decoration_types();
    mods::register_range_select_item_model_properties(options.execution_role);
    mods::register_select_item_model_properties(options.execution_role);
    mods::register_item_tint_sources(options.execution_role);
    mods::register_enchantment_damage_effects();
    mods::register_dimension_transition_screens(options.execution_role);
    mods::register_color_resolvers(options.execution_role);
    mods::register_block_tint_sources(options.execution_role);
    mods::register_particle_providers(options.execution_role);
    mods::freeze_declarations();
    mod_phase.advance(mod_api::__private::ModPhase::Setup)?;
    // Minecraft.<init>: read the last start's flag, then mark this one as not
    // finished until onGameLoadFinished. A dedicated server has no options.txt.
    if options.execution_role.is_client() {
        if unclean_start_recovery(&mut early_settings) {
            log::warn!(
                "Detected unexpected shutdown during last game startup: resetting fullscreen mode"
            );
        }
        early_settings.file.set(
            "startedCleanly",
            horizon_client::options_file::V::Bool(false),
        );
        early_settings.save(&Game::options_path());
    }
    // Minecraft's constructor opens the narrator; a server has none.
    if options.execution_role.is_client() {
        narrator::start(&early_settings);
    }
    // Snapshots of derived tables live in the game folder, never beside the
    // shared resources.
    Behaviours::set_cache_dir(std::env::current_dir()?.join(".horizon-cache"));
    if !synchronous_startup(&options) {
        return run_loading(options, early_settings, vanilla_root, None, started);
    }
    let mut pack_error = None;
    let selected_root = packs::startup_root(
        &vanilla_root,
        &early_settings.resource_packs,
        &[],
        &options.mods,
    )
    .or_else(|e| {
        log::warn!("Horizon: resource packs not applied: {e}");
        pack_error = Some(e.to_string());
        mods::content_root(&vanilla_root, &options.mods)
    })?;
    let mut loaded = load_resources(
        selected_root,
        pack_finders::data_packs(&options.mods),
        launch_world(&options),
        &vanilla_root,
        &early_settings,
        started,
        false,
        options.execution_role,
        &Progress::new(),
        &mut |_, _| {},
    )?;
    let gui = loaded.gui.ok_or("missing menu bundle")?;
    options.resources = loaded.root.clone();
    let audio = if !options.execution_role.is_client() || options.headless {
        Audio::silent(&loaded.pack.source)
    } else {
        Audio::with_device(&loaded.pack.source, &early_settings.sound_device)
    };
    let resources = loaded.resources;
    let game = start_game(
        &resources,
        loaded.pack,
        gui,
        options,
        loaded.pack_error.or(pack_error),
        audio,
        vanilla_root,
        started,
        loaded.rest_loader,
        None,
        None,
        std::mem::take(&mut loaded.client_applies),
        loaded.entity_models,
    )?;
    if let Some(mut game) = game {
        // Minecraft.onGameLoadFinished: a start that made it this far is clean.
        if game.options.execution_role.is_client() {
            game.settings.file.set(
                "startedCleanly",
                horizon_client::options_file::V::Bool(true),
            );
            game.settings.save(&Game::options_path());
        }
        run_event_loop(game)?;
    }
    Ok(())
}

/// The right and left arms of a player model layer (`PlayerModel.rightArm`
/// and `leftArm`, with their sleeves).
fn player_arms(
    root: &std::path::Path,
    key: &str,
    document: Option<&horizon_client::mobs::EntityModelDocument>,
) -> [Option<horizon_client::mobs::Part>; 2] {
    let model = match document {
        Some(document) => document.model(key),
        None => horizon_client::mobs::load_model(root, key),
    };
    ["right_arm", "left_arm"].map(|name| {
        model
            .as_ref()
            .and_then(|m| m.children.iter().find(|c| c.name == name).cloned())
    })
}

/// The most `Player.sleepCounter` reaches: 100 asleep, then up to here over the
/// waking fade.
const MAX_SLEEP_TIMER: u32 = 109;

// Player.tick: cap the sleeping fade at 100, then fade out over 10 waking ticks.
fn tick_sleep_timer(timer: u32, sleeping: bool) -> u32 {
    if sleeping {
        (timer + 1).min(100)
    } else if (1..MAX_SLEEP_TIMER).contains(&timer) {
        timer + 1
    } else {
        0
    }
}

/// Where a ray entering `pos` first meets the block's outline boxes, if it
/// does: a hollow block (torch, fence, flower) lets the ray through.
fn outline_stop(
    origin: [f64; 3],
    dir: [f64; 3],
    reach: f64,
    pos: [i32; 3],
    face: [i32; 3],
    boxes: impl FnOnce() -> Vec<[f64; 6]>,
) -> Option<Hit> {
    let (t, face) = interaction_shapes::entry(origin, dir, &boxes(), reach, face)?;
    Some(Hit {
        pos,
        face,
        point: std::array::from_fn(|i| origin[i] + dir[i] * t),
        inside: false,
    })
}
/// `VoxelShape.clip` of a fluid source's shape for a ray that starts in the
/// source's own block `cell`: the face and point it hits, in `pick_fluid`'s
/// `Hit` terms. `FlowingFluid.getShape` is the whole block under the same
/// fluid and `getOwnHeight` (8/9) tall otherwise. A start inside the shape
/// (tested 1e-7 of the way along) hits at the start, on the face opposite
/// `Direction.getApproximateNearest(dir)`; from above it, a ray going down
/// enters through the top while still in this block.
fn clip_start_source(
    eye: [f64; 3],
    dir: [f64; 3],
    reach: f64,
    cell: [i32; 3],
    same_above: bool,
) -> Option<([i32; 3], [f64; 3])> {
    let height = if same_above {
        1.0
    } else {
        f64::from(8.0f32 / 9.0f32)
    };
    let probe_y = eye[1] + dir[1] * reach * 1.0e-7;
    if probe_y - f64::from(cell[1]) < height {
        // Direction.values() order; the first of equal dot products wins.
        let normals: [[i32; 3]; 6] = [
            [0, -1, 0],
            [0, 1, 0],
            [0, 0, -1],
            [0, 0, 1],
            [-1, 0, 0],
            [1, 0, 0],
        ];
        let mut nearest = normals[2];
        let mut best = f64::NEG_INFINITY;
        for n in normals {
            let dot =
                f64::from(n[0]) * dir[0] + f64::from(n[1]) * dir[1] + f64::from(n[2]) * dir[2];
            if dot > best {
                best = dot;
                nearest = n;
            }
        }
        return Some((nearest.map(|v| -v), eye));
    }
    if dir[1] >= 0.0 {
        return None;
    }
    let t = (f64::from(cell[1]) + height - eye[1]) / dir[1];
    if t > reach {
        return None;
    }
    let point = [
        eye[0] + dir[0] * t,
        eye[1] + dir[1] * t,
        eye[2] + dir[2] * t,
    ];
    (point[0].floor() as i32 == cell[0] && point[2].floor() as i32 == cell[2])
        .then_some(([0, 1, 0], point))
}

fn reset_world_progress(adv: &mut advancements::AdvState, stats: &mut stats::Stats) {
    *adv = Default::default();
    *stats = Default::default();
}

/// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners (aiStep, unless a spectator or a
/// passenger): a corner inside a suffocating block sets a 0.1 speed towards the nearest side that is
/// free, on that side's axis; a later corner overwrites an earlier one. Suffocating is a full
/// collision cube that is also a full render cube, as `in_wall` has it.
pub(crate) fn closest_space_nudge(
    p: &PlayerMotion,
    world: &World,
    facts: &BlockFacts,
) -> [Option<f64>; 2] {
    use horizon_core::geometry::Axis;
    let mut out = [None; 2];
    let body = p.bounds();
    let (min_y, max_y) = (body.min(Axis::Y) + 1.0e-7, body.max(Axis::Y) - 1.0e-7);
    let suffocates_at = |x: i32, z: i32| {
        ((min_y.floor() as i32 - 1)..=(max_y.ceil() as i32 + 1)).any(|y| {
            f64::from(y) < max_y && f64::from(y + 1) > min_y && {
                let f = facts.get(world.block(x, y, z));
                f.full_collision && f.solid_render
            }
        })
    };
    let side = f64::from(p.dimensions.width) * 0.35;
    let (px, pz) = (p.position.x, p.position.z);
    for (x, z) in [
        (px - side, pz + side),
        (px - side, pz - side),
        (px + side, pz - side),
        (px + side, pz + side),
    ] {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        if !suffocates_at(bx, bz) {
            continue;
        }
        let (xd, zd) = (x - f64::from(bx), z - f64::from(bz));
        // WEST, EAST, NORTH, SOUTH: (axis, step, distance to that edge).
        let mut closest = 1.797_693_134_862_315_7e308; // Double.MAX_VALUE
        let mut chosen = None;
        for (axis, step, distance) in [(0, -1, xd), (0, 1, 1.0 - xd), (1, -1, zd), (1, 1, 1.0 - zd)]
        {
            let (nx, nz) = if axis == 0 {
                (bx + step, bz)
            } else {
                (bx, bz + step)
            };
            if distance < closest && !suffocates_at(nx, nz) {
                closest = distance;
                chosen = Some((axis, step));
            }
        }
        if let Some((axis, step)) = chosen {
            out[axis] = Some(0.1 * f64::from(step));
        }
    }
    out
}

/// GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown, and only over
/// a level. The F3 overlay stays on when the world closes, so it must not reach the panorama.
fn debug_crosshair_drawn(
    enabled: bool,
    first_person: bool,
    hide_gui: bool,
    panorama: bool,
) -> bool {
    enabled && first_person && !hide_gui && !panorama
}

/// GameRenderer.shouldRenderBlockOutline, and only over a level. A panorama frame (the title
/// screen, LevelLoadingScreen's Reason.OTHER) replaces the level vanilla draws under it, so the
/// player's target must not leave its outline there.
fn block_outline_drawn(
    hide_gui: bool,
    entity_targeted: bool,
    can_build: bool,
    panorama: bool,
) -> bool {
    !hide_gui && !entity_targeted && can_build && !panorama
}

/// Blocks whose class implements `GameMasterBlock`: the command blocks, structure
/// block, jigsaw and the test blocks.
fn is_game_master_block(name: &str) -> bool {
    horizon_client::command_block::Mode::of_block(name).is_some()
        || matches!(
            name,
            "minecraft:structure_block"
                | "minecraft:jigsaw"
                | "minecraft:test_block"
                | "minecraft:test_instance_block"
        )
}

fn write_level_for_role(
    level: &mut Level,
    dir: &std::path::Path,
    role: execution_role::ExecutionRole,
) -> Result<()> {
    if role.writes_singleplayer_player() {
        level.write(dir)
    } else {
        level.write_without_player(dir)
    }
}

/// A server's data load with no mods and no settings, for loaders under test.
#[cfg(test)]
fn test_server_load() -> horizon_world::native_load_context::LoadContext {
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}

#[cfg(test)]
mod nested_orb_spawn_tests {
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn experience_pickup_keeps_nested_orbs_until_the_next_ordered_pass() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            for (x, value) in [(7.5, 1), (9.5, 3), (11.5, 7)] {
                game.add_orb([x, 80.0, 3.5], value);
            }
            let _ = game.orbs.remove(1);
            let mut expected: Vec<_> = game.orbs.iter().map(|o| (o.id, o.join)).collect();
            crate::game::entity::taken_pass_tests::with_spawn(
                |g| g.add_orb([3.5, 80.0, 3.5], 1),
                || game.tick_experience(),
            );
            assert_eq!(game.orbs.len(), 3);
            let join = game.orbs[2].join;
            expected.push((game.orbs[2].id, join));
            assert_eq!(
                game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(game.orbs[2].age, 0);
            game.ordered.announced = horizon_client::join_seq::JoinSeq::MAX;
            game.tickets.update(None);
            game.begin_ordered_ticks();
            game.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)
                .unwrap();
            let spawned = game.orbs.iter().find(|o| o.join == join).unwrap();
            assert_eq!(spawned.age, 1);
            assert_eq!(
                game.orbs.iter().map(|o| (o.id, o.join)).collect::<Vec<_>>(),
                expected
            );
        });
    }
}

#[cfg(test)]
pub(crate) fn in_own_process(name: &str, body: impl FnOnce()) {
    const OWN_PROCESS: &str = "HORIZON_TEST_OWN_PROCESS";
    if std::env::var_os(OWN_PROCESS).is_some_and(|v| v == name) {
        return body();
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--include-ignored", "--test-threads=1"])
        .env(OWN_PROCESS, name)
        .output()
        .unwrap();
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && report.contains("test result: ok. 1 passed;"),
        "{name} failed in its own process:\n{report}"
    );
}

#[cfg(test)]
mod escape_tests {
    use super::game_tests::tests::server_game;
    use horizon_client::chunk::Chunk;
    use horizon_client::items::ItemStack;
    use horizon_core::random::RandomSource;
    use std::sync::Arc;

    /// `Entity.moveTowardsClosestSpace` takes an item's escape speed from the
    /// item's own random, the one its constructor drew from, and leaves the
    /// level's random as it was (#13043).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_item_escapes_a_block_at_its_own_randoms_speed() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let feet = horizon_core::geometry::Vec3::new(8.5, 70.0, 8.5);
        game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, feet).unwrap());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.update_chunk_tickets();
        assert!(game.entity_ticking_area()(8, 8));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.set(8, 64, 8, stone).unwrap();
        let stack = ItemStack::new("minecraft:stone", 2);
        game.spawn_item(stack, [8.5, 64.4, 8.5], [0.0, 0.0, 0.0], 10);
        let item = game.dropped.last().unwrap();
        let expected = f64::from(item.random.clone().next_float() * 0.2 + 0.1);
        let mut level = game.world.random.clone();
        game.ticked_entities_through = u32::MAX;
        game.tick_items();
        let item = game.dropped.last().unwrap();
        // Up is the nearest open side from the middle of the block.
        // The push replaces the fall's velocity; then the 0.98 drag.
        assert_eq!(item.velocity.y, expected * 0.98);
        assert_eq!(game.world.random.next_float(), level.next_float());
    }
}
