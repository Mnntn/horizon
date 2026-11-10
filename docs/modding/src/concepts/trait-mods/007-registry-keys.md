<!-- chapter: Registry keys -->
<!-- separator: 1 -->

On `r: &mut dyn Registrar`, `r.block_key(id)` and `r.effect_key(id)` request registry entries,
not new content. `Registrar` is the generated union of the kind-local registration traits
(such as `BlocksRegistrar` and `EffectsRegistrar`); mod registration hooks and the
[horizon-machine-sample](https://github.com/arismoko/horizon-machine-sample) continue to
use `dyn Registrar`. Each kind owns its declarations and implementation in `parts/<kind>.rs`;
the union and WebAssembly bindings discover those traits without a shared method list.
The capability-provider lookup union is assembled the same way. Declare them in `register`, retain the returned `BlockKey` or `EffectKey` in the
mod or Part, and reuse them for registry operations. Requests are canonicalized (a bare id
uses `minecraft:`), deduplicated across mods and checked after every mod's content is known.
An unknown entry refuses the load with the requesting mod and canonical id, even if the
key was never attached to a Part. A later mod may supply the requested entry.

The completed block identity table includes every vanilla and mod block, even blocks no mod
requested. Completion preserves requested slots and appends the remaining canonical names in
sorted order. Names belong to the installed mod registry; world load and reload prepare a
separate numeric mapping for every retained key before publishing it. A missing requested
block refuses completion without publishing a partial table. Immutable block definitions
(property names, legal values, state combinations and defaults) are published with those
names before mesh tables are constructed. Workers share read-only definitions; property
reads allocate nothing. Pack reload does not mutate definitions or identities. The world's
numeric mappings and tag membership remain separate reloadable data.

Metadata contexts are read-only and borrow the same table on the host and on workers.
Wire transport records are internal, not saved block values. Persist canonical identifiers
and property strings rather than slots or state ordinals.

`BlockState` is a small owned value with private fields, not a tint-specific snapshot or an
engine numeric ID. `state.block()` reads its `BlockKey`; `state.ordinal()` reads its local
ordinal. Property operations take explicit metadata: retain `let metadata = host.block_states()`
and call `state.property(&metadata, "facing")` or `state.with(&metadata, "facing", "east")`.
`BlockState::of(&metadata, key, constraints)` starts from the published default and applies the
supplied `StateConstraint`s. Changes return a new value and never mutate the original.
Unknown keys, ordinals and property combinations return typed `BlockStateError`s.

`SceneBlock::new(pos, state, entity)` and `StructureTemplateBlock::new(pos, state, nbt)`
use this owned `BlockState`, including all properties. Structure reads preserve vanilla palette
selection, mirror-then-rotation and placement order; an unavailable state mapping refuses the
whole snapshot with `None`. `DrawContext::draw_scene` checks the entire scene before drawing:
an unknown key, invalid ordinal or unavailable mapping draws nothing, never a partial scene or
a replacement air block. Native and wasm use the same values and refusal rules.

Tint callbacks receive the same immutable metadata resource explicitly, without a `Host`.
The game prepares their owned state array before constructing mesh tables; renderer workers
only copy those values and invoke metadata reads. When world numeric state IDs are reordered,
the game resolves each state's property combination against the published definition, so
local ordinals retain their meaning across reloads.

`host.block_states().states_of(key)` returns a `BlockStateRange` in that same immutable
local ordinal order. Its iterator yields owned `BlockState`s; it is not a span of world
numeric IDs. `BlockStateRange::new(first, count)` returns a typed error if the span would
exceed the u16 ordinal domain, and decoded transport spans obey the same check. Membership
checks both the block key and the ordinal. A zero-count span is empty.

A restricted `LevelHost::block_state(pos)` read returns `Option<BlockState>`. A successful
engine read retains its vanilla result: unloaded or out-of-height reads that produce air return
`Some(air)`, using the owning registry's real air state. An unavailable owning state index or a
refused engine-to-API conversion returns `None`; the boundary never fabricates a key or air state
and never panics to satisfy a mandatory value. Other optional/result read contracts are unchanged;
in particular, this does not change `LevelRef`'s loaded-position policy.

`LevelRef::set_block`, `update_neighbors` and `neighbor_changed` take owned block states
and return `Result<bool, LevelWriteError>`. `Stale` identifies a closed or reopened level;
`Unavailable` identifies the wrong side, phase or nonactive write target; `InvalidState`
retains the block-state refusal. Errors do not mutate the level. `Ok(false)` is still
vanilla refusal, such as an unloaded or out-of-height position or an unchanged block,
not a lost handle or a guessed replacement state. Update flags retain their vanilla effect.

Walking a whole registry uses the typed keys, never names: `host.block_keys()` and
`host.item_keys()` return every entry in the registry's registration order (vanilla's
entries first, then mods' in the order they registered); `host.entity_type_keys()`,
`host.menu_keys()` do the same for the built-in entity-type and menu registries.
`host.enchantment_keys()` uses global sorted loaded-registry order, with vanilla and mod
data-pack entries sorted together. A block registry lists a name once
per state, and its iteration lists the block once. Build lookup tables from these lists at
load time instead of resolving names per tick. A table is empty before its registry loads
(blocks and items during rest load, enchantments with the item tables); registration-time
`RegistrationContext` does not see any of them (#1001).

Keys are small, private `u32` registration slots on both native and wasm. `key.id(host)`
returns an owned canonical `String` through the generated read backend for diagnostics; it
requires a game-backed host and an invalid slot is a boundary error, not an empty name.
Block defaults come from immutable API metadata and cross through the game's sole forward/reverse
state index. Behavior queries obtain their engine block from the resulting engine state; there is
no separate per-key numeric block binding. Reload preserves key identity and canonical names.
A refused resource rebind clears the active index, while failed transactional Parts preparation
leaves the previously published Parts and index intact. Effect registration is process-owned and fixed
until restart, matching the engine's effect registry.

A name that only arrives at run time (data, config, a command, a network message) turns into a key
with the host: `host.block_key("minecraft:stone")`, `host.effect_key("mymod:rage")` and
`host.fluid_key("minecraft:water")` return `Option`, and `key.id(host)` gives the name back for
blocks, effects, fluids and items. Names are full registry names; the lookup reads the completed
registry and does not allocate, so a hook can call it. It returns `None` for a name the registry does
not hold, and before the registry is complete, so a mod uses `r.block_key(id)` in `register`. These
work on the client and the server, for native and wasm mods alike.

Block tag membership is bound from the same merged item/block tag snapshot at world load and
reload, including worlds without pack overrides. A read checks that the owning state index is
available, looks up the tag name and tests membership in a prepared API `BlockKey` set; it never
resolves the block's name. A fully namespaced tag needs no allocation (a bare tag adds `minecraft:`).
Binding visits each merged block-tag member once and resolves its API key; it replaces
the previous snapshot rather than accumulating membership. No membership work is added to ticks
that do not call this read. Tags and block mappings belong to their resolving thread; missing
authoritative context or an unavailable mapping returns `None`, not a stale membership result.

`host.tags().blocks(tag)` reads the current authoritative block members in vanilla order as
`BlockKey` values, including blocks no mod requested. Resolve `tag` once with `Tags::key` and
retain it across reloads. Unknown or empty tags return `Some(Vec::new())`; an unavailable
context or any unresolved member returns `None`, never a partial member list. The same service
is available through restricted level Hosts, without borrowing an engine registry.

`host.block_state_default(stone)`, `host.block_states().states_of(stone)`,
`host.block_item(stone)`, `host.block_in_tag(stone, tag)` and restricted-level
`schedule_block_tick(pos, stone, delay)` use `BlockKey`. Property expressions accepted by
`block_state_id`, tag names and existing fluid operation arguments remain strings for now.

Declare a tank-only fluid in your own namespace with `r.declare_fluid("chocolate")`; the returned `FluidKey` identifies it for the mod load. Request vanilla or another mod's declared fluid with `r.fluid_key("minecraft:water")` or `r.fluid_key("other:chocolate")`. Requests resolve after all registrations, so declaration order does not matter; an unknown request stops the load and names the requesting mod. Tank-only fluids have no world simulation or world state. Fluid stacks and recipe ingredients use these keys; remaining fluid operations move to them in subsequent changes.
Declare a fluid that exists in the world with `let mut fluid = WorldFluid::default(); fluid.drop_off = 2; r.declare_world_fluid("oil", fluid);`,
using `mod_api::WorldFluid`, the code form of a fluid file's `world` object ([content mods](../../how-to/content-mods.md)): it adds the block
`mymod:oil`, the flowing fluid `mymod:flowing_oil` and the bucket `mymod:oil_bucket`, and its fluid file, if it has one,
still says how it looks. `r.fluid_part("mymod:oil", Part)` attaches a `FluidPart`, whose `fluid_rules` can change the
declared values by position on the server. It is asked for every cell a spread looks at, so answer fast.

A saved tank whose fluid's mod is absent carries an opaque unresolved identity. Its stack's
`key()` is `None`; `UnresolvedFluid::id(host)` reads its saved name. It can move between
name-backed mod tanks, but not into vanilla buckets or cauldrons.

`ActiveEffect::id` and effect hook payloads carry `EffectKey`; living effect queries,
application and removal use the same resolved key. `ActiveEffect::new(speed, …)`
uses a key requested through `r.effect_key("minecraft:speed")`. Beacon-selection hooks
carry `Option<EffectKey>` for each power; `None` is no selection. Enchantment effect-component
names (for example `minecraft:damage` in `has_enchantment_effect`) are a different registry
and remain strings, as do labels and tags.

### Hot item and block identities

Resolve item and block keys during load and keep them for tick and menu calls.
`item_burn_time`, `item_compost_chance`, `give`, `enchantment_supports`'s item,
and GUI `item` / `item_model` take `ItemKey`. `block_item`, `fluid_bucket` and
`screen_hovered_item` return item keys. `level.block`, `lit_block_name` and
`Hit.block` return block keys; `BlockEntityDrawContext::block` takes one.
`HeldItemSlot.id` (including use-start/stop events) and `Stack.item`
carry item keys, not names. Unknown incoming slots are refused at the boundary.
`block_sprites` takes API-owned `BlockState` values, preserving properties without
parsing state strings or exposing numeric engine state IDs. The pre-change block event carries resolved `from` / `to`
states and does not allocate their names. Property names and values in
`set_block_property` are not block identifiers and remain strings.

These calls remove string transport across wasm. Engine metadata reads may still
hash borrowed registry names; no formatted identifier is built for these inputs.

### Matched recipe lookup

`level.recipe_for(host, &recipe_type, &inputs)` replaces the crafting-only lookup.
Use `RecipeInput::Crafting { width, height, slots }` for `minecraft:crafting`
(row-major slots, dimensions 1 through 3), `Single(stack)` for smelting, blasting,
smoking, campfire cooking and stonecutting, or `Smithing { template, base, addition }`
for `minecraft:smithing`. Recipe **types** are not serializers: shaped and shapeless
files both belong to `minecraft:crafting`, and transform/trim both belong to smithing.

The server uses the same engine matchers and assembled outputs as its menus,
including data-pack overrides of vanilla recipe types. A `RecipeMatch` contains the
matched recipe identifier, typed output, ordered remaining items, experience and
processing time. Lookup consumes nothing; the caller applies inventory changes
transactionally. Keep the result until inputs or the recipe load change.

`Ok(None)` means valid input without a match. Invalid shapes/items and unavailable
levels are typed errors. A type without an engine matcher returns
`RecipeLookupError::NotMatchable`, not an empty match. For mod-owned types, retain
`recipes_of_type` once per load/reload and match that cached data **inside the mod**,
with zero crossings per tick. Dynamic custom outputs and contextual predicates
remain mod-owned; the host does not call guest code to match recipes.

At publication, call `ingredient.item_keys(host)` once per ingredient to retain
`Arc<[ItemKey]>` for local matching. This load-time read returns exactly the plain
item membership for lists, expanded tags and standard shapes, using the engine
matcher. It returns `IngredientKeysError::NotEnumerable` if any custom or component
predicate occurs anywhere in the tree; a displayed item list is not a safe substitute.
`UnknownItem` refuses a missing registry identity. Do not call this method per tick.
The sample's `ProcessingRecipes::publish` retains these keys; `result` then checks
`input.item()` locally, with no `Ingredient::test` host call per candidate.

Stack placement predicates and drawing consumers (HUD items, GUI item decorations,
block-entity items and tooltip images) encode API stacks without enforcing resident
count or damage maxima. Engine-legal over-limit stacks can still be inspected and drawn.
Unknown item keys and invalid/unrepresentable effect payloads still refuse encoding;
this is not a total conversion. Insertion and other world writes retain strict validation.

### API-owned values: migration table

| Previous engine value | API-owned value | Mod changes |
| --- | --- | --- |
| Engine stack in `TooltipDisplay::for_stack` | `mod_api::ItemStack` | Pass the owned API stack supplied to the tooltip event. Its effective tooltip component includes vanilla's implicit defaults. Normal game rendering borrows the engine component without creating an API stack. |
| Engine loot context and candidate stack | `mod_api::{LootContext, LootStack}` | Contexts own their facts; candidate items are `ItemKey`s. Counts remain signed. Ordered `Option<Tag>` patches preserve removals and NBT types, and names are API `Text` trees. Writeback is atomic. |
| Engine `ParticleType` and options | `ParticleKey` and API `ParticleOptions` | Retain a key requested through `Registrar::particle_key`. Every parameter-family constructor takes its key; `simple(key)` also replaces string-based custom provider construction. The engine enum is no longer exported. |
| Engine damage containers | `mod_api::{DamageContainer, DamageValues, Reduction}` | Hooks receive an owned scalar snapshot. Read reductions with `reduction`; edit damage and cooldown through the container setters. Sequencing methods remain game-only. |
| Engine item property values | `mod_api::{ItemBar, ItemRarity}` | Resource methods retain their signatures; damageability, glint and repair checks use game-owned metadata reads. |
| Package manifest `Side` | `mod_api::Side` | Match `Client` and `Both` on the package metadata snapshot. This is not `HostSide` or the executing logical side. |
| Command `Suggestion` | `mod_api::Suggestion` | Construct with `Suggestion::new((start, end), text)`; optional `tooltip` and `int_value` remain writable. Offsets are byte offsets. |
| Package block and item declarations | `mod_api::{Block, Item}` | Use API-owned declaration records in the registration events. Component patches are ordered `(String, Option<Tag>)` entries, with `None` removing the component; the game converts once at load and applies content-file admission checks. |
| Package block entity type storage | `mod_api::BlockEntityTypeDecl` | Existing `r.block_entity_type(...).build()` registration is unchanged. The declaration snapshot no longer exposes the package parser's model; the game converts it once when declarations freeze. |
| Component protocol vocabulary | `text::{TextContent, TextNbtSource, TextObject, TextClick, TextHover}` | Match these enums exhaustively: their serialized vocabulary is closed. Records remain `non_exhaustive`. |
| Renderer GUI commands | `mod_api::GuiDraw` | `DrawContext::gui_paint` returns a typed text-conversion result; handle refusal before drawing. HUD hooks reject invalid writeback as a whole. |
| File-opening click actions | Not in `text::TextClick` | Mods can neither produce nor round-trip `OpenFile`; untrusted file-opening action metadata is dropped. Core tracks local/remote origin per node, including mixed trees; exporting JSON or `TextForm` and decoding it makes every node remote. Adding API `OpenFile` remains a separate vocabulary/link-contract change. |

`mod_api::GameRuleValue` owns the `Bool(bool)` and `Int(i32)` cases, in that order.
`level.game_rule(host, id)` returns `Result<Option<GameRuleValue>, LevelReadError>`:
`Ok(None)` means unknown rule, `Stale` means a gone/reopened level, and `Unavailable`
means the side or phase cannot read it. `level.game_rules(host)` returns the typed
result containing every available rule, not an optional list. The existing boolean
key getter is unchanged. The game converts values after the subscription guard for
`game_rule_changed`; no subscriber means no conversion work.

The API's `GameRuleDecl` records a mod's boolean rule name, category and default.
Its internal `RuleSchema` input describes vanilla categories and dense rule positions
through read-only lookups rather than a copied vanilla list or fixed slot numbers.
Schema validation resolves category references after all mods have declared them and
returns either all declaration slots or every refusal; it never installs a partial table.
The game supplies this schema at Parts installation, then builds each world's engine
rule registry from the installed `GameRuleDecl` snapshots. Existing rule registrar calls
are unchanged; engine registry construction is no longer exposed through the API.

The game owns package and completion conversion. Mods do not import package-loader or
client-core types for these values. Completion edits are copied back after subscribed
listeners run; no listener means no snapshot allocation. Package metadata is copied once
at startup, retaining declared namespaces and dependencies.

### Opaque NBT metadata

The shared JSON/NBT conversion is iterative and follows vanilla's 512 nested
list/compound limit; scalars and NBT typed arrays add no recursive container.
Conversion to JSON intentionally loses numeric tag widths, boolean-as-byte
identity and typed-array identity. `NonFinite` refuses NaN and infinities:
`serde_json::Value` cannot hold them, and Gson's strict JSON writer also refuses
them. `InvalidString` refuses an unpaired UTF-16 surrogate in a Tag string or
compound key. Neither case is replaced with null, a string or U+FFFD.
JSON input already uses UTF-8, so valid non-BMP pairs survive; a lone surrogate
cannot reach this converter through that input representation.

### Owned component trees

The `text` module defines owned component data: literal and translatable contents,
style overrides, ordered children, selector and NBT sources, inline objects, and click
and hover actions. Translation keys and argument components stay unresolved until the
game displays them. `None` style fields inherit; `Some(false)` overrides an inherited
decoration. Metadata, dialog/custom click payloads and hover-item component patches are
owned `Tag` data, never JSON strings or engine state. `TextProfile` carries an optional
name and UUID, signed property records and opaque unknown profile/property fields.
Signed record order and unsigned values within each property name are preserved.
Player names use U+0021..U+007E and at most 16 characters. Vanilla's unsigned
property-map form allows at most 16 distinct names; signed or metadata-bearing
records use a 16-entry list, with UTF-16 limits of 64, 32767 and 1024 units for
property name, value and signature respectively.
Inline player and atlas objects use `Option<Box<Text>>` for their fallback: `None`
keeps the engine-computed default; `Some` preserves a styled or translatable component
rather than flattening it to a string.

The native boundary plans borrowed source topology before cloning payloads. It reads
source-less spans directly and validates every Tag's JSON representation before building
an engine component. `Text::validate()` checks tree/payload bounds and profile fields
without cloning; `validate_depth()` is the component-only preflight. NaN/infinity and
unpaired UTF-16 remain legal Tag data but cause typed `NonFinite` or `InvalidString`
refusals when engine emission requires JSON. Nested JSON null is refused as `NestedNull`
when capturing opaque NBT fields; it is never silently replaced.
Host methods accepting or returning Text return `Result<_, TextFormError>`; mixed
item/Text writes preserve the refusal in `StackWriteError::Text`, and unspawned-mob
reads use `UnspawnedMobError::Text`. `TooDeep` means a component path or opaque NBT
container path exceeds 512. Unavailable backends are distinct from invalid text or
unrepresentable payloads.
A refused hook capture does not dispatch; a refused writeback leaves every field unchanged.
No subscriber means no conversion or snapshot allocation.
Mutable stack arguments are compared using value equality with the snapshot passed to
the listener. An unchanged argument retains the original engine value without strict
write validation (including engine-legal stacks above effective count or damage maxima).
A changed argument is validated in full; a refusal still commits no outputs. The same
rule applies to anvil/grindstone records and menu cursor/quick-move stacks.

Recursive `Clone`, `Eq` and `Drop` of a tree a mod itself built are ordinary Rust ownership,
not part of the boundary guarantee.

`Text::new(content)` constructs any content kind without exposing an engine value.
The content, NBT-source, object, click and hover enums are exhaustive protocol enums;
new cases change the link contract, just as enum-case order does. Mod matches should
name every case rather than silently treating a new protocol case as an old one.
Typed content and style fields are authoritative; metadata retains unrecognized
properties, not overrides of those typed fields. Game-side encoding is fallible
and reuses the engine's translation-argument and fallback construction rules.
Capture keeps dynamic contents and metadata rather than flattening the rendered spans.
It shares the engine's SNBT flag, shadow-colour, click-policy and inline-object parsers.
File-opening metadata is discarded before a listener receives the owned value; a
no-op listener cannot promote an untrusted file action into a local action.
A translatable's fallback is a literal format template, not another language key.
A missing key whose fallback equals an existing key therefore still displays the
fallback text itself; parsing and native constructors follow the same rule.

### Particle identities

`ParticleKey` is an opaque slot in the load-owned registry tables. One completed
snapshot retains both canonical names and provider identities: a vanilla provider's
numeric index or a custom named sprite provider. The load resolves each provider once;
reads of a retained key do not parse identifiers or scan the provider list. Unknown
forward references refuse publication of the entire particle snapshot. Name-table
completion shares the item registry's requested-slot preservation and sorted enumeration.

### Placement and loot snapshots

`mod_api::PlacementContext` owns the placement inputs, including world-space click
coordinates and the placer view/flags. Its `item` is `Option<ItemKey>`: `None` is a
real empty hand, including falling-block placement contexts, not an invented air
item. Nonempty items resolve through the existing load-owned item table. The game
captures this read-only snapshot after the scoped subscription guard; no subscriber
means no snapshot or item lookup. The block-state answer is applied as before.

The `snapshots` module owns placement inputs and loot-roll facts. Placement and tool
items are resolved `ItemKey`s. Loot context entities, damage sources, tools, biome,
block and place facts are owned values; retaining one never retains a borrowed engine
context or a live level. A loot candidate is not an inventory `ItemStack`: its signed
count can be non-positive while loot functions run, in which case it produces no item.
Exploration searches remain deferred data, and name components remain unresolved text.

### Item values and tag names

Every item has an `ItemKey`, including vanilla items no mod requested during registration.
`r.item_key(id)` retains a forward reference in the same load-owned table; after the item
registry is complete, `host.item_key(&identifier)` resolves any registered item. A key is
still a private `u32` index, not a copy of an item definition. Resolve once and retain it.
`host.max_stack_size(key)` and `host.max_damage(key)` read item defaults without exposing
the engine's item table.

`ItemStack` is owned API data with private fields. `ItemStack::new(key, count)` refuses
zero, while `ItemStack::of(key)` makes one item. Count mutation refuses zero and arithmetic
overflow. Registry-dependent maximum count and damage, including component overrides,
are checked by the game when accepting a write; an oversized value is refused with a typed
error, never silently clamped. This is the same boundary on native and wasm. Consuming
`split(n)` returns `StackSplit { taken, remainder }`, both optional; consuming `shrink(n)`
returns the remainder or `None`. There is no empty-stack sentinel. Component descriptors
read and write an optional owned NBT patch; a plain stack has no patch allocation.

`host.tags()` and a restricted level host's `tags()` expose the same `Tags` service. Resolve
a validated identifier once with `tags.key(&identifier)` and retain its `TagKey`. The runtime
owns tag-name slots for the process; reload replaces membership, not names or key identity.
`tags.is_in_tag(item_key, tag_key)` tests merged item membership. `tags.items(tag_key)`
and `tags.fluids(tag_key)` return registered keys in vanilla's member order. Unknown tags
return false or empty lists. Membership reads return `None` on clients, before authoritative
tags load, or if the relevant registry mapping is unavailable. Key resolution itself can be
refused if the runtime cannot allocate a name slot. No lookup stringifies a retained key.
Block member enumeration moves with the registry-wide `BlockKey` table in the next type slice.
The engine `TagRegistry`, `StackData`, `StackView` and `DisplayContext` are not API types.

A hook whose catalog entry has a `scope` (`block`, `block_entity`, `item`, `menu`) and a `key`, the argument
naming the thing it is about, is a method on that scope's trait only: `BlockPart`, `BlockEntityPart` (a
`BlockPart` too), `ItemPart` and `MenuPart`, as vanilla's and NeoForge's `Block` and `Item` methods are. It has
no `Hooks` method: a thing without parts is never asked, and the game asks the parts attached to it in dispatch
order, through the hook's contract, so a `Cancel` or a replacement skips the parts after it. Where a NeoForge
bus event is the counterpart for that moment, the event is a hook of its own on `Hooks`, asked first for every thing, and its cancel
skips the parts: a mod reacting to its own blocks uses a part, one reacting to every block the event. A part's kind is its type: the same kind twice on one
thing, an invalid key declaration, or an unnamespaced id in the other Part registrations stops
the game at startup with a message naming the mods. A panic in a part
adds `, part <type>` to the mod and hook it names. Parts take `&self` and are `Send + Sync`, like `Hooks`.

### Damage and item metadata snapshots

The three damage hooks convert only after their subscription guard. `DamageValues`
retains the entire sequence, including reductions and the pre-hook input, so changing
`new_damage` still reaches the engine's original NaN and negative normalization point.
`DamageValues::set_reduction` edits snapshot data; it does not run a damage calculation.
The fixed reduction storage is private; use its accessors rather than an array index.
`Reduction` case order is append-only. Engine `record_*`, `enter_pre`, `capture_inflicted`
and `pre_change` are not mod API methods.

`ItemResource::damageable`, `foil` and `repairable` delegate to the game's existing
property readers through metadata access. Defaults, explicit overrides, component
removals, compass glint and repair tags follow gameplay's single property path.

### Recipe-file fields and small values

`Host::recipe_ingredient` and `Host::recipe_result` take the field's JSON **text**,
not a `serde_json::Value`. Pass a quoted item id (`r#""iron_ingot""#`), a quoted
`#tag`, or an ingredient array. Parse failures return `RecipeJsonError::InvalidJson`;
unknown items and empty tags still return `Ok(None)`. Result objects retain their
`components` patch. These helpers are reload-time reads, not per-tick JSON parsers.

| Former engine value | API-owned value |
| --- | --- |
| `mod_list::KeyConflictContext` | `mod_api::KeyConflictContext` |
| `firework::FireworkShape` | `mod_api::FireworkShape` |
| `manifest::JarInputsError` | `mod_api::JarInputsError`, with owned diagnostics and original-jar identity |
| Recipe helper `&serde_json::Value` | JSON `&str`, returning `Result<Option<_>, RecipeJsonError>` |
