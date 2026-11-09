//! Items as holders (arismoko/horizon#41 §7): an item's own capability, such as
//! [`ITEM_ITEMS`] for a backpack's slots, reached where its stack lives. NeoForge 26.3 reaches
//! an item through an `ItemAccess`; here the place is an [`ItemPlace`], read and written as one
//! ([`ItemPlace::insert`], [`extract`](ItemPlace::extract) and
//! [`exchange`](ItemPlace::exchange)), so the inventory holding the stack is the one changed.
//! Handlers hold no borrow of the game, as block entity capabilities don't (#71).
//!
//! A write runs in a transaction of its own, nested in the caller's, and changes the place where
//! it is, journaled: an abort puts it back, at every level of an item inside an item. A handler
//! answering more than it was asked is a mod bug, named once, and the write aborts.
use crate::HostTransaction;
use crate::{
    Capability, Direction, ENTITY_ITEMS, EquipmentSlot, Hand, ITEMS, ItemResource, ItemStack,
    LevelRef, MenuRef, MenuSlotBacking, ResourceHandler, ServerHost, Transaction, backend_mut_of,
    backend_of,
};
use std::{collections::HashSet, fmt::Debug, marker::PhantomData, sync::Mutex};

/// Where a stack lives.
// no wire: preserves stable explicit variant tags (Equipment is 9) and interns Mod.provider into a bounded static string.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemPlace {
    /// Slot `slot` of a player's inventory, by `native_inventory_tick`'s numbers: 0..36 (0..9
    /// the hotbar), the armour 100..=103 (feet to head), the off hand -106
    /// (`ItemAccess.forPlayerSlot`).
    PlayerSlot {
        /// The player's UUID.
        player: u128,
        /// The slot number, as listed above.
        slot: i32,
    },
    /// What a player holds in `hand` (`forPlayerInteraction`).
    PlayerHand {
        /// The player's UUID.
        player: u128,
        /// Which hand.
        hand: Hand,
    },
    /// The stack on the cursor of a player's open menu, `menu` (`forPlayerCursor`): nothing
    /// once the menu is closed, replaced or its player gone.
    PlayerCursor {
        /// The player's open menu.
        menu: MenuRef,
    },
    /// Slot `slot` of the block entity at `pos` as `side` reaches it, through its [`ITEMS`]
    /// handler (`forHandlerIndex`).
    BlockEntity {
        /// The level the block entity is in.
        level: LevelRef,
        /// The block entity's position.
        pos: [i32; 3],
        /// The face the handler is reached from; `None` for no particular side.
        side: Option<Direction>,
        /// The slot's index in the handler.
        slot: usize,
    },
    /// The stack of item entity `id` in the level in place.
    ItemEntity {
        /// The item entity's network id.
        id: u32,
    },
    /// Slot `slot` of the inventory of entity `entity` in the level in place: a chest or hopper
    /// minecart's, a chest boat's, or a chested donkey's, mule's or llama's
    /// (`Capabilities.Item.ENTITY` through `forHandlerIndex`).
    EntitySlot {
        /// The entity's network id.
        entity: u32,
        /// The slot's index in the entity's inventory.
        slot: usize,
    },
    /// Slot `slot` of living entity `entity`'s equipment (NeoForge's
    /// `LivingEntityEquipmentWrapper` through `Capabilities.Item.ENTITY`), reached through
    /// [`ENTITY_ITEMS`](crate::ENTITY_ITEMS). Its write is the equipment write
    /// `LivingRef::set_equipment` makes, so the equip and unequip game event, sound and
    /// attribute updates are the same, and a transaction that aborts puts it back.
    Equipment {
        /// The entity's network id.
        entity: u32,
        /// The equipment slot.
        slot: EquipmentSlot,
    },
    /// Slot `slot` of the item at `parent`, through its [`ITEM_ITEMS`] handler: a backpack in a
    /// backpack.
    Nested {
        /// The place of the stack that holds the container.
        parent: Box<ItemPlace>,
        /// The slot's index inside that stack's container.
        slot: usize,
    },
    /// Slot `slot` of a player's open menu, `menu`, a fork addition (NeoForge 26.3 has no
    /// `ItemAccess` for a menu slot): the storage the slot shows ([`MenuRef::slot_backing`]),
    /// its own slot only, under the slot's own rules first ([`MenuRef::slot_holds`],
    /// [`MenuRef::slot_gives`]). Nothing where the menu is closed, replaced or its player gone;
    /// nothing goes into or out of the crafting result.
    MenuSlot {
        /// The open menu.
        menu: MenuRef,
        /// The slot's index in that menu.
        slot: usize,
    },
    /// A place a mod keeps, such as an accessory slot: read and written by the [`PlaceProvider`]
    /// it registered as `provider`, which alone knows what `key` means.
    Mod {
        /// The provider's registered name.
        provider: &'static str,
        /// The provider's own key for the place.
        key: u64,
    },
}

