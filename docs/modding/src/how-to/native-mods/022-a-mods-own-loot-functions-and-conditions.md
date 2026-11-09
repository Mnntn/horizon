<!-- chapter: A mod's own loot functions and conditions -->
<!-- separator: 1 -->

### A mod's own loot functions and conditions

A loot table's function or condition that names a `type` in a namespace outside `minecraft:` reaches `native_loot_function(kind, data, ctx, stack)` or
`native_loot_condition(kind, data, ctx, holds)` when the table rolls. The function changes the stack; the
condition says whether it holds. The context brings the entity, the damage, the player who killed it, the
tool, the block, the luck, the biome and a seed from the roll's random. An unclaimed function leaves the
stack, an unclaimed condition fails, as unknown kinds do; a bare type name is minecraft's own.

The tables the items engine rolls (block drops, entity drops, `loot_table`, `interact_loot` and the
built-in tables) answer both hooks, with the tool and the block in the context. The function hook there
gets an owned API `LootStack`: its resolved `ItemKey`, signed intermediate count, damage, enchantments
(a book's stored ones), potion, ominous bottle amplifier, stew effects and ordered NBT component patches.
Each patch is `(String, Option<Tag>)`; `None` removes a component, and repeated IDs apply in order.
Unknown NBT values retain their exact type, including byte arrays and bytes. Names are API `Text` trees.
The context is an owned `LootContext`, including entity equipment and deferred exploration-map facts.
Conversion runs only after the subscription guard. Writeback validates the entire candidate before replacing
the engine roll; an invalid candidate leaves the roll unchanged. Intermediate counts can be non-positive:
these values are not occupied inventory stacks. A function that asks for a map's structure search (`map`/`exploration`) changes nothing there: the
items engine has no level to search. In the items engine's context the `entity`, `damage`, `player` and
`biome` fields are `None` and `luck` is 0; in the main loot engine's context the `tool` and `block` fields
are `None`.

Both engines' contexts carry `place`, where the roll happens: `origin` (a broken block's centre, a dead mob's
position), `block_entity` (the broken block's entity as it was before the break, saved with its id, position and
`Items`) and `level` (the dimension, the game time, the time of day, the rain and the thunder, for conditions like
vanilla's `time_check` and `weather_check`). The game sets it for a block a player or an explosion breaks and for a
mob's death loot; on other rolls `place` is `None`.
