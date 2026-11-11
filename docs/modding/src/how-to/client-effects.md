# Temporary client sound effects

These effects change what a player hears, not the server's world. They do nothing until a mod asks for them. See the [API reference](../reference/api.md) for the generated signatures.

## Reading the tooltip preference

Read `ClientHost::gui_options().advanced_item_tooltips` for the current
`advancedItemTooltips` preference (F3+H). The snapshot reflects the current client
setting on native and wasm; request a new snapshot after a toggle. This is separate
from the advanced flag supplied to an individual tooltip request.

## Adding tooltip lines

Use `ItemPart::tooltip` for an attached item's own lines at the vanilla
`Item.appendHoverText` position, after the name and before component lines.
Use `Hooks::item_tooltip` for NeoForge's global `ItemTooltipEvent` moment: it
receives every item's finished tooltip, including the parts' earlier lines,
component lines and advanced information. No attached part is needed for a
global listener, so this is also how a mod edits vanilla or other mods' tooltips.
The global event supplies the stack, display settings, player IDs, and the
request's `advanced` and `creative` flags on native and wasm alike.

## Drawing a second world camera

Native internals mods can request extra world cameras through `world_views` and
consume their textures in `world_view_drawn`. These are raw renderer places, not
portable API callbacks: wasm mods cannot use them. Construct a `WorldView` with a
stable ID, camera, yaw, pitch, field of view and pixel size. Return the same
requests whenever the frame asks; at most four valid, distinct IDs draw, with
sides limited to 4096 pixels and the device limit.

Use `world_view_drawn` to copy a still to your own buffer or draw into the supplied
target with its encoder. Copy a picture you need to preserve: targets are reused
while requested and retired when absent. Views draw before `world_drawn`, where
you can composite their pictures into the player's frame. Extra cameras increase
rendering work and do not include particles, weather, hands or shader-pack passes.
See the generated `world_views` hook notes for the full rendering contract and
`tools/run_world_views.py` for the offscreen fixture.

## Muffling sounds

`ClientHost::muffle_sounds(amount, lowpass, ticks, exempt)` temporarily muffles the sound output, for example to accompany ringing ears after a nearby blast:

- `amount`, from 0 to 1, is the fraction of every sound's volume taken away.
- `lowpass`, from 0 to 1, dulls the output, from untouched to a 300 Hz low-pass.
- Both effects start at full strength and fade linearly over `ticks` client ticks. The fade waits while the game is paused.
- The sound events in `exempt` remain clear, including ones already playing. Use this for the mod's ringing sound. Other sounds already playing, music and interface clicks are muffled too.

A new call replaces the current muffle; `ticks` of 0 ends it immediately. The effect belongs to the sound output, so every splitscreen player hears it. The call refuses an amount or strength outside 0 to 1, or an exempt event that is not a namespaced sound id, without changing the output. Server code cannot call this client-only service.

Play the accompanying sound through `ClientHost::play_local_sound` with a `SoundPlay`, rather than a separate legacy sound API.