/// A mod's own places ([`ItemPlace::Mod`]), registered with
/// [`Registrar::place_provider`](crate::ItemsRegistrar::place_provider): NeoForge's own `ItemAccess`s.
/// Like a handler, it holds no borrow: its stacks live where the mod keeps them (a player's
/// attachments). While its mod is disabled, lookups refuse both reads and writes through its
/// places; re-enabling the mod makes the same registrations available again.
pub trait PlaceProvider: Send + Sync + 'static {
    /// The stack at `key`, if any.
    fn stack(&self, host: &mut dyn ServerHost, key: u64) -> Option<ItemStack>;
    /// Puts up to `amount` of `resource` at `key`, in transaction `tx`: how much went. What
    /// doesn't stack with what is there may go where the mod puts it, such as the player's
    /// inventory (`ItemAccess.insert`); none where `key` is no longer a place.
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        key: u64,
        resource: &ItemResource,
        amount: u32,
    ) -> u32;
    /// Takes up to `amount` of `resource` from `key`, in transaction `tx`: how much came.
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        key: u64,
        resource: &ItemResource,
        amount: u32,
    ) -> u32;
}

// no wire: process-local capability declaration carries a provider function pointer and type identity.
/// An item capability: its name, the handler trait `T` it answers with, and the game's own
/// provider, if the game has one.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ItemCapabilityKey<T: ?Sized + 'static> {
    /// The capability's name.
    pub name: &'static str,
    pub(crate) default: Option<fn(&mut dyn ServerHost, &ItemPlace, &ItemStack) -> Option<Box<T>>>,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> ItemCapabilityKey<T> {
    /// A mod's own item capability, answered only by the parts registered for it.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            default: None,
            handler: PhantomData,
        }
    }
}

/// Items held by an item, as NeoForge 26.3's `Capabilities.Item.ITEM`: a handler for the item at
/// one place, whose changes rewrite the item there with an [`exchange`](ItemPlace::exchange) of
/// one, so from a stack of more than one the changed item splits off. The game answers it for an item with `item_container`
/// slots, kept in its `minecraft:container`.
pub const ITEM_ITEMS: ItemCapabilityKey<dyn ResourceHandler<ItemResource>> = ItemCapabilityKey {
    name: "horizon:item_items",
    default: Some(|host, place, stack| {
        backend_of(host)?.item_container_slots(stack).map(|_| {
            Box::new(GameItemItems(place.clone())) as Box<dyn ResourceHandler<ItemResource>>
        })
    }),
    handler: PhantomData,
};

