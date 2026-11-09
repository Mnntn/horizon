//! The hooks the catalog started with: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait DrawContexts {
    #[draw_context(surface = "crate::hud::HudBatch", name = "DrawContext")]
    fn draw_context();
    #[draw_context(
        surface = "crate::game::native_block_entity_draw::BlockEntityDraw",
        name = "BlockEntityDrawContext"
    )]
    fn block_entity_draw_context();
}

pub trait Hooks {
    /// The world frame's local, world-space rim and colored-light sources.
    /// Push requests; Horizon validates and limits them before rendering.
    /// Called only while a world is visible.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn visuals();
    /// At most 32 pairs of already-loaded base and transparent overlay sprites.
    /// Per-pixel alpha in the overlay marks full-bright material, without
    /// illuminating the surrounding block or changing gameplay light.
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn emissive();
    /// The world's phases, read-only; `error_text` carries the engine's
    /// own open-failure text, empty otherwise. Server-side. A server loop or final
    /// save failure fires `WorldCloseFailed` with the error text.
    #[hook(
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "ServerStartingEvent, ServerStartedEvent, ServerStoppingEvent, ServerStoppedEvent"
    )]
    fn world_lifecycle();
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server once its settings are read. Each is
    /// followed by `native_server_started` or
    /// `native_world_lifecycle`'s `WorldOpenFailed`. `dedicated` tells a dedicated server from the single
    /// player one.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerAboutToStartEvent",
        cancellable = false
    )]
    fn server_about_to_start();
    /// Fired once immediately after `server_about_to_start`, before the server
    /// accepts play, on both integrated and dedicated servers. A failed startup
    /// still reaches `server_stopped`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStartingEvent",
        cancellable = false
    )]
    fn server_starting();
    /// The server has opened its worlds and is about to run its tick loop
    /// (NeoForge's `ServerStartedEvent`): every level is loaded, no player has
    /// joined, and the first tick has not run. Fired where `WorldOpenReady` was,
    /// on the single player server thread and on a dedicated server, once per
    /// open that succeeds (a failed open fires `WorldOpenFailed` instead).
    /// `dedicated` tells a dedicated server from the single player one.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStartedEvent",
        cancellable = false
    )]
    fn server_started();
    /// The server begins teardown (NeoForge's `ServerStoppingEvent`), including
    /// after startup or the tick loop failed:
    /// the start of vanilla's `MinecraftServer.stopServer`, with the worlds
    /// still loaded and the players still connected and not yet saved, so a
    /// mod can save its own data or tell them. `native_world_lifecycle`'s
    /// `WorldCloseFinal` follows once the worlds are saved. A server whose loop
    /// fails still fires this before the final save attempt and `server_stopped`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppingEvent",
        fabric = "ServerLifecycleEvents.SERVER_STOPPING",
        cancellable = false
    )]
    fn server_stopping();
    /// Fired once at the end of server teardown, after `server_stopping` and
    /// the final save attempt and storage release. Also fired when startup or
    /// teardown fails after startup began, including when a shutdown listener
    /// panics, on integrated and dedicated servers.
    /// Failed opens save only worlds already adopted by the server (the prepared
    /// level is installed; the session lock alone is not adoption). A failed
    /// save still reaches this hook, but does not emit `WorldCloseFinal`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppedEvent",
        cancellable = false
    )]
    fn server_stopped();
    /// The authoritative server tick's phases: `ServerTickStart` before any
    /// level is ticked (NeoForge's `ServerTickEvent.Pre`), `ServerTickEnd`
    /// after the last (`.Post`). Read blocks with `native_server_block`, by
    /// dimension: which level `self.world` holds here is not defined. A paused
    /// pass fires nothing; a frozen world carries `world_running` false except
    /// during a requested step.
    #[hook(side = "server", contract = "listen", fabric = ["ServerTickEvents.START_SERVER_TICK", "ServerTickEvents.END_SERVER_TICK"], neoforge = "ServerTickEvent.Pre, ServerTickEvent.Post")]
    fn server_tick();
    /// Before an authoritative server level's tick pass. `dimension` is its ID.
    #[hook(side = "server", contract = "listen", neoforge = "LevelTickEvent.Pre")]
    fn level_tick_pre();
    /// After a successful authoritative server level tick pass.
    #[hook(side = "server", contract = "listen", neoforge = "LevelTickEvent.Post")]
    fn level_tick_post();
    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    #[hook(
        side = "server",
        contract = "listen",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_changed",
        neoforge = "none (NeoForge has no set-block event; IBlockExtension.onBlockStateChange runs after the change and may not modify the level)"
    )]
    fn block_changed();
    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    /// The host is the level with its ticks, as vanilla's `neighborChanged` gets the
    /// level: a part may read its inputs (`signal`, `has_neighbor_signal`) and
    /// schedule its tick here, as a diode does (`DiodeBlock.checkTickOnNeighbor`).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        version = 2,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_neighbor_changed",
        neoforge = "none (vanilla Block.neighborChanged; NeoForge adds no event or extension for it); BlockEvent.NeighborNotifyEvent is the neighbour broadcast, a different point"
    )]
    fn neighbor_changed();
    /// An authoritative server's `ServerLevel.tickChunk` is about to
    /// random-tick the block `state` at `pos` (`BlockState.randomTick`):
    /// only blocks that tick randomly, a mod's as its vanilla template block
    /// does. Bone meal's random ticks (`StemBlock.performBonemeal`) don't
    /// fire it. Change the block with `native_set_block_property` and set
    /// `handled` to skip the block's own random tick (a mod crop growing its
    /// own way); a block changed without `handled` takes its own random tick
    /// as it is now, if it still ticks randomly. Lava's fluid random tick
    /// runs either way.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_random_tick",
        neoforge = "none (vanilla BlockBehaviour.randomTick; NeoForge adds no event or extension for it)"
    )]
    fn random_tick();
    /// An authoritative server's `ServerLevel.tickBlock` is about to run a
    /// due scheduled tick of the block `state` at `pos` (`BlockState.tick`):
    /// the block is still the one the tick was scheduled for. Change the
    /// block with `native_set_block_property` and set `handled` to skip the
    /// block's own tick; a block changed without `handled` takes its own
    /// tick as it is now, if it is still the tick's block.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_tick_block",
        neoforge = "none (vanilla BlockBehaviour.tick; NeoForge adds no event or extension for it)"
    )]
    fn tick_block();
    /// An authoritative server is about to compute the state the block
    /// `state` at `pos` takes now that its neighbour in `direction` is
    /// `neighbour` (`BlockBehaviour.updateShape`). `state` may not be in the
    /// world yet (a piston landing it): decide from `state`, not from the
    /// block at `pos`. Answer only for your own blocks: put the new state in
    /// `new` (`state` keeps the block as it is; air breaks it, with its drops,
    /// on a neighbour update) and the block's own `updateShape` doesn't run,
    /// nor the ticks it would schedule. The level is read-only, as vanilla's
    /// `LevelReader`; ticks go through `native_schedule_block_tick` and
    /// `native_schedule_fluid_tick` with `ticks` (`ScheduledTickAccess`). It
    /// runs often, at least six times per `setBlock` that updates shapes
    /// (more for redstone wire's diagonals and knock-on updates): keep it
    /// short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "new",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_update_shape",
        neoforge = "none (vanilla BlockBehaviour.updateShape; NeoForge adds no event or extension for it)"
    )]
    fn update_shape(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockState,
        new: &mut Option<BlockState>,
    );
    /// A command the local player typed or clicked in chat, without its
    /// slash, before the server and Horizon's commands see it: set `ran`
    /// when the mod ran it. Client-side: a server never asks, not for
    /// command blocks, guests or the console.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "ran",
        version = 2,
        neoforge = "none (ClientChatEvent is for chat messages; client commands are added by RegisterClientCommandsEvent)"
    )]
    fn command();
    /// The client's own commands (NeoForge's `RegisterClientCommandsEvent`), which
    /// `command` runs: push a `Command` per usage (`x <mode:word>`).
    /// Asked when a connection starts and again when the server sends its
    /// own tree, so the chat box colours, completes and lists them as the
    /// game's own; a name the game already has keeps that command's
    /// behaviour. A client command has no operator level: one with a nonzero
    /// `level` is left out.
    #[hook(
        side = "client",
        contract = "collect",
        out = "commands",
        neoforge = "RegisterClientCommandsEvent"
    )]
    fn register_client_commands();
    /// The zoom a native mod asks for, applied while playing (the spyglass's
    /// own view wins): set `zoom` when it is none yet.
    #[hook(
        side = "client",
        contract = "replace",
        out = "zoom",
        neoforge = "none (Horizon-only; ViewportEvent.ComputeFov is the nearest way to change the view)"
    )]
    fn zoom();
    /// The minimap a native mod shows: set `minimap` when it is none yet.
    #[hook(
        side = "client",
        contract = "replace",
        out = "minimap",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn minimap();
    /// The player changed setting `key` of mod `mod_id` on its page in Options > Mods
    /// (`Registrar::register_setting`); the new value is kept already, so read it with
    /// `setting_bool` and its siblings. Only mod `mod_id` hears it: a mod is told of its own
    /// settings, never another's.
    #[hook(
        side = "client",
        contract = "listen",
        owner = "mod_id",
        neoforge = "ModConfigEvent.Reloading (a ModConfigSpec value set on the config screen)"
    )]
    fn setting_changed();
    /// The player did `event` on mod `mod_id`'s panel (`native_panel_items`):
    /// act on it when it is yours.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn panel_event();
    /// Initial saved data for a declared block entity (`EntityBlock.newBlockEntity`),
    /// fired inside the write that creates it (`LevelChunk.setBlockState`), on
    /// a server and on a client of one. `state` is the actual placed state for a
    /// block declared through `stateful_block_entity`; its properties are available
    /// through the Host's state registry. The declaration owns the type id. No
    /// answer creates no entity for a state-aware factory; other declared types
    /// start with an empty compound. The level is read-only, as `newBlockEntity`
    /// gets no level.
    #[hook(
        side = "both",
        contract = "replace",
        out = "entity",
        scope = "block_entity",
        key = "block",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_entity",
        neoforge = "none (vanilla EntityBlock.newBlockEntity; NeoForge adds no event or extension for it)"
    )]
    fn block_entity();
    /// Whether block `block`'s block entity ticks (`EntityBlock.getTicker`):
    /// set `ticks` when it is yours. Asked for each of the block's states
    /// when a world opens.
    /// Set `ticks` to true to make it tick, and never clear a value an earlier mod or mixin set.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        host = "read",
        neoforge = "none (vanilla EntityBlock.getTicker; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_ticks();
    /// A server tick of the block entity at `pos`, in vanilla's order among
    /// the others (`Level.tickBlockEntities`), when its `id` is yours: read
    /// and change its saved compound `nbt`, its item slots `items` (keep
    /// their count: as many as its container has, none without one) and the
    /// data slots its menu shows (`data`). It is out of the world during the
    /// call (the block entity at `pos` reads as a `Mod` with an empty id, and
    /// a save leaves it out); if the call removes or replaces it, `entity` is
    /// dropped. Clients, the window's own in single player included, get
    /// what `native_block_entity_update_tag` gives when its chunk is sent or
    /// its block changes.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntityTicker; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_tick();
    /// What clients get of the block entity at `pos` when its `id` is
    /// yours (`BlockEntity.getUpdateTag`): set `tag` to it, read from
    /// `entity` alone. Asked on the server whenever a chunk holding it is
    /// sent and whenever its block changes, on the server thread or, for a
    /// saved chunk sent past the view distance (`lan::far`), another one; a client's copy of the block
    /// entity (made by `native_block_entity` there) merges it into its `nbt`.
    /// Leave it unset to send no tag of yours. A block change puts the one-shot fields of
    /// `ModBlockEntityView::set_client_update_fields` over it, and sends them alone when it is unset.
    #[hook(
        side = "server",
        contract = "replace",
        out = "tag",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntity.getUpdateTag; NeoForge adds no event or extension for it); IBlockEntityExtension.handleUpdateTag is the client half"
    )]
    fn block_entity_update_tag();
    /// Block `block`'s block entity has item slots (a `WorldlyContainer`):
    /// hoppers, comparators and breaking the block use them as vanilla's containers.
    /// Set `container` when the block is yours. Asked once for each mod
    /// block when the ticking table is built; at most 256 slots, and slot
    /// numbers past the count are left out. Its `menu`, one of
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/horizon/menu/`) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Horizon clients through the mod's `native_screen` only.
    #[hook(
        side = "both",
        contract = "replace",
        out = "container",
        host = "read",
        neoforge = "RegisterCapabilitiesEvent"
    )]
    fn block_entity_container();
    /// A menu opens on this client for block `block`, a mod's: set `screen`
    /// when the block is yours, to lay its screen out (`native_screen_draw`
    /// draws it). The server asks it too: a slot it hides is not in the menu
    /// there either, so shift-clicks never fill it.
    #[hook(
        side = "both",
        contract = "replace",
        out = "screen",
        scope = "block_entity",
        key = "block",
        host = "read",
        neoforge = "none (Horizon declares a block's screen through hooks; NeoForge registers a Screen class per menu with RegisterMenuScreensEvent)"
    )]
    fn screen();
    /// Draw block `block`'s screen at `origin` (its top left, GUI pixels),
    /// over a plain panel with its slots' frames and under the items and
    /// labels, from the menu's data slots `data` (the block entity's `data`
    /// on the server): data slots are 16-bit on the wire as in vanilla (a
    /// bigger value wraps), and `data` is empty until the server's first
    /// values arrive, so read it with `get`. Draw with `native_sprite`, `native_fill` and
    /// `native_text`.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_draw();
    /// A left or right (`right`) press at `at` (GUI pixels from the screen's
    /// top left, so outside it too) on block `block`'s screen, a mod's, over
    /// no slot: set `button` to a menu button id when it is on one of yours.
    /// Horizon sends it to the server (`ContainerButtonClick`,
    /// `native_menu_button` there) and the press goes no further.
    #[hook(
        side = "client",
        contract = "replace",
        out = "button",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_click();
    /// The tooltip at `at` (as in `native_screen_click`) on block `block`'s
    /// screen, a mod's, over no item: push its lines when it is over
    /// something of yours. Legacy colour codes (§) work.
    #[hook(
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "block_entity",
        key = "block",
        neoforge = "none (see screen)"
    )]
    fn screen_tooltip();
    /// A player pressed a mod container's button (`clickMenuButton`), on
    /// the server. The block entity is out of the world during the call;
    /// mods may change it and the clicking player's carried stack.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (NeoForge has no such event; vanilla AbstractContainerMenu.clickMenuButton)"
    )]
    fn menu_button();
    /// The commands native mods add to the server's command tree: push
    /// yours. Asked once when a world opens to players (a dedicated
    /// server, a LAN host, single player's server thread); every player's
    /// chat box then completes them as vanilla's, within their level. A
    /// name vanilla has is left out. Asked again after a mod calls
    /// `host.commands().refresh_commands()`. Each executable usage keeps its own
    /// permission level; a lower-level child only opens traversal through a
    /// shared prefix, not execution of that prefix. An argument's kind is one
    /// of `word`, `text`, `integer`, `number`, `boolean`, `player`, `item` and
    /// `block_pos`, or a command argument type a mod registered
    /// (`Registrar::register_command_argument`), named by its namespaced id.
    #[hook(
        side = "server",
        contract = "collect",
        out = "commands",
        neoforge = "RegisterCommandsEvent"
    )]
    fn register_commands();
    /// Execute a parsed mod command after permission checks and argument resolution.
    /// Return `Flow::Cancel` when handled. `source` identifies origin and permission,
    /// independent of `execute as`. Feedback honours silence, track-output and the swapped-in player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "ran",
        neoforge = "none (a command's executor is part of the Brigadier node added in RegisterCommandsEvent; CommandEvent only watches executions)"
    )]
    fn run_mod_command();
    /// A click in a menu a mod opened (`native_open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla AbstractContainerMenu.clicked; NeoForge adds no event or extension for it)"
    )]
    fn menu_clicked();
    /// A menu a mod opened closed: the player closed it, the server or a mod
    /// opened another, or the player left.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Close"
    )]
    fn menu_closed();
    /// `Item.appendHoverText`: an item's own lines in its tooltip, after its
    /// name and before its components' (enchantments, lore, attributes). Add
    /// lines to `lines`, which arrives empty, each with a colour as `rgba` packs
    /// it (`u32::MAX` keeps the text's own); `Text::legacy` reads `§` codes.
    /// The stack has its components (`stack.data()`). Asked each frame the
    /// tooltip shows, never for one the stack hides or a crossbow's loaded
    /// projectile.
    #[hook(
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "item",
        key = "stack",
        neoforge = "ItemTooltipEvent"
    )]
    fn tooltip();
    /// A player uses `stack` (right click) with `self.hand` on the block at
    /// `pos`, `face` the face clicked (`[0, 1, 0]` is the top), once the
    /// block itself let the click through (a chest still opens): where
    /// `ItemStack.useOn` starts. Set `claimed` to take the use: the hand
    /// swings and neither the item's `useOn` nor its `use` runs.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one); an item on cooldown is never used. `hand`, `hit`
    /// (`UseOnContext.getClickLocation`, `getClickedFace`, `isInside`) and
    /// `player` (the acting player's UUID) and `player_entity_id` (the server's
    /// entity id),.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        scope = "item",
        key = "stack",
        neoforge = "UseItemOnBlockEvent"
    )]
    fn use_on(
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
        hand: Hand,
        hit: BlockHit,
        player: u128,
        player_entity_id: i32,
        claimed: &mut bool,
    );
    /// A player uses `stack` (right click) with `self.hand`: in the air, or
    /// on a block once `native_use_on` let it through, where `Item.use`
    /// starts. Each click reaches it once. Set `claimed` to take the use: the
    /// hand swings and the item's own use doesn't run. Server-side, for every
    /// player of the world; an item on cooldown is never used.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        scope = "item",
        key = "stack",
        neoforge = "PlayerInteractEvent.RightClickItem"
    )]
    fn use_item();
    /// A player breaks the block `state` at `pos`, once the game allowed it
    /// (the held item can break blocks, the debug stick didn't take the
    /// click): where `ServerPlayerGameMode.destroyBlock` starts. Set
    /// `canceled` to keep the block: nothing is destroyed or dropped, and a
    /// guest sees it back. Set `exp_to_drop` to `Some(amount)` to replace
    /// the vanilla experience sample later, after item drops are prepared;
    /// `None` keeps vanilla sampling. `block_drops` sees and may change that
    /// amount afterwards; cancellation or disabled block drops suppresses it.
    /// Server-side, for every player of the world (the
    /// helpers read the acting one). Diverges from upstream: renamed from
    /// `destroy_block`, to NeoForge's `BreakBlockEvent`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "BreakBlockEvent"
    )]
    fn break_block();
    /// A player right clicks a block (NeoForge's
    /// `PlayerInteractEvent.RightClickBlock`): once per hand, where
    /// `useItemOn` starts, on the clicking client and on the server (`click`
    /// says which, with the block, the hit, the hand and its stack, and the
    /// player). Before the block's own use (a chest, a door, a lever) and the
    /// item's (`Item.useOn`, `native_use_on`).
    ///
    /// `use_block` forces the block's use off (`False`) or on even while
    /// sneaking with something in hand (`True`); `use_item` forces the
    /// item's `useOn` off, or on even while the item cools down. Cancelling
    /// skips both and answers `cancellation_result` in their place:
    /// `Pass` (the default) goes on to the item's own use (`Item.use`) and
    /// then the other hand, `Fail` ends the click, and `Success`,
    /// `SuccessServer` or `Consume` end it as used, with the swing they name.
    /// A mod that keeps players out of land cancels with `Fail`.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.RightClickBlock",
        cancellable = true
    )]
    fn right_click_block();
    /// On an authoritative server, a fluid flows or fire spreads from `from`
    /// into `to` (`kind` is `fluid` or `fire`): `FlowingFluid.spreadTo`, and
    /// `FireBlock.tick` setting fire to `to` or burning it (`checkBurnOut`).
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_spread",
        neoforge = "none (BlockEvent.FluidPlaceBlockEvent covers only fluids making stone and the like, not flow or fire spread)"
    )]
    fn block_spread();
    /// On an authoritative server, the piston at `piston` is about to push
    /// the blocks at `push` one step `direction` (`[0, 1, 0]` is up; pulling
    /// when it retracts, `extending` false) and destroy those at `destroy`
    /// (NeoForge's `PistonEvent.Pre`, in `PistonBaseBlock.moveBlocks`, once its
    /// structure resolved). Set
    /// `canceled` and nothing moves, as when the structure can't; a sticky
    /// piston then retracts without its block, and clients are sent the
    /// blocks rather than the retraction to replay. Diverges from upstream: `piston_move` is renamed to
    /// `piston_pre`, and `piston_post` is new.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PistonEvent.Pre",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_piston_pre"
    )]
    fn piston_pre(
        dimension: LevelDimension,
        piston: BlockPos,
        direction: [i32; 3],
        extending: bool,
        push: &[BlockPos],
        destroy: &[BlockPos],
        canceled: &mut bool,
    );
    /// On an authoritative server, the piston at `piston` pushed the blocks at
    /// `push` one step `direction` (`[0, 1, 0]` is up; pulling when it
    /// retracts, `extending` false) and destroyed those at `destroy`
    /// (NeoForge's `PistonEvent.Post`, at the end of `PistonBaseBlock.moveBlocks`,
    /// after the neighbour updates). It only hears; `piston_pre` is the veto.
    /// Server-side. Diverges from upstream: `piston_move` is split into
    /// `piston_pre` and `piston_post`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PistonEvent.Post",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_piston_post"
    )]
    fn piston_post();
    /// An explosion at `center` in `dimension` (`minecraft:overworld`) is
    /// about to break `blocks`: `ServerExplosion.explode` with its `toBlow`,
    /// before any block is hit. Take out the positions to spare; they keep
    /// their block and get no fire. Entities are hurt as before. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "blocks",
        neoforge = "ExplosionEvent.Detonate"
    )]
    fn explosion_detonate();
    /// A player fills (`stack` an empty bucket) or empties a bucket at `pos`:
    /// `BucketItem.use`, where it asks `Level.mayInteract`. Set `canceled`
    /// and the bucket does nothing. Server-side, for every player of the
    /// world (`host.players().acting_player()` says who).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge 26.3 has no bucket-fill event)"
    )]
    fn fill_bucket();
    /// A player attacks the entity `entity` (`Entity.getId`) of type `kind`
    /// (`minecraft:item_frame`, `minecraft:cow`, `minecraft:player`) standing
    /// at `pos`: `ServerGamePacketListenerImpl.handleInteract`'s attack, once
    /// in reach, where `Player.attack` starts. The swing has not begun, so the
    /// attacking player's attack strength is still the charge the blow uses
    /// (vanilla reads it at the start of `Player.attack`; the swing resets it
    /// afterwards). Set `canceled` and the blow does nothing (no damage, no
    /// knockback, the frame keeps its item). Server-side, for every player of
    /// the world (`host.players().acting_player()` says who).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "AttackEntityEvent"
    )]
    fn attack_entity();
    /// A message from `player`'s client on `channel` (`mod_id:name`), within
    /// the caps (`native_net.rs`): check `channel`, and treat `data` as the
    /// player's own bytes, checked by nothing. Runs in the tick after it
    /// arrives, with no player's state swapped in. Server-side. The NeoForge
    /// counterpart is the registered `IPayloadHandler.handle` callback, not
    /// the registration event.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "IPayloadHandler.handle"
    )]
    fn net_from_client();
    /// A message from the server on `channel` (`mod_id:name`), whose `data`
    /// is the sender's own bytes. The window's game only. The NeoForge
    /// counterpart is the registered `IPayloadHandler.handle` callback, not
    /// the registration event.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "IPayloadHandler.handle"
    )]
    fn net_from_server();
    /// `player` died: where `ServerPlayer.die` starts, with the death
    /// message's key when the game knows it (`death.attack.lava`; none for a
    /// message it builds from the combat). It only hears. Server-side, for
    /// every player of the world, whether death messages are shown or not.
    #[hook(side = "server", contract = "listen", neoforge = "LivingDeathEvent")]
    fn player_died();
    /// A new mob of `kind` joined `dimension` at `pos`: `natural` when the
    /// world's spawning made it, false when something else did (a breed, a
    /// raid, a spawner, a conversion, `/summon`, and the second mob of a
    /// jockey the world spawned). One call a mob: a `/summon` with passengers
    /// tells each. Not told of a mob loaded with its chunk, leaving a hive or
    /// changing level. It only hears; the mob is already among the game's
    /// mobs. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityJoinLevelEvent, FinalizeSpawnEvent"
    )]
    fn mob_spawned();
    /// `player`'s melee blow killed the mob `entity` of `kind` (other deaths
    /// aren't told yet). It only hears, after the blow landed. Server-side.
    #[hook(side = "server", contract = "listen", neoforge = "LivingDeathEvent")]
    fn mob_killed();
    /// `player` is about to take `count` of `item` from the ground
    /// (`ItemEntity.playerTouch`). Set `canceled` to leave it there: the
    /// stack isn't offered again for 10 ticks. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemEntityPickupEvent.Pre"
    )]
    fn item_picked_up();
    /// A player's hit landed on the mob `entity` of type `kind`
    /// (`minecraft:zombie`), holding `stack`: where `Item.hurtEnemy` is called,
    /// before the post-attack enchantments and the weapon's wear. It only
    /// hears. Another player as the target isn't told yet. Server-side, for
    /// every player of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.hurtEnemy; NeoForge adds no event or extension for it); LivingDamageEvent.Post comes later"
    )]
    fn hurt_enemy();
    /// A server tick of `stack`, the stack a player carries in `slot`: where
    /// `Inventory.tick` calls `Item.inventoryTick`, for every stack, each tick,
    /// `/tick freeze` included (it stops the world, not the player). `slot` is
    /// as `Inventory` saves it (0 to 35; the armour 100 feet to 103 head; -106
    /// the off hand), and `selected` says it is the held slot.
    /// `ItemPlace::PlayerSlot { player, slot }` reads and changes the stack,
    /// which stays in its slot: an emptied one empties the slot, and the
    /// player's client gets the change. Only the `ItemPart`s attached to
    /// `stack`'s item hear it. Server-side, for every living player of the world (the helpers
    /// read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.inventoryTick; NeoForge adds no event or extension for it); PlayerTickEvent ticks the player, not each stack"
    )]
    fn inventory_tick();
    /// A player's equipment slot `slot` (`head`, `chest`, `legs`, `feet`,
    /// `offhand`, and `mainhand` on a swap of the hands or `/item`) took a different
    /// item, `new` (none when it was emptied): `LivingEntity.onEquipItem`,
    /// before the equip sound. It only hears. Never for a spectator.
    /// Server-side, for every player of the world (the helpers read the
    /// acting one).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEquipmentChangeEvent"
    )]
    fn on_equip();
    /// The redstone signal the block `state` at `pos` gives on its `face`, the
    /// side that touches the block reading it: `BlockState.getDirectSignal`
    /// when `direct` (what passes through a block it touches, as a lever's
    /// does), else `getSignal`. `signal` holds vanilla's answer: change it when
    /// the block is yours (0 to 15; anything else is clamped). Asked very often,
    /// from inside the level's update, so answer from your own state: check the
    /// block first, and don't read or change the world here. When the answer
    /// changes, and once the block is placed, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::Neighbours)`. Authoritative
    /// servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_signal",
        neoforge = "none (vanilla BlockBehaviour.getSignal and getDirectSignal; NeoForge adds no event or extension for it)"
    )]
    fn block_signal();
    /// `BlockState.isSignalSource`: set `source` for a block of yours that
    /// gives a signal, so redstone wire beside it connects to it. Authoritative
    /// servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_is_signal_source",
        neoforge = "none (vanilla BlockBehaviour.isSignalSource; NeoForge adds no event or extension for it)"
    )]
    fn is_signal_source();
    /// The player-health HUD rows, once a frame the HUD draws them
    /// (`HealthHud::layout`, not under F1): `inputs` carries the health,
    /// food, air and vehicle state the layout reads, `wi`/`hi` the screen's
    /// GUI size. A mod that sets `draws` replaces every row — armour,
    /// hearts, food and air are the answer's to give; `bubble_pop_sound`
    /// too.
    /// Nothing set draws Horizon's own.
    #[hook(
        side = "client",
        contract = "replace",
        out = "draws",
        neoforge = "RenderGuiLayerEvent.Pre"
    )]
    fn health_hud();
    /// A player right-clicks the entity `entity` (`Entity.getId`) of type
    /// `kind` (`minecraft:cow`) with `self.hand`, holding `stack` (none in an
    /// empty hand): where `Player.interactOn` starts, before the entity's own
    /// `interact` and the item's `interactLivingEntity`. Set `claimed` to take
    /// the click: the hand swings and neither runs. Asked once per hand, as
    /// vanilla tries the main hand then the off hand. Server-side, for every
    /// player of the world (the helpers read the acting one). Diverges from upstream: renamed from
    /// `interact_entity`, to NeoForge's `PlayerInteractEvent.EntityInteract`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "claimed",
        neoforge = "PlayerInteractEvent.EntityInteract",
        paper = "PlayerInteractEntityEvent"
    )]
    fn entity_interact();
    /// A player left-clicks the block `state` at `pos`, `face` the face hit
    /// (`[0, 1, 0]` is the top): where `ServerPlayerGameMode.handleBlockBreakAction`
    /// takes a dig's start, after its reach, spawn protection and world border
    /// checks, before `Block.attack` (a note block plays) or a creative break.
    /// Set `canceled` to ignore the click: the block isn't attacked, dug or
    /// broken on the server, and the player's client gets it back (its own
    /// predicted crack still shows to the clicking player). Server-side, for
    /// every player of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerInteractEvent.LeftClickBlock"
    )]
    fn left_click_block();
    /// Before a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Pre`). Non-cancellable, including death-screen ticks.
    /// `player` identifies the ticked player; on the server their state is
    /// acting. Fires before sleep, cooldowns and body work. Players still tick
    /// under `/tick freeze`, but not while the level is paused.
    #[hook(side = "both", contract = "listen", neoforge = "PlayerTickEvent.Pre")]
    fn player_tick_pre(player: PlayerRef);
    /// After a player's tick on both logical sides (NeoForge 26.3
    /// `PlayerTickEvent.Post`). Non-cancellable, including death-screen ticks.
    /// `player` is the same identity as in `player_tick_pre`; on the server
    /// their state is acting. Runs after body work and server-side item,
    /// arrow and XP pickups, so reads see the completed tick. Client reads
    /// see only the state available to that client, not future server updates.
    #[hook(side = "both", contract = "listen", neoforge = "PlayerTickEvent.Post")]
    fn player_tick_post(player: PlayerRef);
    /// An entity joins the server level `dimension` (NeoForge's
    /// `EntityJoinLevelEvent`): vanilla's `PersistentEntitySectionManager.addEntity`.
    /// `loaded_from_disk` is true for an entity coming back with its chunk,
    /// false for one added fresh (`addFreshEntity`: a drop, an orb, `/summon`,
    /// an arrival through a portal). `entity` is its ID and `kind` its type
    /// (`minecraft:item`). It is already in the level's lists, so helpers that
    /// take an entity ID find it. Set `canceled` to keep it out: it is taken
    /// back out as if never added, and a `/summon` of it fails. Tells every
    /// kind in the game's entity lists (items, orbs, mobs, boats, minecarts,
    /// armor stands, primed TNT, falling blocks, projectiles, rockets,
    /// tridents, end crystals, the dragon, cushions, paintings, item frames),
    /// once each, in the order they were created; check `kind`. A drop, an
    /// orb, a mob, a `/summon` and a chunk's entities are told at once, any
    /// other spawn between the tick's phases (it is checked after the blocks,
    /// the vehicles, the projectiles, the items and the mobs, and at the
    /// tick's end). A mod that cancels one of the others sees it removed
    /// before it ticks again. Not told: the entities Horizon keeps without
    /// simulating (displays, markers, interactions). Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityJoinLevelEvent"
    )]
    fn entity_join_level();
    /// NeoForge's `LivingIncomingDamageEvent`: `LivingEntity.hurtServer`, past its guards (an
    /// invulnerable or dead entity, fire on a fire-resistant one), is about to hurt entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) by `source`: its damage
    /// type (`minecraft:fall`), causing entity (`DamageSource.getEntity`: the shooter of an
    /// arrow), direct entity (`getDirectEntity`: the projectile's entity id when the
    /// damage type names one, on a player, mob or ender dragon; 0 for `magic()` and any
    /// other source with no direct entity, even during that projectile's impact), and
    /// where it comes from (`getSourcePosition`). `victim` is where the entity stands and
    /// looks, and `weapon` the direct entity's weapon (`getWeaponItem`) when it is known.
    /// `container` is an API-owned snapshot before any reduction; engine sequencing
    /// remains game-owned. Change its `new_damage`, the invulnerability ticks a fresh hit
    /// leaves (vanilla's 20), or add an ordered reduction modifier with
    /// `add_reduction_modifier`. Set `canceled` and nothing
    /// of `hurtServer` happens (no damage, hurt sound, knockback, cooldown, statistics or
    /// criteria); a canceled hit on the ender dragon still counts as landed (`EnderDragon.hurt`
    /// returns true whatever `reallyHurt` did). It fires on every damage route, for players,
    /// mobs and the ender dragon. The level is the hit's: its blocks, random and own
    /// attachments; the hurt entity is in the middle of its hit.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingIncomingDamageEvent",
        target = "horizon_client::game::native_hooks::base::HitLevel<'_>::native_living_incoming_damage"
    )]
    fn living_incoming_damage(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        victim: &Victim,
        weapon: Option<&ItemStack>,
        container: &mut DamageContainer,
        canceled: &mut bool,
    );
    /// An entity leaves the server level `dimension` (NeoForge's
    /// `EntityLeaveLevelEvent`): vanilla's `onTrackingEnd`, when it is removed
    /// for any reason (picked up, merged, despawned, burnt, blown up, killed,
    /// taken by a hopper or a mob, gone through a portal) or its chunk unloads,
    /// a level closing with nobody in it included. Not for the entities still
    /// loaded when the server stops, as in vanilla. `entity` is its ID and
    /// `kind` its type (`minecraft:item`); it is no longer in the level's lists.
    /// It only hears. It tells every kind `native_entity_join_level` does
    /// (`kind` is the type), each once, when the game next notices it gone
    /// (the tick or action that removed
    /// it). Check `kind`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityLeaveLevelEvent"
    )]
    fn entity_leave_level();
    /// Block `block`'s block entity has fluid tanks: set `fluids` when the
    /// block is yours, in the container's shape — each tank's capacity in
    /// millibuckets, the tanks each face reaches, and the tanks a fill may
    /// reach and a drain may take. Asked once for each mod block when the
    /// ticking table is built; at most 16 tanks, capacities clamped to
    /// 1..=1 000 000, and tank numbers past the count are left out. The tanks'
    /// contents are kept in the entity's saved compound, under `fluids`, and
    /// the fluid helpers below read and write them.
    #[hook(
        side = "server",
        contract = "replace",
        out = "fluids",
        host = "read",
        neoforge = "RegisterCapabilitiesEvent"
    )]
    fn block_entity_fluids();
    /// `MobEffect.shouldApplyEffectTickThisTick` for a content mod's effect
    /// `effect` (an `EffectKey` for `ruby:bleeding`), each tick a player or a mob has it, in
    /// `MobEffectInstance.tickServer` before it counts down: `tick_count` is
    /// the ticks left (the entity's tick count when infinite). Set `apply`
    /// and `native_apply_effect_tick` follows. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "none (vanilla MobEffect.shouldApplyEffectTickThisTick; NeoForge adds no event or extension for it)"
    )]
    fn should_apply_effect_tick_this_tick();
    /// `MobEffect.applyEffectTick`: the content mod's resolved `EffectKey` `effect` acts on
    /// `entity` in `dimension` (`minecraft:overworld`). Clear `keep` and the
    /// effect ends now, before it counts down. `entity` is a mob's
    /// `Entity.getId`, None for a player (`host.players().acting_player()` says who).
    /// It runs inside the entity's tick: a mob's world borrows the game
    /// there, so the game is read-only, and it does not hold the ticking
    /// mobs, nor the player's effects. `holder` identifies the holder; Host
    /// reads of a mob holder are unavailable while it is ticking. Queued
    /// changes apply after the hook; clearing `keep` ends the effect.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "none (vanilla MobEffect.applyEffectTick; NeoForge adds no event or extension for it)"
    )]
    fn apply_effect_tick(
        effect: EffectKey,
        dimension: &str,
        entity: Option<i32>,
        amplifier: i32,
        holder: LivingRef,
        health: f32,
        max_health: f32,
        food: Option<PlayerFood>,
        changes: &mut Vec<EffectTickChange>,
        keep: &mut bool,
    );
    /// `MobEffect.onEffectAdded`: `LivingEntity.addEffect` gave `entity`
    /// (as in `native_apply_effect_tick`) the content mod's effect `effect`,
    /// which it didn't have. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla MobEffect.onEffectAdded; NeoForge adds no event or extension for it)"
    )]
    fn on_effect_added();
    /// NeoForge's `MobEffectEvent.Added`, vanilla's `MobEffect.onEffectStarted`:
    /// every `LivingEntity.addEffect` of the content mod's effect `effect`,
    /// new or not, changed or not, after `native_on_effect_added`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "MobEffectEvent.Added"
    )]
    fn mob_effect_added();
    /// Chunk `chunk` (`[x, z]`, in chunks) of `dimension` came into the world
    /// (NeoForge's `ChunkEvent.Load`): `new` when it was just generated, false
    /// when read from the save. A flat world's chunk that leaves before it's
    /// first lit isn't saved, so it's generated, and new, again; one generated
    /// as the players left is new when it next comes, unless the server
    /// restarts first, or the level closes for being empty, which drops its
    /// memory of what never saved. Its blocks and block entities are there,
    /// but its saved entities join afterwards and its light may not be ready:
    /// change the world from a later tick. The chunks around the spawn come
    /// before `server_started`. Not told of a chunk a client receives.
    /// The chunks leaving in the same tick were already copied for their last
    /// save: an edit made here to one of them (a neighbour's block entity, say)
    /// is not in that copy, and is dropped with the chunk.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ChunkEvent.Load",
        paper = "ChunkLoadEvent"
    )]
    fn chunk_load();
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), with a level that closes for being empty too. Don't
    /// read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `native_world_lifecycle` tells.
    /// Server-side.
    #[hook(side = "server", contract = "listen", neoforge = "ChunkEvent.Unload")]
    fn chunk_unload();
    /// `player` (`name`) joined the game (NeoForge's
    /// `PlayerLoggedInEvent`): placed in the world and listed in
    /// `host.players().server_players()`, so `native_net_send_to_player` reaches them.
    /// Once a join, with no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerLoggedInEvent"
    )]
    fn player_logged_in();
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `native_player_logged_in` told of. They've already
    /// left `host.players().server_players()`, except when the server stops, which tells
    /// before it saves, with them still listed and able to get a last message.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerLoggedOutEvent"
    )]
    fn player_logged_out();
    /// The world is drawn (see [`WorldDrawn`]). Not called on the title screen's panorama.
    #[hook(
        side = "client",
        contract = "listen",
        target = "mod_render::render::world_drawn",
        neoforge = "RenderLevelStageEvent.AfterLevel"
    )]
    fn world_drawn();
    /// Asked as a frame is built and drawn: set what your mod needs in
    /// `RenderNeeds`, including entity runs and the world's depth without a shader pack.
    #[hook(
        side = "client",
        contract = "collect",
        out = "needs",
        target = "mod_render::render::render_needs",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn render_needs();
    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (width `0..=13`, colour `rgb`) or set to `None` to hide it.
    /// Asked each frame a stack is drawn. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.isBarVisible/getBarWidth/getBarColor; RegisterItemDecorationsEvent registers decorators, not bar queries)"
    )]
    fn item_bar();
    /// `BlockState.hasAnalogOutputSignal`: set `output` for a block of yours
    /// that comparators read (how full or far along it is, as a composter's
    /// level), so they ask `native_analog_output`. Asked from inside the
    /// level's update: answer from `state` alone. Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "output",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_has_analog_output",
        neoforge = "none (vanilla BlockBehaviour.hasAnalogOutputSignal; NeoForge adds no event or extension for it)"
    )]
    fn has_analog_output();
    /// `BlockState.getAnalogOutputSignal`: the signal a comparator reads from
    /// the block `state` at `pos`, `face` the side that touches the comparator
    /// (or the conductor in front of it). `output` holds vanilla's answer:
    /// change it when the block is yours (0 to 15; anything else is clamped).
    /// Asked only for blocks `native_has_analog_output` answered for, from
    /// inside the level's update: don't change the world here. When the
    /// answer changes without the block's state changing, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)`; a change of
    /// state through
    /// `native_set_block` or `native_set_block_property` (neighbour updates
    /// on) tells the comparators itself, as vanilla's `Level.setBlock` does.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "output",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_analog_output",
        neoforge = "none (vanilla BlockBehaviour.getAnalogOutputSignal; NeoForge adds no event or extension for it)"
    )]
    fn analog_output();
    /// Register namespaced NBT component types before content loads.
    #[hook(
        side = "both",
        contract = "collect",
        out = "types",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn component_types();
    /// `DispenserBlock.getDispenseMethod`: choose a behaviour for any item,
    /// including vanilla items, on the authoritative server (not droppers).
    /// The block the dispenser faces is asked first (`dispense_method`), and
    /// its answer keeps this part from being asked. Leave answer None for
    /// vanilla. Otherwise answer `(remainder, event)`:
    /// a remainder of None empties the slot, `Some(stack)` replaces it (the hook
    /// owns consumption: no extra item is removed afterwards). `event` is the
    /// level event the dispenser plays with its smoke: 1000 the click, 1001 the
    /// fail sound (`OptionalDispenseItemBehavior` without success), or 0 for
    /// nothing at all (`DispenseItemBehavior.NOOP`). To drop one normally,
    /// answer `(native_dispense_default(..), 1000)`.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla DispenserBlock.getDispenseMethod; NeoForge adds no event or extension for it)"
    )]
    fn dispense_item();
    /// Client-side image component for this stack's tooltip. Answer its width/height
    /// in GUI pixels; finite positive dimensions reserve space after text and any
    /// vanilla bundle component. None keeps the vanilla tooltip unchanged.
    #[hook(
        side = "client",
        contract = "replace",
        out = "size",
        scope = "item",
        key = "stack",
        neoforge = "RegisterClientTooltipComponentFactoriesEvent"
    )]
    fn tooltip_component();
    /// Render the measured component at origin, inside the tooltip background.
    /// Stack is the component's data. Use native_sprite/native_fill/native_text;
    /// size is the measurement returned by native_tooltip_component this frame.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "RegisterClientTooltipComponentFactoriesEvent"
    )]
    fn tooltip_component_draw();
    /// On an authoritative server, `BlockState.canSurvive` for `state` at
    /// `pos` (`BlockBehaviour.canSurvive`), before the block answers it: as
    /// it is placed (`BlockItem.canPlace`), from a shape update of a block
    /// whose template checks it (a plant, a torch, a sign: false breaks the
    /// block with its drops), and wherever else vanilla asks. `state` may not
    /// be in the world yet, so decide from `state`. Answer only for your own
    /// blocks: `Some(false)` refuses the spot, `Some(true)` allows it, and
    /// the block's own rule doesn't run. The level is read-only. Asked on
    /// every shape update of such a block: keep it short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "survives",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_can_survive",
        neoforge = "none (vanilla BlockBehaviour.canSurvive; NeoForge adds no event or extension for it)"
    )]
    fn can_survive();
    /// On an authoritative server, `VegetationBlock.mayPlaceOn` for the
    /// plant `state` on `ground` at `ground_pos`, before the plant answers
    /// it: the blocks a mod plant (one whose template is a vanilla plant,
    /// such as a crop) can stand on. Its template's `canSurvive` asks it, so
    /// the plant is refused there and pops off when its ground changes to
    /// one it can't stand on. Answer only for your own blocks. The level is
    /// read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "may",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_may_place_on",
        neoforge = "IBlockExtension.canSustainPlant"
    )]
    fn may_place_on();
    /// Draw the mod block entity at `pos`, of block `block`
    /// (`mymod:machine`), on this client: once a frame while it is in view
    /// (vanilla's `BlockEntityRenderer.submit`: within its view distance, 64 blocks unless
    /// `block_entity_view_distance` says otherwise, its chunk
    /// faded in, and its bounds, `native_block_entity_render_bounding_box`,
    /// in the frustum), when its `id` is yours. `state` is the block's state
    /// (`blockEntity.getBlockState()`: read its `facing` with
    /// `native_block_state_property`), `entity` holds what the server sent
    /// (`native_block_entity_update_tag`). Add geometry with
    /// `native_draw_quad`, `native_draw_cube`, `native_draw_block` and
    /// `native_draw_item` under `draw`'s pose stack, which starts at the
    /// block's lower north-west corner, in blocks; `draw` also has the partial
    /// tick, the game time and the block's light.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "block",
        neoforge = "EntityRenderersEvent.RegisterRenderers"
    )]
    fn block_entity_submit();
    /// Where the mod block entity at `pos` draws, for culling
    /// (NeoForge's `getRenderBoundingBox`): `bounds` is min x, y, z then max
    /// x, y, z, in blocks from the block's lower north-west corner. It starts
    /// as the block with a block of margin around it; widen it when your
    /// `native_block_entity_submit` draws further (a tall shaft, a beam), or
    /// set an infinite value for one never culled. Asked once a frame for each
    /// within its view distance (`block_entity_view_distance`) whose chunk is drawn.
    #[hook(
        side = "client",
        contract = "collect",
        out = "bounds",
        scope = "block_entity",
        key = "block",
        neoforge = "IBlockEntityRendererExtension.getRenderBoundingBox"
    )]
    fn block_entity_render_bounding_box();
    /// A block nothing in Horizon knows a `BonemealableBlock` for is about to
    /// take bone meal: vanilla's `BonemealableBlock.isValidBonemealTarget`,
    /// `state` the block's own state at `pos`. Answer `valid` for your own
    /// blocks: the meal is used up on one as vanilla uses it up (the growth
    /// particles and the sound follow, whether or not the roll below grows
    /// it), from a player's `BoneMealItem.useOn`, from a dispenser and, on a
    /// crop (a villager bonemeals only a `CropBlock`), from a villager. Asked
    /// on both sides, as vanilla asks it on both, so a client predicts the
    /// use with your answer: say no where you can only grow on a server
    /// (match `LevelSide`). `Some(false)` refuses the meal, even on a block
    /// whose template takes it (vanilla's `growCrop` then keeps the meal);
    /// left `None`, the block keeps the `BonemealableBlock` its template gives
    /// it, or takes no bone meal.
    #[hook(
        side = "both",
        contract = "replace",
        out = "valid",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_is_valid_bonemeal_target",
        neoforge = "none (vanilla BonemealableBlock.isValidBonemealTarget; NeoForge adds no event or extension for it); BonemealEvent fires when meal is used, before the block's contract"
    )]
    fn is_valid_bonemeal_target();
    /// `BonemealableBlock.isBonemealSuccess` for a block that answered
    /// `native_is_valid_bonemeal_target`, after it did: set `success` to grow
    /// it this time. Vanilla draws the level's random here
    /// (`isBonemealSuccess(Level, RandomSource, BlockPos, BlockState)`), so
    /// draw from `native_level_random` to keep vanilla's order.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "collect",
        out = "success",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_is_bonemeal_success",
        neoforge = "none (vanilla BonemealableBlock.isBonemealSuccess; NeoForge adds no event or extension for it)"
    )]
    fn is_bonemeal_success();
    /// `BonemealableBlock.performBonemeal` for a block that answered
    /// `native_is_valid_bonemeal_target` and whose `native_is_bonemeal_success`
    /// said yes: grow it here, `state` its state at `pos`. The meal is used up
    /// either way. Change the block with the level's
    /// `native_set_block_property`, and schedule with `native_schedule_block_tick`
    /// (`ticks` is the hook's). Authoritative servers only.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_perform_bonemeal",
        neoforge = "none (vanilla BonemealableBlock.performBonemeal; NeoForge adds no event or extension for it)"
    )]
    fn perform_bonemeal();
    /// `Block.stepOn` for a mod block, on its authoritative server level.
    /// No self: the simulation owns the entity during this callback.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla Block.stepOn; NeoForge adds no event or extension for it)"
    )]
    fn block_step_on();
    /// A consumable's effect of a mod's own kind: the item's `on_consume_effects`
    /// entry is of a type in a namespace of its own, and the loader kept it
    /// for this call. The kind and the definition payload as a borrowed `Tag` are here; the acting
    /// player is the one consuming, and the game's own effects,
    /// sounds, teleports and drops are open. An unclaimed kind does
    /// nothing. This is the runtime `ConsumeEffect.apply` moment, not effect
    /// type registration through NeoForge's `RegisterEvent`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (runtime ConsumeEffect.apply; RegisterEvent registers effect types, not invocations)"
    )]
    fn consume_effect();
    /// A native mod's hook point (docs/modding/src/how-to/native-mods.md), declared in `hooks/*.rs` with the game's:
    /// the loot table `id` (`minecraft:chests/simple_dungeon`, `minecraft:blocks/stone`,
    /// `minecraft:entities/zombie`) as it loads, its JSON after the data packs', before anything
    /// rolls it (NeoForge's `LootTableLoadEvent`, Fabric's `LootTableEvents.MODIFY`). Add pools to
    /// `table["pools"]` or entries to a pool's `entries`, change or replace it (no pools: it drops
    /// nothing; keep its `random_sequence` when replacing it, as a mob's table without one drops
    /// nothing). Called once per table in id order when the server loads or reloads its data, with the
    /// `DataLoadContext` of that owning load (`origin()` is always `ServerDataLoad`: the client never loads
    /// loot tables, so the hook never fires there).
    #[hook(
        side = "server",
        contract = "collect",
        out = "table",
        context = "data_load",
        version = 2,
        target = "horizon_world::loot::native_loot_table_load",
        neoforge = "LootTableLoadEvent"
    )]
    fn loot_table_load();
    /// A block broke or a mob died, and `drops` are the items about to drop, in
    /// the order they drop: what the loot tables rolled (after
    /// `native_loot_table_load`'s changes), and for a mob what it wore, held or
    /// carried (a fox's mouth item, an allay's, piglin's or horse's inventory),
    /// but not an enderman's carried block or a copper golem's preserved
    /// equipment, which drop after. A charged creeper's head and a fox's mouth
    /// item come before the mob's own table. A block's stacks get its block
    /// entity's data (a shulker box's items) after this. Add, change or take out
    /// stacks; empty ones don't drop (NeoForge's global loot modifiers, Fabric's
    /// `LootTableEvents.MODIFY_DROPS`); `context` says what dropped them and
    /// how, and it is called even when `drops` is empty. A player's use drops
    /// through it too (`LootDrops::used`: shears, a brush, an item
    /// frame), with that player named. Lead drops from untying or cutting a
    /// leash, and from a mob a player killed while leashed, pass through here
    /// too, with that player named. It hears blocks that a
    /// player, an explosion or a mob breaks, not other breaks (leaves decaying,
    /// a torch losing its wall, a piston, `/setblock ... destroy`), nor a
    /// creative break or a tool that can't harvest the block, nor experience.
    /// Server-side, for every player.
    #[hook(
        side = "server",
        contract = "collect",
        out = "drops",
        neoforge = "LivingDropsEvent, BlockDropsEvent"
    )]
    fn loot_drops();
    /// A block is about to take the state it is placed in
    /// (`Block.getStateForPlacement(BlockPlaceContext)`), `state` the block's
    /// own default state. Answer only for your blocks: put the state you want
    /// in `answer` as `Some(Some(state))` — a state of that block, as vanilla
    /// sets properties on its own `defaultBlockState` (another block's is
    /// ignored) — or refuse the placement with `Some(None)`: the block goes
    /// nowhere and the stack is kept, as vanilla's `null`. Left empty, the
    /// block's own placement runs. `context` is
    /// vanilla's `BlockPlaceContext` (the clicked face, the placer's rotation,
    /// the clicked position, whether the clicked block is being replaced, the
    /// item in hand) and `fluid` the fluid in the position the block would go
    /// in, which a waterlogged block reads. Build the state from `state` with
    /// `native_state_with_property`, and know whether `state` is yours with
    /// `native_state_block`. It is asked for every placer that goes
    /// through vanilla's placement (a player's `BlockItem.place`, a
    /// dispenser's `DirectionalPlaceContext`), and the answer still takes its
    /// survival and entity obstruction checks. A trait mod returns
    /// `Some(Some(state))` to choose a state, `Some(None)` to refuse, or
    /// `None` to leave the answer to the next mod or vanilla. Changes to the event's fluid
    /// copy are mod-provided data forwarded to later Parts, never consumed by the engine.
    /// Authoritative servers only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "answer",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_state_for_placement",
        neoforge = "none (vanilla Block.getStateForPlacement; NeoForge adds no event or extension for it)"
    )]
    fn state_for_placement();
    /// A `minecraft:post_attack` effect of a mod's own kind, on a mob a
    /// player's weapon hit: the entry says `enchanted` attacker and `affected`
    /// victim, its `requirements` passed, and its `effect` names a `type` in a
    /// namespace outside `minecraft:`. Told the enchantment id, the kind, the
    /// `effect` payload as a borrowed `Tag`, the level and the mob's id (it may have just died). The
    /// acting player is the one whose weapon carries the enchantment. Each
    /// such entry is told once a hit, and once for each mob a sweep lands on.
    /// A hit on a player, and an entry inside `minecraft:all_of`, aren't told
    /// yet. An unclaimed kind does nothing.
    #[hook(side = "server", contract = "listen", neoforge = "RegisterEvent")]
    fn enchant_effect();
    /// The open screen has drawn itself and its tooltips (`Screen.render`,
    /// the point of `ScreenEvent.Render.Post`): draw over it with
    /// `native_fill`, `native_sprite` and `native_text`, `screen.rect` being
    /// its container panel (or the whole window) and `screen.id` the screen
    /// it is. Once a frame on the window, never while playing with no screen;
    /// `&self`, so it reads and draws only. The Win and Options screens call
    /// it on their own path, after their drawing.
    ///
    /// It does not fire for the screens drawn over gameplay after it (a
    /// server dialog, the server pack, link and code-of-conduct prompts, a
    /// book, a sign editor): those keep `Screen::Playing`, which it skips (the
    /// server pack prompt hides an open screen too, and that screen does not
    /// fire under it), and draw over what it drew, while vanilla's
    /// `ScreenEvent.Render.Post` fires for them. The title's own screens
    /// (`title`, `world_select`, `world_create`, `multiplayer`…) fire it too,
    /// their `screen.id` from the title state machine and the whole window
    /// their `rect`, and no dialog or prompt over them.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ScreenEvent.Render.Post",
        cancellable = false
    )]
    fn screen_render_post();
    /// NeoForge 26.3's ModifyDefaultComponentsEvent: set or remove entries in
    /// `components`, the default component map for `item`, including vanilla items.
    /// Asked once per item after registries freeze, before the owning load
    /// publishes immutable defaults. Receives a read-only DataLoadContext with
    /// the actual ServerDataLoad or ClientPreload origin, frozen registry names,
    /// settings captured once for loaded packages at this load boundary, and
    /// the shared LoadedMods view. Readers and datapack reloads do not replay
    /// this event. No runtime Host, world handles or transaction access.
    #[hook(
        side = "both",
        contract = "collect",
        out = "components",
        context = "data_load",
        version = 2,
        target = "horizon_world::item_defaults::native_modify_default_components",
        neoforge = "ModifyDefaultComponentsEvent"
    )]
    fn modify_default_components();
    /// A mouse button went down over the open screen
    /// (`ScreenEvent.MouseButtonPressed.Pre`, vanilla's
    /// `Screen.mouseClicked`): `screen` is the screen it is, `screen.mouse`
    /// where the pointer is (so there is no position of its own), `button` the
    /// button's GLFW index (0 left, 1 right, 2 middle, 3 back, 4 forward; a
    /// button past those is the platform's own number, which differs between
    /// systems) and `modifiers` GLFW's bits (shift 1, control 2,
    /// alt 4, super 8). Set `canceled` and the press goes no further: no slot,
    /// no screen button, no drag, no hotbar key, no sort or header button.
    ///
    /// It fires before anything the screen does with the press, on the window
    /// only, and while a screen of the game's or the title's is open (Options
    /// and the pack list opened from the title screen count, with no player
    /// and no `menu`; the title's own screens, their
    /// `screen.id` from the title state machine). It does not fire on the
    /// connect and loading screens, nor for a server
    /// dialog, the server pack, link and code-of-conduct prompts, a book or a
    /// sign editor drawn over gameplay, nor while a world loads or saves: the
    /// click there stays the game's own. A gamepad click or a click the
    /// keyboard's focus sends is not the mouse and fires nothing.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonPressed.Pre",
        cancellable = true
    )]
    fn screen_mouse_button_pressed_pre();
    /// A mouse button went up over the open screen
    /// (`ScreenEvent.MouseButtonReleased.Pre`, vanilla's
    /// `Screen.mouseReleased`), with the same `screen`, `button` and
    /// `modifiers` as `native_screen_mouse_button_pressed_pre` and fired at
    /// the same point. Set `canceled` and the release goes no further: a
    /// pressed slot keeps what it took and no drag, slider or scrollbar lets
    /// go.
    ///
    /// Cancel a release only when the press it ends was cancelled too: a
    /// release the screen took ends the drag or the scroll it began, as it
    /// does in vanilla, and no guard here un-cancels it. Same screens as the
    /// press; a press that opened another screen ends on that one.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseButtonReleased.Pre",
        cancellable = true
    )]
    fn screen_mouse_button_released_pre();
    /// The camera shake native mods ask for in this frame, `partial` ticks
    /// past the last tick (0 to 1): add your pitch, yaw and roll in degrees
    /// and your offset in blocks, along the camera's right, up and back, to
    /// `shake`, so several mods' shakes add up. Read while playing only;
    /// Horizon scales the sum by Distortion Effects and bounds it
    /// (`native_shake_matrix`).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeCameraAngles"
    )]
    fn camera_shake();
    /// The values native mods hand their shader packs in this frame,
    /// `partial` ticks past the last tick (0 to 1): a pack's WGSL reads
    /// them as `frame.mods` (eight `vec4<f32>`). Write the slots your own
    /// pack reads; a mod that bundles its pack as `native-<mod id>` is the
    /// one that pack is for. Asked once a frame while a pack is on; a value
    /// that isn't finite reads as 0 (`native_shader_params_now`).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (Horizon-only; NeoForge has no counterpart)"
    )]
    fn shader_params();
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "ExtractBlockOutlineRenderStateEvent"
    )]
    fn block_highlight();
    /// Before one first-person hand, item, map or empty arm is drawn
    /// (`RenderHandEvent`, client). Adjust `render`'s pose or cancel the
    /// default geometry. Equip progress includes the item's swap scale.
    /// Unlike NeoForge, this does not expose the full pose stack, player
    /// render state or submission collector, and cannot draw a replacement.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderHandEvent",
        cancellable = true
    )]
    fn render_hand();
    /// Render-thread notification after a resource-pack reload has applied successfully.
    /// Atlas, models, renderer and client caches already refer to the new resources.
    /// Selection/F3+T and server-pack reloads share this point; initial startup does not
    /// (`client_setup` is the once-only moment after the initial load). Runs in
    /// Reloading: published asset/data queries may refresh caches, but world
    /// handles and transactions remain unavailable.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ClientResourceLoadFinishedEvent"
    )]
    fn client_resources_reloaded();
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// Coordinates are the icon's top-left in GUI units, as the batch takes them;
    /// no item translation is active.
    /// Runs wherever the shared item-decoration path is used (hotbar, menus, tooltips,
    /// and the other screens that draw item icons: advancements, beacon, stats, trading).
    /// The NeoForge counterpart is `IItemDecorator.render`; registration through
    /// `RegisterItemDecorationsEvent` is a separate moment.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemDecorator.render"
    )]
    fn item_decorations();
    /// `Item.getUseDuration` of `stack`, in ticks: `duration` arrives with
    /// vanilla's (a consumable's time, 72000 for a shield, a spear, a bow,
    /// 0 for an item with no use over time). A mod's own item (outside
    /// `minecraft:`) with no `consumable`, `blocks_attacks` or kinetic
    /// weapon that answers above 0 is used over time on a right click, as an
    /// `Item.use` that calls `startUsingItem`, until it is let go or the time
    /// runs out (`native_living_entity_use_item_finish`). A consumable's time
    /// changes too, potions and the other vanilla items keep their own.
    /// Asked when a use starts, each tick a mod's item is in use, and each
    /// frame the hand draws one in use, on both sides. It has no user, as
    /// it is asked so often: answer from the stack alone.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "none (vanilla Item.getUseDuration; NeoForge adds no event or extension for it); LivingEntityUseItemEvent.Start can set a use's duration"
    )]
    fn use_duration();
    /// `Item.getUseAnimation` of `stack` in use, for first- and third-person
    /// rendering. `animation` arrives with the item's vanilla answer; mods may
    /// change it. A `Spear` override on an item without kinetic data is refused
    /// as `None`. Asked on the client when each render consumer updates; answer
    /// from the stack alone.
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.getUseAnimation; NeoForge adds no event or extension for it)"
    )]
    fn use_animation();
    /// `Player.canEat` for the food `stack`, from `Consumable.canConsume`
    /// when eating starts: `vanilla_can_eat` is vanilla's answer (creative,
    /// `can_always_eat`, or hungry). Answer `Some(false)` to refuse the food,
    /// `Some(true)` to let a full player eat; `can_eat` left `None` keeps
    /// `vanilla_can_eat`. Both sides (the client starts eating as the
    /// server does), so answer from the stack and the player alone.
    #[hook(
        side = "both",
        contract = "replace",
        out = "can_eat",
        host = "read",
        neoforge = "none (vanilla Player.canEat; NeoForge adds no event or extension for it)"
    )]
    fn can_eat();
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`native_use_duration`'s answer). `entity` is a mob's
    /// `Entity.getId`, None for a player (`host.players().acting_player()` says who);
    /// a mob fires it only for `LivingRef::start_using_item`. Change `duration` (consumables, potions,
    /// spyglasses and mods' items count to it; below 0 refuses the use, and
    /// 0 runs it out on its first tick, as `native_living_entity_use_item_tick`
    /// running it out, unless the item's own is 0 already), or set
    /// `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: on the server for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only on the server (`native_window` is false
    /// there), not on a server's client.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Start",
        cancellable = true
    )]
    fn living_entity_use_item_start();
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `native_living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions, spyglasses and mods' items. Server-side, for every player
    /// of the world (the helpers read the acting one).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEntityUseItemEvent.Finish"
    )]
    fn living_entity_use_item_finish();
    /// The wheel turned over the open screen (`ScreenEvent.MouseScrolled.Pre`,
    /// vanilla's `Screen.mouseScrolled`): `screen` is the screen it is,
    /// `screen.mouse` where the pointer is (so there is no position of its
    /// own), `scroll` the wheel's movement in x and y as the game's own scroll
    /// handler scales it (a line times the wheel sensitivity: with Discrete
    /// Scrolling on, a notch is ±1, its sign only; a touchpad's pixels are
    /// divided by 30), and
    /// `modifiers` GLFW's bits (shift 1, control 2, alt 4, super 8). Set
    /// `canceled` and nothing the screen would scroll moves: not the bundle
    /// under the pointer, nor a list, slider, scrollbar or map. Nothing else
    /// reads the wheel while a screen of the game's is open, so no hotbar slot
    /// changes either.
    ///
    /// It fires before the screen scrolls anything, for the window's wheel
    /// (horizon-ctl's `scroll-at` sends it through the window too), and
    /// while a screen of the game's or the title's is open (Options and the
    /// pack list opened from the title screen count, with no player and no
    /// `menu`; the title's own). It
    /// does not fire on the connect and loading
    /// screens, nor for a server dialog, the server pack, link and
    /// code-of-conduct prompts, a book or a sign editor drawn over gameplay,
    /// nor while a world loads or saves: the scroll there stays the game's
    /// own. A wheel a gamepad or the keyboard's focus sends is not the mouse
    /// and fires nothing.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseScrolled.Pre",
        cancellable = true
    )]
    fn screen_mouse_scrolled_pre();
    /// A command a player typed (NeoForge's `CommandEvent`, before
    /// `Commands.performCommand` runs it), without its slash: any command,
    /// vanilla's and the mods' own, also one the player may not run. Set
    /// `canceled` to stop it: nothing runs and nothing is answered but the
    /// lines pushed to `replies`, which go to the player as system chat.
    /// No player's state is swapped in and the server's player list is busy
    /// with this one, so `PlayerRef::tell` and the other player helpers
    /// reach nobody here: answer through `replies`. Once for each command a
    /// player sends, on a server, and for each line of a quiet command batch
    /// (`horizon:quiet_commands`, whose replies are dropped). A command the
    /// server refuses before reading it (unsigned chat arguments, the chat
    /// rate limit) never reaches it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "CommandEvent",
        cancellable = true
    )]
    fn player_command();
    /// A key went down over the open screen (`ScreenEvent.KeyPressed.Pre`,
    /// vanilla's `Screen.keyPressed`): `screen` is the screen it is, `key`
    /// vanilla's name for the key (`key.keyboard.e`) — a key with no name
    /// fires nothing — `repeat` whether it is
    /// the keyboard's auto-repeat of a held press (Horizon's extra; NeoForge
    /// folds repeats into the event), and `modifiers` GLFW's bits (shift 1,
    /// control 2, alt 4, super 8). Set `canceled` and the press goes no
    /// further: no focus move, no Escape-close, no inventory or hotbar key, no
    /// typing by the key itself.
    ///
    /// It fires for the window's presses and their repeats, and for the
    /// control socket's `key` (a release fires nothing), after the window's
    /// own shortcuts have run — the fullscreen and screenshot keys, the debug
    /// keys and the narrator — and while a screen of the game's or the title's
    /// is open (Options and the pack list opened from the title screen count,
    /// with no player and no `menu`; the title's own). It does
    /// not fire on the connect and loading screens, nor for a server dialog, the server pack,
    /// link and code-of-conduct prompts, a book or a sign editor drawn over
    /// gameplay, nor while a world loads or saves: the key there stays the
    /// game's own. A cancelled press's release still runs, and its characters
    /// are GLFW's separate char callback and still come; see
    /// `native_screen_char_typed_pre`.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyPressed.Pre",
        cancellable = true
    )]
    fn screen_key_pressed_pre();
    /// A character was typed at the open screen
    /// (`ScreenEvent.CharacterTyped.Pre`, vanilla's `Screen.charTyped`):
    /// `screen` is the screen it is, `c` the character, `modifiers` GLFW's
    /// bits (shift 1, control 2, alt 4, super 8), and `field` a copy of the
    /// focused text box. Set `canceled` and that one character never reaches
    /// a text box.
    ///
    /// `field` is `None` with no box focused, and for the boxes that keep
    /// their own validators: Options' and World Options' fields, the game
    /// rules' and the block editor's. `cursor` and `highlight` count Unicode
    /// scalar characters, and a character sees the box as the one before it
    /// left it. Change `value`, `cursor` or `highlight` and the box takes the
    /// change, cancelled or not, so cancel to replace the character yourself.
    /// New text goes through the box's own filter and length limit, and what
    /// follows the box (chat suggestions, a search's results, the anvil's
    /// name) follows it. `max_length`, `multiline` and whether `field` is
    /// `None` stay the box's: a change to them does nothing.
    ///
    /// It fires once per printable character of a key press's text and once
    /// per character of the input method's committed text, never for a control
    /// character (Enter, Backspace, Tab, Delete, Escape, a Control letter):
    /// those are keys, and the game reads them as keys, not as text. A key's
    /// characters fire after the key has acted, as GLFW's char callback comes
    /// after its key callback (`KeyboardHandler.charTyped`): `screen` is the
    /// screen open then, so a key that closes the screen (the inventory key)
    /// leaves none to hear its character, where vanilla's closed screen hears
    /// nothing, and one that moves the focus types into the box it moved to.
    /// They fire even when `native_screen_key_pressed_pre` cancelled the press
    /// that carried the text, as NeoForge delivers `CharacterTyped.Pre` for a
    /// cancelled `KeyPressed.Pre`: the characters left reach the text box
    /// without the key's own effects. Cancelling one character drops only that
    /// one. The control socket's `type` fires it too, but not for its `\b`
    /// backspace. Same screens as the key press, and it fires with no text box
    /// focused (there is nothing to take the character then).
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.CharacterTyped.Pre",
        cancellable = true
    )]
    fn screen_char_typed_pre();
    /// The server is assembling its data tables from the world's data packs
    /// (`ReloadableServerResources`): a world is opening, or `/reload` was
    /// run, and the recipes, tags, loot and advancements are built after
    /// this. Push `(path, bytes)` pack files and they join this load. `path`
    /// is the overlay's key, `<kind>/<namespace>:<path>` without `.json`, over
    /// the kinds the tables read: `recipe/mymod:thing`,
    /// `tags/mymod:block/thing`, `loot_table/…`, `advancement/…`,
    /// `trade_set/…`, `villager_trade/…`, `dialog/…`. A file that isn't JSON
    /// is skipped with a warning. Asked once per load on both paths, so this
    /// is where a mod recomputes them.
    /// Contributed files sit below the world's own data packs: a pack's file
    /// for the same path still overrides a mod's. Server-side (the single
    /// player's integrated server included). Listener applies run first. A later reload refusal
    /// preserves the old engine generation, but does not undo callback effects. Contributions
    /// share one list, so a failed file cannot be attributed to a particular mod.
    #[hook(
        side = "server",
        contract = "collect",
        out = "files",
        neoforge = "AddPackFindersEvent"
    )]
    fn data_pack_files();
    /// Change the colours of `biome` (`minecraft:plains`) on this client: the
    /// grass, foliage (leaves, vines), dry foliage (leaf litter) and water
    /// tints, 0xRRGGBB, as its biome file and the colormaps set them. Asked
    /// for every biome when a mod calls `native_refresh_biome_colors`, which
    /// rebuilds the terrain with them; a season mod answers from its season
    /// and calls that when the season turns. The dark forest's modifier
    /// still applies to the grass set here; the swamp's noise colours
    /// replace it.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (NeoForge adds ColorResolvers with RegisterColorHandlersEvent.ColorResolvers; it does not retint a biome's own colours)"
    )]
    fn biome_colors();
    /// World field of view, after normal calculation and zoom and before projection/Nausea.
    /// Called for world rendering and near-plane overlays; hand/HUD FOV uses its own path.
    /// Only finite values in 1..=179 degrees replace the computed FOV.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFov"
    )]
    fn compute_fov();
    /// Draw the mob `mob` (its id, kind, custom name and the frame's body
    /// and head turns and walk cycle, `LivingEntityRenderState`) on this
    /// client with a model of your own, once a frame while it is in view
    /// (vanilla's `EntityRenderer.submit`), and set `drawn` to leave out its
    /// vanilla model, armour and held items (NeoForge's `RenderLivingEvent.Pre`,
    /// cancelled). Add geometry with `native_draw_cube`, `native_draw_quad`,
    /// `native_draw_block` and `native_draw_item` under `draw`'s pose stack,
    /// which starts at the mob's feet turned with its body: +z is the way it
    /// faces, +x its left, +y up, in blocks. `draw` also has the partial tick,
    /// the game time and the mob's light. Its name tag, shadow, flames and
    /// glowing outline draw as for any mob. Asked for every mob in view but
    /// players; a mob your server half spawned (`native_spawn_mob`) is told
    /// apart by its id, sent with mod messages, or by its custom name.
    /// A mob is drawn afresh each frame: `draw.keep()` does nothing here.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "drawn",
        neoforge = "RenderLivingEvent.Pre",
        cancellable = true
    )]
    fn mob_submit();
    /// Final scaled third-person camera distance, before visual-shape collision clipping.
    /// `entity` includes its living scale/distance and optional mount scale/distance.
    /// Only finite requests in 0..=1024 blocks replace the vanilla distance.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "CalculateDetachedCameraDistanceEvent"
    )]
    fn camera_distance();
    /// Computed world fog RGB, after ordinary effects and before upload. Title panoramas skip it.
    /// Only finite RGB components in 0..=1 are accepted; alpha and distances stay unchanged.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.ComputeFogColor"
    )]
    fn compute_fog_color();
    /// Raw mouse button input on any screen, before dispatch; cancel skips dispatch and Post.
    /// Button/modifiers use the same GLFW encoding as screen hooks. Synthetic clicks skip it.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseButton.Pre",
        cancellable = true
    )]
    fn mouse_button_pre();
    /// An accepted raw button event finished dispatch, including a screen handler's early return.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "InputEvent.MouseButton.Post"
    )]
    fn mouse_button_post();
    /// `PlayerRespawnPositionEvent`: where `player` respawns, once the server has
    /// found it: the bed or anchor they stand up at (facing it), or the world
    /// spawn's free column. `original_level` and `original_position` are that
    /// point; `missing_respawn_block` says the bed or anchor was gone, so the
    /// player came back at the world spawn. Change `level`, `position`, `yaw`
    /// and `pitch` to send them elsewhere: within the level a teleport, into
    /// another level a level change. `copy_original_spawn_position` (default:
    /// the block wasn't missing) says whether they keep their respawn point for
    /// next time. Horizon places the player first and finds the point once its
    /// chunks have loaded, so this fires then, before `player_respawned` only
    /// when no chunks had to load. Not for the End exit, which isn't a respawn
    /// here (as for `player_respawned`). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerRespawnPositionEvent"
    )]
    fn player_respawn_position(
        player: PlayerRef,
        original_level: LevelDimension,
        original_position: [f64; 3],
        missing_respawn_block: bool,
        level: &mut LevelDimension,
        position: &mut [f64; 3],
        yaw: &mut f32,
        pitch: &mut f32,
        copy_original_spawn_position: &mut bool,
    );
    /// `player` (`name`) respawned (NeoForge's `PlayerRespawnEvent`): at the
    /// end of `PlayerList.respawn`, once they are placed and their health,
    /// food and experience reset, with their state swapped in. Once for each
    /// death that ends in a respawn, on a server; not for the End-exit one.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerRespawnEvent"
    )]
    fn player_respawned();
    /// Final world fog, after the computed color hook, ordinary calculations and Horizon's fog
    /// option, before upload. Title panoramas skip it. Invalid color or distance values retain the
    /// original whole fog.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "ViewportEvent.RenderFog"
    )]
    fn render_fog();
    /// Before an open screen handles a named key's release (same screen gate as key presses).
    /// Cancel bypasses screen handling; physical keys and mapped keyboard hold/toggle state release.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.KeyReleased.Pre",
        cancellable = true
    )]
    fn screen_key_released_pre();
    /// Choose this client's music, once a client tick: `music` and `volume`
    /// hold what vanilla chose (`Minecraft.getSituationalMusic` and
    /// `getMusicVolume`: the biome's track, the creative or underwater one, a
    /// boss's, the menu's; None for silence). Set `music` to a track of your
    /// own, a sound event of your content mod's `sounds.json` such as
    /// `Music::new("mymod:music.spring", 1200, 6000, false)` (the delays in
    /// ticks between songs, and whether it cuts off a different song playing),
    /// None to keep vanilla's quiet, or change `volume` (0 to 1) to fade it.
    #[hook(side = "client", contract = "listen", neoforge = "SelectMusicEvent")]
    fn music();
    /// The tracks to play over (or instead of) the music on this client,
    /// asked once a client tick: push a `MusicLayer` (a sound event, looped,
    /// and its volume, 0 to 1) for each. A layer eases to a new volume over up
    /// to two seconds, and one left out fades out and stops; layers that
    /// first appear in the same tick start together, so list every stem of a
    /// piece from the start, the silent ones at volume 0, and raise them (a
    /// fuller arrangement as a village grows). Their sound events should be
    /// `stream: true` in `sounds.json`.
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        neoforge = "none (SelectMusicEvent swaps the one track; NeoForge has no layered music)"
    )]
    fn music_layers();
    /// The server is about to send its data to the players (NeoForge's
    /// `OnDatapackSyncEvent`, its `sendRecipes`): push the recipe types
    /// (`minecraft:smelting`, `mymod:crusher`) whose recipes your client half
    /// needs. Horizon sends their final recipes — the world's data packs
    /// merged, load conditions held — to every Horizon client when it joins
    /// and again after each `/reload`; a vanilla client is sent nothing new.
    /// Asked once per load, when the server's data tables are built, and the
    /// same types go to every player: unlike NeoForge, which asks again for
    /// each joining player, it can't differ between players. Push nothing and
    /// nothing is sent; push nothing after a load that pushed some and the
    /// clients go back to their own recipes. Server-side (the single
    /// player's integrated server included).
    #[hook(
        side = "server",
        contract = "collect",
        out = "recipe_types",
        neoforge = "OnDatapackSyncEvent"
    )]
    fn datapack_sync();
    /// The server's recipes of `recipe_types` arrived (NeoForge's
    /// `RecipesReceivedEvent`): from now on the recipe helpers
    /// (`native_recipes`, `native_recipes_of_type`) give the server's recipes
    /// of those types in place of the window's own, so rebuild what you made
    /// of them. It comes after a join, once the window has its player, and
    /// after every `/reload`, on a server whose mods asked for them
    /// (`native_datapack_sync`), single player included; with no types when
    /// the server stopped sending them and the window is back on its own
    /// recipes. A recipe the
    /// window's own recipe book doesn't know has `book_id` -1. Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RecipesReceivedEvent"
    )]
    fn recipes_received();
    /// A real window key event after handling, including repeats and consumed shortcuts
    /// (`InputEvent.Key`). `key` is vanilla's name for the physical key (`key.keyboard.a`,
    /// `key.keyboard.unknown` for one it doesn't name), `action` what it did, and the
    /// modifier bits the screen hooks' GLFW encoding. Not cancellable.
    #[hook(side = "client", contract = "listen", neoforge = "InputEvent.Key")]
    fn input_key();
    /// Gameplay scrolling after accumulation, before hotbar/spectator handling.
    /// `scroll` is scaled x/y, `accumulated` whole x/y steps, `buttons` left/middle/right,
    /// and `cursor` the GUI position. Cancel skips gameplay scrolling.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.MouseScrollingEvent",
        cancellable = true
    )]
    fn mouse_scrolling();
    /// Before a real held-button movement reaches screen drag handlers.
    /// `screen.mouse` is the current GUI position; `delta` is its GUI-space movement.
    /// Cancel keeps the new cursor position but skips all screen drag handlers.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "ScreenEvent.MouseDragged.Pre",
        cancellable = true
    )]
    fn screen_mouse_dragged_pre();
    /// The number of players with the mod container at `pos` of `dimension`
    /// open went from `previous` to `viewers` (vanilla's
    /// `ContainerOpenersCounter`, as a barrel keeps it): 0 to more than 0 is
    /// `onOpen`, back to 0 is `onClose`, any change `openerCountChanged`, so a
    /// block can show itself open while `viewers` is not 0. Counted every
    /// tick from the players' open menus, so a player who leaves without
    /// closing (a lost connection, death, another dimension, out of reach, the
    /// block gone) is heard too (`recheckOpeners`). Once for each change, with
    /// no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla ContainerOpenersCounter; NeoForge adds no event or extension for it); PlayerContainerEvent.Open and Close fire per player"
    )]
    fn mod_container_viewers();
    /// NeoForge's `LivingKnockBackEvent`: `LivingEntity.knockback` is about to push entity
    /// `entity` of type `kind` (`minecraft:zombie`, `minecraft:player`) `strength` away from the
    /// direction (`x`, `z`), on the raw strength, before the entity's knockback resistance takes
    /// its share and its motion changes. Vanilla's direction points at the source of a hit (the
    /// attacker's side), and the entity is pushed the other way. `source` is the push's
    /// `DamageSource`, named as `native_living_incoming_damage` names a hit's. `attacker` is the
    /// attacking entity's network id, falling back to the victim's last hurt-by mob, or 0 when
    /// unknown; `attacker_main` and `attacker_off` are what it holds (a player's selected and
    /// off-hand stacks, a mob's main hand), none when unknown. Change `strength`, `x` or `z`, or
    /// set `canceled`, and the push follows them: canceled, the entity's motion is left alone. A
    /// `strength` of 0 or less after the hook is no push, and a direction too short to tell
    /// draws a random one after it, as in vanilla. A value a mod leaves that is not a finite
    /// number is put back as it was. It fires for the players, mobs and armor stands the game
    /// knocks back: a hit's own push, a sprinting or Knockback hit, a sweep, a spear, a
    /// shield's push on its holder, a goat's ram and a mob's melee push. It does not fire for a
    /// sulfur cube with a block that a hit with a causing entity knocks back
    /// (`SulfurCube.knockback` does not call `super` there), for the pushes vanilla doesn't make
    /// with `LivingEntity.knockback` (the ender dragon's wings, Punch arrows, explosions, wind
    /// charges, a mace smash's area, a hoglin's throw), nor for the ones Horizon makes another
    /// way (the ender dragon's). The level is the hit's: its blocks, random and own attachments.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        host = "level",
        neoforge = "LivingKnockBackEvent",
        target = "horizon_client::game::native_hooks::base::HitLevel<'_>::native_living_knockback"
    )]
    fn living_knockback(
        entity: i32,
        kind: &str,
        source: &HurtSource<'_>,
        attacker: i32,
        attacker_main: Option<&ItemStack>,
        attacker_off: Option<&ItemStack>,
        strength: &mut f64,
        x: &mut f64,
        z: &mut f64,
        canceled: &mut bool,
    );
    /// Whether the experience orb `into` takes the orb `from` in (vanilla's
    /// `ExperienceOrb.canMerge`): the same value, and ids a multiple of 40 apart. Asked
    /// for each pair whose boxes meet, when an orb spawns (`tryMergeToExisting`: `from` is
    /// 0, the new orb isn't an entity yet, and only `from_value` is known) and every 20
    /// ticks as an orb scans for neighbours (`scanForMerges`). `can_merge` holds vanilla's
    /// answer; change it either way. A merged orb counts as `count` orbs of `into`'s
    /// value, so forcing orbs of different values together drops points unless the mod
    /// raises `into`'s value with `native_set_experience_orb` in `native_experience_orb_merge`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla ExperienceOrb.canMerge; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_can_merge();
    /// The experience orb `into` is about to take `from` in, after
    /// `native_experience_orb_can_merge` said yes (`from` is 0 and `from_value` the
    /// points of a new orb that is spawning). Set `canceled` to stop it: both orbs stay, and
    /// a spawning orb tries the next orb in reach, then spawns on its own. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "none (vanilla ExperienceOrb.tryMergeToExisting; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_merge();
    /// Slot placement and stack limits for registered vanilla and mod menus;
    /// mounts use `horizon:mount_inventory`, while player inventory and the
    /// creative picker have no registered type. `slot` is the backing index,
    /// including player inventory slots. The slot's own answers arrive first;
    /// the maximum is bounded to 1–99 and the item limit. Refusal also prevents
    /// quick-move top-ups. Runs in the restricted context on server and client
    /// prediction; answer from the stack and context alone, and fast.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "menu",
        key = "menu",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::menu::native_menu_slot_rules",
        neoforge = "none (vanilla Slot.mayPlace and mayPickup; NeoForge adds no event or extension for it)"
    )]
    fn menu_slot_rules();
    /// NeoForge's `LivingEntityUseItemEvent.Tick`: each tick `entity` uses
    /// `stack` in `hand`, where `LivingEntity.updatingUsingItem` starts,
    /// `duration` the ticks left. `entity` is as in
    /// `native_living_entity_use_item_start`. Change `duration` to move the
    /// end (consumables, potions, spyglasses and mods' items count to it,
    /// and so do a client's eating crumbs and first person pose, as the
    /// started duration of `native_living_entity_use_item_start` does). A
    /// cancel sets it to -1. Once a mod set it, the use goes on
    /// only while it is above 0: at 0 or less, by a cancel or set, the
    /// item's tick is skipped and the server finishes the use
    /// (`completeUsingItem`: `finishUsingItem`, the food eaten,
    /// `native_living_entity_use_item_finish`), while an item used on
    /// release (a crossbow) stays frozen until let go. NeoForge's javadoc
    /// says a cancel stops the use, but its code finishes it; this follows
    /// the code. On a server's client a run-out use stays in use, frozen,
    /// so letting go still tells the server. Both sides, as vanilla's
    /// `updatingUsingItem`: answer the same on both, and change the world
    /// only on the server (`native_window` is false there), not on a
    /// server's client. Runs every tick of every use:
    /// check the item first.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Tick",
        cancellable = true
    )]
    fn living_entity_use_item_tick();
    /// `Item.onUseTick` of `stack`, used by `entity` with `remaining` ticks
    /// left, always above 0: what a mod's own item does each tick it is used
    /// (cook, charge, drain). It comes where `ItemStack.onUseTick` calls the
    /// item, once `native_living_entity_use_item_tick` let the use go on:
    /// after a consumable's crumbs, for consumables, shields and mods' items
    /// (vanilla's other items do their own, and a kinetic weapon has none on
    /// the server). Both sides, as `native_living_entity_use_item_tick`:
    /// change the world only where this `Game` runs the world, not on a
    /// server's client. Runs every tick of every use: check the item first.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "LivingEntityUseItemEvent.Tick"
    )]
    fn on_use_tick();
    /// NeoForge's `LivingEntityUseItemEvent.Stop`: `entity` lets go of
    /// `stack` in `hand` before its use ran its time
    /// (`LivingEntity.releaseUsingItem`: the use button let go, a screen
    /// opened), `duration` the ticks left. Set `canceled` to skip the item's
    /// release (a bow doesn't shoot, a spyglass makes no sound,
    /// `native_release_using` isn't called); the use stops anyway. Not when
    /// the hand changed item, which stops the use without a release. Both
    /// sides, as `native_living_entity_use_item_tick`.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Stop",
        cancellable = true
    )]
    fn living_entity_use_item_stop();
    /// `Item.releaseUsing` of `stack`: `entity` let it go with `time_left`
    /// ticks of its `native_use_duration` left, so it was held for
    /// `duration - time_left` (a bow-like item shoots by that charge). Set
    /// `used` as vanilla's return, and the stack's use cooldown starts
    /// (`ItemStack.releaseUsing`). After the vanilla item's own release, once
    /// `native_living_entity_use_item_stop` let it through. Both sides, as
    /// vanilla's: change the world only where this `Game` runs the world,
    /// not on a server's client.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "LivingEntityUseItemEvent.Stop"
    )]
    fn release_using();
    /// A loot function of a mod's own kind: the entry named a `type` in a
    /// namespace outside `minecraft:`, and the roll kept it for this call.
    /// The kind and JSON file input accompany owned API context and candidate snapshots.
    /// Candidate counts remain signed; ordered NBT patches preserve removals and types.
    /// Conversion happens only for subscribed hooks, and refused writeback leaves the roll unchanged.
    #[hook(
        side = "server",
        contract = "collect",
        out = "stack",
        target = "horizon_world::loot::native_loot_function",
        neoforge = "RegisterEvent"
    )]
    fn loot_function();
    /// A loot condition of a mod's own kind: the entry named a `type` in a
    /// namespace outside `minecraft:`. The kind, the JSON, the context and
    /// whether it holds are here (`holds` starts `false`; a mod that claims
    /// the kind sets it). The context is an owned snapshot, captured after the subscription guard.
    #[hook(
        side = "server",
        contract = "collect",
        out = "holds",
        target = "horizon_world::loot::native_loot_condition",
        neoforge = "RegisterEvent"
    )]
    fn loot_condition();
    /// `dimension`'s level opened on the server (NeoForge's `LevelEvent.Load`,
    /// posted in `MinecraftServer.createLevels`): before any of its chunks
    /// (`native_chunk_load`), so a mod sets up what it keeps for the level
    /// here. The level the world opens in comes before `server_started`; the
    /// others open when someone first goes there (a player, a portal, an
    /// `execute in` command), with the level in place. Once for each open:
    /// `native_level_unload` follows. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Load",
        cancellable = false
    )]
    fn level_load();
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`), after its last save:
    /// drop what the mod kept for it. A level other than the Overworld closes
    /// once nobody has been in it for 300 ticks, after its chunks' unloads;
    /// every level still open closes when the server stops, before
    /// `WorldCloseFinal`, and when its open or its loop fails, before
    /// `WorldOpenFailed` or `WorldCloseFailed`. Once for each
    /// `native_level_load`. Don't read its blocks. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false
    )]
    fn level_unload();
    /// The client made a new level for `dimension` (NeoForge's
    /// `LevelEvent.Load` from the `ClientLevel` constructor): on joining a
    /// server, single player's own included, and on changing dimension.
    /// Its chunks come after. When the dimension changes, the old level's
    /// `native_client_level_unload` follows this, as `Minecraft.setLevel`
    /// tells it after the new level is made. Client-side, on the window; the
    /// server's levels have `native_level_load`.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "LevelEvent.Load",
        cancellable = false
    )]
    fn client_level_load();
    /// The client dropped its level for `dimension` (`LevelEvent.Unload`
    /// from `Minecraft.setLevel` and `disconnect`): replaced by another
    /// dimension's, or on leaving the server. Once for each
    /// `native_client_level_load`. Not told when the game itself quits.
    /// Client-side.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false
    )]
    fn client_level_unload();
    /// Whether a mod hears projectiles' impacts (`native_projectile_impact`):
    /// set `hears`. Asked once a server tick before the projectiles fly; only
    /// then does the tick keep the copy of each flying projectile that a
    /// canceled impact flies on from, so a game without such a mod pays
    /// nothing. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "hears",
        host = "read",
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn hears_projectile_impacts();
    /// A projectile hit something (NeoForge's `ProjectileImpactEvent`, posted
    /// where vanilla's projectile has found what it meets, before the hit
    /// acts): `projectile` (its entity id) of type `kind` (`minecraft:arrow`),
    /// shot by `owner` (a mob's or a player's entity id, 0 for none) in the level
    /// `dimension`. It hit the entity `entity` of type `entity_kind` (a player is
    /// `minecraft:player`; a dragon's part is `minecraft:ender_dragon`), or, with
    /// `entity` None, the block at `block` on its `face` (the face's normal,
    /// `[0, 1, 0]` for the top), at the point `at`. Set `canceled` and the impact
    /// does not happen: no damage, no knockback, no sound, no block reaction, the
    /// arrow does not stick, a pearl does not teleport, a potion or a fireball
    /// does not burst. The projectile flies on and is not discarded, so a mod
    /// that cancels also decides what becomes of it. It continues through its
    /// whole motion and may hit something on a later tick. Covers what
    /// `tick_projectiles` flies: arrows, tridents, snowballs,
    /// eggs, pearls, potions, fireballs, wither skulls, wind charges, llama spit and
    /// shulker bullets, from players, mobs and dispensers. It does not fire for
    /// a breeze's deflection, for a hit on a wind charge, for fireworks or for
    /// fishing bobbers. A dragon part or End crystal is told by the id of the
    /// dragon or crystal. Heard only when a mod also sets
    /// `native_hears_projectile_impacts`. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ProjectileImpactEvent",
        cancellable = true
    )]
    fn projectile_impact(
        dimension: &str,
        projectile: u32,
        kind: &str,
        owner: u32,
        entity: Option<u32>,
        entity_kind: &str,
        block: [i32; 3],
        face: [i32; 3],
        at: [f64; 3],
        canceled: &mut bool,
    );
    /// An entity mounts or dismounts a vehicle (NeoForge's `EntityMountEvent`, posted from
    /// `Entity.startRiding` and `removeVehicle`): `entity` of type `kind` (`minecraft:player`,
    /// `minecraft:zombie`) in the server level `dimension`, getting on (`mounting`) or off the
    /// `vehicle` of type `vehicle_kind` (`minecraft:oak_boat`, `minecraft:minecart`,
    /// `minecraft:horse`, `minecraft:cushion`). Set `canceled` and it does not: a player stays
    /// where they were, a mob does not board, a `/ride` fails, and a rider whose dismount is
    /// canceled keeps riding. Told for a player boarding a boat, minecart, cushion or a mount (pig,
    /// strider, horse and its kin, llama, nautilus, happy ghast) and getting off one by their
    /// own input, `/ride`, a stab, a buck, a teleport or spectator mode; and for the mobs
    /// Horizon seats on a vehicle (jockeys, raiders on a ravager, `/ride`) and lets off one
    /// (`/ride`, a stab). It is not told for a rider whose vehicle was removed from the
    /// level (a boat or minecart destroyed, a broken cushion), nor for riders loaded with a
    /// save. The acting player is the one whose action runs, a guest on a server. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityMountEvent",
        cancellable = true
    )]
    fn entity_mount(
        dimension: &str,
        entity: u32,
        kind: &str,
        vehicle: u32,
        vehicle_kind: &str,
        mounting: bool,
        canceled: &mut bool,
    );
    /// An interaction mapping is about to act on the tick's crosshair target. `attempt`
    /// distinguishes a queued press from held use or mining. Use asks for each hand tried,
    /// main first. Cancelling the main hand ends the click. A cancelled attack or use still
    /// swings unless the listener clears `swing_hand`. Fires on the window client.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "InputEvent.InteractionKeyMappingTriggered",
        version = 2,
        cancellable = true
    )]
    fn interaction_key_mapping_triggered();
    /// Before one of vanilla's HUD layers is drawn (`RenderGuiLayerEvent.Pre`):
    /// draw here to put something below the layer, or set `canceled` to hide
    /// it. `layer` is the layer's id, in the order Horizon draws them:
    /// `minecraft:crosshair`, `hotbar`, `effects`, `boss_overlay`,
    /// `vehicle_health` (only riding a mount), `contextual_info_bar` (the
    /// mount's jump bar, the locator bar or the experience bar, with the level
    /// number), `player_health` (the health, armor, food and air rows, drawn in
    /// one pass), `selected_item_name`, `sleep_overlay`, `overlay_message`
    /// (the action bar), `title`, `scoreboard_sidebar`, `subtitle_overlay`,
    /// `chat` and `tab_list`. A layer F1 hides fires nothing, and the chat
    /// screen draws the focused chat itself, so `chat` is only the unfocused
    /// lines. The camera overlays and the debug screen are not layers here:
    /// they are not drawn in this pass. Draw with `native_fill`, `native_text`
    /// and the sprite helpers; `draw.size()` is the GUI
    /// size. A layer a mod registered (`register_gui_layers`) fires this
    /// and `gui_layer_post` too, with its own id, in its place among
    /// these.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "canceled",
        neoforge = "RenderGuiLayerEvent.Pre",
        cancellable = true
    )]
    fn gui_layer_pre();
    /// After one of vanilla's HUD layers (or after the layer a mod hid), with
    /// the ids of `native_gui_layer_pre`: draw here to put something above
    /// the layer (`RenderGuiLayerEvent.Post`).
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "RenderGuiLayerEvent.Post",
        cancellable = false
    )]
    fn gui_layer_post();
    /// Icons a mod puts on map `map` each time it is drawn: held, in an
    /// item frame or on the cartography table's screen (a map's pixels are
    /// its own; nothing is saved in the map). Push `MapDecoration`s
    /// (an atlas sprite, a position in half map pixels, a rotation, and
    /// whether it shows in a frame and on the screen like vanilla's markers
    /// do); they are drawn after the vanilla ones with the same pose, and a
    /// sprite the atlas lacks is left out. `map.center`, `map.scale` and
    /// `map.dimension` say which blocks the map shows, and
    /// `MapDecoration::x` has the formula. Vanilla's decorations are
    /// not passed in. Asked every frame a map is drawn, so keep it cheap.
    /// The client half of `MapDecorationType` and `addDecoration`: a type
    /// registered on the server and saved in the map's data is not here.
    #[hook(
        side = "client",
        contract = "collect",
        out = "decorations",
        neoforge = "MapDecorationType",
        cancellable = false
    )]
    fn map_decorations();
    /// A client started tracking an entity (NeoForge's
    /// `PlayerEvent.StartTracking`, posted at the end of
    /// `ServerEntity.addPairing`): `player` (`name`) is now sent `entity`
    /// (its ID, `uuid` its UUID, `kind` its type, `minecraft:zombie`) and its
    /// changes. It comes into range, its client's chunks reach it, or it is
    /// put in the level; another player is an entity too, and a player never
    /// tracks their own. A mod sends the player what they need to show it
    /// (`native_net_send_to_player`). The entity is as it was last tick: it
    /// may be gone by now. Heard after the tick's players and entities have
    /// moved, in order, with no player's state swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.StartTracking",
        cancellable = false
    )]
    fn player_start_tracking();
    /// A client stopped tracking an entity (NeoForge's
    /// `PlayerEvent.StopTracking`, posted at the end of
    /// `ServerEntity.removePairing`): it went out of range, left the level or
    /// died, or `player` (`name`) left, changed level or respawned, which
    /// stops everything they tracked. `entity`, `uuid` and `kind` are as
    /// `native_player_start_tracking` told. Once for each start, a leaving
    /// player's after `native_player_logged_out`. The entity is not
    /// necessarily still there, and a player who left is gone: no player's
    /// state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.StopTracking",
        cancellable = false
    )]
    fn player_stop_tracking();
    /// Block `block`'s block entity has an energy buffer
    /// (`Capabilities.Energy.BLOCK`): set `energy` when the block is yours,
    /// as `SimpleEnergyHandler` is built: its capacity, the most one insert
    /// and one extract moves, and the faces a fill may come through and a
    /// drain may leave by. Asked once for each mod block when the ticking
    /// table is built; a capacity under 1 leaves the buffer out, and the
    /// limits are clamped to 0..=capacity. The amount is kept in the entity's
    /// saved compound, as the int `energy`, and the energy helpers below read
    /// and write it.
    #[hook(
        side = "server",
        contract = "replace",
        out = "energy",
        host = "read",
        neoforge = "Capabilities.Energy.BLOCK"
    )]
    fn block_entity_energy();
    /// The server is assembling its data tables, as for
    /// `native_data_pack_files` (a world's first open and every `/reload`):
    /// push the data map types your mod registers (NeoForge's
    /// `RegisterDataMapTypesEvent`). A type is an `id` (`mod_id:name`, not in
    /// `minecraft`) for a `registry`, `minecraft:item` or `minecraft:block`
    /// so far. Its values come from the world's data packs'
    /// `data/<mod_id>/data_maps/<registry path>/<name>.json`, and from files
    /// you push through `native_data_pack_files` under the key
    /// `data_maps/<mod_id>:<registry path>/<name>`: `replace`, `values`
    /// (entry ids and `#tags` to JSON values) and `remove`, a newer value
    /// overriding an older. Read them with `native_data_map` and
    /// `native_data_map_entries`, in `native_data_maps.rs`. Server-side, not
    /// synced to clients.
    #[hook(
        side = "server",
        contract = "collect",
        out = "types",
        neoforge = "RegisterDataMapTypesEvent"
    )]
    fn data_map_types();
    /// An entity's breathing this tick (`LivingBreatheEvent`, posted from
    /// `LivingEntity.baseTick` every tick for a living entity with the generic
    /// air logic): `entity` is its ID and `kind` its type (`minecraft:player`,
    /// `minecraft:zombie`). `can_breathe`, `consume` and `refill` come in as
    /// Horizon does it today (a mod-free game changes nothing); set them to
    /// change this tick. With `can_breathe` the air rises by `refill`, up to its
    /// maximum (300); without, it falls by `consume`, and at -20 it is set to 0
    /// and the entity takes 2 drowning damage. Negative amounts count as 0.
    /// Asked on servers for players and for mobs, except those with a
    /// `WaterAnimal`'s or an axolotl's own air logic. It is given the loop `context` of the server that ticks the entity (its registries, settings and mod list), because it fires inside the tick's own air update.
    #[hook(
        side = "server",
        contract = "listen",
        context = "loop",
        neoforge = "LivingBreatheEvent",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::breathing::native_living_breathe"
    )]
    fn living_breathe();
    /// Blocks this mod adds, declared in code rather than one file each
    /// (NeoForge's `DeferredRegister.Blocks` drained by `RegisterEvent`):
    /// push a `mod_api::Block` per block, the same declaration
    /// a content mod's `data/<id>/horizon/block/<path>.json` holds (a
    /// vanilla `template`, `properties`, `state_properties`, `shapes`...),
    /// with its id in your mod's namespace. `item: true` gives it a block
    /// item, as a file does unless told not to (`..Default::default()`
    /// leaves it false). They join the content mods' blocks: the one whose
    /// id is that namespace when it is on (your models, textures, loot and
    /// recipes are its files), else one of their own, with the template's
    /// look. Ids, saves and the check that a joining client has the same
    /// blocks are the files' (docs/modding/src/how-to/content-mods.md). Asked once at start, on every
    /// side, before any resource loads: push the same ones every time, from
    /// nothing but your code. Declarations that don't pass the files' checks
    /// are logged and that namespace's are left out.
    #[notes = r#"## Villager farming

