# NeoForge extension methods and Horizon

This page compares the public methods of NeoForge 26.3's object-extension interfaces with what a Horizon mod can
answer today. It is the reference the Parts lane starts from: before proposing a new block or item hook, check
whether content data, a handle or an existing Part already answers the same question.

Read from NeoForge `26.3.0.101` (`neoforge-26.3.0.101-universal.jar`, `net.neoforged.neoforge.common.extensions`)
with `javap -p`, and from Horizon at commit `061a06c7b`. Method lists and parameter names are factual citations; no
NeoForge source or Javadoc is copied. Counts exclude each interface's private `self()` accessor.

## How to read the verdicts

| Verdict | Meaning |
| --- | --- |
| **Covered** | A mod can answer the same question for its own object, at the same moment, through content data, a handle or a Part method. |
| **Partial** | The game lets a mod answer something close, but one of these differs: the object (a global hook instead of a per-block/item method), the moment, the arguments, or the answer (a factor or notice where NeoForge returns a decision). |
| **Missing** | Neither data, a handle nor a hook exposes the question. |
| **n/a** | The method has no Horizon equivalent because the architecture removes its reason to exist. |

A **Part method** is a generated method on `BlockPart`, `ItemPart`, `BlockEntityPart`, `MenuPart` or `EntityPart`,
declared with `scope` in `crates/horizon-mod-vocab/src/hooks/<area>.rs`. A Part method runs only for the objects a
mod attached its part to. An **unscoped hook** in the same files is global: every mod hears it and filters by the
arguments itself. **Data** means a content declaration in `crates/horizon-mod-package/src/content.rs`, read into
the game through `crates/horizon-items/src/blocks.rs`. **Handle** means a value or reference type in
`crates/horizon-mod-vocab/src/` (`ItemStack`, `ItemResource`, `AttachmentStore`).

`EntityPart` exists today but has two methods, `register_goals` and `spawn_cluster_size` (`docs/modding/src/concepts/trait-mods/023-entity-parts-and-mob-goals.md`).
It does not answer `IEntityExtension` methods; those are covered only where a global hook or a handle does.

## Counts

| Interface | Public methods | Covered | Partial | Missing | n/a |
| --- | ---: | ---: | ---: | ---: | ---: |
| `IBlockExtension` | 60 | 6 | 21 | 33 | 0 |
| `IBlockStateExtension` | 57 | 5 | 20 | 32 | 0 |
| `IItemExtension` | 46 | 9 | 15 | 22 | 0 |
| `IEntityExtension` | 33 | 1 | 16 | 16 | 0 |
| `ILivingEntityExtension` | 6 | 0 | 3 | 3 | 0 |
| `IBlockEntityExtension` | 9 | 1 | 3 | 4 | 1 |
| `IFluidExtension` | 9 | 0 | 6 | 3 | 0 |
| `IMenuTypeExtension` | 2 | 0 | 2 | 0 | 0 |