/// The game's own [`ITEM_ITEMS`] for the item at `.0`, over the `item_container` helpers.
struct GameItemItems(ItemPlace);
impl GameItemItems {
    fn slot(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemStack> {
        let held = self.0.stack(host)?;
        backend_of(&*host)?
            .item_container_slots(&held)?
            .get(index)?
            .clone()
    }
    /// Rewrites one item at `.0` as `changed`: `moved` when that goes through, else 0.
    fn rewrite(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        changed: &ItemStack,
        moved: u32,
    ) -> u32 {
        match self.0.exchange(host, tx, &ItemResource::of(changed), 1) {
            1 => moved,
            _ => 0,
        }
    }
}
impl ResourceHandler<ItemResource> for GameItemItems {
    fn size(&self, host: &mut dyn ServerHost) -> usize {
        self.0
            .stack(host)
            .and_then(|held| {
                backend_of(&*host).and_then(|backend| backend.item_container_slots(&held))
            })
            .map_or(0, |slots| slots.len())
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        Some(ItemResource::of(&self.slot(host, index)?))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.slot(host, index).map_or(0, |stack| stack.count())
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
        self.0.stack(host).map_or(0, |held| {
            backend_of(&*host).map_or(0, |backend| {
                backend.item_container_capacity(&held, index, resource.as_stack())
            })
        })
    }
    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let Some(offer) = resource.to_stack(amount) else {
            return 0;
        };
        let Some((went, changed)) = self.0.stack(host).and_then(|held| {
            backend_of(&*host)
                .and_then(|backend| backend.item_container_insert_slot(&held, index, &offer))
        }) else {
            return 0;
        };
        match went {
            0 => 0,
            went => self.rewrite(host, tx, &changed, went.min(amount)),
        }
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        if !self
            .slot(host, index)
            .is_some_and(|held| resource.matches(&held))
        {
            return 0;
        }
        let Some((Some(out), changed)) = self.0.stack(host).and_then(|held| {
            backend_of(&*host)
                .and_then(|backend| backend.item_container_extract(&held, index, amount))
        }) else {
            return 0;
        };
        self.rewrite(host, tx, &changed, out.count().min(amount))
    }
}

/// The owners of item handlers that answered more than they were asked, warned once.
static BROKEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// `owner`'s handler at `at` answered more than it was asked: a mod bug, warned once per mod.
pub(crate) fn warn_broken(owner: Option<&'static str>, at: &dyn Debug) {
    let owner = owner.unwrap_or("the game").to_owned();
    let mut warned = BROKEN.lock().unwrap_or_else(|e| e.into_inner());
    if warned.get_or_insert_default().insert(owner.clone()) {
        log::warn!(
            "Horizon: {owner}'s handler at {at:?} answered more than it was asked; the move it \
             was part of is undone. A handler moves at most what it is asked."
        );
    }
}

#[cfg(test)]
pub(crate) fn warned_about(owner: &str) -> bool {
    let broken = BROKEN.lock().unwrap_or_else(|e| e.into_inner());
    broken.as_ref().is_some_and(|owners| owners.contains(owner))
}

impl ItemPlace {
    /// The stack here, if any (`getResource` and `getAmount`).
    pub fn stack(&self, host: &mut dyn ServerHost) -> Option<ItemStack> {
        match self {
            ItemPlace::BlockEntity {
                level,
                pos,
                side,
                slot,
            } => {
                let handler = level.capability(host, &ITEMS, *pos, *side)?;
                let resource = handler.resource(host, *slot)?;
                resource.to_stack(handler.amount(host, *slot))
            }
            ItemPlace::Nested { parent, slot } => {
                let handler = parent.capability(host, &ITEM_ITEMS)?;
                let resource = handler.resource(host, *slot)?;
                resource.to_stack(handler.amount(host, *slot))
            }
            ItemPlace::Mod { provider, key } => {
                let (provider, _) = host.capability_providers().place_provider(provider)?;
                provider.stack(host, *key)
            }
            ItemPlace::MenuSlot { menu, slot } => menu.slot(host, *slot)?,
            _ => backend_mut_of(host).and_then(|backend| backend.game_stack(self)),
        }
    }

    /// What is here, without its count (`ItemAccess.getResource`).
    pub fn resource(&self, host: &mut dyn ServerHost) -> Option<ItemResource> {
        self.stack(host).map(|stack| ItemResource::of(&stack))
    }

    /// How many are here (`ItemAccess.getAmount`).
    pub fn amount(&self, host: &mut dyn ServerHost) -> u32 {
        self.stack(host).map_or(0, |stack| stack.count())
    }

