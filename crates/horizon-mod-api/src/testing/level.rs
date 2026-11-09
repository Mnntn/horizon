//! The level fake: blocks, block entities with the game's storages, and entities.
use super::FAKE_EPOCH;
use crate::{BlockEntityTypeKey, FluidStack, ItemStack, LevelRef, WorldEpoch};
use std::collections::{BTreeMap, HashMap};

// no wire: in-process testing harness state, not a mod boundary payload.
/// One level of a test world: the blocks a test names, the block entities standing in it with
/// the storages the game answers for them, and its entities. Positions nothing was put at read
/// as empty, as unloaded blocks do.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeLevel {
    level: LevelRef,
    blocks: HashMap<[i32; 3], String>,
    block_entities: BTreeMap<[i32; 3], FakeBlockEntity>,
    entities: BTreeMap<u32, FakeEntity>,
}

impl Default for FakeLevel {
    /// The overworld.
    fn default() -> Self {
        Self::new("minecraft:overworld")
    }
}

impl FakeLevel {
    /// An empty level of `dimension` (`minecraft:the_nether`).
    pub fn new(dimension: &'static str) -> Self {
        Self {
            level: LevelRef::new(
                WorldEpoch::new(FAKE_EPOCH),
                crate::content::registry_keys::dimension_key(dimension),
            ),
            blocks: HashMap::new(),
            block_entities: BTreeMap::new(),
            entities: BTreeMap::new(),
        }
    }
    /// The handle of this level.
    pub fn level(&self) -> LevelRef {
        self.level
    }
    /// Puts block `name` (`minecraft:stone`) at `pos`.
    pub fn set_block(&mut self, pos: [i32; 3], name: &str) -> &mut Self {
        self.blocks.insert(pos, name.to_owned());
        self
    }
    /// The block entity at `pos`, a vanilla one with no storage put there if none stands there.
    pub fn block_entity(&mut self, pos: [i32; 3]) -> &mut FakeBlockEntity {
        self.block_entities.entry(pos).or_default()
    }
    /// The block entity at `pos`, if one stands there.
    pub fn block_entity_at(&self, pos: [i32; 3]) -> Option<&FakeBlockEntity> {
        self.block_entities.get(&pos)
    }
    /// Entity `id` of type `kind` (`minecraft:pig`) in this level, put there if missing; its
    /// UUID is its id until set.
    pub fn entity(&mut self, id: u32, kind: &str) -> &mut FakeEntity {
        let level = self.level;
        self.entities.entry(id).or_insert_with(|| FakeEntity {
            kind: kind.to_owned(),
            level,
            uuid: u128::from(id),
        })
    }

