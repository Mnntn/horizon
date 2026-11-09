<!-- chapter: Host helpers and remaining gaps -->
<!-- separator: 1 -->

`Host` exposes the documented `Game` helpers declared by area in
`crates/horizon-mod-api/src/host/<area>.rs`, one trait per area; `Host`, `ServerHost` and `ClientHost` are the
unions of those traits, so a mod calls through them and never names an area trait. Each helper keeps its
receiver and documentation; a helper without `self` (`config_files`) takes
`&self` in `Host`. Level services stay on the level hosts above.
For the per-method comparison with NeoForge's block, item, entity, block-entity and fluid extension interfaces,
including which gaps are tracked, see [NeoForge extension coverage](../../reference/neoforge-extensions.md).
For item registry facts, `host.item_crafting_remainder(&stack)` returns a count-one
plain remainder stack, or `None` when there is none or the item is unknown. Horizon
currently stores only the remainder item id, not a template's count or components.
`host.block_item(stone)` returns its item registry name; an unavailable block mapping
returns `None`, while a known block with no item returns `Some("minecraft:air")`.
Both reads are available on either side.

For engine contributors, every area method declares its load access once with `#[access(...)]`.
`Metadata` admits registry facts during setup and reload; `WorldOnly`, `ClientSetup` and
`PublishedResources` retain their phase-specific contracts. The area's `host_area` attribute
builds the test-double defaults, and the API generator emits every game forwarder at build time.
Implement only the inherent `Game` function with the same name, not a hand-written forwarder.
Static implementations use `#[static_call]`; service accessors use `#[service]`.
A helper's `receiver` also exposes it on that handle. Receivers come from public
`*Ref` structs in `mod_api.rs`: `BlockEntityRef` gives `block_entity`, for example.
Adding a handle requires no separate generator registration.
The following documented `Game` helpers are not in the trait yet:

- `spawn_mob`, `mob`, `mobs`, `block_in_tag`, `item_in_tag`,
  `server_block_in_tag`, `server_item_in_tag`, `server_block`,
  `set_display`, `sidebar`, `experience_orbs`,
  `item_entities`, `player`, `panel_recipes`, `recipes`, `component`,
  `set_component`, `remove_component`, `spawn_falling_block`, `explode`,
  `container_slots` uses gameplay-private types.
- `with_mod_block_entity` is generic, so its signature cannot be called through `dyn Host`.

Experience-orb values use vanilla's signed `i32` representation in
`ExperienceOrb`, `ServerHost::set_experience_orb`, and the pickup and merge hooks.
Reading or changing a value preserves zero and negative saved values rather than
clamping them through an unsigned representation. A negative value does not grant
positive experience when picked up.

Advancement triggers instead live on `PlayerRef::trigger_criterion`, with a mod-owned
`CriterionTest` and the game's scoped `CriterionContext` resource. The context keeps real
world-aware item predicates; it is not a conditions-only snapshot. See
[advancement triggers](../../reference/hook-notes.md#advancement-triggers) for the contract and
the wasm runtime's current same-group callback limitation.

The following catalog hooks are not in the trait:

- `player_command`, `item_decorations`,
  `client_resources_reloaded`, `camera_shake`, `screen_key_pressed_pre`, `screen_char_typed_pre`,
  `screen_mouse_scrolled_pre`, `use_duration`, `use_animation`, `can_eat`,
  `living_entity_use_item_start`, `living_entity_use_item_finish`, `render_hand`, `shader_params`
  and `block_highlight`.
- `biome_colors`, `camera_distance`, `compute_fog_color`, `compute_fov`,
  `data_pack_files`, `mob_submit`, `mouse_button_pre`, `mouse_button_post`, `player_respawned`
  and `tags_updated`.

The client `render_tooltip_texture` hook (NeoForge's `RenderTooltipEvent.Texture`) picks a tooltip box's look. Setting `texture` to a tooltip style `<namespace>:<name>` draws the GUI sprites `<namespace>:tooltip/<name>_background` and `_frame` from the resource packs, nine-sliced by their `gui.scaling` metadata or stretched without it, as vanilla draws an item's `tooltip_style`; the atlas a resource reload restitches is the one drawn. Left `None`, the box keeps Horizon's solid fills, whose background and top and bottom border gradient colours it changes as mutable ARGB `u32` values; it does not change text colours. It covers solid-fill item tooltips (including creative, recipe book, chat, trading and bundle-selected item names), screen text boxes, and native panel tooltips. The stack is `None` for text-only boxes, including recipe-book item lines where the stack is discarded before drawing. Sprite-based screen tooltips (including options and title screens) use a separate renderer and are not covered.
