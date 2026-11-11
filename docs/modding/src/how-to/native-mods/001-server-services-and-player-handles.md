<!-- chapter: Server services and player handles -->
<!-- separator: 1 -->

## Server services and player handles

Server hooks reach commands through `host.commands()`: `run_command(line)` uses
console permission level 4 and the existing console command path, while
`feedback(text)` answers the current command source. `host.messages()` provides
`broadcast(text)` (including the broadcast hook and server log) and
`text_allowed(text)` (the server chat filter).

On the client, `host.chat_line(Text::literal("Local notice"))?` adds a line
only to the local chat HUD and history, without sending a packet or firing
`client_chat_received`. Handle its `Result<(), mod_api::TextFormError>`: text exceeding the
boundary's size or nesting limits is refused before it reaches the HUD.

`host.players().server_players()` returns `Vec<PlayerRef>` in connection order;
`host.players().acting_player()` returns the optional `PlayerRef` whose state is
swapped in for this hook. The online list includes connected players even during
nested player turns; a displaced body's reads still refuse where they did before.
`host.players().player_by_name(name)` finds a connected real player by account
name, case-insensitively, or returns `None`; it does not match nicknames or fake
players. `host.levels().list_levels()` returns loaded server level handles in
vanilla dimension order, or an empty list before a server world opens. Both
services are available through `&dyn ServerHost` and do not mutate the world.
Read identity through `player.uuid()`, `player.name(host)`,
`player.permission_level(host)` and `player.address(host)`. The address is the
connection IP without a port (the forwarded address behind a Velocity proxy),
empty for an acting player with no known connection. Name, permission and address
reads retain connected identities during displacement and refuse stale handles.
`player.level(host)` returns the authoritative `LevelRef`, and
`player.dimension(host)` returns its dimension `Identifier`, including registered
custom dimensions. These reads take `&dyn Host`; dedicated and integrated servers
read the player's current dimension, while a client reads its local player's level.
They return `None` for unknown,
disconnected, replaced or stale player handles. Connected players retain their
current dimension through nested guest turns; the level may be parked rather
than the dimension whose tick is currently running.
Use the entity and living handles for other world-state reads.
No snapshot of event data is returned by these services.

Send to one player with `player.tell(host, text)`, `action_bar(host, text)` or
`tab_list(host, header, footer)`. `set_nickname(host, nickname)`,
`set_sleep_ignored(host, ignored)` and `hide(host, permission)` keep their existing
server behavior. Messaging reaches the acting player and other connected players,
including displaced online records; it refuses stale or disconnected identities.
Vanish state is stored by UUID and can be changed with a retained handle after
disconnection, until its world epoch closes. It cannot target arbitrary offline
UUIDs; retain the handle value, not the borrowed service. Nickname and sleep
writes still require an online player. The settings survive reconnects.

A player handle also answers the game's own rules for that player, with the checks and state
the real actions use: `may_use_item_at(host, pos, face, stack)` (`Player.mayUseItemAt`: builders
always may, adventure and spectator players only with a matching `can_place_on`) and
`can_harm(host, other)` (`ServerPlayer.canHarmPlayer`: the `pvp` rule and their teams).
`respawn_config(host)` reads the respawn point (`Some(None)` for none) and
`set_respawn_config(host, config)` writes it as `/spawnpoint` does, `player_set_spawn` veto
included. `text_filtering(host)` is the option the player's client announced. For the night skip,
`sleep_ignored(host)` reads what `set_sleep_ignored` left, `counts_for_sleep(host)` says whether
the player is counted, and `sleep_timer(host)` / `set_sleep_timer(host, ticks)` read and write
`Player.sleepCounter` (0 to 109; 100 is slept long enough). Reads take a `Host`, writes a
`ServerHost`.

These small services exist only on `ServerHost`, not `Host`, `ClientHost` or a
restricted level host. Native and wasm mods use the same calls: service methods
are generated imports, and acquiring a service just reborrows the current call's
frame resource. Block and fluid scheduling stays on `LevelTicksHost`, and
`tick_stats()` stays on `ServerHost`.