    /// Puts up to `amount` of `resource` here, in transaction `tx`: how much went
    /// (`ItemAccess.insert`). What doesn't stack with what is here, or doesn't fit, goes where
    /// vanilla puts an item split off it: a player's into their inventory, else dropped at the
    /// root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `forInfiniteMaterials`); an item entity's dropped beside it at the root commit; a
    /// block entity's, an entity's or an item's slot's into that container's other slots
    /// (`forHandlerIndex`); a mod's place's where its [`PlaceProvider`] puts it.
    pub fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.moved(host, tx, resource, amount, true)
    }

    /// Takes up to `amount` of `resource` from here, in transaction `tx`: how much came
    /// (`ItemAccess.extract`). A creative player's hand gives without losing any.
    pub fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.moved(host, tx, resource, amount, false)
    }

    /// Takes `amount` of what is here and puts as many of `resource` in their place
    /// (`ItemAccess.exchange`): from a bigger stack, they go where [`insert`](Self::insert) puts
    /// what doesn't stack, as vanilla's `ItemUtils.createFilledResult` puts a filled bucket. In a
    /// transaction of its own, nested in `tx`: all of it, or nothing and 0. Inside an item that
    /// shares its place with others like it, it's done in one of them: that one splits off,
    /// changed, and the others stay as they were.
    pub fn exchange(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let _ = tx;
        let swap = |host: &mut dyn ServerHost, tx: &Transaction| {
            let old = self.resource(host).ok_or(())?;
            let out = self.extract(host, tx, &old, amount);
            match out == amount && self.insert(host, tx, resource, out) == out {
                true => Ok(out),
                false => Err(()),
            }
        };
        let done =
            host.transaction(|host, tx| alone(self, host, tx, &mut |host, tx| swap(host, tx)));
        done.unwrap_or(0)
    }

    /// `insert` or `extract`, in a transaction of its own: a handler answering more than it was
    /// asked has it undone, and its mod named.
    fn moved(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
        insert: bool,
    ) -> u32 {
        let _ = tx;
        // A menu slot's own rules come before its storage's: `Slot.mayPlace` and
        // `getMaxStackSize` for what goes in, `Slot.mayPickup` for what comes out.
        let amount = match self {
            ItemPlace::MenuSlot { menu, slot } if insert => {
                let holds = menu.slot_holds(host, *slot, resource.as_stack());
                let room = holds.unwrap_or(0).saturating_sub(self.amount(host));
                amount.min(room)
            }
            ItemPlace::MenuSlot { menu, slot } if menu.slot_gives(host, *slot) != Some(true) => 0,
            _ => amount,
        };
        if amount == 0 {
            return 0;
        }
        // Through index `slot` of `handler`, then for an insert that `spills` the whole handler,
        // each in passes while one moves something (`Inventory.add`'s loop: a
        // handler may move less per call than it could): what `slot` doesn't take goes to the
        // other indices (`forHandlerIndex`).
        let through = |handler: Capability<dyn ResourceHandler<ItemResource>>,
                       host: &mut dyn ServerHost,
                       tx: &Transaction,
                       slot: usize,
                       spills: bool| {
            let mut left = amount;
            let passes: &[bool] = if insert && spills {
                &[false, true]
            } else {
                &[false]
            };
            for &anywhere in passes {
                while left > 0 {
                    let moved = match (insert, anywhere) {
                        (true, false) => handler.insert(host, tx, slot, resource, left),
                        (true, true) => handler.insert_any(host, tx, resource, left),
                        (false, _) => handler.extract(host, tx, slot, resource, left),
                    };
                    match handler.checked(moved, left, self).ok_or(())? {
                        0 => break,
                        moved => left -= moved,
                    }
                }
            }
            Ok(amount - left)
        };
        let moved = host.transaction(|host, tx| match self {
            ItemPlace::BlockEntity {
                level,
                pos,
                side,
                slot,
            } => {
                let handler = level.capability(host, &ITEMS, *pos, *side).ok_or(())?;
                through(handler, host, tx, *slot, true)
            }
            ItemPlace::Nested { parent, slot } => {
                let handler = parent.capability(host, &ITEM_ITEMS).ok_or(())?;
                through(handler, host, tx, *slot, true)
            }
            ItemPlace::MenuSlot { menu, slot } => {
                let (handler, index) = match menu.slot_backing(host, *slot).ok_or(())? {
                    MenuSlotBacking::Player(index) => {
                        let player = menu.player.entity();
                        (player.capability(host, &ENTITY_ITEMS, ()), index)
                    }
                    MenuSlotBacking::BlockEntity { level, pos, slot } => {
                        (level.capability(host, &ITEMS, pos, None), slot)
                    }
                    MenuSlotBacking::Entity { entity, slot } => {
                        (entity.capability(host, &ENTITY_ITEMS, ()), slot)
                    }
                    MenuSlotBacking::CraftResult => return Ok(0),
                    _ => {
                        return Ok(match insert {
                            true => backend_mut_of(host).map_or(0, |backend| {
                                backend.game_insert(self, resource, amount, tx).unwrap_or(0)
                            }),
                            false => backend_mut_of(host).map_or(0, |backend| {
                                backend
                                    .game_extract(self, resource, amount, tx)
                                    .unwrap_or(0)
                            }),
                        });
                    }
                };
                through(handler.ok_or(())?, host, tx, index, false)
            }
            ItemPlace::Mod { provider, key } => {
                let (provider, owner) = host
                    .capability_providers()
                    .place_provider(provider)
                    .ok_or(())?;
                let moved = match insert {
                    true => provider.insert(host, tx, *key, resource, amount),
                    false => provider.extract(host, tx, *key, resource, amount),
                };
                if moved > amount {
                    warn_broken(Some(owner), self);
                    return Err(());
                }
                Ok(moved)
            }
            _ => Ok(match insert {
                true => backend_mut_of(host).map_or(0, |backend| {
                    backend.game_insert(self, resource, amount, tx).unwrap_or(0)
                }),
                false => backend_mut_of(host).map_or(0, |backend| {
                    backend
                        .game_extract(self, resource, amount, tx)
                        .unwrap_or(0)
                }),
            }),
        });
        moved.unwrap_or(0)
    }
}