Of the 60 block methods, 57 exist a second time with the state as receiver, so the state-extension counts are
the block counts minus three; see [`IBlockStateExtension`](#iblockstateextension). The block follow-up is
[#1038](https://github.com/arismoko/horizon/issues/1038) and the item follow-up is
[#1039](https://github.com/arismoko/horizon/issues/1039). Their groups were written from the earlier audit and
do not name every Missing row yet. Both predate hooks that have since landed, so some of their rows are narrower
or already answered; the last section lists what changed.

The earlier audit's counts included `self()` and folded the four `isInFluidType` overloads into one; this page
counts each declared public method, so the entity total is 33 rather than 30.

## `IBlockExtension`

Data fields are static values from a block's content declaration, inherited from its template when unset. They
answer the method for a mod block but cannot vary by level, position or entity. The engine's own copy of the
same facts is `StateFacts` in `crates/horizon-items/src/blocks.rs`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getFriction(state, level, pos, entity)` | Partial | `BlockProperties.friction` → `StateFacts.friction` | Static; the entity is not consulted. |
| `hasDynamicLightEmission(state)` | Missing | — | `StateFacts.emission` is a fixed level, not a dynamic emission. |
| `getLightEmission(state, level, pos)` | Partial | `BlockProperties.light` → `StateFacts.emission` | Static; no level or position. |
| `ignitedByLava(state, level, pos, direction)` | Partial | template state flag (`StateFacts`, `state_flags::IGNITED_BY_LAVA`) | Inherited from the template's prepared data; no content field or Part changes it. |
| `isLadder(state, level, pos, entity)` | Covered | `BlockPart::is_ladder` (`hooks/is_ladder.rs`) | Asked of the block a living entity stands in, with the entity id, after the `#climbable` tag: the part's answer replaces the tag's, either way. Players and mobs; armor stands never ask. |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Missing | — | Engine rule in the climbing code; not exposed. |
| `isBurning(state, level, pos)` | Missing | — | No per-block burning query; `block_burn` is fire removing a block. |
| `canHarvestBlock(state, level, pos, player)` | Covered | `BlockPart::can_harvest_block` (`hooks/can_harvest_block.rs`) | Asked where `player_harvest_check` fires, before it: the global hook still hears and may change the part's answer. |
| `onDestroyedByPlayer(state, level, pos, player, stack, willHarvest, fluid)` | Partial | `break_block` (`hooks/base.rs`), `block_drops` (`hooks/block_drops.rs`) | `break_block` cancels the break or replaces the XP sample; `block_drops` edits drops. Neither returns "the block was destroyed" for the caller. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `onDestroyedByPushReaction(state, level, pos, direction, fluid)` | Missing | — | No hook at the piston/fluid destruction point. |
| `getRespawnPosition(state, entityType, level, pos, angle)` | Partial | `player_respawn_position` (`hooks/base.rs`) | The global `PlayerRespawnPositionEvent` moves the player after vanilla found the bed or anchor; the per-block search is engine-owned. |
| `getExplosionResistance(state, level, pos, explosion)` | Partial | `BlockProperties.explosion_resistance` → `StateFacts.explosion_resistance` | Static; the explosion is not consulted. |
| `getCloneItemStack(level, pos, state, includeData, player)` | Covered | `BlockPart::clone_item_stack` (`hooks/clone_item_stack.rs`) | `include_data` and the player are passed; leaving the stack unset keeps the game's item. |
| `addLandingEffects(state, level, pos, stateBelow, entity, count)` | Missing | — | `fall_on` edits fall damage, not the landing particle/sound burst. |
| `addRunningEffects(state, level, pos, entity)` | Missing | — | No hook for the running dust burst. |
| `playFallSound(state, level, pos, entity)` | Partial | `BlockProperties.sounds` | Picks a vanilla sound type; no custom event, volume or pitch. |
| `playStepSound(state, level, pos, entity, volume, pitch)` | Partial | `BlockProperties.sounds` | Same static sound type; step volume and pitch are not settable. |
| `canSustainPlant(state, level, pos, direction, plantState)` | Covered | `BlockPart::can_sustain_plant` (`hooks/can_sustain_plant.rs`) | The soil's answer for the plant above it (allow, deny or none), asked before the plant's `may_place_on`, which still answers for your plant. The direction is always up. |
| `onTreeGrow(state, level, posConsumer, random, pos, feature)` | Missing | — | `block_grow_feature` observes or edits the feature a sapling grows into; it does not let a block run its own tree growth. |
| `isFertile(state, level, pos)` | Missing | — | No fertile-block query; bonemeal hooks fire later. |
| `getExpDrop(state, level, pos, blockEntity, breaker, tool)` | Partial | `break_block.exp_to_drop` (`hooks/base.rs`) | Replaces the XP sample for a player break; global, not per-block, and the block entity/tool/breaker are not arguments. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `rotate(state, level, pos, rotation)` | Missing | — | Structure rotation is engine behaviour; no callback. |
| `getEnchantPowerBonus(state, level, pos)` | Missing | — | Bookshelf enchanting power is a hard-coded block rule. |
| `onNeighborChange(state, level, pos, fromPos)` | Missing | — | `neighbor_changed` is `Block.neighborChanged`, a different callback. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `shouldCheckWeakPower(state, level, pos, direction)` | Covered | `BlockPart::should_check_weak_power` (`hooks/should_check_weak_power.rs`) | The argument direction is the input side, as NeoForge's. |
| `getWeakChanges(state, level, pos)` | Missing | — | No weak-change observer query. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getSoundType(state, level, pos, entity)` | Partial | `BlockProperties.sounds`, `host.block_states().sound(state)` | Static set of break/step/place/hit/fall sounds; no per-position or per-entity choice. |
| `getBeaconColorMultiplier(state, level, pos, beaconPos)` | Missing | — | Beacon beam colour is engine-owned. |
| `getStateAtViewpoint(state, level, pos, viewpoint)` | Missing | — | No viewpoint-state query. |
| `getBlockPathType(state, level, pos, mob)` | Partial | `BlockPart::is_pathfindable` (`hooks/is_pathfindable.rs`) | Answers whether a `PathComputationType` may pass, once per state and type; not a `PathType` per mob or position. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Missing | — | No adjacent-path-type query. |
| `isSlimeBlock(state)` | Missing | — | No per-state slime query; the piston/fall logic is engine-owned. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `isStickyBlock(state)` | Missing | — | Same. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `canStickTo(state, otherState)` | Missing | — | Same. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `isFlammable(state, level, pos, direction)` | Partial | `BlockPart::fire_odds` (`hooks/base.rs`) | Sets ignite and burn odds (`0, 0` = never burns), but there is no boolean flammable predicate. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getFireSpreadSpeed(state, level, pos, direction)` | Partial | `BlockPart::fire_odds` (`hooks/base.rs`) | Ignite/spread odds (the first element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table for the block. The answer is state-based, with no position or direction as in NeoForge's contextual override. |
| `getFlammability(state, level, pos, direction)` | Partial | `BlockPart::fire_odds` (`hooks/base.rs`) | Burn/destruction odds (the second element of `fire_odds`'s `(ignite, burn)` tuple) replace vanilla's table for the block. The answer is state-based, with no position or direction as in NeoForge's contextual override. |
| `onCaughtFire(state, level, pos, direction, igniter, stack)` | Missing | — | No per-block caught-fire callback. |
| `isFireSource(state, level, pos, direction)` | Missing | — | Fire-source rules are engine-owned. |
| `canEntityDestroy(state, level, pos, entity)` | Missing | — | Mob-grief checks are engine rules; `entity_mob_griefing` is a global event at a different moment. |
| `canDropFromExplosion(state, level, pos, explosion)` | Partial | `explosion_detonate` (`hooks/base.rs`) | Removes positions from the blast before blocks are hit; no per-block keep/drop query. |
| `onBlockExploded(state, level, pos, explosion)` | Partial | `explosion_detonate` (`hooks/base.rs`) | Same global pre-blast hook; no per-block callback after the block is removed. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `collisionExtendsVertically(state, level, pos, entity)` | Missing | — | Collision shapes are data; no per-entity extension query. |
| `shouldDisplayFluidOverlay(state, level, pos, fluid)` | Missing | — | Overlay drawing is engine-owned. |
| `getToolModifiedState(state, context, ability, simulate)` | Partial | `block_tool_modification` (`hooks/block_tool_modification.rs`) | Global `BlockEvent.BlockToolModificationEvent` cancels or replaces the final state; not a Part method and no `simulate` flag. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `isScaffolding(state, level, pos, entity)` | Missing | — | Scaffolding is identified by block class; no content field or Part. |
| `hidesNeighborFace(level, pos, state, neighborState, direction)` | Missing | — | `block_render_visibility` hides a whole state from the mesher, not one face. |
| `supportsExternalFaceHiding(state)` | Missing | — | No per-face culling query. |
| `onBlockStateChange(level, pos, oldState, newState)` | Partial | `block_changed` (`hooks/base.rs`, a global server hook) | Fires after the commit but before block side effects and neighbour updates, and only on an authoritative server. It may not modify the level. |
| `canBeHydrated(state, level, pos, fluid)` | Missing | — | Farmland hydration is engine-owned; fluid `canHydrate` is data. |
| `getMapColor(state, level, pos, defaultColor)` | Partial | `BlockProperties.map_color` → `StateFacts.map_color` | Static; no position. |
| `getAppearance(state, level, pos, direction, neighborState, neighborPos)` | Missing | — | No appearance query. |
| `getPistonPushReaction(state)` | Covered | `BlockProperties.push_reaction` | Static; `PUSH_PULL`, `PUSH`, `POPPED`, `IMMOVEABLE`, `IGNORE_ENTITY`. |
| `isEmpty(state)` | Missing | — | Air/replaceable facts exist (`block_states().replaceable`), but no `isEmpty` answer for a state. |
| `getBubbleColumnDirection(state)` | Missing | — | Bubble-column rules are engine-owned. |
| `shouldHideAdjacentFluidFace(state, direction, fluid)` | Missing | — | Fluid-face culling is engine-owned. |
| `getBounceRestitution(level, pos, state, entity)` | Partial | `BlockPart::bounce_on` (`hooks/block_landing.rs`) | Edits the entity's movement after it bounced; the restitution factor is not exposed. [#1038](https://github.com/arismoko/horizon/issues/1038). |
| `getRelocability(level, pos, state)` | Missing | — | No relocation query (pistons, structure blocks). |
| `canBeConnectedTo(state, direction, neighborBlock)` | Partial | `BlockPart::connects_to` (`hooks/connects_to.rs`) | Asks the connecting fence, wall, bars or pane (your block), not the block being connected to: a modded target cannot answer for a vanilla fence, and only those shapes ask at all. |
| `getModelData(state, level, pos)` | Missing | — | No `ModelData`; block-entity drawing takes a `block_entity_submit` part with its own model path. |

## `IBlockStateExtension`

This interface repeats the block questions with the state as receiver instead of the first argument. All 57
methods are the same behaviours and take the same verdict as the `IBlockExtension` row of the same name, with
the parameters shifted by one. Three block methods have no state-receiver form:

| Block-only method | Verdict there |
| --- | --- |
| `getPistonPushReaction(state)` | Covered (content `push_reaction`) |
| `getMapColor(state, level, pos, defaultColor)` | Partial (content `map_color`) |
| `makesOpenTrapdoorAboveClimbable(state, level, pos, stateAbove)` | Missing |

Counts: 5 covered, 20 partial, 32 missing among the 57. The handful of signature differences (`canStickTo` takes
the neighbour state; `getCloneItemStack` reorders its arguments; `getEnchantPowerBonus` takes a `LevelReader`)
do not change any verdict.

## `IItemExtension`

Item data is a 26.3 component patch over a vanilla template (`crates/horizon-mod-package/src/content.rs`),
which is the same place NeoForge's item components live. Reads are on `host` (`host/items.rs`) or on the owned
`ItemStack` / `ItemResource` values.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getDefaultAttributeModifiers(stack)` | Partial | `item_attribute_modifier` (`hooks/item_attribute_modifier.rs`) | Global `ItemAttributeModifierEvent` edits the stack's modifiers; the stack filters it, but it is not a Part method. |
| `onDroppedByPlayer(stack, player)` | Partial | `item_toss` (`hooks/item_toss.rs`) | Global `ItemTossEvent`, cancellable; the item is not a Part attachment. |
| `getHighlightTip(stack, name)` | Missing | — | No highlight-name hook. |
| `onItemUseFirst(stack, context)` | Covered | `ItemPart::on_item_use_first` (`hooks/on_item_use_first.rs`) | Runs after `use_item_on_block` let it through, before the block's own use. |
| `isPiglinCurrency(stack)` | Missing | — | Piglin barter currency is a hard-coded item tag. |
| `makesPiglinsNeutral(stack, entity)` | Missing | — | Piglin neutrality is a hard-coded item/tag rule. |
| `isCombineRepairable(stack)` | Partial | `minecraft:repairable` component via `ItemResource::repairable` | Data answers which material repairs it; the anvil combine predicate is not exposed. |
| `getXpRepairRatio(stack)` | Partial | `mending_repair` (`hooks/base.rs`, a global hook) | Changes the repair amount for Mending; the per-item ratio is not a separate read, and the hook is not scoped to the item. |
| `onStopUsing(stack, entity, timeLeft)` | Partial | `ItemPart::release_using` (`hooks/base.rs`) | Runs on an early release only, after the vanilla item's release; finishing the use or switching to another item stops it without asking. Both sides. |
| `onLeftClickEntity(stack, player, entity)` | Covered | `ItemPart::on_left_click_entity` (`hooks/item_attack.rs`) | Runs where `Player.attack` starts, after `attack_entity` let the blow through; a cancel leaves the target unhurt, with no `hurt_enemy` and no wear. |
| `getCraftingRemainder(instance)` | Covered | `ItemPart::crafting_remainder`, content `crafting_remainder` | The per-stack Part override replaces the data remainder in crafting, fuel and quick moves. |
| `getEntityLifespan(stack, level)` | Partial | `item_expire` (`hooks/base.rs`) | Adds extra ticks to a dropped item's life; no absolute lifespan and not per-item. |
| `hasCustomEntity(stack)` | Missing | — | No custom dropped-entity selection. |
| `createEntity(level, entity, stack)` | Missing | — | Same. |
| `onEntityItemUpdate(stack, itemEntity)` | Missing | — | No per-item dropped-entity tick. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `doesSneakBypassUse(stack, level, pos, player)` | Missing | — | No sneak-bypass query. |
| `canEquip(stack, slot, entity)` | Partial | `equippable.allowed_entities`, `entity_capability::goes_in_slot` | Component data decides slot and allowed entities; no per-stack code predicate. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `getEquipmentSlot(stack)` | Covered | `ItemResource::equippable_slot`, `host.item_equippable_slot` | Read from the effective `equippable` component. |
| `onEntitySwing(stack, entity, hand)` | Covered | `ItemPart::on_entity_swing` (`hooks/item_attack.rs`) | Runs where `LivingEntity.swing` starts, on both sides; a cancel starts no swing (no animation, no packet). |
| `getDamage(stack)` | Covered | `ItemStack::damage` | The stack's numeric damage override. |
| `getMaxDamage(stack)` | Partial | `minecraft:max_damage` effective component, `host.max_damage` | The stack's effective component includes a patch over the item default; `host.max_damage` reads that default only. |
| `isDamaged(stack)` | Covered | `ItemStack::damage`, `damage_component` | Distinguishes explicit zero from absent. |
| `setDamage(stack, damage)` | Partial | `ItemStack::set_damage` | Stores the requested value; the game checks it against the effective maximum when it accepts the write instead of clamping here. |
| `canPerformAction(instance, ability)` | Partial | `ItemPart::can_perform_action` (`hooks/can_perform_action.rs`), `block_tool_modification` (`hooks/block_tool_modification.rs`) | The part answers per stack and per namespaced ability id, starting from vanilla's answer, where vanilla checks the item kind: `minecraft:shield_block` (a raised item blocking attacks, its use time and animation), `minecraft:shears_harvest` (sheep, mooshroom, snow golem, bogged, bee nests, a dispenser's shears) and `minecraft:shears_carve` (a pumpkin). The block side of a tool action (strip, wax, path, ...) is `block_tool_modification`; no tool ability (`axe_strip`...) is asked of the item yet. [#1039](https://github.com/arismoko/horizon/issues/1039). |
| `getMaxStackSize(stack)` | Partial | `max_stack_size` component, `host.max_stack_size` | The stack's effective component includes a patch over the item default; `host.max_stack_size` reads that default only, and invalid values are refused at the write. |
| `isPrimaryItemFor(stack, enchantment)` | Partial | `host.enchantment_supports` | Answers supported items; the narrower "primary item" distinction is not exposed. |
| `supportsEnchantment(stack, enchantment)` | Partial | `host.enchantment_supports` | A client-side read of `Enchantment.isSupportedItem` for an item id; not per stack and without NeoForge's enchanted-book special case. |
| `getEnchantmentLevel(instance, enchantment)` | Covered | `get_enchantment_level` (`hooks/get_enchantment_level.rs`) | Edits the level gameplay sees. |
| `getAllEnchantments(stack, lookup)` | Covered | `get_enchantment_level` with no target | The hook's `target` is `None` for every-enchantment reads. |
| `shouldCauseReequipAnimation(stack, oldStack, equipped)` | Missing | — | Client animation rule; no hook. |
| `shouldCauseBlockBreakReset(oldStack, newStack)` | Missing | — | Client mining-reset rule; no hook. |
| `canContinueUsing(stack, otherStack)` | Missing | — | No continue-use query. |
| `getCreatorModId(registries, stack)` | Missing | — | Diagnostic ownership string; Horizon keeps the item's namespace in its key. |
| `onAnimalArmorTick(stack, level, mob)` | Missing | — | No per-item animal-armour tick. |
| `damageItem(stack, amount, entity, onBreak)` | Covered | `ItemPart::damage_item` (`hooks/damage_item.rs`) | Runs on the server for the parts of the worn item, after Unbreaking, on every wear path (a player's tools, armour and shield, a dispenser's tool, a mob's armour). The part gets the stack and the amount about to be lost, and changes it (0 or less leaves the item whole) or cancels the wear. It is not given the wearing entity (`host.players().acting_player()` names the player on a player's paths) and has no `onBreak` consumer: the break is `player_destroy_item`. |
| `onDestroyed(itemEntity, source)` | Missing | — | `item_despawn` is lifetime removal, not damage destruction. |
| `isGazeDisguise(stack, player, target)` | Missing | — | No gaze-disguise query. |
| `canWalkOnPowderedSnow(stack, entity)` | Missing | — | Powder-snow walking is engine-owned. |
| `isDamageable(stack)` | Covered | `ItemResource::damageable(host)` | Reads the effective components: `minecraft:max_damage` and `minecraft:damage` present, `minecraft:unbreakable` absent. |
| `getSweepHitBox(stack, player, target)` | Missing | — | `sweep_attack` decides whether a sweep happens, not its box. |
| `isNotReplaceableByPickAction(stack, player, slot)` | Missing | — | Pick-block replacement is engine-owned. |
| `canGrindstoneRepair(stack)` | Missing | — | `grindstone_place` observes or edits the result; no per-item predicate. |
| `canBeHurtBy(stack, source)` | Missing | — | No item damage-immunity query. |
| `applyEnchantments(stack, enchantments)` | Missing | — | No item-level enchantment application hook. |
| `canFitInsideContainerItems(stack)` | Partial | `MenuPart::menu_slot_rules` (`hooks/base.rs`) | Slot rules implement container-item restrictions (a shulker box in a slot); no per-stack override. |
| `onGlideDamage(stack, entity, slot)` | Missing | — | No per-item glide-damage callback. |

## `IEntityExtension`

There is no per-entity Part method yet (`EntityPart` only registers goals), so entity coverage is data, handles
and global hooks. On the server a mod can act on entities through `EntityRef` and `AttachmentStore`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `captureDrops()` | Missing | — | `living_drops` edits a dying mob's drops; it does not capture an arbitrary entity's drops. |
| `captureDrops(collection)` | Missing | — | Same. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::Entity` | Saved and synced attachments replace the writable compound; the data is typed, not raw NBT. |
| `shouldRiderSit()` | Missing | — | Rider pose is engine-owned. |
| `canRiderInteract()` | Missing | — | No rider-interaction query. |
| `canBeRiddenUnderFluidType(type, rider)` | Missing | — | `entity_mount` fires on mount/dismount, not on fluid riding. |
| `canTrample(level, state, pos, distance)` | Partial | `farmland_trample` (`hooks/farmland_trample.rs`) | Global `BlockEvent.FarmlandTrampleEvent`; per-entity override is not exposed. |
| `getClassification(includeVehicles)` | Partial | `host.entity_classification`, content entity type | A read of the template's category; a mod entity cannot set a different one per entity. |
| `isAddedToLevel()` | Partial | `entity_join_level` / `entity_leave_level`, `host.entity_added_to_level` | State is tracked by the game; the handles tell a mod after the fact. |
| `onAddedToLevel()` | Partial | `entity_join_level` (`hooks/base.rs`) | Fires after the entity is in the level's lists; cancellable, unlike the method. |
| `onRemovedFromLevel()` | Partial | `entity_leave_level` (`hooks/base.rs`) | Fires after removal and only listens. |
| `revive()` | Missing | — | No entity revive API. |
| `isMultipartEntity()` | Missing | — | The dragon's parts are internal to the engine. |
| `getParts()` | Missing | — | Same; `EntityRef` names dragon parts in some reads but does not expose the array. |
| `getFluidTypeHeight(type)` | Missing | — | Fluid properties are per-fluid data; no per-entity height. |
| `getMaxHeightFluidType()` | Missing | — | Same. |
| `isInFluidType(state)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | Only water and lava answered; no arbitrary fluid. |
| `isInFluidType(type)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | Same. |
| `isInFluidType(predicate)` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | No predicate form. |
| `isInFluidType()` | Partial | `host.entity_is_in_water`, `entity_is_in_lava` | "Any fluid" is water-or-lava only. |
| `getFirstEyeInFluidType()` | Missing | — | Eye position per fluid is engine-owned. |
| `isEyeInFluidType(type)` | Missing | — | Same. |
| `canStartSwimming()` | Missing | — | Swimming start is engine-owned. |
| `getFluidMotionScale(type)` | Missing | — | No per-entity fluid scale. |
| `isPushedByFluid(type)` | Partial | `FluidTypeProperties.can_push_entity` | Per-fluid data, not a per-entity answer. |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data. |
| `canFluidExtinguish(type)` | Partial | `FluidTypeProperties.can_extinguish` | Per-fluid data. |
| `getFluidFallDistanceModifier(type)` | Partial | `FluidTypeProperties.fall_distance_modifier` | Per-fluid data. |
| `canHydrateInFluidType(type)` | Partial | `FluidTypeProperties.can_hydrate` | Per-fluid data. |
| `getSoundFromFluidType(type, action)` | Missing | — | No fluid sound-action table. |
| `sendPairingData(player, consumer)` | Missing | — | No pairing-payload API. |
| `copyAttachmentsFrom(entity, wasDeath)` | Partial | attachments survive dimension change; `player_clone` (`hooks/base.rs`) | Copy rules are the attachment store's; there is no per-entity hook at the copy. |
| `getBlockBounciness(pos, state)` | Partial | `BlockPart::bounce_on` (`hooks/block_landing.rs`) | Edits post-bounce movement instead of returning a bounciness factor. |

## `ILivingEntityExtension`

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `canSwimInFluidType(type)` | Partial | `FluidTypeProperties.can_swim` | Per-fluid data, not per living entity. |
| `jumpInFluid(type)` | Missing | — | Fluid jump behaviour is engine-owned. |
| `sinkInFluid(type)` | Missing | — | Same. |
| `canDrownInFluidType(type)` | Partial | `living_drown` (`hooks/base.rs`), `FluidTypeProperties.can_drown` | The global drown event can cancel or change damage; fluid data decides whether it applies. |
| `moveInFluid(type, movement, fallDistance)` | Missing | — | Movement in fluid is engine-owned; no per-entity method. |
| `onDamageTaken(container)` | Partial | `living_damage_post` (`hooks/living_damage_post.rs`), `living_incoming_damage` (`hooks/base.rs`) | Global server hooks: the incoming container before the hit, and a post-damage notice only once health dropped, not a per-entity callback for every hurt. |

## `IBlockEntityExtension`

Block entity parts are attached with `r.block_entity_part`; `BlockEntityPart` extends `BlockPart` and carries the
block-entity hooks (`crates/horizon-mod-vocab/src/hooks/` with `scope = "block_entity"`). Saved data and attachments
are reachable through `BlockEntityRef`.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `onDataPacket(connection, input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | The server half (`getUpdateTag`) is settable; the client's merge of the received tag is engine-side. |
| `handleUpdateTag(input)` | Partial | `BlockEntityPart::block_entity_update_tag` (`hooks/base.rs`) | Same: the data is covered, the client callback is not. |
| `getPersistentData()` | Covered | `AttachmentStore`, `AttachmentHolder::BlockEntity` | Typed saved/synced attachments replace the writable compound. |
| `onChunkUnloaded()` | Missing | — | `chunk_unload` is the chunk event; there is no per-block-entity callback. |
| `onLoad()` | Partial | `BlockEntityPart::block_entity` (`hooks/base.rs`) | Initial saved data is set at creation, before the saved NBT is applied; no after-load notice. |
| `requestModelDataUpdate()` | Missing | — | No model-data invalidation API. |
| `getModelData()` | Missing | — | No `ModelData`; drawing uses `block_entity_submit` and the model path. |
| `invalidateCapabilities()` | n/a | capability providers are queried per lookup (`crates/horizon-mod-vocab/src/parts/block_entities.rs`) | There is no cached handler to invalidate. |
| `applyStructureRotation(mirror, rotation)` | Missing | — | Structure rotation is engine-owned; no callback. |

## `IFluidExtension`

Fluid type facts are one record, `FluidTypeProperties` (`crates/horizon-mod-vocab/src/block/fluid_states.rs`), read with
`host.fluid_states().type_properties(key)`. They are static per fluid, not per position or block, and they are
derived today: the record describes water's and lava's families and gives every other fluid inert values
(`crates/horizon-client/src/game/modding/native_block_states.rs`). A mod's own fluid declares a sprite and tint,
not its type properties, so "partial" below means the vanilla answer is readable but a mod fluid cannot set it.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `getExplosionResistance(state, level, pos, explosion)` | Missing | — | No fluid explosion resistance. |
| `getFluidType()` | Partial | `host.fluid_states().type_properties` | Reads water's and lava's type record; a mod's own fluid cannot declare one and reads as inert. |
| `canConvertToSource(state, level, pos)` | Partial | `create_fluid_source` (`hooks/create_fluid_source.rs`) | Global `CreateFluidSourceEvent`; not per-fluid and not a Part. |
| `supportsBoating(state, boat)` | Partial | `FluidTypeProperties.supports_boating` | Static, and only a water-family fluid supports boating; a mod fluid cannot opt in. |
| `getBlockPathType(state, level, pos, mob, canFloat)` | Partial | `BlockPart::is_pathfindable` (`hooks/is_pathfindable.rs`) | Block-level path computation type only; no fluid `PathType`. |
| `getAdjacentBlockPathType(state, level, pos, mob, pathType)` | Missing | — | No adjacent fluid path type. |
| `canHydrate(state, level, pos, farmland)` | Partial | `FluidTypeProperties.can_hydrate` | Static, and only a water-family fluid hydrates; a mod fluid cannot opt in. |
| `canExtinguish(state, level, pos)` | Partial | `FluidTypeProperties.can_extinguish` | Static, and only a water-family fluid extinguishes; a mod fluid cannot opt in. |
| `appendHoverText(stack, context, display, lines, flag)` | Missing | — | `FluidStack` has no tooltip hook. |

## `IMenuTypeExtension`

Menu types are content data (`MenuType` in `crates/horizon-mod-package/src/content.rs`, files under
`data/<id>/horizon/menu/`), and the game's own menu engine opens them (`menu::Kind::Mod`). A `MenuPart` can answer
slot rules, not construct a menu.

| Method | Verdict | Horizon | Notes |
| --- | --- | --- | --- |
| `create(IContainerFactory)` (static) | Partial | content `MenuType` declaration | Declares slot count, data slots and take-only slots, but cannot supply a factory that builds the client menu from server-sent data. |
| `create(windowId, inventory, buffer)` | Partial | menu engine + `MenuPart::menu_slot_rules` | The engine builds the menu from the declaration; a mod cannot run code at construction. |

## What the follow-up issues cover

[#1038](https://github.com/arismoko/horizon/issues/1038) (blocks) and
[#1039](https://github.com/arismoko/horizon/issues/1039) (items) were written from the earlier audit. Some of
their rows have landed or become partial since and should be narrowed when they are filed:

- **Changed on the block side:** `shouldCheckWeakPower` and `should_redstone_wire_connect_to` (the issue says
  reuse them) are Parts; `getFlammability` / `getFireSpreadSpeed` are now `fire_odds` (state-based, without a
  position or direction); `canSustainPlant` is `can_sustain_plant` (the soil) before `may_place_on` (the
  plant); `canHarvestBlock` is `can_harvest_block` before `player_harvest_check`; `isLadder` is `is_ladder`;
  `getToolModifiedState` is partly `block_tool_modification`.
- **Changed on the item side:** `getEquipmentSlot` is `ItemResource::equippable_slot`; `getCraftingRemainder`
  has `ItemPart::crafting_remainder` plus content data and `host.item_crafting_remainder`; `canEquip` has the `equippable` component; `getXpRepairRatio` has a global `mending_repair` that changes the amount.
- **Block group rows that still need their own answer:** `getExpDrop`, `onBlockExploded`,
  `getWeakChanges`, `onNeighborChange`, `getBounceRestitution`, `isSlimeBlock`, `isStickyBlock`,
  `canStickTo`, `onDestroyedByPlayer` (all Partial or Missing above).
- **Item group rows that still need their own answer:** `onEntityItemUpdate`, `canPerformAction` (both
  Partial or Missing above).
- **Missing but not in either group:** the remaining block, entity, block-entity and fluid rows marked Missing
  above; they have no follow-up issue yet. The largest are the entity fluid queries, block entity model data and
  fluid tooltips.