    pub(super) fn block_at(&self, level: LevelRef, pos: [i32; 3]) -> Option<&str> {
        (level == self.level)
            .then(|| self.blocks.get(&pos))
            .flatten()
            .map(String::as_str)
    }
    pub(super) fn standing(&self, level: LevelRef, pos: [i32; 3]) -> Option<&FakeBlockEntity> {
        (level == self.level)
            .then(|| self.block_entities.get(&pos))
            .flatten()
    }
    pub(super) fn standing_mut(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<&mut FakeBlockEntity> {
        (level == self.level)
            .then(|| self.block_entities.get_mut(&pos))
            .flatten()
    }
    pub(super) fn entity_at(&self, id: u32) -> Option<&FakeEntity> {
        self.entities.get(&id)
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A block entity of a test level: its type, vanilla when unset, and the storages the game
/// answers for it from every side.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeBlockEntity {
    kind: Option<BlockEntityTypeKey>,
    /// Its item slots (a chest's), if it has any.
    pub items: Option<FakeContainer>,
    /// Its fluid tanks.
    pub tanks: Vec<FakeTank>,
    /// Its energy storage, if it has one.
    pub energy: Option<FakeBattery>,
}

impl FakeBlockEntity {
    /// Makes it a block entity of the mod type `id` (`mymod:pipe`).
    pub fn of_type(&mut self, id: &str) -> &mut Self {
        self.kind = crate::parts::requested_keys()
            .block_entity_types
            .slots
            .get(id)
            .copied()
            .map(BlockEntityTypeKey);
        self
    }
    /// Gives it `slots` empty item slots, of the vanilla limit of 64.
    pub fn with_items(&mut self, slots: usize) -> &mut Self {
        self.items = Some(FakeContainer::new(slots));
        self
    }
    /// Its mod type, none for a vanilla block entity.
    pub fn kind(&self) -> Option<&BlockEntityTypeKey> {
        self.kind.as_ref()
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A block entity's item slots, as the game's `Container` handler answers: a slot takes the
/// item it holds, or any when empty, up to `limit` and the item's stack size.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeContainer {
    /// The slots, in order.
    pub slots: Vec<Option<ItemStack>>,
    /// The most any slot holds (`Container.getMaxStackSize`).
    pub limit: u32,
}

impl FakeContainer {
    /// `slots` empty slots of limit 64.
    pub fn new(slots: usize) -> Self {
        Self {
            slots: vec![None; slots],
            limit: 64,
        }
    }
    /// How many of all the slots hold of item `item`.
    pub fn count(&self, item: &str) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|held| crate::content::registry_keys::item_id(held.item()) == Some(item))
            .map(ItemStack::count)
            .sum()
    }

    /// How many of an item of stack size `max_stack` slot `slot` holds when empty.
    pub(super) fn capacity(&self, slot: usize, max_stack: u32) -> u32 {
        if slot < self.slots.len() {
            self.limit.min(max_stack)
        } else {
            0
        }
    }
    /// Puts up to `amount` of `resource` into slot `slot`; how much fit.
    pub(super) fn insert(
        &mut self,
        slot: usize,
        resource: &crate::ItemResource,
        amount: u32,
        max_stack: u32,
    ) -> u32 {
        let capacity = self.capacity(slot, max_stack);
        let Some(held) = self.slots.get_mut(slot) else {
            return 0;
        };
        let room = match held {
            Some(held) if !resource.matches(held) => 0,
            Some(held) => capacity.saturating_sub(held.count()),
            None => capacity,
        };
        let put = room.min(amount);
        if put > 0 {
            let count = held.as_ref().map_or(0, ItemStack::count) + put;
            *held = resource.to_stack(count);
        }
        put
    }
    /// Takes up to `max` out of slot `slot`; what came out.
    pub(super) fn extract(&mut self, slot: usize, max: u32) -> Option<ItemStack> {
        let held = self.slots.get_mut(slot)?;
        let split = held.take()?.split(max);
        *held = split.remainder;
        split.taken
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A fluid tank, as the game's tanks answer: it fills with the fluid it holds, or any when
/// empty, up to `capacity` millibuckets.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeTank {
    /// What it holds.
    pub held: Option<FluidStack>,
    /// The most it holds, in millibuckets.
    pub capacity: u32,
}

impl FakeTank {
    /// An empty tank of `capacity` millibuckets.
    pub fn new(capacity: u32) -> Self {
        Self {
            held: None,
            capacity,
        }
    }
    /// How many millibuckets it holds.
    pub fn amount(&self) -> u32 {
        self.held.as_ref().map_or(0, |held| held.amount)
    }

    /// Puts what of `fluid` fits; the rest.
    pub(super) fn insert(&mut self, fluid: FluidStack) -> Option<FluidStack> {
        if self
            .held
            .as_ref()
            .is_some_and(|held| !held.same_fluid_same_components(&fluid))
        {
            return Some(fluid);
        }
        let held = self.amount();
        let add = fluid.amount.min(self.capacity.saturating_sub(held));
        if add > 0 {
            self.held = Some(fluid.with_amount(held + add));
        }
        (add < fluid.amount).then(|| fluid.with_amount(fluid.amount - add))
    }
    /// Takes up to `max` millibuckets out; what came out.
    pub(super) fn extract(&mut self, max: u32) -> Option<FluidStack> {
        let held = self.held.clone()?;
        let take = held.amount.min(max);
        if take == 0 {
            return None;
        }
        self.held = (take < held.amount).then(|| held.with_amount(held.amount - take));
        Some(held.with_amount(take))
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// An energy storage, as NeoForge's `EnergyStorage`: what it holds of its capacity, and the
/// most one insert or one extract moves.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeBattery {
    /// What it holds.
    pub amount: u32,
    /// The most it holds.
    pub capacity: u32,
    /// The most one insert moves (`maxReceive`).
    pub max_insert: u32,
    /// The most one extract moves (`maxExtract`).
    pub max_extract: u32,
}

impl FakeBattery {
    /// A storage holding `amount` of `capacity`, which moves any amount per call.
    pub fn new(amount: u32, capacity: u32) -> Self {
        Self {
            amount,
            capacity,
            max_insert: u32::MAX,
            max_extract: u32::MAX,
        }
    }

    /// Puts in what of `amount` fits; how much went.
    pub(super) fn insert(&mut self, amount: u32) -> u32 {
        let went = amount
            .min(self.max_insert)
            .min(self.capacity.saturating_sub(self.amount));
        self.amount += went;
        went
    }
    /// Takes out up to `amount`; how much came.
    pub(super) fn extract(&mut self, amount: u32) -> u32 {
        let came = amount.min(self.max_extract).min(self.amount);
        self.amount -= came;
        came
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// An entity of a test world: its type, the level it is in and its UUID.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakeEntity {
    /// Its type (`minecraft:pig`).
    pub kind: String,
    /// The level it is in.
    pub level: LevelRef,
    /// Its UUID.
    pub uuid: u128,
}