/// Runs `f` with each item `place` is inside alone in its place, the outermost first
/// ([`one_of`] at every level): the writes `f` makes through them, however many, all reach the
/// same one, which alone changes.
fn alone(
    place: &ItemPlace,
    host: &mut dyn ServerHost,
    tx: &Transaction,
    f: &mut dyn FnMut(&mut dyn ServerHost, &Transaction) -> Result<u32, ()>,
) -> Result<u32, ()> {
    match place {
        ItemPlace::Nested { parent, .. } => alone(parent, host, tx, &mut |host, tx| {
            one_of(parent, host, tx, &mut *f)
        }),
        _ => f(host, tx),
    }
}

/// Runs `f` on one item of the stack at `parent` (NeoForge's `oneByOne`): from a stack of more,
/// the others are taken out first and put back after, and if `f` changed the one, it goes where
/// an exchange puts it. All written in place in `tx`, so an abort puts every level back.
fn one_of(
    parent: &ItemPlace,
    host: &mut dyn ServerHost,
    tx: &Transaction,
    f: impl FnOnce(&mut dyn ServerHost, &Transaction) -> Result<u32, ()>,
) -> Result<u32, ()> {
    let held = parent.stack(host).ok_or(())?;
    let others = held.count() - 1;
    if others == 0 {
        return f(host, tx);
    }
    let resource = ItemResource::of(&held);
    let all = |moved: u32, asked: u32| (moved == asked).then_some(()).ok_or(());
    all(parent.extract(host, tx, &resource, others), others)?;
    let moved = f(host, tx)?;
    let changed = parent.resource(host).filter(|now| *now != resource);
    if let Some(changed) = &changed {
        all(parent.extract(host, tx, changed, 1), 1)?;
    }
    all(parent.insert(host, tx, &resource, others), others)?;
    if let Some(changed) = &changed {
        all(parent.insert(host, tx, changed, 1), 1)?;
    }
    Ok(moved)
}

impl ItemPlace {
    /// `key`'s handler for the item here (`ItemAccess.getCapability`): the first provider to
    /// answer, of those mods registered for its item `before_game`, the game's own, and the
    /// mods' others, each in registration order. Disabled mods' providers are skipped until
    /// their mod is re-enabled.
    pub fn capability<T: ?Sized + 'static>(
        &self,
        host: &mut dyn ServerHost,
        key: &ItemCapabilityKey<T>,
    ) -> Option<Capability<T>> {
        let stack = self.stack(host)?;
        let mut game = None;
        let table = host.capability_providers();
        let provided = table.provided(crate::ProvidedBy::Item, key.name);
        crate::answered(
            crate::ask_providers(
                host,
                |index| {
                    provided
                        .then(|| table.item_provider(key.name, stack.item(), index))
                        .flatten()
                },
                crate::ItemProviderOf::get::<T>,
                |provider, host| provider.provide(host, self),
                |host| {
                    game = key.default.and_then(|default| default(host, self, &stack));
                    game.is_some()
                },
            ),
            game,
        )
    }
}
