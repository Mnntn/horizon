//! Capability parts (arismoko/horizon#41 §7): providers a mod registers for its block entity
//! type to answer a capability, such as [`ITEMS`], beside the game's own (NeoForge's
//! `RegisterCapabilitiesEvent`). A handler holds no borrow of the game: each call is given the
//! host, so a handler may look up and call a neighbour's capability (a pipe), and it keeps its
//! state where the game keeps the block entity's (its slots, its saved compound, its
//! attachments).
use crate::{
    Direction, Host, ItemResource, LevelRef, ResourceHandler, ServerHost, Transaction,
    backend_mut_of, backend_of,
};
use std::{fmt::Debug, marker::PhantomData, ops::Deref};

// no wire: process-local capability declaration carries a provider function pointer and type identity.
/// A capability: its name (`horizon:items`, `mymod:chemicals`), the handler trait `T` it answers
/// with, and the game's own provider, if the game has one.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CapabilityKey<T: ?Sized + 'static> {
    /// The name.
    pub name: &'static str,
    pub(crate) default: Option<fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>>>,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

/// A block entity a capability is asked of, by its level and position, and the side it is
/// reached through (none: from no particular side): NeoForge's block capability context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockSide {
    /// The level.
    pub level: LevelRef,
    /// The pos.
    pub pos: [i32; 3],
    /// The side.
    pub side: Option<Direction>,
}

// no wire: owned trait-object capability handler; invoked through its provider, not serialized.
/// A capability's handler, with who answered: a mod's provider or the game's own.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct Capability<T: ?Sized> {
    pub(crate) handler: Box<T>,
    /// The mod whose provider answered; none for the game.
    pub(crate) owner: Option<&'static str>,
}

impl<T: ?Sized> Deref for Capability<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.handler
    }
}

impl<T: ?Sized> Capability<T> {
    /// The id of the mod whose provider answered, or "the game", for warnings.
    pub fn owner(&self) -> String {
        self.owner.unwrap_or("the game").to_owned()
    }
    /// `answer`, the handler's amount for a request of `asked` at `at`: itself, or none beyond
    /// `asked`, a mod bug warned once naming the mod. The caller aborts the move.
    pub(crate) fn checked(&self, answer: u32, asked: u32, at: &dyn Debug) -> Option<u32> {
        if answer > asked {
            crate::block::item_place::warn_broken(self.owner, at);
            return None;
        }
        Some(answer)
    }
}

impl<T: ?Sized + 'static> CapabilityKey<T> {
    /// A mod's own capability, answered only by the parts registered for it.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            default: None,
            handler: PhantomData,
        }
    }
}

/// Item slots, as NeoForge 26.3's `Capabilities.Item.BLOCK`: a block entity's slots as one side
/// reaches them. The game answers it for every block entity with slots: chests (a double chest's
/// two halves as one), furnaces, hoppers and the other vanilla containers, by their
/// `WorldlyContainer` sides, and a mod block entity's `block_entity_container`.
pub const ITEMS: CapabilityKey<dyn ResourceHandler<ItemResource>> = CapabilityKey {
    name: "horizon:items",
    default: Some(|host, at| {
        backend_of(host)?
            .container_size(at)
            .map(|_| Box::new(GameItems(at)) as Box<dyn ResourceHandler<ItemResource>>)
    }),
    handler: PhantomData,
};

/// The game's own [`ITEMS`]: the block entity's slots `.side` reaches, by the rules hoppers
/// follow.
struct GameItems(BlockSide);
impl ResourceHandler<ItemResource> for GameItems {
    fn set_max_stack_size(&self, host: &mut dyn ServerHost, tx: &Transaction, limit: u32) -> bool {
        backend_mut_of(host)
            .is_some_and(|backend| backend.container_set_stack_limit(self.0, limit, tx))
    }

    fn size(&self, host: &mut dyn ServerHost) -> usize {
        backend_of(&*host)
            .and_then(|backend| backend.container_size(self.0))
            .unwrap_or(0)
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        let stack = backend_of(&*host)?.container_stack(self.0, index)?;
        Some(ItemResource::of(&stack))
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        backend_of(&*host)
            .and_then(|backend| backend.container_stack(self.0, index))
            .map_or(0, |stack| stack.count())
    }
    fn capacity(&self, host: &mut dyn ServerHost, index: usize, resource: &ItemResource) -> u32 {
        backend_of(&*host).map_or(0, |backend| {
            backend.container_capacity(self.0, index, resource.as_stack())
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
        if amount == 0 {
            return 0;
        }
        let Some(backend) = backend_mut_of(host) else {
            return 0;
        };
        let Ok(rest) = backend.container_insert(self.0, index, resource, amount, tx) else {
            return 0;
        };
        amount - rest.map_or(0, |rest| rest.count().min(amount))
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        let held = backend_of(&*host).and_then(|backend| backend.container_stack(self.0, index));
        if !held.is_some_and(|held| resource.matches(&held)) {
            return 0;
        }
        let Some(backend) = backend_mut_of(host) else {
            return 0;
        };
        backend
            .container_extract(self.0, index, amount, tx)
            .map_or(0, |out| out.count().min(amount))
    }
}

impl LevelRef {
    /// `key`'s handler for the block entity at `pos` in this level, reached through `side`
    /// (`Level.getCapability`): the first provider to answer, of those mods registered for its
    /// type `before_game`, the game's own, and the mods' others, each in registration order. None
    /// for a stale handle, and for a level not in place ([`LevelRef::in_place`]), whose block
    /// entities the game doesn't write: no provider is asked for one. Disabled mods' providers
    /// are skipped; re-enabling a mod makes its registrations available to the next lookup.
    pub fn capability<T: ?Sized + 'static>(
        self,
        host: &mut dyn ServerHost,
        key: &CapabilityKey<T>,
        pos: [i32; 3],
        side: Option<Direction>,
    ) -> Option<Capability<T>> {
        if !self.in_place(host) {
            return None;
        }
        let at = BlockSide {
            level: self,
            pos,
            side,
        };
        let mut game = None;
        let table = host.capability_providers();
        let kind = table
            .provided(crate::ProvidedBy::BlockEntity, key.name)
            .then(|| at.kind(host))
            .flatten();
        crate::answered(
            crate::ask_providers(
                host,
                |index| table.block_entity_provider(key.name, kind?, index),
                crate::BlockProviderOf::get::<T>,
                |provider, host| provider.provide(host, at),
                |host| {
                    game = key.default.and_then(|default| default(host, at));
                    game.is_some()
                },
            ),
            game,
        )
    }
}

impl BlockSide {
    /// The type of the block entity here (`minecraft:chest`, `mymod:press`), which capability
    /// providers are registered by.
    pub(crate) fn kind(self, host: &dyn Host) -> Option<crate::BlockEntityTypeKey> {
        let entity = host.block_entity(self.level, self.pos)?;
        entity.kind(host)
    }
}
