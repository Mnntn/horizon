<!-- chapter: Native mods -->
<!-- separator: 1 -->

## Native mods

A native mod's config files go in the game's shared `config/` folder, beside `options.txt` (on a dedicated server,
beside `server.properties`), as Fabric and NeoForge mods' do: the mod picks its own file names, such as
`config/<mod id>.json` or a folder `config/<mod id>/`. It reads and writes them with `Host::config_files()`, which is
bound to the calling mod and refuses any other mod's files. A mod that lists its files with
`native_register_configs` gets them read for it, written from their defaults when missing, and handed over again
when edited (`native_config_loading`, `native_config_reloading`, as NeoForge's `ModConfigEvent`), and a world can
keep its own server config in `serverconfig/` ([native mods](../native-mods.md)). Settings kept with `set_setting` stay in `horizon-mods/.horizon-mod-data/<id>/`.
`setting` and `set_setting` are bound to the calling mod and its registered keys: naming another mod's ID or an unregistered key never reads or changes a file.
Typed reads cache values by data folder and key; a successful `set_setting` invalidates that key, while a failed write leaves the cached value unchanged. Raw `setting` reads access the file.
Register typed settings with `Registrar::register_setting` to put them on Options > Mods; edits persist before
`setting_changed` runs, and only the owning mod receives that event.
8c838a6b381b93150deaeed2057acd1

For a client HUD's session information, use the read-only
`ClientHost::client_session()` service and the local `PlayerRef`'s
`block_breaking` and `ambient_mood` reads rather than raw injections. Missing
sessions, external display names and memory addresses remain explicit optional
values; all counters stay local. See [client session and local-player status](../../reference/hook-notes.md#client-session-and-local-player-status)
for their exact availability and frame-time semantics.

`native_compute_fov_modifier(player, base, modifier)` runs on the client each tick for the local player's FOV scale, after vanilla effects and before zoom and camera easing. `player` contains the calculation inputs and `base` is the original scale. Non-finite modifiers or values outside 0.01..=10 fall back to `base`. It does not affect a spectator camera on a mob.

The client hook `native_render_name_tag(entity_id, kind, vanilla_visible, visible, text)` runs at player and mob, armor stand, and item frame name decisions before shared geometry. Set `visible` to override vanilla and `text` to `Some` literal replacement text (NeoForge `RenderNameTagEvent`). Distance and entity availability still apply; the local player's tag is considered only when its own camera and settings gates allow it.

Native mods are native code with the player's rights, not sandboxed. The first install shows a trust notice,
and every build waits behind a red warning; the registry lists one only after a review (its later releases are
checked by machine). They play in a game of their own, built for the instance they are installed in.

The game's Mods screen only lists code mods (it has no switch). A code mod's settings page,
when it has one, opens from the Config button. The launcher decides which mods are on. A browser install that
finishes after a world opens waits until the world closes before activating its content.

Villager handles preserve the full signed integer level (at least 1), including values above the
five vanilla trade tiers. Higher levels use the master badge and do not gain further vanilla trades.

### Measuring native mod calls

An administrator with permission level 3 can run `/modprofile start`, exercise the mod,
then run `/modprofile stop`. Profiling is off by default. Starting twice or stopping without
an active session reports an error. `/modprofile report` prints the last completed summary
without writing a file or stopping an active session. Feedback goes only to the sender.
The same controls work through `horizon-ctl command modprofile start` (and `stop` or `report`).

Stop saves one local, atomically written `profiles/modprofile-<timestamp>-<counter>.json`
under the game's folder. The export is bounded to 8 MiB; a save failure is reported and the
stopped summary remains available. Load the file locally in Firefox Profiler's Marker Table;
Horizon never uploads it. The raw Gecko version 37 markers contain mod/hook/Part identities,
counts, exact nanosecond timings and log2 histograms, never arguments or player/world data.
Timings are inclusive: an outer call includes nested calls. These are process-wide aggregates,
not invocation traces or sampling shares. This first command release measures native guarded
calls only; wasm timing and a dedicated profiling guide follow separately.
