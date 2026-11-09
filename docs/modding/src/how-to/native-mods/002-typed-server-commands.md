<!-- chapter: Typed server commands -->
<!-- separator: 1 -->

## Typed server commands

Register each server usage with `register_commands` and `Command`. The
`run_mod_command` event carries a `CommandInvocation`,
not a raw line to split. Match its `literals` to your command path, then read named
values through `command.argument(name)` and `CommandArgumentValue`. An integer is
already an `i32`; a player argument is a live `PlayerRef`, not selector text.
Return `Flow::Cancel` when your listener handles the command.

The server parses and checks permissions before resolving arguments or calling a
listener. Omitted optional arguments are absent, including an omitted optional
player: they do not silently select the source player. A present player argument
must select exactly one online player; missing players or invalid input produce
command feedback without calling the listener. Word and greedy text arguments
are strings, numbers are finite doubles, booleans are typed, and item arguments
are component-bearing stacks of one item. Block positions are resolved and floored
from the source's world, relative or local coordinates without loading chunks.
A player's source is their position and rotation. The console has no source
entity and uses the Overworld spawn's block corner with zero rotation, so `@s`
cannot resolve there. Every source receives the same owned data, on native and wasm.

`event.source` is a non-exhaustive `CommandSource`: `Player { player, .. }`,
`Console { .. }`, `CommandBlock { position, level, .. }`,
`CommandBlockMinecart { entity, .. }` or `Function { id, .. }`.
`source.permission()` returns the effective `PermissionLevel`: functions use
function permission; blocks and carts use game-master permission. `execute as`,
`at` and `in` change argument resolution, not the origin. Feedback through
`host.commands().feedback` honours the underlying source's silence and track-output.

## Custom argument types

Register a value kind once from `Hooks::register`, then use its namespaced id as an
argument's kind:

```rust,ignore
use mod_api::{CommandArgumentError, CommandArgumentKind, CommandArgumentType,
              CommandSuggestion, Registrar, RegistrationContext, Text};

struct Machine;

impl CommandArgumentType for Machine {
    fn kind(&self) -> CommandArgumentKind {
        CommandArgumentKind::Word
    }

    fn parse(&self, input: &str) -> Result<String, CommandArgumentError> {
        match input {
            "press" | "furnace" => Ok(input.to_owned()),
            _ => Err(CommandArgumentError::new(Text::translate(
                "example.commands.machine.unknown", vec![Text::literal(input)]
            ))),
        }
    }

    fn suggest(&self, prefix: &str) -> Vec<CommandSuggestion> {
        ["press", "furnace"]
            .into_iter()
            .filter(|machine| machine.starts_with(prefix))
            .map(CommandSuggestion::new)
            .collect()
    }
}

fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
    r.register_command_argument("machine", Machine);
}

fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
    event.commands.push(Command::new("machine use <machine:mymod:machine>"));
}
```

The server parses the argument with `parse` before calling `run_mod_command`; a
rejected value shows the returned `Text` and no listener runs. A valid value arrives
as `CommandArgumentValue::Custom { kind, value }`, where `kind` is `mymod:machine`
and `value` is the canonical text `parse` returned. `suggest` is asked for the
completions while the player types.

`CommandArgumentKind::Word` takes one word; `Greedy` takes the rest of the line and
must be the usage's last argument. Players whose client does not know the type still
parse and complete the command: the server sends the argument as a Brigadier string
of that shape with `minecraft:ask_server`, and their chat asks the server for the
suggestions. (Fabric sends the registered argument type's id, which an unmodded
client cannot read; Horizon's string keeps the tree valid.)

The namespaced id is in the registering mod's namespace; an id registered twice, a
name in another namespace or an empty name refuses the mods at startup, naming the
mod. A client mod registers client command argument types the same way; they are
available to `register_client_commands`.

`player_command` stays player-only pre-execution cancellation (NeoForge's `CommandEvent`), including vanilla commands.

When the usages a mod registers change at runtime, call
`host.commands().refresh_commands()` on the server. At the start of the next
server tick, every mod is asked to register again. The server rebuilds the typed
command dispatcher and sends each player's tree after `player_commands_sent`
filters it, as vanilla does after `/reload`. Repeated requests before that tick
coalesce; without a world open to players, nothing is rebuilt.