Farmers recognize mod blocks made from farmland and `CropBlock` templates, including declarations from
`native_blocks`. A crop block's generated item is its seed: add that item to
`minecraft:villager_plantable_seeds` to let farmers plant its default block state from their inventory.
Untagged items and tagged items without a block are skipped. Crop maturity follows the template's class:
beetroots mature at age 3, ordinary crops at 7; torchflower crops never reach their maximum age 2 while
still crops. Pitcher crops are not `CropBlock`s and are not harvested by this behavior. Farming still
requires a farmer at work and `mob_griefing` enabled."#]
    #[hook(
        side = "both",
        contract = "collect",
        out = "blocks",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn blocks();
    /// Items this mod adds, declared in code (NeoForge's
    /// `DeferredRegister.Items`), as `native_blocks` declares blocks: a
    /// `mod_api::Item` per item, a content mod's
    /// `data/<id>/horizon/item/<path>.json` (a vanilla `template`,
    /// `max_stack_size`, `rarity`, `components`, `crafting_remainder`). A
    /// block's item comes with its block (`item: true`), not from here.
    /// Asked once at start, on every side.
    #[hook(
        side = "both",
        contract = "collect",
        out = "items",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn items();
    /// An entity of type `kind` (`minecraft:zombie`, `minecraft:item`) with ID `entity` is
    /// about to tick in the server level `dimension` (NeoForge's `EntityTickEvent.Pre`, posted
    /// around `Entity.commonTick` and `tick` in `ServerLevel.tickNonPassenger`). Set `canceled`
    /// and it does not tick this tick: a mob has no AI, physics or effects, an item no movement,
    /// ageing or merging, a minecart or boat no motion, and `native_entity_tick_post` is not told.
    /// Horizon ticks a family of entities together, so mobs and items hear every Pre before the
    /// first of them ticks; minecarts and boats hear theirs one by one. Told for the mobs, item
    /// entities, minecarts and boats that tick: those in an entity-ticking chunk, not a player's
    /// boat or cart (it ticks with the player). Check `kind`; this runs for every ticking entity
    /// each tick, so keep it cheap. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre();
    /// The entity `entity` of type `kind` ticked in the server level `dimension` (NeoForge's
    /// `EntityTickEvent.Post`): for each entity `native_entity_tick_pre` let through, after it
    /// ticked, also one that died or was removed in its tick. Mobs and items hear their Posts
    /// once the whole family has ticked; minecarts and boats right after their own tick. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EntityTickEvent.Post",
        cancellable = false
    )]
    fn entity_tick_post();
    /// Whether a mod listens to `native_entity_tick_pre` and `native_entity_tick_post`: a mod
    /// that hooks them hooks this one too and sets `listened` (HEAD, `*listened = true`).
    /// Without it the game tells no entity's tick, and keeps no list of them. Asked once for
    /// each family (mobs, items, minecarts, boats) each tick. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn entity_tick_listened();
    /// A mob's or an armor stand's equipment slot took a different stack
    /// (NeoForge's `LivingEquipmentChangeEvent`, posted in
    /// `LivingEntity.collectEquipmentChanges`): `entity` is its ID and `kind`
    /// its type (`minecraft:zombie`), `slot` the `EquipmentSlot` (`mainhand`,
    /// `offhand`, `feet`, `legs`, `chest`, `head`, `body`, `saddle`), `from`
    /// what the slot held when this was last heard (none for empty) and `to`
    /// what it holds now. Once for each slot that changed, in that order,
    /// after the mobs' tick, so a mob that joins the level wearing something
    /// counts from empty on its first tick. It only hears: change the mob with
    /// its own helpers. Players are heard by `native_on_equip`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LivingEquipmentChangeEvent",
        cancellable = false
    )]
    fn living_equipment_change();
    /// Whether a mod listens to the mobs' `native_living_equipment_change`: a
    /// mod that hooks it hooks this one too and sets `listened` (HEAD,
    /// `*listened = true`). Without it the game does not read every mob's
    /// equipment each tick, and mobs are not told (armor stands still are). A
    /// mob that ticks once it is set counts from empty. Asked once a tick, after
    /// the mobs' tick. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn living_equipment_listened();
    /// Draw the player `player` (`who`: its profile name, and whether it is
    /// this window's own player) with a model of your own, once a frame while
    /// it is in view (`RenderPlayerEvent.Pre`), and set `drawn` to leave out its
    /// vanilla model, armour, held items, cape and the other layers. This
    /// window's player is drawn only in third person (and F5 front view); an
    /// invisible player is not asked. `draw` is the frame of `native_mob_submit`:
    /// at the player's feet, turned with the body. The name tag, shadow,
    /// flames and glowing outline draw as before. `hidden.hide(part)` leaves
    /// only that part out of the vanilla model this frame (after Skin
    /// Customization), as a mod drawing its own outer skin layer does: a
    /// hidden part takes its children with it (the hat goes with the head),
    /// while its armour, held items and other layers, the cape among them,
    /// keep every part.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "drawn",
        neoforge = "RenderPlayerEvent.Pre",
        cancellable = true
    )]
    fn player_submit();
    /// A layer on every mob and player in view, once a frame after its
    /// vanilla model and layers (`EntityRenderersEvent.AddLayers`, with
    /// `RenderLivingEvent.Post` and `RenderPlayerEvent.Post`): add geometry
    /// in the entity's frame, which starts at its feet turned with the body
    /// (`mob.head_yaw`, `mob.pitch` and the walk cycle place a part, the eyes
    /// being at the entity type's eye height). `who` is set for a player.
    /// `parts` is a humanoid model's parts as vanilla posed them this frame
    /// (`HumanoidModel.setupAnim`), read-only: attach geometry to an arm or
    /// the head without animating it again; None for a model that isn't
    /// humanoid. Not asked for what a mod drew itself (`native_mob_submit`,
    /// `native_player_submit`) nor for a mob that isn't drawn at all.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "EntityRenderersEvent.AddLayers",
        cancellable = false
    )]
    fn mob_layer();
    /// Whether a mod listens to `native_player_submit` or `native_mob_layer`: a
    /// mod that hooks either hooks this one too and sets `listened` (HEAD,
    /// `*listened = true`). Without it the game poses no player or mob for
    /// them, each frame. Client-side.
    #[hook(
        side = "client",
        contract = "collect",
        out = "listened",
        cancellable = false,
        neoforge = "none (Horizon-only; NeoForge has no counterpart); an opt-in gate for a hook"
    )]
    fn player_render_listened();
    /// A player finished editing a sign: `ServerGamePacketListenerImpl.updateSignText`
    /// has taken their update packet and every check has passed (they were
    /// editing that sign, within reach, the sign is not waxed, the chat filter
    /// let the text through), right before the lines go on the sign. `pos` in
    /// `dimension`, the `front` or back side, and the four `lines` as written,
    /// plain text with the formatting codes still in (vanilla strips them
    /// afterwards). Rewrite `lines` (made four lines again) and that is what
    /// the sign gets, without a second look from the chat filter, or set
    /// `cancel` and the sign keeps its old text. With the player's state
    /// swapped in, so `PlayerRef::tell` and the helpers that act on the player
    /// reach them. Once for each update a player sends on a server. Not told:
    /// the host's own editing of a sign on a LAN or single-player world
    /// (the editor writes into their world directly), and the sign changes
    /// that are no edit of the text (dyeing, glow ink, waxing, `/data`,
    /// `/setblock`, mods' `native_with_mod_block_entity`).
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no sign-editing event)",
        cancellable = true
    )]
    fn sign_changed(
        player: u128,
        name: &str,
        dimension: &str,
        pos: [i32; 3],
        front: bool,
        lines: &mut Vec<String>,
        cancel: &mut bool,
    );
    /// Models no blockstate names that this client bakes beside the block
    /// models (NeoForge's `ModelEvent.RegisterStandalone`): push their ids
    /// (`mymod:block/shaft_half`, the model at
    /// `assets/mymod/models/block/shaft_half.json`). Parts a block entity or
    /// a mob draws, with `native_draw_model`, so that no block is registered
    /// only to bake its model. Asked once at start, before the resources
    /// load; every resource load, a pack reload included, bakes them again
    /// from that pack's files. A model that doesn't read is logged and left
    /// out; an id without a namespace is dropped. Only the client asks it,
    /// as NeoForge's ModelEvent.RegisterStandalone is a client mod-bus event.
    #[hook(
        side = "client",
        contract = "collect",
        out = "models",
        context = "registration",
        neoforge = "ModelEvent.RegisterStandalone"
    )]
    fn standalone_models();
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `horizon:mount_inventory`). The player's own inventory
    /// is not one. Heard once a tick, so a menu opened and closed again
    /// inside one tick is not; one that replaces another comes after that
    /// one's close. The player's state is swapped in. `native_player_container_close`
    /// follows. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Open",
        cancellable = false
    )]
    fn player_container_open();
    /// `player` (`name`) closed the menu `native_player_container_open` told
    /// of (NeoForge's `PlayerContainerEvent.Close`, posted in
    /// `ServerPlayer.doCloseContainer` after the menu's `removed`): they
    /// closed it, walked out of reach, died, changed level, or another menu
    /// opened. A player who leaves closes it before `native_player_logged_out`
    /// (vanilla posts nothing for a disconnect; Horizon does, so each open has
    /// its close). Once for each open; not told when the server stops. The
    /// menu's items are where it left them: the block's, and the cursor stack
    /// back in the inventory. State swapped in, when the player has a body.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Close",
        cancellable = false
    )]
    fn player_container_close();
    /// A player's client started (`flying`) or stopped flying (vanilla's
    /// `handlePlayerAbilities`, which honours it only for a player who may
    /// fly: creative or spectator), heard once for each change, on a server,
    /// with the player's state swapped in, so `PlayerRef::tell` and the other
    /// helpers reach them. The player is already flying or not as asked, as
    /// vanilla leaves them; set `cancel` to refuse it: they keep the state
    /// they had and the server sends their abilities back, as vanilla does
    /// for what it refuses, so the client lands or flies again. Only what a
    /// client asks for is heard: not the flight a game-mode change starts or
    /// ends (`/gamemode`), and not a LAN host's own player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no flight toggle event; Bukkit's PlayerToggleFlightEvent)",
        cancellable = true
    )]
    fn player_toggle_flight();
    /// An anvil at `pos` took damage, changing its block from `from` to `to`.
    /// `to` is air when the anvil broke. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn anvil_damaged();
    /// NeoForge's `ArrowLooseEvent`: the player releases a bow `stack` with
    /// `has_ammo` and `charge` ticks held. Change `charge` or set `canceled`
    /// to prevent the shot; negative charge also prevents it. Unlike
    /// `native_living_entity_use_item_stop`, this does not skip the whole item
    /// release; `native_entity_shoot_bow` reports a spawned arrow afterward
    /// with read-only force. Fires on the Game running the bow release, including
    /// LAN guests; a remote client's own release path is not covered. Crossbows
    /// are not covered; `native_arrow_nock` is the bow's start.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ArrowLooseEvent",
        cancellable = true
    )]
    fn arrow_loose();
    /// A beacon at `pos` activated or deactivated after its beam was checked.
    /// `active` says whether it now has a complete pyramid. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn beacon_state_change();
    /// On an authoritative server, a bell at `pos` is about to ring. Set
    /// `cancel` to stop the ring and its raider reveal.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BellRingEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_bell_ring"
    )]
    fn bell_ring();
    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_break_progress_update();
    /// On an authoritative server, fire is about to burn the block at `pos`
    /// away. Set `cancel` to keep that block.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockBurnEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_burn"
    )]
    fn block_burn();
    /// A block finished cooking `item` into `result` at `pos`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_cook();
    /// A server player stopped digging `pos` before the block broke, after
    /// the server cleared the dig and sent the block back to the client.
    /// The player's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockDamageAbortEvent)"
    )]
    fn block_damage_abort();
    /// A dispenser equipped `item` on the first eligible `target` in front
    /// of its block at `pos`. `target` is `player`, `mob` or `armor_stand`.
    /// The equip, item consumption, sound and game event have completed.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_dispense_armor();
    /// A block at `pos` is about to fade from `from` to `to` because of
    /// melting or coral death. Set `cancel` to keep it. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_fade"
    )]
    fn block_fade();
    /// A dispenser at `pos` failed to dispense `item`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_failed_dispense();
    /// Precipitation is about to form ice or place or thicken snow at `pos`.
    /// `from` and `to` are block state IDs. Set `cancel` to keep the old state.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_form"
    )]
    fn block_form();
    /// On an authoritative server, fire is about to be placed at `pos` by
    /// flint and steel, a fire charge, lava, lightning or fire spread. Set
    /// `player` is the acting server player when there is one. Set `cancel` to
    /// leave the position unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockIgniteEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_ignite"
    )]
    fn block_ignite();
    /// A guest's block inventory lock check at `pos`. `locked` is the
    /// vanilla result and may be changed. The guest's state is not swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "locked",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_lock_check();
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn block_receive_game();
    /// A dispenser sheared `entity` of kind `kind` at the block in front of
    /// its own block at `pos`, after the shearing effects. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_shear_entity();
    /// A brewing stand finished brewing at `pos`, after its bottle contents
    /// were changed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_complete();
    /// A brewing stand at `pos` consumed `fuel` and gained brewing fuel. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_fuel();
    /// A brewing stand at `pos` began brewing `ingredient`, after fuel and the timer were set. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn brewing_start();
    /// A server broadcast line is about to be logged and sent. Change `line`
    /// or set `cancel` to drop it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's BroadcastMessageEvent)",
        cancellable = true
    )]
    fn broadcast_message();
    /// An item was placed on a campfire at `pos` and began cooking.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn campfire_start();
    /// On an authoritative server, a cauldron at `pos` is about to change
    /// fill level from `from_level` to `to_level` for `reason`. Set `cancel`
    /// to leave it unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's CauldronLevelChangeEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_cauldron_level_changed"
    )]
    fn cauldron_level_changed(
        dimension: LevelDimension,
        player: Option<u128>,
        pos: BlockPos,
        from_level: i32,
        to_level: i32,
        reason: &str,
        cancel: &mut bool,
    );
    /// A server player or fox that ate chorus fruit (or another item with the
    /// `teleport_randomly` consume effect) is about to try a random spot
    /// (NeoForge's `EntityTeleportEvent.ItemConsumption`, vanilla's
    /// `TeleportRandomlyConsumeEffect.apply`): once per attempt (up to 16),
    /// after they stopped riding. `from` is their feet, `target` the spot
    /// drawn; a mod may move it (its y kept inside the level's height, a
    /// value that is not a number ignored), and the game still looks down
    /// from it for ground it may land on. Canceling ends the effect: no teleport and no
    /// more attempts. `consumer` names the living actor through its handle.
    /// `consumed` is the stack captured before consumption, including
    /// its count and components; it is not reread from the hand after listener writes.
    /// A player's state is swapped in (`host.players().acting_player()`). A fox's
    /// callback has the full server host before its living AI/movement step and
    /// before the next mob ticks. Listener writes survive; changing the consumer's
    /// item, position or eligibility refuses the captured effect instead of
    /// overwriting that write. Cancellation still finishes consumption.
    /// Diverges from upstream: renamed from
    /// `chorus_fruit_teleport`.
    #[hook(
        side = "server",
        contract = "cancel",
        version = 2,
        out = "cancel",
        neoforge = "EntityTeleportEvent.ItemConsumption",
        cancellable = true
    )]
    fn item_consumption_teleport();
    /// A player added a compostable item to the composter at `pos`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn compost_item();
    /// A crafter at `pos` completed recipe `recipe`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn crafter_craft();
    /// A player ignited a creeper with an item.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn creeper_ignite();
    /// Lightning charged a creeper.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn creeper_power();
    /// NeoForge's `CriticalHitEvent`: a player's melee hit on a living
    /// `target_kind` (`minecraft:zombie`, `minecraft:armor_stand`) that deals
    /// damage, where `Player.attack` decides whether it is critical.
    /// `vanilla_critical` is vanilla's answer; `critical` starts as it and may
    /// be forced or denied (as in vanilla a critical hit prevents the sweep, so
    /// a forced crit loses it; a denied crit loses the crit sound and
    /// particles); `damage_multiplier` starts at 1.5 for vanilla's critical and
    /// 1.0 otherwise, and scales the damage only while `critical` ends true. Not fired for a
    /// target that is no living entity (a dragon part) or when the hit does
    /// no damage. Fires on the Game that runs the attack.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "CriticalHitEvent",
        cancellable = false
    )]
    fn critical_hit();
    /// The server's difficulty changed, from `from` to `to` (`peaceful`,
    /// `easy`, `normal`, `hard`; NeoForge's `DifficultyChangeEvent`). It is
    /// vanilla's `MinecraftServer.setDifficulty`, which `/difficulty`, a
    /// dedicated server's management and a test environment's difficulty all
    /// go through, and it is told only when the difficulty is another one. It
    /// only hears: the change has already happened. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "DifficultyChangeEvent"
    )]
    fn difficulty_changed();
    /// The Ender Dragon fight is about to place its first dragon egg at `pos`.
    /// Set `cancel` to leave the block unchanged. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "DragonEggFormEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_dragon_egg_form"
    )]
    fn dragon_egg_form();
    /// A server player applied an enchantment at `pos`; `item` is the finished item and `levels` the levels charged. The player's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn enchant_item();
    /// A server player's or mob's ender pearl is about to teleport its `owner`
    /// (`EntityTeleportEvent.EnderPearl`, `ThrownEnderpearl.onHit`). `owner` is
    /// the live entity handle, `pearl` its projectile id, `from` the owner's feet
    /// and `target` the pearl's previous position. A mod may move the target;
    /// non-finite changes keep the original spot. Canceling leaves the owner
    /// in place with no teleport sound, damage or endermite roll; the pearl is
    /// used up. Player owners keep their acting-player context and mutable fall
    /// `damage` (5; nonpositive is none). Mob owners take no fall damage and
    /// make no endermite draw; their `damage` starts at zero and is player-only.
    /// An owner moved, removed, killed or put to sleep by a listener is not moved
    /// again using the old decision. The impact hook still hears the hit.
    #[hook(
        side = "server",
        contract = "cancel",
        version = 2,
        out = "cancel",
        neoforge = "EntityTeleportEvent.EnderPearl",
        cancellable = true
    )]
    fn ender_pearl_teleport(
        owner: EntityRef,
        pearl: u32,
        from: [f64; 3],
        target: &mut [f64; 3],
        damage: &mut f32,
        cancel: &mut bool,
    );
    /// Entities from `chunk` finished loading into the server world. `count`
    /// is the number of saved entity records read.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entities_load();
    /// Entities in `chunk` left the active server world and were queued for
    /// saving. `count` is the number of entity records.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entities_unload();
    /// A mob broke the door at `pos`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_break_door();
    /// Two mobs completed a breeding action. `child` is absent when vanilla
    /// lays an egg instead of creating a baby.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_breed();
    /// A mob changed a block at `pos`, after the new state was applied.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_change_block();
    /// A mob is about to drop `count` of `item` at `pos` (Bukkit's
    /// `EntityDropItemEvent`).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityDropItemEvent)"
    )]
    fn entity_drop_item();
    /// A bee entered the hive at `pos` and was stored there.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_block();
    /// A mob entered love mode after player interaction. `player` is the
    /// feeder when known.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_enter_love_mode();
    /// A server entity finished its portal wait and is beginning to leave
    /// `dimension` through `pos`. The destination follows the portal rules.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_portal();
    /// A guest player's synchronized pose changed from `from` to `to`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_pose_change();
    /// A player bow or crossbow launched an arrow with `force`, after the
    /// projectile entered the level.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_shoot_bow();
    /// A mob became tamed after its owner and persistent state were set.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_tame();
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Horizon's same-level target path; it does not change vanilla's
    /// movement.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_teleport();
    /// A guest's fall flying state changed on the server. The change has
    /// already been applied.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_toggle_glide();
    /// A leashed mob's leash was removed, before any lead drop is handled
    /// (Bukkit's `EntityUnleashEvent`).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityUnleashEvent)"
    )]
    fn entity_unleash();
    /// An experience bottle broke and its experience was awarded.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn exp_bottle();
    /// Before `player` picks up orb `orb` in `dimension`. Mutations to `value`
    /// and `count` take effect only when not canceled; value is clamped to
    /// 1..=i32::MAX and count to at least one.
    /// Canceling leaves the orb and pickup delay unchanged. Server-side. Diverges from upstream: renamed from `experience_orb_pickup`, to NeoForge's `PlayerXpEvent.PickupXp`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "PlayerXpEvent.PickupXp",
        cancellable = true
    )]
    fn pickup_xp();
    /// The selected damaged Mending `stack` and the proposed durability repair
    /// for `experience` points. The amount is clamped to the stack's current damage. Zero leaves the
    /// experience unspent when it overrides a positive proposal. If the selected
    /// stack changes, the repair is skipped.
    /// Server-side, including `PlayerRef::give_experience_points`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge only scales a repair with IItemExtension.getXpRepairRatio)"
    )]
    fn mending_repair();
    /// A server explosion is about to begin at `center`, with `radius` (Bukkit's
    /// `ExplosionPrimeEvent`). This is listen-only: the explosion proceeds as
    /// usual. Server-side, including the single-player server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's ExplosionPrimeEvent)"
    )]
    fn explosion_prime();
    /// On an authoritative server, how fire treats the block `state`
    /// (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`): put
    /// `(ignite, burn)` in `odds` for your own blocks, and vanilla's table
    /// isn't read. Ignite odds above zero let fire stand beside the block and
    /// spread into the air around it (planks 5, wool 30, leaves 30); burn odds
    /// are how fast fire destroys it (planks 20, wool 60, leaves 60). `(0, 0)`
    /// never burns; keep both within vanilla's 0 to 100. A waterlogged state
    /// never burns and isn't asked. Answer only for your own blocks and leave
    /// `odds` empty otherwise; the level is read-only. Asked hundreds of times
    /// per fire tick: keep it short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "odds",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_fire_odds",
        neoforge = "IBlockExtension.getFlammability, IBlockExtension.getFireSpreadSpeed"
    )]
    fn fire_odds();
    /// On an authoritative server, `CampfireBlock.isSmokeSource` for the
    /// block `state` under a campfire, as the campfire is placed or the block
    /// under it changes: `Some(true)` for your own blocks makes the campfire a
    /// signal fire whose smoke rises higher, as over a hay bale. The level is
    /// read-only.
    #[hook(
        side = "server",
        contract = "replace",
        out = "smoke",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_smoke_source",
        neoforge = "none (vanilla CampfireBlock.isSmokeSource; NeoForge adds no event or extension for it)"
    )]
    fn smoke_source();
    /// A firework rocket completed its explosion effects.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn firework_explode();
    /// A flowing fluid at `pos` is about to change level. `fluid` is `water`
    /// or `lava`; the levels are 0 through 8. Set `cancel` to keep the old
    /// level. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_fluid_level_change"
    )]
    fn fluid_level_change();
    /// The frame rate this window is about to be held to: Max Framerate after the
    /// inactivity throttle and Battery Mode (260 is unlimited). `focused` is whether the
    /// window has focus, `minimized` whether it is minimised and `in_world` whether a world
    /// is open. A mod may lower `limit`, as Dynamic FPS does for a window in the background:
    /// Horizon keeps the lower of its own limit and the mod's, and ignores values below 1.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (vanilla FramerateLimitTracker.getFramerateLimit fires no event)",
        cancellable = false
    )]
    fn framerate_limit();
    /// A furnace at `pos` consumed fuel and began or resumed burning it.
    /// `fuel` is the item consumed from its fuel slot. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_burn();
    /// A server player took an output from a furnace at `pos`; `item` is the output item id and
    /// `count` is how many were taken. The player's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_extract();
    /// A furnace finished smelting `item` into `result` at `pos`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_smelt();
    /// A furnace at `pos` began cooking `item` (its cooking timer advanced
    /// from zero). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn furnace_start_smelt();
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)"
    )]
    fn hopper_inventory_search();
    /// A hopper at `pos` is about to pick up `count` of `item`. Set `cancel` to leave the dropped stack in the world. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no direct equivalent; Bukkit's InventoryPickupItemEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_hopper_pickup_item"
    )]
    fn hopper_pickup_item();
    /// A recipe ingredient of a mod's own kind: the entry named a type in a
    /// namespace outside `minecraft:`, and the loader kept it for this call.
    /// The kind and the converted NBT compound the file wrote are here; fill `items` with the
    /// item ids the ingredient matches (item ids only: components are not
    /// checked here; a match-time hook follows separately). Duplicates are
    /// removed keeping first-seen order. The hook is called once per
    /// occurrence of the kind (nested inside a compound, intersection or
    /// difference too, and under any slot a recipe reads an ingredient from) in
    /// each load, before the load's tables parse the recipes, with the
    /// `DataLoadContext` of that owning load: the client's own recipe load has
    /// `origin()` `ClientPreload`, the server's has `ServerDataLoad`. A server
    /// loads its recipes once, however many tables and copies read them (its
    /// recipe tables, the stations and the copy synchronised to guests, on a
    /// dedicated server too), and a `/reload` asks again once; a client that
    /// only joins a remote server is never told `ServerDataLoad`. The answer
    /// must be the same every time. An
    /// unclaimed kind leaves the list empty and the recipe fails, as an unknown
    /// kind does.
    #[hook(
        side = "both",
        contract = "collect",
        out = "items",
        context = "data_load",
        version = 3,
        target = "horizon_client_lib::crafting::native_hooks::native_ingredient",
        neoforge = "ICustomIngredient"
    )]
    fn ingredient();
    /// A server player is about to open an inventory supplied by a block at
    /// `pos`. Set `cancel` to leave the inventory closed. The player's state
    /// is not swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn inventory_block_start();
    /// A hopper is about to move one item between containers, a composter's
    /// bone meal included. Set `cancel` to leave it in its source slot. A chest
    /// boat or minecart is reported at the block it is in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)",
        cancellable = true
    )]
    fn inventory_move_item();
    /// A player took a crafting grid's result (`ResultSlot.onTake`, NeoForge's
    /// `PlayerEvent.ItemCraftedEvent`): `player` is the player's uuid, `recipe`
    /// the recipe the grid matched (`minecraft:oak_planks`) and `stack` the
    /// result it took, with its components. It only hears: the take has
    /// already happened. It fires for the crafting grid of an inventory or a
    /// crafting table, on the integrated server and a dedicated server, guests
    /// included, and not for a stonecutter's, a smithing table's or a
    /// furnace's result. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.ItemCraftedEvent"
    )]
    fn item_crafted();
    /// A dropped item entity reached its 6000-tick lifetime and is about to be
    /// removed (Bukkit's `ItemDespawnEvent`).
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's ItemDespawnEvent)"
    )]
    fn item_despawn();
    /// A dropped item reaches its lifetime on the server (`ItemEntity.tick`,
    /// NeoForge's `ItemExpireEvent`). Every mod may set or add to `extra_life`,
    /// in ticks, which starts at zero; a positive total keeps the item that much
    /// longer, zero or less lets it expire. Extensions are capped at 32766
    /// ticks.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ItemExpireEvent",
        cancellable = false
    )]
    fn item_expire();
    /// A dropped item entity was successfully added to the server level.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn item_spawn();
    /// A non-persistent leaf block at distance 7 is about to decay at `pos`.
    /// Set `cancel` to keep it. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "LeavesDecayEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_leaves_decay"
    )]
    fn leaves_decay();
    /// A server lightning bolt is about to be created. `cause` is `weather`,
    /// `trident`, `command` or `mod` (a native mod's `strike_lightning`); `player` is the responsible player when known.
    /// Set `cancel` to prevent the bolt and its effects.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn lightning_strike();
    /// A player shift-clicked slot `slot` of the menu of the mod block
    /// entity at `pos` (`AbstractContainerMenu.quickMoveStack`, the method a
    /// Java menu overrides), on the server, before the game's own routing:
    /// move the stack where your block wants it. `slot` is the menu's index:
    /// below `container_slots` it is the block's own slot (`entity.items`),
    /// from there on the player's inventory (27 main slots, then the hotbar).
    /// `stack` is a copy of what the slot holds; take what you move out of
    /// it (put it in `entity.items`, or in another block through its
    /// `ITEMS` capability) and set `handled`: the slot then holds what is
    /// left of `stack` (whatever you wrote to that slot of `entity.items`),
    /// the game's routing is skipped, and the player's client gets the menu
    /// again. Left unhandled, `entity.items` goes back to what it held and
    /// the game moves the stack as for any container; a later mod sees what
    /// an earlier one did to `stack`.
    ///
    /// For a mod container on a vanilla menu type and on a mod menu type,
    /// the left or right button, a slot that holds something and that the
    /// mod's screen shows; not for a spectator or a dead player, a menu no
    /// longer valid, nor during a drag.
    /// Asked once per click, where vanilla asks `quickMoveStack` again while
    /// it keeps moving the same item. The player's state is swapped in, so
    /// `host.players().acting_player()` names them; `entity` is out of the world
    /// during the call, as in `native_menu_button`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "handled",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (a Java menu overrides AbstractContainerMenu.quickMoveStack; no event)",
        cancellable = true
    )]
    fn menu_quick_move();
    /// Farmland at `pos` is about to change moisture from `from` to `to`.
    /// Set `cancel` to keep its old moisture. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "MoistureChangeEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_moisture_change"
    )]
    fn moisture_change();
    /// A player took an anvil's result at `pos` and the levels, inputs and anvil wear are done (`AnvilCraftEvent.Post`); a broken anvil is already air. `craft` holds the inputs and the result as they were before. Not called when `native_anvil_craft_pre` cancelled.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AnvilCraftEvent.Post"
    )]
    fn anvil_craft_post();
    /// A player took an anvil's result at `pos` (`AnvilCraftEvent.Pre`), before the levels are charged, the inputs used up and the anvil worn. `craft` holds the inputs and the result as they were before. Set `cancel` to refuse the take: the result stays in its slot and the inputs, levels, inventory and cursor are as before the click.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "AnvilCraftEvent.Pre",
        cancellable = true
    )]
    fn anvil_craft_pre();
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerArmorStandManipulateEvent)",
        cancellable = true
    )]
    fn armor_stand_manipulate();
    /// A server player empties a bucket at `pos` (`fluid`: `minecraft:water`,
    /// `minecraft:lava`, `minecraft:powder_snow`; a fish or axolotl bucket
    /// empties `minecraft:water`): where `BucketItem.use` has found the block
    /// the fluid goes into and the player may build there, right before it is
    /// placed. Set `cancel` and nothing is poured and the bucket stays full.
    /// The player's state is swapped in, so `PlayerRef::tell` answers them. Only
    /// for guests of a server. `native_fill_bucket` hears this click too (and
    /// the filling one) without the player and the fluid. Not heard: a
    /// dispenser, a cauldron, and a fluid a block places itself.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla BucketItem.emptyContents; NeoForge has no matching event)",
        cancellable = true
    )]
    fn bucket_empty();
    /// A player opens one of the game's own menus (`ServerPlayer.openMenu`; the
    /// job of NeoForge's `PlayerContainerEvent.Open`, which comes after the
    /// menu is open and cannot stop it): `kind` is the menu type
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:crafting`,
    /// `minecraft:merchant`, `minecraft:anvil`, `minecraft:lectern`...; a
    /// horse's inventory is `minecraft:horse_inventory`) and `pos` the block
    /// it belongs to, none for a villager, a mount or a vehicle's chest.
    /// Heard after the checks that refuse the menu (a locked container, a
    /// spectator) and before the menu that was open closes. Set `cancel` and
    /// nothing opens: the open menu stays as it was and the click is spent.
    /// The player's state is swapped in for the call, as for
    /// `native_run_mod_command`, so `host.players().acting_player()`, `PlayerRef::tell` and
    /// the item helpers reach them. Not heard: a menu a mod opens
    /// (`native_open_menu`), the inventory screen, and the editors (sign,
    /// book, command block). Server-side, for every player of the world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (not NeoForge's PlayerContainerEvent.Open: this hook can cancel the opening)",
        cancellable = true
    )]
    fn container_opened();
    /// A creative-mode player sets a slot of their inventory from the
    /// creative screen (`ServerGamePacketListenerImpl.handleSetCreativeModeSlot`):
    /// `slot` is the inventory menu's slot (5 to 8 armour, 9 to 35 the
    /// inventory, 36 to 44 the hotbar, 45 the off hand; below 0 is a stack
    /// thrown out of the screen, which is dropped) and `item` (`minecraft:air`
    /// for an emptied slot) and `count` what is put there. Heard once the
    /// packet passed vanilla's checks (a creative player, a real slot, a valid
    /// stack) and before the slot changes or the stack drops. Set `cancel`
    /// and the packet is ignored. The player's state is swapped in for the
    /// call, as for `native_run_mod_command`, so `host.players().acting_player()`,
    /// `PlayerRef::tell` and the item helpers reach them. Not heard: stacks a
    /// creative player gets other ways (middle-click pick block, `/give`).
    /// Server-side, for every player of the world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla ServerGamePacketListenerImpl.handleSetCreativeModeSlot; NeoForge adds no event or extension for it)"
    )]
    fn creative_slot();
    /// The food level of the server player `player` (`name`) is about to change
    /// from `from` to `to` (Bukkit's `FoodLevelChangeEvent`):
    /// eating, hunger and saturation effects, the exhaustion that costs a
    /// point, starving. Change `to` (kept between 0 and 20; saturation never
    /// exceeds it) or set `cancel` to leave the level at `from`, saturation
    /// included. Heard once for the change a player's turn made as a whole, so
    /// two changes in one tick arrive as one. Not heard: the full food bar a
    /// respawn gives, the food a player joins with and the host of a LAN
    /// world. The player's state is swapped in, so `host.players().acting_player()`,
    /// `PlayerRef::tell` and the item helpers reach them. On a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla: FoodData; Bukkit's FoodLevelChangeEvent)"
    )]
    fn food_level_changed();
    /// An item frame (`minecraft:item_frame`, `minecraft:glow_item_frame`) or
    /// a painting (`minecraft:painting`), `kind` its entity type and `pos` the
    /// block it hangs in, is about to break: a server player's hit
    /// (`player`, their state swapped in, so `PlayerRef::tell` answers them), a
    /// blast, or the wall it hangs on gone (`player` None for both). Where
    /// `BlockAttachedEntity.hurtServer` and its survival check would kill
    /// it, before anything drops or sounds. Set `cancel` and it stays, as it
    /// was. A hit that only takes the item out of a frame does not break the
    /// frame and is not asked. The game's own player's hit is not asked, nor
    /// is `/kill`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla hanging entity breaking; NeoForge adds no event or extension for it); AttackEntityEvent fires for players only"
    )]
    fn hanging_break();
    /// A server player places a hanging entity: an item frame
    /// (`minecraft:item_frame`, `minecraft:glow_item_frame`), a painting
    /// (`minecraft:painting`) or a leash knot (`minecraft:leash_knot`, when a
    /// lead is tied to a fence that has none yet), `kind` its entity type and
    /// `pos` the block it takes (the fence's for a knot). Where
    /// `HangingEntityItem.useOn` and `LeadItem.useOn` create it, once it is
    /// known to fit and before anything is placed or used up. Set `cancel`
    /// and nothing is placed: the item stays in the player's hand. The
    /// player's state is swapped in, so `PlayerRef::tell` answers them. Only for
    /// guests of a server; `/summon`, a dispenser and the game's own player
    /// are not heard, nor is a lead tied to a mob or a boat.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (vanilla HangingEntityItem.useOn; NeoForge adds no event or extension for it); PlayerInteractEvent.RightClickBlock fires before it"
    )]
    fn hanging_place();
    /// The item `item` of a server player is about to lose `amount` points of
    /// durability (`ItemStack.hurtAndBreak`; Bukkit's `PlayerItemDamageEvent`):
    /// the tool that mines or hits, the armour that takes a hit, a shield, a
    /// rod, an elytra. `amount` is what is left after Unbreaking; change it
    /// (0 or less leaves the item whole) or set `cancel` for no wear at all. It
    /// is not heard for a creative player's tools (they take no wear) nor when
    /// Unbreaking already saved the item, nor for the wear of a mob's armour or
    /// of a tool a dispenser uses, nor the host of a LAN world. The player's
    /// state is swapped in, so `host.players().acting_player()`, `PlayerRef::tell` and the
    /// item helpers reach them. Once for each wear, on a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (vanilla: ItemStack.hurtAndBreak; PlayerDestroyItemEvent is only the break)"
    )]
    fn item_damaged();
    /// `player` (`name`) changed dimension (NeoForge's
    /// `PlayerChangedDimensionEvent`): `from` and `to` are dimension ids
    /// (`minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end`).
    /// Heard once per move, when it has ended: the portal's exit is found
    /// and the player stands there (after the destination has loaded), a
    /// command's teleport has landed, or a respawn in another level than the
    /// death's is placed for good (a bed or anchor, or the world spawn's
    /// free column, after the fallback when a respawn block was gone). The
    /// level of `to` is the one in place, and the player's state is swapped
    /// in, as for `native_run_mod_command`: `host.players().acting_player()`,
    /// `PlayerRef::tell`, the item helpers and the rest reach them. Heard for:
    /// Nether and End portals (also riding a vehicle, and leaving the End as
    /// its winner), `/execute in ... run tp` and any other command that
    /// teleports a player into another level, and the death respawn. Not
    /// heard: a move within a level (`/tp`, an End gateway), a player who
    /// leaves before the move ends, a move that ends where it started (a
    /// respawn in the dimension of the death), and anything outside a
    /// dedicated server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.PlayerChangedDimensionEvent"
    )]
    fn player_changed_dimension();
    /// A player's data was copied to their new player (NeoForge's
    /// `PlayerEvent.Clone`, from `ServerPlayer.restoreFrom`): `was_death` is
    /// true after a death, false when they left the End after the credits
    /// (`respawn` with `keepAllPlayerData`). The uuid stays the same, so the
    /// old and the new player share `player`. Heard before the new player is
    /// placed, on a server, with no player's state swapped in. Not
    /// cancellable.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.Clone (not cancellable)",
        cancellable = false
    )]
    fn player_clone();
    /// The command tree a server sends a player (NeoForge's and Paper's
    /// "commands sent to a player" stage): when they join and when their
    /// level changes (`/op`, `/deop`, the LAN settings). `names` are the
    /// root commands their client would get, vanilla's they may use and the
    /// mods'; take a name out and that client gets the tree without it
    /// (and without what it leads to), so it neither completes nor
    /// highlights it. This hides the command from the client only: the
    /// server still runs a typed one, `native_player_command` stops it.
    /// For a player who is already on, their state is swapped in and the
    /// helpers (`PlayerRef::tell`...) work; for one who is joining it is not, and
    /// their client has no world to tell yet. The tree the hub sends again by
    /// itself (after a respawn or a change of level) keeps the last answer
    /// and does not ask again; the integrated server's owner is not asked
    /// when the world's cheats are switched.
    #[hook(
        side = "server",
        contract = "collect",
        out = "names",
        neoforge = "none (NeoForge 26.3 has no such event)"
    )]
    fn player_commands_sent();
    /// The item `item` of the server player `player` (`name`) broke in `slot`
    /// (`mainhand`, `offhand`, `head`, `chest`, `legs`, `feet`, `body` or
    /// `saddle`): `LivingEntity.onEquippedItemBroken`, Bukkit's
    /// `PlayerItemBreakEvent`. Listen only; armour has a slot, not a hand.
    /// The player's state is swapped in. Once for each break, on a server; a
    /// creative player and the host of a LAN world are not heard.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's PlayerDestroyItemEvent carries the original stack and a hand, which armour has not; this hook only listens and names the slot instead. Bukkit's PlayerItemBreakEvent)"
    )]
    fn player_destroy_item();
    /// A server player's fishing rod, with `state`: 0 the cast
    /// (`FishingRodItem.use` throwing the bobber), 1 a fish biting
    /// (`FishingHook.catchingFish`), 2 the reel-in of a bite, which gives the
    /// catch, 3 the reel-in with nothing on the line, 4 the reel-in of a
    /// bobber hooked to an entity or item (like Bukkit's `PlayerFishEvent`;
    /// NeoForge's `ItemFishedEvent` is later, in `FishingHook.retrieve`). Set `cancel` on the cast and nothing
    /// is thrown; on a bite the fish gets away and the wait starts over; on
    /// a reel-in (2 to 4) nothing happens, the bobber stays out and the rod
    /// takes no wear. The bobber ending by itself (too far, the rod put
    /// away, the owner dying), nor the host of a LAN world's rod. The
    /// player's state is swapped in, so `host.players().acting_player()`, `PlayerRef::tell`
    /// and the item helpers reach them. Once for each cast, bite and reel-in,
    /// on a server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge's ItemFishedEvent fires after the loot roll, in FishingHook.retrieve; Bukkit's PlayerFishEvent)"
    )]
    fn player_fish();
    /// A server player put a plant into a flower pot, after the potted block state was stored and the
    /// item consumed. `plant` is the item id and `pos` the pot position. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_flower_pot_manipulate();
    /// A server player's game mode is about to change (NeoForge's
    /// `PlayerChangeGameModeEvent`): from `from` to `to`, both the shared
    /// `GameMode` used by commands and `PlayerRef`. `player` is the UUID and
    /// `name` the name. Heard for
    /// the `/gamemode` command, the F3+F4 switcher's packet and a hardcore
    /// world's respawn into spectator, once for each change to a different
    /// mode; not for the world's default mode a player joins with. Return
    /// `Flow::Cancel` and the player keeps their mode (nothing is sent, the
    /// command counts it as no change). For a change the game makes inside
    /// a player's turn their state is swapped in; for the hardcore respawn
    /// it is not.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerEvent.PlayerChangeGameModeEvent"
    )]
    fn player_game_mode_changed();
    /// A server player inserted a book into a lectern, after the block entity and inventory changed.
    /// `book` is the item id. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_insert_lectern_book();
    /// A server player changed an item frame after its contents or rotation changed. `action` is
    /// `put`, `rotate`, `remove_item` or `break`; `item` is empty when no item remains, and `rotation`
    /// is the resulting step. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_frame_change();
    /// Mending repaired a server player's item with experience from an orb. `repair` is durability
    /// restored and `experience` is the orb's value. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_mend();
    /// A server player changed the page of a lectern, after its page and redstone pulse changed.
    /// `page` is the zero-based new page. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_lectern_page_change();
    /// A player was loaded from the world's save (NeoForge's
    /// `PlayerEvent.LoadFromFile`, `PlayerList.loadPlayerData`): their `.dat`
    /// has been read and applied, and they are not placed in a level yet.
    /// `directory` is the world's player directory (`players/data`): a mod
    /// reads its own file there, named `<uuid>.<suffix>` as
    /// `getPlayerFile(suffix)` names it. Heard once for each join, for the
    /// owner of an integrated server as for anyone else, on a server, with no
    /// player's state swapped in. Not cancellable.
    ///
    /// Not on wasm: it gives a path in the game's file system, which a wasm mod has none of; a wasm mod keeps a player's data in attachments.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.LoadFromFile (not cancellable)",
        cancellable = false
    )]
    fn player_load_from_file();
    /// A player whose login the server has accepted (NeoForge's
    /// `PlayerNegotiationEvent` stage): the name, whitelist, ban and
    /// server-full checks have passed and the player holds a place, but their
    /// saved state is not loaded and they are not placed in a level yet. Set
    /// `cancel` to refuse them: they are disconnected with `reason` (a plain
    /// "Disconnected" if it is left unset) and nothing else of the join (the
    /// join message, `native_player_logged_in`) happens. Once for each player
    /// who joins, on a server. The player is not placed yet and their client
    /// has no world to show a message in, so no player's state is swapped in
    /// and `PlayerRef::tell` reaches nobody: the refusal `reason` is the answer.
    /// The checks that need no mod (name, whitelist, bans, a full server,
    /// the proxy's forwarding) refuse earlier, on the connection, and this
    /// hook does not hear those players.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerLoginEvent; NeoForge's PlayerNegotiationEvent is not cancellable)",
        cancellable = true
    )]
    fn player_login();
    /// A player was saved to the world's store (NeoForge's
    /// `PlayerEvent.SaveToFile`, `PlayerList.save`): when they leave or the
    /// world closes, when a dedicated or integrated server saves its guests,
    /// and for an integrated server's owner when the world saves them.
    /// Once for each save. `directory`
    /// is the world's player directory (`players/data`): write your own file
    /// there, named `<uuid>.<suffix>`, never the player's `.dat`. The player
    /// may be leaving, so no player's state is swapped in. Not cancellable.
    /// Autosave snapshots are written asynchronously: this notification runs
    /// on a later server tick after a successful write, possibly after WorldSaved.
    /// Live state may have advanced beyond the saved snapshot; a process exit
    /// before completions are drained can prevent notification delivery.
    ///
    /// Not on wasm: it gives a path in the game's file system, which a wasm mod has none of; a wasm mod keeps a player's data in attachments.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerEvent.SaveToFile (not cancellable)",
        cancellable = false
    )]
    fn player_save_to_file();
    /// A server player's sprint started or stopped: their client sent
    /// `START_SPRINTING` or `STOP_SPRINTING` (the sprint key, a double tap
    /// forward, or letting go) and the server's player is now `sprinting`
    /// (vanilla's `setSprinting` in `handlePlayerCommand`). Heard only when it
    /// changes. It cannot be stopped, and it hears what the client asks, not
    /// what the server does: a sprint that ends by itself (a hit's knockback,
    /// hunger, a wall) is not reported. The player's state is swapped in, so
    /// the helpers and `PlayerRef::tell` work. Once for each change, on a server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no sprint event)"
    )]
    fn player_sprint();
    /// `player` (`name`) pressed the swap-hands key: heard before the main and
    /// off hand items swap (the `SWAP_ITEM_WITH_OFFHAND` player action that
    /// `ServerGamePacketListenerImpl.handlePlayerAction` answers; Bukkit's
    /// `PlayerSwapHandItemsEvent`), not for a spectator. Set `cancel` to keep
    /// the items where they are: the player's client, which swapped them at
    /// once, gets its slots back. The player's state is swapped in, so
    /// `host.players().acting_player()`, `PlayerRef::tell` and the item helpers reach them.
    /// Once for each press, on a server. Not heard: the same swap made in an
    /// open inventory (hovering a slot and pressing the key, a menu click) and
    /// the host of a LAN world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingSwapItemsEvent.Hands"
    )]
    fn player_swap_hands();
    /// A server player took the book from a lectern, after the menu and block entity changed. `book`
    /// is the removed item id. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_take_lectern_book();
    /// A server player a command teleported (NeoForge's
    /// `EntityTeleportEvent`, for a player): `/tp` and `/teleport`, inside
    /// their level or into another one. `from` and `to` are their feet before
    /// and after, `dimension` the level they are in now. Heard after the move
    /// has reached the player's state and their client (an inter-level
    /// teleport: when the new level is entered, before it has loaded around
    /// them), so it cannot stop it. The player's state is swapped in, so the
    /// helpers that read the acting player and `PlayerRef::tell` work. Once for
    /// each teleported player, on a server. Ender pearls and chorus fruit
    /// have their own hooks; a spectator's teleport menu is not heard yet;
    /// portals and respawns are not teleports here.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (EntityTeleportEvent.TeleportCommand fires before the move and can cancel it; this hook is told after the move)"
    )]
    fn player_teleported();
    /// A totem of undying saved the server player `player` (`name`): their
    /// hurt would have killed them and the totem held in either hand fired
    /// (`LivingEntity.checkTotemDeathProtection`; Bukkit's
    /// `EntityResurrectEvent`), heard just before it is used up and its
    /// effects apply. Listen only: whether the totem saves is decided inside
    /// the health state machine, before a mod is asked, so a mod cannot veto
    /// it from here. The player's state is swapped in, so
    /// `host.players().acting_player()`, `PlayerRef::tell` and the item helpers reach them.
    /// Once for each totem, on a server; the host of a LAN world's totem is
    /// not heard.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's LivingUseTotemEvent can cancel the save; this hook only listens. Bukkit's EntityResurrectEvent)"
    )]
    fn player_totem();
    /// A server player's client answered a resource pack in play (vanilla's
    /// `ServerboundResourcePackPacket`): `status` is `Action`'s number, 0
    /// successfully loaded, 1 declined, 2 failed download, 3 accepted, 4
    /// downloaded, 5 invalid URL, 6 failed reload, 7 discarded. Heard when the
    /// packet arrives, after the server has handled it; it cannot be stopped.
    /// The player's state is swapped in, so the helpers and `PlayerRef::tell`
    /// work. The pack a dedicated server offers in the configuration
    /// (`resource-pack` in `server.properties`) is answered before the
    /// player has joined, and those answers are not heard. Once for each
    /// answer, on a server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no resource-pack status event)"
    )]
    fn resource_pack_status();
    /// The world's spawn point changed to `pos` and `angle`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn spawn_change();
    /// TNT at `pos` is about to be lit (primed): a flint and steel or a fire
    /// charge on it, fire burning it, a redstone signal, an arrow, or a
    /// dispenser's flint and steel or TNT item. `player` is the server player
    /// whose click lit it, with their state swapped in (so `PlayerRef::tell`
    /// answers them), None when no player is behind it (fire, redstone, an
    /// arrow, a dispenser). Set `cancel` and it is not primed: a TNT block
    /// stays where it is (one that fire or a signal had already taken out is
    /// put back, without a block update), a dispenser's TNT item stays in it.
    /// Not asked: TNT set off by a blast (`native_explosion_detonate` spares
    /// blocks), a TNT minecart and a sulfur cube. Server-side and in single
    /// player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (NeoForge has no TNT priming event; the primed TNT entity joins with EntityJoinLevelEvent)"
    )]
    fn tnt_primed();
    /// A boat or minecart was created at `pos`, after it joined the level.
    /// `kind` is the vehicle type. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleCreateEvent)"
    )]
    fn vehicle_create();
    /// A vehicle is about to take damage from a player's attack. `kind` is
    /// `boat` or the minecart type; mods may change `damage` or set `cancel`
    /// to leave its damage and wobble unchanged. The attack itself still
    /// completes. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleDamageEvent)",
        cancellable = true
    )]
    fn vehicle_damage();
    /// A boat or minecart is about to be destroyed at `pos`. TNT minecarts
    /// that prime instead of breaking are not heard. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's VehicleDestroyEvent)"
    )]
    fn vehicle_destroy();
    /// A boat or minecart (`kind`, `minecraft:oak_boat`) moved since the last
    /// tick (`VehicleMoveEvent`): `from` and `to` are its positions, heard
    /// after the tick moved it. Once a tick for each vehicle that carries a
    /// server player, whose state is then swapped in (as for
    /// `native_run_mod_command`: `host.players().acting_player()`, `PlayerRef::tell` and
    /// the item helpers reach them; `player` is their UUID), and for one a
    /// server player rode and left, until it has stood still for ten
    /// seconds (`player` is none, nobody is swapped in). It only hears.
    /// Cheap when nothing moves: one position compare a tick for each watched
    /// vehicle. Not heard: a vehicle moved before anyone rode it, a mount
    /// (horse, pig...) or a vehicle carrying only a mob. Server-side, in the
    /// level the vehicle is in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no vehicle move event; EntityTickEvent covers the tick)"
    )]
    fn vehicle_moved();
    /// The server world's border bounds changed, either immediately or when
    /// its timed size change was requested.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_bounds_changed();
    /// The server world's border center changed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_center_changed();
    /// A server player completed an advancement, after progress was recorded.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AdvancementEvent.AdvancementEarnEvent",
        cancellable = false
    )]
    fn player_advancement_done();
    /// A criterion of a server player's advancement was granted or revoked, after its progress changed
    /// and before any completion rewards. A grant or revoke that changes nothing is not heard.
    /// `progress` is an owned snapshot of every criterion (including its stored grant date),
    /// the requirement groups and their completion at this exact mutation. It belongs to
    /// the mutated progress, including first-tick and reload grants queued by the server hub.
    /// During a nested player turn, a suspended player's hub record may contain displaced
    /// progress. A listener may retain a clone; later mutations do not change the snapshot.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "AdvancementEvent.AdvancementProgressEvent",
        cancellable = false
    )]
    fn player_advancement_progress(
        player: u128,
        player_entity_id: i32,
        name: &str,
        advancement: &str,
        criterion: &str,
        progress_type: ProgressType,
        progress: &AdvancementProgress,
    );
    /// A guest's arm swing was handled by the server. `hand` is `mainhand`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerAnimationEvent)"
    )]
    fn player_arm_swing();
    /// A server player reset their attack strength by swinging (`handlePunch`). The swing and reset
    /// have already happened. The player's state is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_attack_cooldown_reset();
    /// A server player is about to enter a bed after the sleep checks pass. Set `cancel` to keep them
    /// out. The player's state is swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerBedEnterEvent)",
        cancellable = true
    )]
    fn player_bed_enter();
    /// A server player woke or stood up from the bed at `pos`, after their
    /// sleeping state changed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerWakeUpEvent",
        cancellable = false
    )]
    fn player_bed_leave();
    /// A server player caught a mob in a bucket, after it was removed and the filled bucket was given.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_bucket_entity();
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerBundleItemSelect)"
    )]
    fn player_bundle_item_select();
    /// A server player changed a beacon's selected effects, after they were stored. Each selected
    /// effect is an `Option<EffectKey>`; None means no effect. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_change_beacon_effect();
    /// A server player finished loading the world around them after joining, respawning or changing
    /// dimension. The guest's state is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerClientLoadedWorld)"
    )]
    fn player_client_loaded_world();
    /// A server player has slept for 100 ticks in a bed and now counts as deeply asleep for the
    /// night-skip check.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_deep_sleep();
    /// A server player's book edit or signing packet is about to be applied. `pages` is its page
    /// count; `signing` means the packet has a title. Set `cancel` to drop the edit.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerEditBookEvent)",
        cancellable = true
    )]
    fn player_edit_book();
    /// A server player threw an egg, after its projectile was launched and the use was applied.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_egg_throw();
    /// A server player used a firework rocket to boost while gliding, after the rocket was launched
    /// and consumed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_elytra_boost();
    /// A server player's XP orb pickup cooldown changed to `ticks`. It is
    /// reported after the value changes. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_exp_cooldown_change();
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerInputEvent)"
    )]
    fn player_input();
    /// An item cooldown is about to start for a server player. `item` is the item id and `ticks` its
    /// duration. The player's state is not swapped in. Set `cancel` to skip starting the cooldown.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Paper's PlayerItemCooldownEvent)",
        cancellable = true
    )]
    fn player_item_cooldown();
    /// `PlayerSwitchHotbarSlotEvent.Pre`: a server player is switching from hotbar slot
    /// `old_slot` to `new_slot` (0 to 8), after the slot range check and before the
    /// selected slot changes. Fired only when the slot differs. The player's state is not
    /// swapped in. Set `cancel` to keep the old slot; the client is sent that slot again,
    /// and `player_switch_hotbar_slot_post` doesn't fire.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayerSwitchHotbarSlotEvent.Pre",
        paper = "PlayerItemHeldEvent",
        cancellable = true
    )]
    fn player_switch_hotbar_slot_pre();
    /// `PlayerSwitchHotbarSlotEvent.Post`: a server player switched from hotbar slot
    /// `old_slot` to `new_slot` (0 to 8), after the selected slot changed and a main-hand
    /// item use stopped. Fired only when the slot differs and
    /// `player_switch_hotbar_slot_pre` didn't cancel it. The player's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerSwitchHotbarSlotEvent.Post",
        cancellable = false
    )]
    fn player_switch_hotbar_slot_post();
    /// A guest's accepted movement made the server jump them from the ground. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerJumpEvent)"
    )]
    fn player_jump();
    /// A guest was disconnected by a server kick, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerKickEvent)"
    )]
    fn player_kicked();
    /// A server player launched a projectile item, after it was created.
    /// `item` is the projectile's held item.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerLaunchProjectile)"
    )]
    fn player_launch_projectile();
    /// A server player's experience level changed, after the new level was applied.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerLevelChangeEvent)",
        paper = "PlayerLevelChangeEvent"
    )]
    fn player_level_change();
    /// A server player selected a loom pattern, after the loom menu accepted the button. `pattern` is
    /// the zero-based button index. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_loom_pattern_select();
    /// A server player filled an empty map, after the filled map and its saved data were created.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_map_filled();
    /// A server player named a mob with a name tag, after its name and persistence changed and the tag
    /// was consumed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_name_entity();
    /// A server player opened a sign for editing, after the editor state was made. `front` selects the
    /// front or back side. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_open_sign();
    /// A server player picked an item from a block or entity, before it enters the hotbar. `pos` is
    /// zero for an entity; `entity_id` is zero for a block. The player's state is swapped in. Set
    /// `cancel` to leave the inventory and client unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Paper's PlayerPickItemEvent)",
        cancellable = true
    )]
    fn player_pick_item();
    /// A server player picked up an arrow after it was removed and its pickup was applied. `item` is
    /// the arrow's pickup item id. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_pickup_arrow();
    /// A server player released a drawn bow with `arrow` selected, by the use-item
    /// key or a mod's `LivingRef::release_using_item`. The player's state is swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerReadyArrow)"
    )]
    fn player_ready_arrow();
    /// A server player's recipe book requested a known recipe be placed in
    /// their crafting grid. `recipe` is the recipe id; `all` is the shift
    /// click form. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_recipe_book_click();
    /// A server player's recipe book open or filter setting changed. `book`
    /// is the book type's protocol id. The guest's state is not swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerRecipeBookSettingsChange)",
        paper = "PlayerRecipeBookSettingsChangeEvent"
    )]
    fn player_recipe_book_settings_change();
    /// A recipe is about to be unlocked for a server player. Set `cancel` to keep it locked. The
    /// player's state is not swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerRecipeDiscoverEvent)",
        cancellable = true
    )]
    fn player_recipe_discovered();
    /// A server player launched with a riptide trident, after the velocity,
    /// spin attack and grounded lift have been applied. The player's state is
    /// swapped in. No reply is needed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerRiptideEvent)"
    )]
    fn player_riptide();
    /// A player shearing a mob is about to do so. Set `cancel` to leave the mob and shears unchanged.
    /// The guest's state is swapped in.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerShearEntityEvent)",
        cancellable = true
    )]
    fn player_shear_entity();
    /// A server player's shield was disabled by a blocked melee hit. The shield cooldown is set
    /// immediately after this call. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_shield_disable();
    /// A statistic is about to be awarded to a server player, before its count and scoreboard criteria
    /// change. `stat_type` and `stat` are ids such as `minecraft:mined` and `minecraft:stone`, and
    /// `value` the amount. Set `cancel` to award nothing. Server-side. Ordinary interactions
    /// fire at vanilla's award point; inside a transaction, the award and this hook wait until
    /// the outermost successful commit, after tentative world and inventory writes.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "StatAwardEvent",
        cancellable = true
    )]
    fn player_stat_award();
    /// A server player selected a stonecutter recipe, after the menu accepted
    /// its button. `recipe` is the zero-based button index. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_stonecutter_recipe_select();
    /// A server player stopped spectating `entity_id`, after their camera returned to the player.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_stop_spectating_entity();
    /// A server player released the use-item key, or a mod's `LivingRef::release_using_item` did,
    /// before the item finished being used. The player's state is swapped in.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerStopUsingItem)"
    )]
    fn player_stop_using_item();
    /// A server player swapped their main and off hands, after both inventory slots changed.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_swap_with_equipment_slot();
    /// A server player is about to travel through an End gateway at `gateway`. Set `cancel` to leave
    /// them at the entrance; the gateway is not consumed and its cooldown does not start. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn player_teleport_end_gateway();
    /// A server player toggled an ageable mob's age lock with a golden
    /// dandelion, after the lock and item use changed.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_toggle_entity_age_lock();
    /// A guest's client changed sneaking in its player input packet, after the hub applied the new state.
    /// The guest's state is swapped in. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerToggleSneakEvent)"
    )]
    fn player_toggle_sneak();
    /// A server player completed a villager or wandering trader trade. `entity_id` is the merchant
    /// network id. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_trade();
    /// A server player removed an entity's leash, after it was cleared.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_unleash_entity();
    /// A server applied `velocity` to a player, when syncing changed motion to that player's client.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_velocity();
    /// A Nether portal was built for a dimension travel, or an End platform
    /// was made for an arrival. `pos` is the lower corner of the Nether
    /// portal rectangle or the End platform origin. The completed blocks are
    /// already in the world.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn portal_create();
    /// A thrown splash or lingering potion applied its hit effects.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn potion_splash();
    /// A hopper at `pos` began an item transfer attempt. This runs after its
    /// cooldown and enabled checks, before pushing or pulling any items.
    /// Server-side, including the single-player server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no hopper event)"
    )]
    fn prepare_hopper_transfer();
    /// A crafting-grid result was prepared for a server player; `pos` is the crafting table, or None for the inventory grid. Change `result` before it is sent.
    #[hook(
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "none (NeoForge has no such event)",
        paper = "PrepareItemCraftEvent"
    )]
    fn prepare_item_craft();
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn prepare_item_enchant();
    /// A smithing result was prepared for a server player at `pos`. Change it before it is sent.
    #[hook(
        side = "server",
        contract = "replace",
        out = "result",
        neoforge = "none (NeoForge has no such event)",
        paper = "PrepareSmithingEvent"
    )]
    fn prepare_smithing();
    /// A server projectile was added to the level. `player` is set only for
    /// a player launch; other sources have an empty `name`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn projectile_launch();
    /// A raid wave has been spawned at `pos`; `wave` is its one-based wave
    /// number and `raiders` counts the raiders, not the mobs they ride.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn raid_spawn_wave();
    /// A server raid at `pos` changed to `state` (`triggered`, `victory`,
    /// `loss` or `stopped`). `raid` identifies its owning level, including a
    /// parked level. A terminal raid already removed from storage cannot be read.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn raid_state_change();
    /// A sculk catalyst at `pos` has begun blooming after taking experience.
    /// The catalyst block state has changed. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn sculk_bloom();
    /// Enough players have slept through the night in level `dimension`
    /// and the clock is about to move to the morning (NeoForge's
    /// `SleepFinishedTimeEvent`, vanilla's `ServerLevel.tick` when it moves
    /// the clock to `wake_up_from_sleep`): `new_time` comes in as the next
    /// morning's day time and may be changed to wake everyone at another
    /// time. A time before `current` (the day time now) is refused, as
    /// `setTimeAddition` refuses one, and the morning stays. Unlike
    /// NeoForge, which refuses a bad value as it is set, the check runs once
    /// after every mod, so a later mod sees the value an earlier one left.
    /// On the server, only when the `advance_time` rule is on;
    /// `native_time_skip` then hears the jump to the time chosen here and
    /// may still cancel it. The players wake and the weather resets as
    /// usual either way. Set `canceled` and the clock is left where it is, as NeoForge's cancel leaves it:
    /// `native_time_skip` is not asked either.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "SleepFinishedTimeEvent",
        cancellable = true
    )]
    fn sleep_finished_time();
    /// A sponge at `pos` absorbed water and became wet. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_sponge_absorb"
    )]
    fn sponge_absorb();
    /// A guest asked to complete `command`; owned API `Suggestion` records may be
    /// added to or filtered from `suggestions`. `range` is the replaced byte range.
    /// The game snapshots completions only when a listener subscribes.
    #[hook(
        side = "server",
        contract = "collect",
        out = "suggestions",
        neoforge = "none (NeoForge has no generic tab completion event)"
    )]
    fn tab_complete();
    /// A projectile hit the target block at `pos`, which now has `strength`
    /// output power. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn target_hit();
    /// The server changed its thunder state through `/weather`. Set `cancel`
    /// to keep the previous thunder state and timer. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn thunder_change();
    /// The overworld clock is about to jump, from sleep or `/time`. Set
    /// `cancel` to keep `from`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn time_skip();
    /// A server player selected trade `index` from `villager` in the trading menu.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn trade_select();
    /// A trade of a villager or a wandering trader was made
    /// (`MerchantResultSlot.onTake` once the offer is taken): `merchant` is
    /// the trader's entity id and `kind` its type (`minecraft:villager`,
    /// `minecraft:wandering_trader`), `offer` the live one in the trading
    /// session (not a copy) as the trade left it: `uses` counted, `result`
    /// what the player got. Once per trade, so a shift-click that trades
    /// several times is heard for each, after the trade's statistic and
    /// `villager_trade` trigger. A mod may change the offer (Bukkit's
    /// `PlayerTradeEvent#getTrade` is mutable): `uses`, `max_uses`, `demand`,
    /// `special_price_diff`, `price_multiplier`, `xp`. The change stands: the
    /// next click and the offers sent to the player see it. Not asked for a
    /// trade window without a trader. The player's state is swapped in
    /// (`host.players().acting_player()`). Server-side. Version 2: was a listen hook
    /// returning the offer read-only; now collects the live offer as `out`
    /// (the moment upstream's separate `merchant_offer_traded` covered).
    #[hook(
        side = "server",
        contract = "collect",
        out = "offer",
        neoforge = "TradeWithVillagerEvent",
        cancellable = false,
        version = 2
    )]
    fn trade_with_villager();
    /// A vault at `pos` changed to `state`. Its block state is updated.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn vault_change_state();
    /// A vault at `pos` is about to eject a loot item.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockDispenseLoot)"
    )]
    fn vault_loot();
    /// The server changed its weather parameters. `raining` and `thundering`
    /// are the requested final states. Set `cancel` to keep the previous
    /// weather parameters. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn weather_change();
    /// The dedicated server's whitelist changed to `enabled`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's WhitelistToggleEvent)",
        paper = "WhitelistToggleEvent"
    )]
    fn whitelist_state_update();
    /// A timed world-border size change finished on the server, after its
    /// final size was applied. Server-side, including the single-player
    /// server.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's WorldBorderBoundsChangeFinish)",
        cancellable = false
    )]
    fn world_border_bounds_change_finish();
    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_saved();
    /// An entity that can't breathe and has no air (`LivingDrownEvent`, posted
    /// from `LivingEntity.baseTick`), with vanilla's answers as the defaults: it
    /// drowns at -20 air, for 2 damage and 8 bubbles. Change `is_drowning` or
    /// `damage_amount`, or cancel to skip drowning this tick. Bubble count is
    /// read-only; changes to the event field do not affect the vanilla entity
    /// event. Asked on servers, for the entities `native_living_breathe` is.
    /// It is given the loop `context` of the server that ticks the entity (its
    /// registries, settings and mod list), because it fires inside the tick's
    /// own air and damage update.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingDrownEvent",
        version = 2,
        target = "horizon_client_lib::breathing::native_living_drown"
    )]
    fn living_drown();
    /// A living entity is about to handle a fall (`LivingFallEvent`). Mods may
    /// change the distance and damage multiplier, or cancel fall handling. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own landing, before its damage and block effects, so it takes no `Host`.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingFallEvent",
        version = 2,
        target = "horizon_client_lib::landing::native_living_fall"
    )]
    fn living_fall();
    /// A living entity is about to heal. Mods may change the amount or cancel.
    /// `entity` identifies the affected entity. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list), because it fires inside the tick's own health update, where a deferred call would reorder vanilla's effects.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "LivingHealEvent",
        version = 2,
        target = "horizon_client_lib::hunger::native_living_heal"
    )]
    fn living_heal();
    /// A living entity jumped (`LivingEvent.LivingJumpEvent`). It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own jump, after the velocity write, so it takes no `Host`.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        neoforge = "LivingEvent.LivingJumpEvent",
        version = 2,
        target = "horizon_client_lib::motion::native_living_jump"
    )]
    fn living_jump();
}

/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"a dead player's respawn point: `native_player_respawn_position(player, name, dimension, pos, yaw, pitch)` is heard once per respawn before the player is placed (NeoForge's `PlayerRespawnPositionEvent`, Bukkit's `PlayerRespawnEvent.setRespawnLocation`), with their bed, anchor, forced point or the world spawn. A changed dimension or position places the player exactly there, with no bed or anchor used; yaw and pitch set their look. An unknown dimension or a non-finite value leaves that part unchanged. Bundles that use it need `"hooks": @API:player_respawn_position@`."###]
    fn player_respawn_position();
}
