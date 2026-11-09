//! Item-moving blocks: `HopperBlockEntity` (push, pull, item entities,
//! transfer cooldowns), `DispenserBlock` / `DropperBlock.dispenseFrom` with
//! the vanilla dispense behaviours Horizon has entities for, and container
//! comparator signals (`AbstractContainerMenu.getRedstoneSignalFromContainer`).
use super::{Game, storage::is_shulker_box};
use mod_api::__private::game::*;

#[cfg(test)]
mod contents_changed_tests;
mod crafter;
pub(in crate::game) mod native_energy;
pub(in crate::game) mod native_fluids;
pub(in crate::game) mod native_items;
#[cfg(test)]
mod tank_reopen_tests;
use horizon_client::{
    Result,
    containers::{BlockEntity, HOPPER_SLOTS, ModContainer},
    items::{ItemStack, Items},
    world::World,
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use horizon_world::{
    level::{Behaviours, LevelAccess},
    registry::BlockStateId,
};
use mod_api::{
    self, BlockSide, Capability, ENTITY_AUTOMATION, EntityRef, ItemResource, ResourceHandler,
};
use std::{ops::ControlFlow, sync::Arc};

type Pos = (i32, i32, i32);

/// What item automation reaches among the entities at its target (`ENTITY_AUTOMATION`).
enum AutomationEntity {
    /// A container entity the game answers for, by id: reached as a vanilla container.
    Container(u32),
    /// An entity a mod's provider answers for, with its handler.
    Part(EntityRef, Capability<dyn ResourceHandler<ItemResource>>),
}

/// A block or entity destination, reached through its slots or a mod's handler.
enum ContainerTarget<'a> {
    View(&'a View),
    Part(Capability<dyn ResourceHandler<ItemResource>>),
}

/// `HopperBlockEntity` transient state (`TransferCooldown`, `tickedGameTime`).
#[derive(Clone, Copy, Debug)]
pub(super) struct HopperState {
    pub(super) cooldown: i32,
    ticked: i64,
}
impl Default for HopperState {
    fn default() -> Self {
        Self {
            cooldown: -1,
            ticked: 0,
        }
    }
}

impl HopperState {
    /// `HopperBlockEntity.loadAdditional`: only the cooldown is saved; the
    /// last tick time starts at zero for the new block entity.
    pub(super) fn load(tag: &horizon_core::nbt::Tag) -> Self {
        let canonical = horizon_client::block_entity_nbt::canonical(tag);
        let cooldown = match canonical.as_ref().and_then(|t| t.get("TransferCooldown")) {
            Some(horizon_core::nbt::Tag::Int(value)) => *value,
            _ => -1,
        };
        Self {
            cooldown,
            ticked: 0,
        }
    }
}

impl Game<'_> {
    pub(super) fn hopper_cooldown(&self, pos: Pos) -> i32 {
        self.hoppers.get(&pos).map_or_else(
            || {
                self.world
                    .chunks
                    .get(&(pos.0 >> 4, pos.2 >> 4))
                    .and_then(|c| c.saved.as_deref())
                    .and_then(|s| s.block_entity_sources.iter().find(|(p, _)| *p == pos))
                    .map_or(-1, |(_, tag)| HopperState::load(tag).cooldown)
            },
            |s| s.cooldown,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Generic,
    /// `AbstractFurnaceBlockEntity` (a `WorldlyContainer`).
    Furnace,
    Hopper,
    /// `CrafterBlockEntity`: disabled slots and the smallest-stack-first rule.
    Crafter,
    /// `BrewingStandBlockEntity` (a `WorldlyContainer`).
    Brewing,
    /// `ChiseledBookShelfBlockEntity`: books only, one to a slot.
    Bookshelf,
    /// `JukeboxBlockEntity`'s single slot: a playable disc only.
    Jukebox,
    /// A mod block's block entity (`ModContainer`).
    Mod,
    /// `ShulkerBoxBlockEntity`: a shulker box doesn't go through a face into one.
    Shulker,
    /// `ComposterBlock.getContainer` at this level (a `WorldlyContainerHolder`
    /// without a block entity): input below 7, empty at 7, bone meal at 8.
    Composter(i32),
}

/// A container: one block entity, or a double chest (`CompoundContainer`).
#[derive(Clone, Debug)]
struct View {
    parts: Vec<(Pos, usize)>,
    kind: Kind,
    /// A mod block's container rules (`Kind::Mod`).
    container: Option<Arc<ModContainer>>,
}
impl View {
    fn len(&self) -> usize {
        self.parts.iter().map(|p| p.1).sum()
    }
    fn locate(&self, mut slot: usize) -> Option<(Pos, usize)> {
        for &(pos, size) in &self.parts {
            if slot < size {
                return Some((pos, slot));
            }
            slot -= size;
        }
        None
    }
    /// `WorldlyContainer.getSlotsForFace` or every slot.
    fn slots_for_face(&self, face: Option<Direction>) -> Vec<usize> {
        match (self.kind, face) {
            (Kind::Furnace, Some(Direction::Down)) => vec![2, 1],
            (Kind::Furnace, Some(Direction::Up)) => vec![0],
            (Kind::Furnace, Some(_)) => vec![1],
            (Kind::Brewing, Some(Direction::Up)) => vec![3],
            (Kind::Brewing, Some(Direction::Down)) => vec![0, 1, 2, 3],
            (Kind::Brewing, Some(_)) => vec![0, 1, 2, 4],
            (Kind::Composter(8), Some(Direction::Down)) => vec![0],
            (Kind::Composter(level), Some(Direction::Up)) if level < 7 => vec![0],
            (Kind::Composter(_), _) => vec![],
            (Kind::Mod, Some(face)) => self
                .container
                .as_ref()
                .map_or_else(Vec::new, |c| c.faces[face as usize].clone()),
            _ => (0..self.len()).collect(),
        }
    }
    /// `Container.getMaxStackSize`: what one slot holds of any item, 1 for a chiseled
    /// bookshelf's books and a jukebox's disc, else vanilla's default.
    fn slot_limit(&self) -> u32 {
        match self.kind {
            Kind::Bookshelf | Kind::Jukebox => 1,
            _ => 99,
        }
    }
    /// The slots `HopperBlockEntity.addItem` tries through `face`.
    fn slots_to_add(&self, face: Option<Direction>) -> Vec<usize> {
        if matches!(self.kind, Kind::Furnace | Kind::Brewing | Kind::Mod) && face.is_some() {
            self.slots_for_face(face)
        } else {
            (0..self.len()).collect()
        }
    }
}

pub(super) fn dir_from_name(name: &str) -> Option<Direction> {
    Some(match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        "east" => Direction::East,
        _ => return None,
    })
}
fn step(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}
fn offset(p: Pos, d: Direction) -> Pos {
    let s = step(d);
    (p.0 + s[0], p.1 + s[1], p.2 + s[2])
}
/// State-level container queries shared by the game and comparator signals.
pub(super) struct Containers<'a> {
    world: &'a World,
    behaviours: &'a Behaviours,
    items: &'a Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    /// Brewing recipes, for what a brewing stand accepts (not needed for signals).
    brewing: Option<&'a horizon_client::brewing::Brewing>,
    /// The stack of a full composter's `OutputContainer`.
    bone_meal: ItemStack,
}
impl Containers<'_> {
    fn prop(&self, state: BlockStateId, key: &str) -> Option<&str> {
        self.registry
            .state(state)?
            .properties
            .get(key)
            .map(String::as_str)
    }
    fn name(&self, state: BlockStateId) -> &str {
        self.registry.state(state).map_or("", |s| s.name.as_str())
    }
    /// `ItemStack.getMaxStackSize`: its component, else the item's.
    pub(super) fn max_stack(&self, stack: &ItemStack) -> u32 {
        if let Some(horizon_core::nbt::Tag::Int(n)) = stack
            .data()
            .and_then(|d| d.other("minecraft:max_stack_size"))
        {
            return (*n).clamp(1, 99) as u32;
        }
        self.items
            .get(&stack.item)
            .map_or(64, |i| i.max_stack)
            .max(1)
    }
    /// `HopperBlockEntity.getBlockContainer` (double chests combined, right half
    /// first). `blocked_chests`: `ChestBlock.getContainer(.., false)` returns
    /// nothing for a chest under a redstone conductor.
    fn view(&self, pos: Pos, blocked_chests: bool) -> Option<View> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let name = self.name(state);
        if name == "minecraft:composter" {
            let level = self.prop(state, "level").and_then(|l| l.parse().ok())?;
            return Some(View {
                parts: vec![(pos, usize::from(level != 7))],
                kind: Kind::Composter(level),
                container: None,
            });
        }
        let entity = self.world.block_entities.get(&pos)?;
        // A beehive keeps bees, not items, a campfire cooks its own and
        // suspicious sand or gravel hides its loot: none is a `Container`
        // (HopperBlockEntity.getContainerAt), so a dropper drops in front of
        // them and a hopper leaves them alone.
        if matches!(
            entity,
            BlockEntity::Beehive(_)
                | BlockEntity::Campfire(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::EndPortal
        ) {
            return None;
        }
        // A mod's block entity is a container when its block says so. The one
        // ticking now is out of the world (an empty id marks its place): what
        // went into its place would be lost when it comes back.
        if let BlockEntity::Mod(m) = entity {
            if m.id().is_empty() {
                return None;
            }
            let container = Arc::clone(self.world.block_entities.mod_container(state.0)?);
            return Some(View {
                parts: vec![(pos, container.slots)],
                kind: Kind::Mod,
                container: Some(container),
            });
        }
        let kind = match (entity, name) {
            (BlockEntity::Furnace(_), _) => Kind::Furnace,
            (BlockEntity::Crafter(_), _) => Kind::Crafter,
            (BlockEntity::Brewing(_), _) => Kind::Brewing,
            (BlockEntity::ChiseledBookshelf(_), _) => Kind::Bookshelf,
            (BlockEntity::Jukebox(_), _) => Kind::Jukebox,
            (_, "minecraft:hopper") => Kind::Hopper,
            (_, n) if is_shulker_box(n) => Kind::Shulker,
            _ => Kind::Generic,
        };
        let size = entity.slots().len();
        if horizon_client::containers::is_chest_block(name) {
            let is_blocked = |p: Pos| {
                let above = self.world.block(p.0, p.1 + 1, p.2);
                self.behaviours.facts(above).flags
                    & horizon_world::level::data::state_flags::CONDUCTOR
                    != 0
            };
            if blocked_chests && is_blocked(pos) {
                return None;
            }
            let half = |s: BlockStateId| {
                Some(horizon_client::containers::ChestHalf {
                    block: self.name(s),
                    kind: self.prop(s, "type")?,
                    facing: self.prop(s, "facing")?,
                })
            };
            if let Some(own) = half(state)
                && let Some((dx, dz)) = own.connected_step()
            {
                let other = (pos.0 + dx, pos.1, pos.2 + dz);
                let other_state = self.world.block(other.0, other.1, other.2);
                let paired = half(other_state).is_some_and(|o| own.pairs_with(&o))
                    && self
                        .world
                        .block_entities
                        .get(&other)
                        .is_some_and(|e| e.slots().len() == 27);
                if paired {
                    if blocked_chests && is_blocked(other) {
                        return None;
                    }
                    let parts = if own.kind == "right" {
                        vec![(pos, size), (other, 27)]
                    } else {
                        vec![(other, 27), (pos, size)]
                    };
                    return Some(View {
                        parts,
                        kind,
                        container: None,
                    });
                }
            }
        }
        Some(View {
            parts: vec![(pos, size)],
            kind,
            container: None,
        })
    }
    fn get(&self, view: &View, slot: usize) -> Option<&ItemStack> {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer holds one bone meal; InputContainer composts at once.
            return (level == 8 && slot == 0).then_some(&self.bone_meal);
        }
        let (pos, i) = view.locate(slot)?;
        self.world
            .block_entities
            .get(&pos)?
            .slots()
            .get(i)?
            .as_ref()
            // A mod can leave an empty stack in its own slots.
            .filter(|s| s.count > 0)
    }
    fn is_empty(&self, view: &View) -> bool {
        (0..view.len()).all(|i| self.get(view, i).is_none())
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces) with slot `slot`
    /// empty: the item rule of the slot, whatever it holds now.
    fn accepts(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if let Some(c) = &view.container {
            return c.inputs.contains(&slot);
        }
        match view.kind {
            // ShulkerBoxBlockEntity.canPlaceItemThroughFace
            Kind::Shulker => !is_shulker_box(&stack.item),
            Kind::Crafter => match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => slot < 9 && !c.disabled[slot],
                _ => false,
            },
            // JukeboxBlockEntity.canPlaceItem: a playable disc.
            Kind::Jukebox => self
                .items
                .get(&stack.item)
                .is_some_and(|i| i.jukebox_song.is_some()),
            // ListBackedContainer.canPlaceItem: `acceptsItemType` (the books).
            Kind::Bookshelf => self
                .items
                .item_tags
                .get("minecraft:bookshelf_books")
                .is_some_and(|m| m.contains(&*stack.item)),
            // BrewingStandBlockEntity.canPlaceItem
            Kind::Brewing => match slot {
                4 => horizon_client::brewing::brewing_fuel(self.items, stack).is_some(),
                3 => self.brewing.is_some_and(|b| b.is_reagent(stack)),
                _ => {
                    self.brewing.is_some_and(|b| b.is_bottle(stack))
                        || self
                            .items
                            .item_tags
                            .get("minecraft:brewing_potion_inputs")
                            .is_some_and(|m| m.contains(&*stack.item))
                }
            },
            Kind::Furnace => match slot {
                2 => false,
                1 => {
                    &*stack.item == "minecraft:bucket"
                        || self
                            .items
                            .get(&stack.item)
                            .is_some_and(|i| i.fuel.is_some())
                }
                _ => true,
            },
            _ => true,
        }
    }
    /// `Container.canPlaceItem` (+ `canPlaceItemThroughFace` for furnaces): `accepts`, and what
    /// the slot holds now. The slots of one item (a jukebox's, a chiseled bookshelf's, a brewing
    /// stand's bottles) take one only when empty, a furnace's fuel slot no bucket onto a
    /// bucket, and a crafter fills its smallest stack first.
    fn can_place(&self, view: &View, slot: usize, stack: &ItemStack) -> bool {
        if !self.accepts(view, slot, stack) {
            return false;
        }
        let held = self.get(view, slot);
        let is_bucket = |s: &ItemStack| &*s.item == "minecraft:bucket";
        match view.kind {
            _ if view.container.is_some() => true,
            Kind::Jukebox | Kind::Bookshelf => held.is_none(),
            Kind::Brewing => slot >= 3 || held.is_none(),
            Kind::Furnace => slot != 1 || !is_bucket(stack) || !held.is_some_and(is_bucket),
            Kind::Crafter => match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => {
                    // The slot's own stack sets the limit (`slotStack.getMaxStackSize`).
                    let max = c.items.get(slot).and_then(Option::as_ref).unwrap_or(stack);
                    c.can_place(slot, stack, self.max_stack(max))
                }
                _ => false,
            },
            _ => true,
        }
    }
    /// How many of `stack` slot `slot` holds when empty, whatever it holds now: none where
    /// `accepts` refuses it, else the slot's limit (`Container.getMaxStackSize(stack)`: the
    /// container's and the item's, the lower).
    fn capacity(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.accepts(view, slot, stack) {
            return 0;
        }
        let capacity = view.slot_limit().min(self.max_stack(stack));
        view.locate(slot)
            .and_then(|(pos, _)| self.world.container_stack_limit(pos))
            .map_or(capacity, |limit| capacity.min(limit))
    }
    /// How many of `stack` slot `slot` takes now: none where `can_place` refuses it, else
    /// `capacity` less what it holds.
    fn fits(&self, view: &View, slot: usize, stack: &ItemStack) -> u32 {
        if !self.can_place(view, slot, stack) {
            return 0;
        }
        room(
            self.capacity(view, slot, stack),
            self.get(view, slot),
            stack,
        )
    }
    /// `canTakeItemThroughFace` (furnaces and brewing stands restrict it).
    fn can_take(&self, view: &View, slot: usize, stack: &ItemStack, face: Direction) -> bool {
        if let Kind::Composter(level) = view.kind {
            // OutputContainer.canTakeItemThroughFace.
            return level == 8 && face == Direction::Down && &*stack.item == "minecraft:bone_meal";
        }
        if view.kind == Kind::Brewing {
            return slot != 3 || &*stack.item == "minecraft:glass_bottle";
        }
        if let Some(c) = &view.container {
            return c.outputs.contains(&slot);
        }
        view.kind != Kind::Furnace
            || face != Direction::Down
            || slot != 1
            || matches!(&*stack.item, "minecraft:water_bucket" | "minecraft:bucket")
    }
    /// `HopperBlockEntity.isFullContainer`.
    fn is_full(&self, view: &View, face: Direction) -> bool {
        view.slots_for_face(Some(face)).into_iter().all(|i| {
            self.get(view, i)
                .is_some_and(|s| s.count >= self.max_stack(s))
        })
    }
    /// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
    fn signal(&self, view: &View) -> i32 {
        if view.kind == Kind::Crafter {
            // CrafterBlockEntity.getRedstoneSignal: filled or disabled slots.
            return match self.world.block_entities.get(&view.parts[0].0) {
                Some(BlockEntity::Crafter(c)) => c.redstone_signal(),
                _ => 0,
            };
        }
        let mut total = 0.0f32;
        for i in 0..view.len() {
            if let Some(s) = self.get(view, i) {
                total += s.count as f32 / self.max_stack(s) as f32;
            }
        }
        total /= view.len().max(1) as f32;
        // Mth.lerpDiscrete(total, 0, 15). A mod can write a stack over its own
        // limit, so the ratio itself is clamped, as vanilla's invariant does.
        (total.min(1.0) * 14.0).floor() as i32 + i32::from(total > 0.0)
    }
}

/// How many of `stack` go into a slot holding `current`, up to `limit`: into an empty slot, or
/// onto the same item and components (`HopperBlockEntity.canMergeItems`).
fn room(limit: u32, current: Option<&ItemStack>, stack: &ItemStack) -> u32 {
    match current {
        None => stack.count.min(limit),
        Some(current) if current.same_item_same_components(stack) => {
            stack.count.min(limit.saturating_sub(current.count))
        }
        Some(_) => 0,
    }
}

/// The block columns any item entity's box reaches, so that a hopper with
/// no item over it skips the scan of every item. Kept for one block-entity
/// pass, in which items do not move: removing one leaves a superset, still
/// right, and adding one (always pushed at the end, under a new id) or
/// removing the last rebuilds the columns.
#[derive(Default)]
pub(super) struct ItemColumns {
    /// The item count and last item id when last asked, if built.
    seen: Option<(usize, Option<u32>)>,
    cells: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
}
impl ItemColumns {
    /// Whether an item's box may reach into column (x, z).
    fn may_reach(
        &mut self,
        items: &[horizon_client::entities::ItemEntity],
        x: i32,
        z: i32,
    ) -> bool {
        let (len, last) = (items.len(), items.last().map(|i| i.id));
        let stale = self
            .seen
            .is_none_or(|(seen_len, seen_last)| len > seen_len || last != seen_last);
        self.seen = Some((len, last));
        if stale {
            self.cells.clear();
            for item in items {
                let b = item.bounds();
                let (lo, hi) = (b.min_position(), b.max_position());
                for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                    for cz in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        self.cells.insert((cx, cz));
                    }
                }
            }
        }
        self.cells.contains(&(x, z))
    }
    /// Whether an item's box may reach into any column of x0..=x1, z0..=z1.
    pub(super) fn may_reach_area(
        &mut self,
        items: &[horizon_client::entities::ItemEntity],
        (x0, x1): (i32, i32),
        (z0, z1): (i32, i32),
    ) -> bool {
        (x0..=x1).any(|x| (z0..=z1).any(|z| self.may_reach(items, x, z)))
    }
}

/// [`ItemColumns`] for a pass in which items can move (a command, an
/// explosion's push), kept with the item list's
/// [`ListStamp`](horizon_client::join_seq::ListStamp): after any write the
/// first question scans as before (`true`), and the second with nothing
/// written in between builds the columns. A pass that writes the items
/// between questions costs no more than the scans did.
#[derive(Default)]
pub(super) struct StampedItemColumns {
    stamp: Option<horizon_client::join_seq::ListStamp>,
    columns: ItemColumns,
}
impl StampedItemColumns {
    /// [`ItemColumns::may_reach_area`] for the items as they are now.
    pub(super) fn may_reach_area(
        &mut self,
        items: &horizon_client::join_seq::JoinList<horizon_client::entities::ItemEntity>,
        xs: (i32, i32),
        zs: (i32, i32),
    ) -> bool {
        if self.stamp != Some(items.stamp()) {
            self.stamp = Some(items.stamp());
            self.columns = ItemColumns::default();
            return true;
        }
        self.columns.may_reach_area(items, xs, zs)
    }
}

fn bone_meal() -> ItemStack {
    ItemStack {
        item: "minecraft:bone_meal".into(),
        count: 1,
        damage: 0,
        extra: None,
    }
}

/// Comparator input from a container block entity (0 when there is none).
pub(super) fn container_signal(
    world: &World,
    behaviours: &Behaviours,
    items: &Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    pos: BlockPos,
) -> i32 {
    let c = Containers {
        world,
        behaviours,
        items,
        registry,
        brewing: None,
        bone_meal: bone_meal(),
    };
    c.view((pos.x, pos.y, pos.z), true)
        .filter(|v| !matches!(v.kind, Kind::Composter(_)))
        .map_or(0, |v| c.signal(&v))
}

/// `ChiseledBookShelfBlock.getAnalogOutputSignal` and
/// `ShelfBlock.getAnalogOutputSignal`, from the block entity Horizon keeps as
/// it was loaded; `None` for any other block. `direction` points from the
/// block to the comparator.
pub(super) fn shelf_signal(
    world: &World,
    registry: &'static horizon_world::registry::BlockRegistry,
    state: BlockStateId,
    pos: BlockPos,
    direction: Direction,
) -> Option<i32> {
    let info = registry.state(state)?;
    let chiseled = info.name == "minecraft:chiseled_bookshelf";
    if !chiseled && !info.name.ends_with("_shelf") {
        return None;
    }
    if chiseled {
        // The last slot touched, plus one (`lastInteractedSlot` starts at -1).
        return Some(match world.block_entities.get(&(pos.x, pos.y, pos.z)) {
            Some(BlockEntity::ChiseledBookshelf(b)) => b.last_slot + 1,
            _ => 0,
        });
    }
    // Only read from the back: a bit for each of the three slots in use.
    let facing = info.properties.get("facing").and_then(|f| dir_from_name(f));
    if facing.map(Direction::opposite) != Some(direction) {
        return Some(0);
    }
    let Some(BlockEntity::Shelf(shelf)) = world.block_entities.get(&(pos.x, pos.y, pos.z)) else {
        return Some(0);
    };
    Some(
        shelf
            .items
            .iter()
            .enumerate()
            .map(|(slot, item)| i32::from(item.is_some()) << slot)
            .sum(),
    )
}

impl Game<'_> {
    pub(super) fn containers(&self) -> Containers<'_> {
        Containers {
            world: &self.world,
            behaviours: self.res.behaviours(),
            items: self.items(),
            registry: self.pack.assets.registry,
            brewing: Some(&self.stations().brewing),
            bone_meal: bone_meal(),
        }
    }
    pub(super) fn block_can_place(&self, pos: [i32; 3], slot: usize, stack: &ItemStack) -> bool {
        let containers = self.containers();
        let Some(view) = containers.view((pos[0], pos[1], pos[2]), false) else {
            return false;
        };
        view.kind == Kind::Shulker || containers.can_place(&view, slot, stack)
    }
    fn slot_mut_at(&mut self, view: &View, slot: usize) -> Option<&mut Option<ItemStack>> {
        let (pos, i) = view.locate(slot)?;
        let entity = self.world.block_entities.get_mut(&pos)?;
        // A mod's slots grow to its container's count when first used.
        if let BlockEntity::Mod(m) = entity
            && m.items.len() <= i
        {
            m.items.resize(view.len(), None);
        }
        entity.slots_mut().get_mut(i)
    }
    /// `BlockEntity.setChanged` for every block entity of a container.
    fn view_changed(&mut self, view: &View) {
        // A composter's containers change its block state instead.
        if let Kind::Composter(_) = view.kind {
            return;
        }
        for &(pos, _) in &view.parts {
            self.container_changed(pos);
        }
    }
    /// `BlockEntity.setChanged` at `pos`: comparators re-read the container.
    pub(super) fn container_changed(&mut self, pos: Pos) {
        if pos.1 == super::minecarts::CART_KEY_Y {
            return;
        }
        if mod_api::__private::has_scoped_hook(mod_api::HookId::ContentsChanged)
            && let Some(level) = self.native_level_here()
        {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            self.native_contents_changed(level, at, self.world.block(pos.0, pos.1, pos.2));
        }
        self.chiseled_bookshelf_synced(pos);
        if matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Shelf(_))
        ) {
            self.mark_block_entity_updated(pos);
        }
        // A double chest's other half changes with it.
        let halves: Vec<Pos> = std::iter::once(pos)
            .chain(self.chest_other_half(pos))
            .collect();
        for pos in halves {
            self.world.modified.insert((pos.0 >> 4, pos.2 >> 4));
            self.with_level(|b, level, ticks| {
                b.container_changed(level, ticks, BlockPos::new(pos.0, pos.1, pos.2))
            });
        }
    }
    /// `HopperBlockEntity.tryMoveInItem`.
    fn try_move_in_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        stack: ItemStack,
        slot: usize,
    ) -> Option<ItemStack> {
        let (rest, moved) = self.move_in_item(from_hopper, view, stack, slot);
        if moved {
            self.view_changed(view);
            // JukeboxBlockEntity.setTheItem through a hopper's or a dropper's
            // transfer: the disc it lands on starts playing.
            if view.kind == Kind::Jukebox {
                let (x, y, z) = view.parts[0].0;
                self.jukebox_inserted([x, y, z]);
            }
        }
        rest
    }
    /// `tryMoveInItem` before its `setChanged`: the remainder, and whether
    /// anything moved.
    fn move_in_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        mut stack: ItemStack,
        slot: usize,
    ) -> (Option<ItemStack>, bool) {
        let c = self.containers();
        if !c.can_place(view, slot, &stack) {
            return (Some(stack), false);
        }
        let was_empty = c.is_empty(view);
        let vanilla_max = c.max_stack(&stack);
        let override_limit = view
            .locate(slot)
            .and_then(|(pos, _)| c.world.container_stack_limit(pos));
        let restricted = override_limit.is_some_and(|limit| limit < vanilla_max);
        let max = override_limit.map_or(vanilla_max, |limit| vanilla_max.min(limit));
        if restricted && max == 0 {
            return (Some(stack), false);
        }
        let mut success = false;
        let mut filled_empty = false;
        let Some(target) = self.slot_mut_at(view, slot) else {
            return (Some(stack), false);
        };
        // An empty stack a mod left in its slot: the slot is free (`isEmpty`).
        if target.as_ref().is_some_and(|s| s.count == 0) {
            *target = None;
        }
        match target {
            None => {
                // HopperBlockEntity.setItem limits the accepted stack, then
                // tryMoveInItem consumes the input even when it exceeded that limit.
                *target = Some(ItemStack {
                    count: stack.count.min(max),
                    ..stack.clone()
                });
                stack.count = if restricted { stack.count.saturating_sub(max) } else { 0 };
                success = true;
                filled_empty = true;
            }
            Some(current)
                // HopperBlockEntity.canMergeItems
                if current.count <= max && current.same_item_same_components(&stack) =>
            {
                let space = max.saturating_sub(current.count);
                let count = stack.count.min(space);
                stack.count -= count;
                current.count += count;
                success = count > 0;
            }
            Some(_) => {}
        }
        // tryMoveInItem's setItem on an empty slot: AbstractFurnaceBlockEntity.setItem restarts
        // the cooking for a new input (a merge only grows the stack, and keeps it).
        let recipes = world_recipes!(self);
        if filled_empty
            && let Some((pos, 0)) = view.locate(slot)
            && let Some(BlockEntity::Furnace(furnace)) = self.world.block_entities.get_mut(&pos)
        {
            use horizon_client::furnace::FurnaceCooking;
            let input = furnace.items[0].take();
            furnace.set_input(&self.loop_context, recipes, input);
        }
        if success && was_empty && view.kind == Kind::Hopper {
            let dest = view.parts[0].0;
            let from_ticked = from_hopper.and_then(|p| self.hoppers.get(&p).map(|h| h.ticked));
            let state = self.hoppers.entry(dest).or_default();
            if state.cooldown <= 8 {
                let skip = i32::from(from_ticked.is_some_and(|t| state.ticked >= t));
                state.cooldown = 8 - skip;
            }
        }
        ((stack.count > 0).then_some(stack), success)
    }
    /// `HopperBlockEntity.addItem(from, container, stack, direction)`; the remainder.
    fn add_item(
        &mut self,
        from_hopper: Option<Pos>,
        view: &View,
        stack: ItemStack,
        face: Option<Direction>,
    ) -> Option<ItemStack> {
        if let Kind::Composter(_) = view.kind {
            let (x, y, z) = view.parts[0].0;
            return self.composter_container_insert([x, y, z], stack, face == Some(Direction::Up));
        }
        let mut rest = Some(stack);
        for slot in view.slots_to_add(face) {
            let Some(stack) = rest.take() else { break };
            rest = self.try_move_in_item(from_hopper, view, stack, slot);
        }
        rest
    }
    /// Remove one item from a slot (`removeItem(slot, 1)`).
    fn take_one(&mut self, view: &View, slot: usize) -> Option<ItemStack> {
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut().filter(|s| s.count > 0)?;
        let one = ItemStack {
            count: 1,
            ..current.clone()
        };
        current.count -= 1;
        if current.count == 0 {
            *target = None;
        }
        Some(one)
    }
    /// Undo `take_one` after a failed move.
    fn put_back(&mut self, view: &View, slot: usize, one: ItemStack) {
        if let Some(target) = self.slot_mut_at(view, slot) {
            match target {
                Some(current) => current.count += one.count,
                None => *target = Some(one),
            }
        }
    }
    /// `HopperBlockEntity.pushItemsTick` of the hopper at `pos`, if its block
    /// is still a hopper (`BlockEntityType.isValid`).
    pub(super) fn tick_hopper(&mut self, pos: Pos) {
        let time = self.world.game_time;
        let waiting = {
            let state = self.hoppers.entry(pos).or_default();
            state.cooldown = state.cooldown.wrapping_sub(1);
            state.ticked = time;
            state.cooldown > 0
        };
        if waiting {
            return;
        }
        // Most ticks only advance the cooldown: the block is checked when the
        // hopper is about to move, not on every tick it waits.
        let hopper = horizon_client::block_entity_types::factory_block(
            &self
                .res
                .facts()
                .get(self.world.block(pos.0, pos.1, pos.2))
                .name,
        ) == "minecraft:hopper";
        if !hopper {
            self.hoppers.remove(&pos);
            return;
        }
        if let Some(state) = self.hoppers.get_mut(&pos) {
            state.cooldown = 0;
        }
        self.hopper_try_move_items(pos);
    }
    /// Drops the cooldowns of hoppers whose block entity is gone.
    pub(super) fn forget_gone_hoppers(&mut self) {
        let entities = &self.world.block_entities;
        self.hoppers.retain(|p, _| {
            matches!(entities.get(p), Some(BlockEntity::Chest(items)) if items.len() == HOPPER_SLOTS)
        });
    }
    /// `HopperBlockEntity.tryMoveItems` with `suckInItems`.
    pub(super) fn hopper_try_move_items(&mut self, pos: Pos) {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        let enabled = c.prop(state, "enabled") != Some("false");
        let Some(view) = c.view(pos, false) else {
            return;
        };
        if self.hoppers.get(&pos).is_some_and(|h| h.cooldown > 0) || !enabled {
            return;
        }
        self.native_prepare_hopper_transfer([pos.0, pos.1, pos.2]);
        let c = self.containers();
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        let c = self.containers();
        let full =
            (0..view.len()).all(|i| c.get(&view, i).is_some_and(|s| s.count == c.max_stack(s)));
        if !full {
            changed |= self.hopper_suck(pos, &view);
        }
        if changed {
            self.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
    }
    /// `HopperBlockEntity.entityInside` for the item entities that moved this
    /// tick: an item whose box meets the hopper's bowl (`Hopper.INSIDE`) and
    /// its suck area runs `tryMoveItems` with that item as the pickup, in the
    /// entity phase, before the hopper's own tick counts its cooldown down.
    pub(super) fn hoppers_catch_items(&mut self) {
        let mut i = 0;
        while i < self.dropped.len() {
            let item = &self.dropped[i];
            // `ItemEntity.tick` moves 1 of 4 ticks while resting;
            // `checkInsideBlocks` only follows a move.
            let moved = item.position != item.previous
                || item.ticks.wrapping_add(item.id).is_multiple_of(4);
            if !moved || item.stack.count == 0 {
                i += 1;
                continue;
            }
            let b = item.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let mut found = None;
            'blocks: for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                        let state = self.world.block(x, y, z);
                        if horizon_client::block_entity_types::factory_block(
                            &self.res.facts().get(state).name,
                        ) != "minecraft:hopper"
                        {
                            continue;
                        }
                        // Hopper.INSIDE: the bowl, 2..14 wide and from 11/16 up.
                        let (fx, fy, fz) = (f64::from(x), f64::from(y), f64::from(z));
                        if lo.x < fx + 0.875
                            && hi.x > fx + 0.125
                            && lo.y < fy + 1.0
                            && hi.y > fy + 11.0 / 16.0
                            && lo.z < fz + 0.875
                            && hi.z > fz + 0.125
                        {
                            found = Some((x, y, z));
                            break 'blocks;
                        }
                    }
                }
            }
            let Some(pos) = found else {
                i += 1;
                continue;
            };
            if self.hopper_take_item(pos, i) {
                continue; // the item left the list; `i` is the next one
            }
            i += 1;
        }
    }
    /// `tryMoveItems(…, () -> addItem(hopper, itemEntity))`; true when the
    /// item entity was taken whole.
    fn hopper_take_item(&mut self, pos: Pos, i: usize) -> bool {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        let Some(view) = c.view(pos, false) else {
            return false;
        };
        if c.prop(state, "enabled") == Some("false")
            || self.hoppers.get(&pos).is_some_and(|h| h.cooldown > 0)
        {
            return false;
        }
        let mut changed = false;
        if !c.is_empty(&view) {
            changed = self.hopper_eject(pos, &view);
        }
        let c = self.containers();
        let full =
            (0..view.len()).all(|n| c.get(&view, n).is_some_and(|s| s.count == c.max_stack(s)));
        let mut taken = false;
        if !full {
            let stack = self.dropped[i].stack.clone();
            let allowed = self.with_level(|_, level, _| {
                level.allow_hopper_pickup_item(
                    BlockPos::new(pos.0, pos.1, pos.2),
                    &stack.item,
                    stack.count,
                )
            });
            match allowed
                .then(|| self.add_item(None, &view, stack, None))
                .flatten()
            {
                None if !allowed => {}
                None => {
                    self.remove_dropped(i);
                    taken = true;
                    changed = true;
                }
                Some(rest) => self.dropped[i].stack = rest,
            }
        }
        if changed {
            self.hoppers.entry(pos).or_default().cooldown = 8;
            self.container_changed(pos);
        }
        taken
    }
    /// The item handler a mod's provider gives for the mod block entity at `pos` through
    /// `side`, where it comes before the game's own slots or the game has none there. With no
    /// item provider registered, one table check.
    pub(super) fn item_part_at(
        &mut self,
        pos: Pos,
        side: Direction,
    ) -> Option<Capability<dyn ResourceHandler<ItemResource>>> {
        let at = BlockSide {
            level: self.native_level_here()?,
            pos: [pos.0, pos.1, pos.2],
            side: Some(side),
        };
        let providers = mod_api::block_entity_providers(&mod_api::ITEMS, self, at)?;
        let game = self.container_size(at).is_some();
        providers.ask(self, game)
    }
    /// Offers `stack` to a capability part and, in the same transaction, `take`s what it took
    /// from where it came, after the part's own writes, so a part handing it on back into the
    /// source loses nothing: how many went. A part taking none, a part answering more than it
    /// was offered (a mod bug) and a failed `take` undo the move (none went).
    pub(super) fn put_into_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
        stack: &ItemStack,
        take: impl FnOnce(&mut Self, u32) -> Option<()>,
    ) -> u32 {
        use super::api_values::TryIntoApi;
        let Ok(snapshot) = stack.try_into_api() else {
            return 0;
        };
        let resource = ItemResource::of(&snapshot);
        mod_api::run_transaction(self, |game, tx| {
            let host: &mut dyn mod_api::ServerHost = game;
            let went = part.insert_any(host, tx, &resource, stack.count);
            match part.checked(went, stack.count, &at) {
                Some(0) | None => Err(()),
                Some(went) => take(game, went).map(|()| went).ok_or(()),
            }
        })
        .unwrap_or(0)
    }
    /// Takes one item from `slot` of `view`, as it is now, journaled for the transaction.
    fn take_one_journaled(&mut self, view: &View, slot: usize) -> Option<()> {
        for &(pos, _) in &view.parts {
            self.journal_block_entity(pos);
        }
        self.take_one(view, slot).map(drop)
    }
    /// `InventoryMoveItemEvent` for `item` going from `source` to `destination`, where one end
    /// is a capability part: whether a mod canceled the move.
    fn part_move_canceled(&mut self, source: [i32; 3], destination: [i32; 3], item: &str) -> bool {
        let mut cancel = false;
        self.native_inventory_move_item(source, destination, item, &mut cancel);
        cancel
    }
    /// `ejectItems` into a capability part, looked up through the side it faces: the first hopper item
    /// one of its slots takes, one item, as a hopper moves, in a transaction. A part taking
    /// more than the one is a mod bug, and the move is undone.
    fn eject_into_part(
        &mut self,
        hopper: &View,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
    ) -> bool {
        let from = self.container_world_pos(hopper);
        for slot in 0..hopper.len() {
            let Some(stack) = self.containers().get(hopper, slot).cloned() else {
                continue;
            };
            if self.part_move_canceled(from, at, &stack.item) {
                continue;
            }
            let one = ItemStack { count: 1, ..stack };
            if self.put_into_part(part, at, &one, |game, _| {
                game.take_one_journaled(hopper, slot)
            }) == 1
            {
                return true;
            }
        }
        false
    }
    /// `suckInItems` from a capability part through its bottom: one item from the first of its
    /// slots that gives one the hopper has room for, in a transaction, as `eject_into_part`.
    fn suck_from_part(
        &mut self,
        part: &Capability<dyn ResourceHandler<ItemResource>>,
        at: [i32; 3],
        hopper: &View,
    ) -> bool {
        let into = self.container_world_pos(hopper);
        for slot in 0..part.size(self) {
            let Some(resource) = part.resource(self, slot) else {
                continue;
            };
            use super::api_values::TryIntoEngine;
            let Ok(one) = resource.as_stack().try_into_engine(self.items()) else {
                continue;
            };
            let c = self.containers();
            if !(0..hopper.len()).any(|s| c.fits(hopper, s, &one) > 0)
                || self.part_move_canceled(at, into, &one.item)
            {
                continue;
            }
            let moved = mod_api::run_transaction(self, |game, tx| {
                let host: &mut dyn mod_api::ServerHost = game;
                let came = part.extract(host, tx, slot, &resource, 1);
                if part.checked(came, 1, &at) != Some(1) {
                    return Err(());
                }
                for &(pos, _) in &hopper.parts {
                    game.journal_block_entity(pos);
                }
                game.add_item(None, hopper, one, None)
                    .map_or(Ok(()), |_| Err(()))
            });
            if moved.is_ok() {
                return true;
            }
        }
        false
    }
    /// `HopperBlockEntity.ejectItems`: into the block container in front, else
    /// a container entity there (`getContainerAt`). A capability part in front takes the
    /// container's place.
    fn hopper_eject(&mut self, pos: Pos, hopper: &View) -> bool {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let c = self.containers();
        let Some(facing) = c.prop(state, "facing").and_then(dir_from_name) else {
            return false;
        };
        let front = offset(pos, facing);
        let face = facing.opposite();
        if let Some(part) = self.item_part_at(front, face) {
            let at = [front.0, front.1, front.2];
            self.native_hopper_inventory_search([pos.0, pos.1, pos.2], Some(at), None);
            return self.eject_into_part(hopper, &part, at);
        }
        if let Some(target) = self.containers().view(front, false) {
            self.native_hopper_inventory_search(
                [pos.0, pos.1, pos.2],
                Some([front.0, front.1, front.2]),
                None,
            );
            return self.eject_into(pos, hopper, &target, face);
        }
        let Some(found) = self.automation_entity_at(
            [
                f64::from(front.0) + 0.5,
                f64::from(front.1) + 0.5,
                f64::from(front.2) + 0.5,
            ],
            face,
        ) else {
            return false;
        };
        match found {
            AutomationEntity::Container(id) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(id));
                self.with_entity_container(id, |g, target| g.eject_into(pos, hopper, target, face))
                    .unwrap_or(false)
            }
            AutomationEntity::Part(entity, part) => {
                self.native_hopper_inventory_search([pos.0, pos.1, pos.2], None, Some(entity.id()));
                self.eject_into_part(hopper, &part, [front.0, front.1, front.2])
            }
        }
    }
    /// Where a container is in the world for the mods: its block, or the block a chest
    /// boat or minecart is in (its view sits under a placeholder key).
    fn container_world_pos(&self, view: &View) -> [i32; 3] {
        let (x, y, z) = view.parts[0].0;
        if x == i32::MAX - 15 && y == super::minecarts::CART_KEY_Y {
            let id = z as u32;
            let at = self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.position)
                .or_else(|| self.boats.iter().find(|b| b.id == id).map(|b| b.position));
            if let Some(p) = at {
                return [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            }
        }
        [x, y, z]
    }
    /// `InventoryMoveItemEvent` for the item in `slot` of `holder` going from `source` to
    /// `destination`: whether a mod canceled the move. Nothing is looked up with no mod loaded.
    fn hopper_move_canceled(
        &self,
        holder: &View,
        slot: usize,
        source: &View,
        destination: &View,
    ) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
            return false;
        }
        let containers = self.containers();
        let Some(stack) = containers.get(holder, slot) else {
            return false;
        };
        let mut cancel = false;
        self.native_inventory_move_item(
            self.container_world_pos(source),
            self.container_world_pos(destination),
            &stack.item,
            &mut cancel,
        );
        cancel
    }
    /// The rest of `HopperBlockEntity.ejectItems`: one item into `target`.
    fn eject_into(&mut self, pos: Pos, hopper: &View, target: &View, face: Direction) -> bool {
        if self.containers().is_full(target, face) {
            return false;
        }
        for slot in 0..hopper.len() {
            if self.hopper_move_canceled(hopper, slot, hopper, target) {
                continue;
            }
            let Some(one) = self.take_one(hopper, slot) else {
                continue;
            };
            match self.add_item(Some(pos), target, one, Some(face)) {
                None => {
                    self.view_changed(target);
                    return true;
                }
                Some(rest) => self.put_back(hopper, slot, rest),
            }
        }
        false
    }
    /// `HopperBlockEntity.suckInItems`: from the container above, or item entities.
    fn hopper_suck(&mut self, pos: Pos, hopper: &View) -> bool {
        let above = (pos.0, pos.1 + 1, pos.2);
        let at = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 1.5,
            f64::from(pos.2) + 0.5,
        ];
        if let Some(took) = self.suck_from_container(above, at, hopper) {
            return took;
        }
        let above_state = self.world.block(above.0, above.1, above.2);
        let blocked = self.res.facts().get(above_state).full_collision
            && !self
                .res
                .behaviours()
                .tag("minecraft:does_not_block_hoppers")
                .is_some_and(|t| t.contains(self.res.behaviours().block(above_state)));
        if blocked {
            return false;
        }
        // Hopper.SUCK_AABB: the hopper bowl and the block above.
        let (min, max) = (
            [
                f64::from(pos.0),
                f64::from(pos.1) + 11.0 / 16.0,
                f64::from(pos.2),
            ],
            [
                f64::from(pos.0) + 1.0,
                f64::from(pos.1) + 2.0,
                f64::from(pos.2) + 1.0,
            ],
        );
        if let Some(columns) = self.item_columns.as_mut()
            && !columns.may_reach(&self.dropped, pos.0, pos.2)
        {
            return false;
        }
        self.suck_item_entities(Some(pos), hopper, min, max)
    }
    /// The container half of `HopperBlockEntity.suckInItems`: one item from the
    /// container at `above`, through its bottom face; with no block container
    /// there, from a container entity whose box meets the one around `at`
    /// (`getSourceContainerOrHandler`). `None` when there is no container. A capability part
    /// above takes the container's place.
    fn suck_from_container(&mut self, above: Pos, at: [f64; 3], hopper: &View) -> Option<bool> {
        if let Some(part) = self.item_part_at(above, Direction::Down) {
            let (x, y, z) = hopper.parts[0].0;
            let above = [above.0, above.1, above.2];
            self.native_hopper_inventory_search([x, y, z], Some(above), None);
            return Some(self.suck_from_part(&part, above, hopper));
        }
        let c = self.containers();
        let Some(source) = c.view(above, false) else {
            let found = self.automation_entity_at(at, Direction::Down)?;
            let (x, y, z) = hopper.parts[0].0;
            return match found {
                AutomationEntity::Container(id) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(id));
                    self.with_entity_container(id, |g, source| {
                        g.suck_from_slots(None, source, hopper)
                    })
                }
                AutomationEntity::Part(entity, part) => {
                    self.native_hopper_inventory_search([x, y, z], None, Some(entity.id()));
                    Some(self.suck_from_part(&part, [above.0, above.1, above.2], hopper))
                }
            };
        };
        self.native_hopper_inventory_search(
            [
                hopper.parts[0].0.0,
                hopper.parts[0].0.1,
                hopper.parts[0].0.2,
            ],
            Some([above.0, above.1, above.2]),
            None,
        );
        let c = self.containers();
        if let Kind::Composter(level) = source.kind {
            if level != 8 {
                return Some(false);
            }
            // tryTakeInItemFromSlot on the OutputContainer: removeItem calls
            // its setChanged, which empties the composter, and so does the
            // second setChanged after the move or, when the hopper has no
            // room, the setItem putting the bone meal back into the detached
            // container (whose bone meal is lost with it).
            let bone_meal = c.bone_meal.clone();
            if mod_api::__private::listened(mod_api::HookId::InventoryMoveItem) {
                let mut cancel = false;
                self.native_inventory_move_item(
                    [above.0, above.1, above.2],
                    self.container_world_pos(hopper),
                    &bone_meal.item,
                    &mut cancel,
                );
                if cancel {
                    return Some(false);
                }
            }
            let at = BlockPos::new(above.0, above.1, above.2);
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            let moved = self.add_item(None, hopper, bone_meal, None).is_none();
            self.with_level(|b, level, ticks| b.composter_empty(level, ticks, at));
            self.world.modified.insert((above.0 >> 4, above.2 >> 4));
            return Some(moved);
        }
        let source_hopper = (source.kind == Kind::Hopper).then_some(above);
        Some(self.suck_from_slots(source_hopper, &source, hopper))
    }
    /// `HopperBlockEntity.tryTakeInItemFromSlot` over the `DOWN` face of `source`.
    fn suck_from_slots(
        &mut self,
        source_hopper: Option<Pos>,
        source: &View,
        hopper: &View,
    ) -> bool {
        for slot in source.slots_for_face(Some(Direction::Down)) {
            let c = self.containers();
            let Some(stack) = c.get(source, slot) else {
                continue;
            };
            if !c.can_take(source, slot, stack, Direction::Down) {
                continue;
            }
            if self.hopper_move_canceled(source, slot, source, hopper) {
                continue;
            }
            let Some(one) = self.take_one(source, slot) else {
                continue;
            };
            match self.add_item(source_hopper, hopper, one, None) {
                None => {
                    self.view_changed(source);
                    // JukeboxBlockEntity.removeItem -> setTheItem(EMPTY): a
                    // disc a hopper takes out stops the song.
                    if source.kind == Kind::Jukebox {
                        let (x, y, z) = source.parts[0].0;
                        self.jukebox_taken([x, y, z]);
                    }
                    return true;
                }
                Some(rest) => self.put_back(source, slot, rest),
            }
        }
        false
    }
    /// The alive container entities (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and
    /// hopper minecarts, chest boats) whose box meets the 1x1x1 box around `at`.
    fn container_entities_at(&self, at: [f64; 3]) -> Vec<u32> {
        let meets = |b: horizon_core::geometry::Aabb| {
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < at[0] + 0.5
                && hi.x > at[0] - 0.5
                && lo.y < at[1] + 0.5
                && hi.y > at[1] - 0.5
                && lo.z < at[2] + 0.5
                && hi.z > at[2] - 0.5
        };
        let mut found: Vec<u32> = self
            .carts
            .iter()
            .filter(|c| {
                matches!(
                    c.kind,
                    super::minecarts::CartKind::Chest | super::minecarts::CartKind::Hopper
                ) && meets(c.bounds())
            })
            .map(|c| c.id)
            .collect();
        found.extend(
            self.boats
                .iter()
                .filter(|b| b.chest && meets(b.bounds()))
                .map(|b| b.id),
        );
        found
    }
    /// The index `RandomSource.nextInt(len)` picks of `len` candidates: the level's one draw,
    /// none for no candidate.
    fn pick_index(&mut self, len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        let pick = self.world.random.next_int_bound(len as i32).unwrap_or(0);
        Some(pick as usize)
    }
    /// The entity item automation reaches around `at` from `side`, through `ENTITY_AUTOMATION`: one of
    /// the container entities the game has there, and, once a mod registered a provider for it,
    /// the entities whose mod provider answers it, picked at random as `getEntityContainer` picks, the one draw.
    /// A game container entity a mod provider answers ahead of the game is reached through
    /// that answer. With no mod providing `ENTITY_AUTOMATION`, only vanilla containers are
    /// considered: nothing more is looked up.
    fn automation_entity_at(&mut self, at: [f64; 3], side: Direction) -> Option<AutomationEntity> {
        let vanilla = self.container_entities_at(at);
        if !mod_api::has_entity_providers(&ENTITY_AUTOMATION) {
            let pick = self.pick_index(vanilla.len())?;
            return Some(AutomationEntity::Container(vanilla[pick]));
        }
        let mut found: Vec<AutomationEntity> = Vec::new();
        let context = Some(side);
        let ask = |game: &mut Self, entity: EntityRef, has_game_answer: bool| {
            let providers = mod_api::entity_providers(&ENTITY_AUTOMATION, &*game, entity, context)?;
            providers.ask(game, has_game_answer)
        };
        for &id in &vanilla {
            let entity = self.entity(id);
            match entity.and_then(|entity| Some((entity, ask(self, entity, true)?))) {
                Some((entity, part)) => found.push(AutomationEntity::Part(entity, part)),
                None => found.push(AutomationEntity::Container(id)),
            }
        }
        if let Some(level) = self.native_level_here() {
            let min = [at[0] - 0.5, at[1] - 0.5, at[2] - 0.5];
            let max = [at[0] + 0.5, at[1] + 0.5, at[2] + 0.5];
            for entity in self.level_entities_in(level, min, max).unwrap_or_default() {
                if !vanilla.contains(&entity.id())
                    && let Some(part) = ask(self, entity, false)
                {
                    found.push(AutomationEntity::Part(entity, part));
                }
            }
        }
        let pick = self.pick_index(found.len())?;
        Some(found.swap_remove(pick))
    }
    /// Runs `f` on the block container at `pos`, or one entity there, with providers taking
    /// the game's slots' place when registered ahead of them.
    fn with_container_at<R>(
        &mut self,
        pos: Pos,
        side: Direction,
        f: impl FnOnce(&mut Self, ContainerTarget<'_>) -> R,
    ) -> Option<R> {
        if let Some(part) = self.item_part_at(pos, side) {
            return Some(f(self, ContainerTarget::Part(part)));
        }
        if let Some(view) = self.containers().view(pos, false) {
            return Some(f(self, ContainerTarget::View(&view)));
        }
        let centre = [
            f64::from(pos.0) + 0.5,
            f64::from(pos.1) + 0.5,
            f64::from(pos.2) + 0.5,
        ];
        match self.automation_entity_at(centre, side)? {
            AutomationEntity::Container(id) => {
                self.with_entity_container(id, |game, view| f(game, ContainerTarget::View(view)))
            }
            AutomationEntity::Part(_, part) => Some(f(self, ContainerTarget::Part(part))),
        }
    }
    /// Runs `f` on a container entity's items as a container, under the
    /// placeholder key its menu uses (the open menu's copy when it is open),
    /// then hands the items back to the entity.
    fn with_entity_container<R>(
        &mut self,
        id: u32,
        f: impl FnOnce(&mut Self, &View) -> R,
    ) -> Option<R> {
        let mut items = if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.carts.update(i, |c| std::mem::take(&mut c.items))
        } else {
            let i = self.boats.iter().position(|b| b.id == id)?;
            self.boats.update(i, |b| std::mem::take(&mut b.items))
        };
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
        let (view, open) = self.lend_items(key, &mut items);
        let result = f(self, &view);
        self.reclaim_items(key, open, &mut items);
        if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.carts.update(i, |c| c.items = items);
        } else if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.boats.update(i, |b| b.items = items);
        }
        Some(result)
    }
    /// A vehicle's slots as a container under its placeholder `key`. When its
    /// menu is open the placeholder is the live copy and stays as it is.
    /// Returns the view and whether the menu was open.
    fn lend_items(&mut self, key: Pos, items: &mut Vec<Option<ItemStack>>) -> (View, bool) {
        let open = self.world.block_entities.contains_key(&key);
        if !open {
            let slots = BlockEntity::Chest(std::mem::take(items));
            self.world.block_entities.insert(key, slots);
        }
        let size = self
            .world
            .block_entities
            .get(&key)
            .map_or(0, |e| e.slots().len());
        let view = View {
            parts: vec![(key, size)],
            kind: Kind::Generic,
            container: None,
        };
        (view, open)
    }
    /// Gives `lend_items`' slots back to the vehicle, so its own items stay
    /// the one source of truth: a menu copy that is still around (open, or
    /// left by a guest's close) gets the changed items copied to `items`.
    fn reclaim_items(&mut self, key: Pos, open: bool, items: &mut Vec<Option<ItemStack>>) {
        let slots = if open {
            self.world.block_entities.get(&key).cloned()
        } else {
            self.world.block_entities.remove(&key)
        };
        if let Some(BlockEntity::Chest(slots)) = slots {
            *items = slots;
        }
    }
    /// The item half of `HopperBlockEntity.suckInItems` (`getItemsAtAndAbove`):
    /// adds the item entities that meet the box, until one is taken whole.
    fn suck_item_entities(
        &mut self,
        hopper_pos: Option<Pos>,
        hopper: &View,
        min: [f64; 3],
        max: [f64; 3],
    ) -> bool {
        let mut i = 0;
        while i < self.dropped.len() {
            let b = self.dropped[i].bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let inside = lo.x < max[0]
                && hi.x > min[0]
                && lo.y < max[1]
                && hi.y > min[1]
                && lo.z < max[2]
                && hi.z > min[2];
            if inside {
                let stack = self.dropped[i].stack.clone();
                let allowed = hopper_pos.is_none_or(|pos| {
                    self.with_level(|_, level, _| {
                        level.allow_hopper_pickup_item(
                            BlockPos::new(pos.0, pos.1, pos.2),
                            &stack.item,
                            stack.count,
                        )
                    })
                });
                if !allowed {
                    i += 1;
                    continue;
                }
                match self.add_item(None, hopper, stack, None) {
                    None => {
                        self.remove_dropped(i);
                        return true;
                    }
                    Some(rest) => self.dropped[i].stack = rest,
                }
            }
            i += 1;
        }
        false
    }
    /// `suck_item_entities` for a hopper minecart, which first asks the item
    /// columns whether any item's box reaches the columns of the box: an item
    /// that meets it reaches one of them. The columns follow the item list's
    /// stamp, so any write since they were built (an item's move, an
    /// explosion's push, a command, a pickup) makes them scan, then rebuild.
    fn cart_suck_items(&mut self, hopper: &View, min: [f64; 3], max: [f64; 3]) -> bool {
        if !self
            .pickup_columns
            .get_or_insert_with(Default::default)
            .may_reach_area(
                &self.dropped,
                (min[0].floor() as i32, max[0].floor() as i32),
                (min[2].floor() as i32, max[2].floor() as i32),
            )
        {
            return false;
        }
        self.suck_item_entities(None, hopper, min, max)
    }
    /// `MinecartHopper.tick`'s `tryConsumeItems` (`suckInItems`) for an enabled
    /// hopper minecart: one item from the container above, else the item
    /// entities over it or touching it, each tick without a cooldown.
    /// `consumedItemThisFrame` in 26.3 caps a tick at one item: the tick tries
    /// once, and `makeStepAlongTrack` may try again, but only one succeeds.
    pub(super) fn hopper_cart_suck(&mut self, cart: &mut super::minecarts::Minecart) {
        // The cart's slots are edited as a container under the placeholder key
        // its menu uses; when that menu is open the placeholder is the live copy.
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, cart.id as i32);
        let (view, open) = self.lend_items(key, &mut cart.items);
        let p = cart.position;
        // getLevelY is the cart's y + 0.5, so the container is the block at
        // y + 1.5, and Hopper.SUCK_AABB (moved by getLevel - 0.5) starts 11/16
        // above the cart.
        let above = (
            p.x.floor() as i32,
            (p.y + 1.5).floor() as i32,
            p.z.floor() as i32,
        );
        let at = [p.x, p.y + 1.5, p.z];
        let took = match self.suck_from_container(above, at, &view) {
            Some(took) => took,
            None => self.cart_suck_items(
                &view,
                [p.x - 0.5, p.y + 11.0 / 16.0, p.z - 0.5],
                [p.x + 0.5, p.y + 2.0, p.z + 0.5],
            ),
        };
        if !took {
            // The cart's own box, inflated by 0.25 sideways.
            let b = cart.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            self.cart_suck_items(
                &view,
                [lo.x - 0.25, lo.y, lo.z - 0.25],
                [hi.x + 0.25, hi.y, hi.z + 0.25],
            );
        }
        self.reclaim_items(key, open, &mut cart.items);
    }

    // ---- dispensers and droppers ----

    /// `DispenserBlock.dispenseFrom` / `DropperBlock.dispenseFrom`.
    pub(super) fn dispense_from(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let name = self.res.facts().get(state).name.clone();
        if &*name == "minecraft:crafter" {
            return self.crafter_dispense(pos, state);
        }
        let dropper = &*name == "minecraft:dropper";
        if !dropper && &*name != "minecraft:dispenser" {
            return Ok(());
        }
        let Some(view) = self.containers().view(key, false) else {
            return Ok(());
        };
        // DispenserBlockEntity.getRandomSlot
        let mut slot = None;
        let mut odds = 1;
        for i in 0..view.len() {
            if self.containers().get(&view, i).is_some() {
                if self.world.random.next_int_bound(odds).unwrap_or(1) == 0 {
                    slot = Some(i);
                }
                odds += 1;
            }
        }
        let Some(slot) = slot else {
            self.dispenser_event(1001, key, 0);
            if !dropper {
                self.post_block_game_event(
                    "minecraft:block_activate",
                    [pos.x, pos.y, pos.z],
                    None,
                    Some(state),
                );
            }
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "facing")
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        let Some(mut stack) = self.containers().get(&view, slot).cloned() else {
            return Ok(());
        };
        let target = offset(key, facing);
        let remaining = if dropper {
            let side = facing.opposite();
            let inserted = self.with_container_at(target, side, |game, into| match into {
                ContainerTarget::View(into) => {
                    let one = ItemStack {
                        count: 1,
                        ..stack.clone()
                    };
                    let inserted = game.add_item(None, into, one, Some(side)).is_none();
                    if inserted {
                        game.view_changed(into);
                    }
                    ControlFlow::Continue(inserted)
                }
                ContainerTarget::Part(part) => {
                    // `dropperInsertHook`: the handler's writes and taking one from the
                    // live source slot share a transaction, including feedback into it.
                    stack.count = 1;
                    game.put_into_part(&part, [target.0, target.1, target.2], &stack, |game, _| {
                        game.take_one_journaled(&view, slot)
                    });
                    ControlFlow::Break(())
                }
            });
            match inserted {
                Some(ControlFlow::Break(())) => {
                    self.container_changed(key);
                    return Ok(());
                }
                Some(ControlFlow::Continue(inserted)) => Some(ItemStack {
                    count: stack.count - u32::from(inserted),
                    ..stack
                })
                .filter(|s| s.count > 0),
                None => self.dispense_default_in_level(key, facing, stack),
            }
        } else {
            self.dispense_item(key, facing, stack)?
        };
        if let Some(entry) = self.slot_mut_at(&view, slot) {
            *entry = remaining;
        }
        self.container_changed(key);
        Ok(())
    }
}

// ---- hopper, dispenser and dropper screens ----

/// Which machine screen the open container uses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Hopper,
    Dispenser,
    Dropper,
}

impl Game<'_> {
    fn machine_screen(&self) -> Option<Screen> {
        let key = self.container?;
        // A mod's screen over a hopper or dispenser menu lays out its own slots.
        if self.native_screen_inventory_y().is_some() {
            return None;
        }
        // A server's menu shows by its type, whatever block (or minecart) holds it.
        if let Some(menu) = self.remote_menu_type() {
            return match menu {
                "minecraft:hopper" => Some(Screen::Hopper),
                "minecraft:generic_3x3" => Some(Screen::Dispenser),
                _ => None,
            };
        }
        if key.1 == super::minecarts::CART_KEY_Y {
            return (self.open_cart_kind() == Some(super::minecarts::CartKind::Hopper))
                .then_some(Screen::Hopper);
        }
        let state = self.world.block(key.0, key.1, key.2);
        match &*self.res.facts().get(state).name {
            "minecraft:hopper" => Some(Screen::Hopper),
            "minecraft:dispenser" => Some(Screen::Dispenser),
            "minecraft:dropper" => Some(Screen::Dropper),
            _ => None,
        }
    }
    /// `imageHeight` of the open screen (HopperScreen is 133 tall).
    pub(super) fn screen_height(&self) -> f32 {
        if let Some(height) = self.native_screen_height() {
            return height;
        }
        if self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == super::stations::Kind::Beacon)
        {
            return super::beacon::HEIGHT;
        }
        if let Some(rows) = self.chest_rows() {
            // ContainerScreen: 114 + 18 per row.
            return 114.0 + rows as f32 * 18.0;
        }
        if self.container_block_is_shulker() {
            return 167.0;
        }
        if self.machine_screen() == Some(Screen::Hopper) {
            133.0
        } else {
            166.0
        }
    }
    /// HopperMenu / DispenserMenu slot layouts; None for other screens.
    pub(super) fn machine_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let screen = self.machine_screen()?;
        let inventory_y = if screen == Screen::Hopper { 51.0 } else { 84.0 };
        Some(match i {
            0..9 => (8.0 + i as f32 * 18.0, inventory_y + 58.0),
            9..36 => (
                8.0 + ((i - 9) % 9) as f32 * 18.0,
                inventory_y + ((i - 9) / 9) as f32 * 18.0,
            ),
            300..500 if screen == Screen::Hopper => (44.0 + (i - 300) as f32 * 18.0, 20.0),
            300..500 => (
                62.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            ),
            _ => return None,
        })
    }
    /// HopperScreen / DispenserScreen background and labels; false for other containers.
    pub(super) fn draw_machine_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) -> bool {
        let Some(screen) = self.machine_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        let key_is_cart = self
            .container
            .is_some_and(|k| k.1 == super::minecarts::CART_KEY_Y);
        let (texture, height, key, fallback) = match screen {
            Screen::Hopper if key_is_cart => (
                "gui/container/hopper",
                133.0,
                "entity.minecraft.hopper_minecart",
                "Minecart with Hopper",
            ),
            Screen::Hopper => (
                "gui/container/hopper",
                133.0,
                "container.hopper",
                "Item Hopper",
            ),
            Screen::Dispenser => (
                "gui/container/dispenser",
                166.0,
                "container.dispenser",
                "Dispenser",
            ),
            Screen::Dropper => (
                "gui/container/dispenser",
                166.0,
                "container.dropper",
                "Dropper",
            ),
        };
        if let Some(mut r) = atlas.sprites.get(texture).copied() {
            let dv = (r.v1 - r.v0) / r.h as f32;
            let du = (r.u1 - r.u0) / r.w as f32;
            r.u1 = r.u0 + 176.0 * du;
            r.v1 = r.v0 + height * dv;
            batch.rect_uv(ox, oy, 176.0, height, r, u32::MAX);
        }
        let grey = horizon_client::hud::rgba(64, 64, 64, 255);
        let title = &self.remote_menu_title().unwrap_or_else(|| {
            horizon_client::text::Text::literal(
                self.container_title(self.translate_with_fallback(key, fallback)),
            )
            .resolve_legacy()
        });
        super::screen_info::name_container(|| title.plain());
        let title_x = if screen == Screen::Hopper {
            8.0
        } else {
            (176.0 - atlas.font.width_text(title)) / 2.0
        };
        batch.text_styled(atlas, title, ox + title_x, oy + 6.0, grey, false);
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + height - 94.0, grey, false);
        true
    }
}

#[cfg(test)]
mod brewing_fuel_transfer_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::brewing::BrewingStand;

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn hopper_brewing_fuel_transfer_respects_stack_components() {
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let hopper_pos = (8, 64, 8);
            let stand_pos = (9, 64, 8);
            for (pos, block) in [
                (hopper_pos, "minecraft:hopper"),
                (stand_pos, "minecraft:brewing_stand"),
            ] {
                let state = game.pack.assets.registry.default_state(block).unwrap();
                game.world.set(pos.0, pos.1, pos.2, state).unwrap();
            }
            let default = ItemStack::new("minecraft:blaze_powder", 2);
            let mut custom = ItemStack::new("minecraft:stone", 2);
            custom.data_mut().set_loot_component(
                "minecraft:brewing_fuel",
                &Some(
                    horizon_core::json_nbt::json_to_tag(
                        &serde_json::json!({"uses": 7, "speed_multiplier": 2.5}),
                    )
                    .unwrap(),
                ),
            );
            let mut removed = default.clone();
            removed.data_mut().set_other(
                "!minecraft:brewing_fuel",
                Some(horizon_core::nbt::Tag::Compound(Default::default())),
            );
            for (stack, accepted) in [
                (default, true),
                (custom, true),
                (removed, false),
                (ItemStack::new("minecraft:stone", 2), false),
            ] {
                game.world
                    .block_entities
                    .insert(hopper_pos, BlockEntity::Chest(vec![Some(stack); 5]));
                game.world.block_entities.insert(
                    stand_pos,
                    BlockEntity::Brewing(Box::new(BrewingStand::new())),
                );
                let containers = game.containers();
                let hopper = containers.view(hopper_pos, false).unwrap();
                let stand = containers.view(stand_pos, false).unwrap();
                let fuel = containers.get(&hopper, 0).unwrap();
                assert_eq!(containers.can_place(&stand, 4, fuel), accepted);
                assert_eq!(
                    game.eject_into(hopper_pos, &hopper, &stand, Direction::West),
                    accepted
                );
                let containers = game.containers();
                assert_eq!(
                    containers.get(&hopper, 0).unwrap().count,
                    if accepted { 1 } else { 2 }
                );
                assert_eq!(
                    containers.get(&stand, 4).map(|s| s.count),
                    accepted.then_some(1)
                );
            }
        });
    }
}

#[cfg(test)]
mod item_column_tests {
    use super::ItemColumns;
    use horizon_client::{entities::ItemEntity, items::ItemStack};
    use horizon_core::geometry::Vec3;

    fn item(id: u32, x: f64, y: f64, z: f64) -> ItemEntity {
        let at = Vec3::new(x, y, z);
        ItemEntity {
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: horizon_core::random::LegacyRandom::new(0),
            rotation: [0.0, 0.0],
            join: Default::default(),
            no_gravity: false,
            health: 5,
            fire_ticks: 0,
            id,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            target: None,
            position: at,
            previous: at,
            velocity: Vec3::ZERO,
            on_ground: true,
            vertical_collision: false,
            horizontal_collision: false,
            age: 0,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
        }
    }
    /// `hopper_suck`'s test: some item box meets the hopper's SUCK_AABB.
    fn scan(items: &[ItemEntity], (x, y, z): (i32, i32, i32)) -> bool {
        let (min, max) = (
            Vec3::new(f64::from(x), f64::from(y) + 11.0 / 16.0, f64::from(z)),
            Vec3::new(f64::from(x) + 1.0, f64::from(y) + 2.0, f64::from(z) + 1.0),
        );
        items.iter().any(|i| {
            let b = i.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            lo.x < max.x
                && hi.x > min.x
                && lo.y < max.y
                && hi.y > min.y
                && lo.z < max.z
                && hi.z > min.z
        })
    }

    /// An item moved in place (a `/tp`, an explosion's push) keeps the list's
    /// length and last id; the stamped columns still see where it went.
    #[test]
    fn a_moved_item_is_seen_where_it_went() {
        let mut items = horizon_client::join_seq::JoinList::<_, true>::from_iter([
            item(1, 0.5, 64.0, 0.5),
            item(2, 40.5, 64.0, 40.5),
        ]);
        let mut columns = super::StampedItemColumns::default();
        let far = ((100, 100), (100, 100));
        // The first question for a list scans; the second builds the columns.
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, far.0, far.1));
        items.update(0, |item| {
            item.position = horizon_core::geometry::Vec3::new(100.5, 64.0, 100.5)
        });
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(columns.may_reach_area(&items, far.0, far.1));
        assert!(!columns.may_reach_area(&items, (0, 0), (0, 0)));
    }

    #[test]
    fn no_player_with_an_item_in_reach_is_skipped() {
        // guest_pickups' horizontal test, on every item.
        let touches = |items: &[ItemEntity], p: [f64; 3]| {
            items
                .iter()
                .any(|d| (d.position.x - p[0]).abs() < 1.5 && (d.position.z - p[2]).abs() < 1.5)
        };
        let mut seed = 0x0bad_5eed_u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let (mut skipped, mut found) = (0, 0);
        for round in 0..4000u32 {
            if round % 25 == 0 {
                // On column edges as often as not.
                let mut at = || {
                    let v = next(40.0) - 20.0;
                    if next(1.0) < 0.5 { v.round() } else { v }
                };
                items.push(item(round, at(), 64.0, at()));
            } else if round % 101 == 0 && !items.is_empty() {
                items.remove(next(items.len() as f64) as usize);
            }
            let p = [next(44.0) - 22.0, 64.0, next(44.0) - 22.0];
            let reach = columns.may_reach_area(
                &items,
                ((p[0] - 1.5).floor() as i32, (p[0] + 1.5).floor() as i32),
                ((p[2] - 1.5).floor() as i32, (p[2] + 1.5).floor() as i32),
            );
            let touching = touches(&items, p);
            assert!(
                reach || !touching,
                "round {round}: skipped a player touching an item"
            );
            skipped += usize::from(!reach);
            found += usize::from(touching);
        }
        assert!(
            skipped > 1000 && found > 100,
            "skipped {skipped}, found {found}"
        );
    }

    #[test]
    fn no_hopper_with_an_item_over_it_is_skipped() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        let mut items: Vec<ItemEntity> = Vec::new();
        let mut columns = ItemColumns::default();
        let mut next_id = 0;
        let mut skipped = 0;
        for round in 0..3000 {
            // Items come and go as hoppers take them and crafters drop them.
            match round % 5 {
                0 | 1 => {
                    next_id += 1;
                    // On edges too: a quarter of them sit on a block line.
                    let snap = |v: f64, edge: bool| if edge { v.floor() + 0.125 } else { v };
                    let edge = round % 4 == 0;
                    items.push(item(
                        next_id,
                        snap(next(24.0) - 12.0, edge),
                        64.0,
                        snap(next(24.0) - 12.0, edge),
                    ));
                }
                2 if !items.is_empty() => {
                    let i = next(items.len() as f64) as usize;
                    items.remove(i);
                }
                _ => {}
            }
            let hopper = (next(26.0) as i32 - 13, 63, next(26.0) as i32 - 13);
            let reach = columns.may_reach(&items, hopper.0, hopper.2);
            let found = scan(&items, hopper);
            assert!(
                reach || !found,
                "round {round}: skipped a hopper with an item over it"
            );
            skipped += usize::from(!reach);
        }
        assert!(skipped > 1000, "skipped only {skipped}");
    }
}

#[cfg(test)]
mod mod_container_tests {
    use super::{Kind, ModContainer, View, room};
    use horizon_client::items::ItemStack;
    use horizon_core::geometry::Direction;
    use std::sync::Arc;

    /// A mod's container reaches the slots its block names for each face;
    /// without a face, every slot.
    #[test]
    fn mod_container_slots_by_face() {
        let mut faces: [Vec<usize>; 6] = Default::default();
        faces[Direction::Up as usize] = vec![0];
        faces[Direction::Down as usize] = vec![2];
        faces[Direction::North as usize] = vec![1];
        let view = View {
            parts: vec![((0, 0, 0), 3)],
            kind: Kind::Mod,
            container: Some(Arc::new(ModContainer {
                slots: 3,
                faces,
                inputs: vec![0, 1],
                outputs: vec![2],
                menu: None,
            })),
        };
        assert_eq!(view.slots_for_face(Some(Direction::Up)), [0]);
        assert_eq!(view.slots_for_face(Some(Direction::Down)), [2]);
        assert_eq!(view.slots_for_face(Some(Direction::North)), [1]);
        assert!(view.slots_for_face(Some(Direction::East)).is_empty());
        assert_eq!(view.slots_for_face(None), [0, 1, 2]);
    }

    /// A slot takes no more than its container's limit and the item's: one book into a chiseled
    /// bookshelf slot of 64 offered, a chest slot 64 and then the rest of 64.
    #[test]
    fn a_slot_takes_up_to_its_containers_limit() {
        let books = |count| ItemStack {
            item: "minecraft:book".into(),
            count,
            damage: 0,
            extra: None,
        };
        let limit = |kind| {
            let view = View {
                parts: vec![((0, 0, 0), 6)],
                kind,
                container: None,
            };
            view.slot_limit().min(64)
        };
        assert_eq!(room(limit(Kind::Bookshelf), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Jukebox), None, &books(64)), 1);
        assert_eq!(room(limit(Kind::Generic), None, &books(64)), 64);
        assert_eq!(room(limit(Kind::Generic), Some(&books(60)), &books(64)), 4);
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::{Containers, Kind, View, bone_meal};
    use horizon_client::{brewing::Brewing, containers::BlockEntity, items::ItemStack};
    use std::collections::HashSet;

    /// A one-item slot answers its capacity as if empty (NeoForge's `getCapacityAsLong`), so a
    /// write may swap what it holds, while it takes nothing more now: a jukebox's disc, a
    /// chiseled bookshelf's book and a brewing stand's bottle.
    #[test]
    fn a_full_one_item_slot_still_has_room_for_a_replacement() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("block-behaviours.json").exists() {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let registry = assets.registry;
        let facts = horizon_client::blocks::BlockFacts::load(&root, registry).unwrap();
        let overlay = &horizon_world::loot::PackOverlay::NONE;
        let defaults = horizon_world::item_defaults::read(&root).unwrap();
        let items = horizon_client::items::Items::load(&root, registry, &facts, overlay, &defaults)
            .unwrap();
        let behaviours = horizon_world::level::Behaviours::load(registry, &root).unwrap();
        let brewing = Brewing {
            recipes: Vec::new(),
            reagents: HashSet::new(),
            inputs: HashSet::from(["minecraft:potion".to_owned()]),
        };
        let air = registry.default_state("minecraft:air").unwrap();
        let mut world = horizon_client::world::World::new(air, 0);
        let cases = [
            (
                "minecraft:jukebox",
                Kind::Jukebox,
                1,
                "minecraft:music_disc_cat",
                "minecraft:music_disc_13",
            ),
            (
                "minecraft:chiseled_bookshelf",
                Kind::Bookshelf,
                6,
                "minecraft:book",
                "minecraft:enchanted_book",
            ),
            (
                "minecraft:brewing_stand",
                Kind::Brewing,
                5,
                "minecraft:potion",
                "minecraft:potion",
            ),
        ];
        for (x, &(block, _, _, held, _)) in (0..).zip(&cases) {
            let mut entity = BlockEntity::for_block(block).unwrap();
            entity.slots_mut()[0] = Some(ItemStack::new(held, 1));
            world.block_entities.insert((x, 64, 0), entity);
        }
        let c = Containers {
            world: &world,
            behaviours: &behaviours,
            items: &items,
            registry,
            brewing: Some(&brewing),
            bone_meal: bone_meal(),
        };
        for (x, &(block, kind, len, _, offered)) in (0..).zip(&cases) {
            let view = View {
                parts: vec![((x, 64, 0), len)],
                kind,
                container: None,
            };
            let offered = ItemStack::new(offered, 1);
            assert_eq!(c.capacity(&view, 0, &offered), 1, "{block}");
            assert_eq!(c.fits(&view, 0, &offered), 0, "{block}");
            assert_eq!(
                c.capacity(&view, 0, &ItemStack::new("minecraft:stick", 1)),
                0,
                "{block}"
            );
        }
    }
}

#[cfg(test)]
mod part_front_tests {
    use super::super::native_entity_refs::tests::{declared_part_root, with_game_at};
    use horizon_client::chunk::Chunk;
    use horizon_client::containers::BlockEntity;
    use horizon_client::items::ItemStack;
    use horizon_core::position::BlockPos;
    use mod_api::BlockUpdateFlags;
    use std::sync::Arc;

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    /// A crafter facing west into a pipe that hands on east, back into the crafter: the four
    /// sticks land in its grid, and using up the planks takes none of them.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_part_handing_a_crafters_output_back_into_its_grid_loses_none() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_part_handing_a_crafters_output_back_into_its_grid_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = horizon_world::chunk::BlockState {
                name: "minecraft:crafter".into(),
                properties: [("orientation".into(), "west_up".into())]
                    .into_iter()
                    .collect(),
            };
            let crafter = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], crafter, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get_mut(&(2, 64, 8)) {
                c.items[3] = Some(stack("minecraft:oak_planks", 1));
                c.items[6] = Some(stack("minecraft:oak_planks", 1));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), crafter)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Crafter(c)) => c
                    .items
                    .iter()
                    .flatten()
                    .map(|s| (s.item.clone(), s.count))
                    .collect::<Vec<_>>(),
                _ => panic!("no crafter"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (vec![("minecraft:stick".into(), 4)], 0),
                "a pipe routing output back into the grid must conserve four crafted sticks"
            );
        });
    }

    /// A dropper facing west into a pipe that hands on east, back into the dropper: the item
    /// goes round, and taking it from its slot as it is then keeps both.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_part_handing_a_droppers_item_back_into_it_loses_none() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_part_handing_a_droppers_item_back_into_it_loses_none",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = horizon_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "west".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&state).unwrap();
            assert_eq!(
                game.level_set_block(level, [2, 64, 8], dropper, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let pipe = game
                .pack
                .assets
                .registry
                .default_state("test:pipe")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 8], pipe, BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 8))
            {
                items[0] = Some(stack("minecraft:stick", 2));
            }
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(2, 64, 8), dropper)
                .unwrap();
            let held = match game.world.block_entities.get(&(2, 64, 8)) {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("no dropper"),
            };
            assert_eq!(
                (held, game.dropped.len() - dropped),
                (2, 0),
                "a pipe routing back into the source must conserve its items"
            );
        });
    }

    /// A dropper and a crafter facing a mod's item part (NeoForge's `dropperInsertHook` and
    /// `insertCrafterOutput`): a truthful part, a pipe into the chest east of it, takes what
    /// they put out in a container's place; a part answering more than it was offered has its
    /// move undone, so the dropper keeps its item and the crafter's result is dispensed.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them() {
        let Some(root) = declared_part_root(
            "game::machines::part_front_tests::a_dropper_and_a_crafter_put_into_a_mod_part_in_front_of_them",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = |game: &super::Game<'_>, name: &str, prop: (&str, &str)| {
                let state = horizon_world::chunk::BlockState {
                    name: name.to_owned(),
                    properties: [(prop.0.to_owned(), prop.1.to_owned())]
                        .into_iter()
                        .collect(),
                };
                game.pack.assets.registry.resolve(&state).unwrap()
            };
            let dropper = state(game, "minecraft:dropper", ("facing", "east"));
            let crafter = state(game, "minecraft:crafter", ("orientation", "east_up"));
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            let chest_items =
                |game: &super::Game<'_>, z| match game.world.block_entities.get(&(2, 64, z)) {
                    Some(BlockEntity::Chest(items)) => {
                        items.iter().flatten().map(|s| s.count).sum::<u32>()
                    }
                    _ => panic!("no chest"),
                };
            for (z, part, taken) in [(0, "test:pipe", true), (4, "test:greedy", false)] {
                for (x, block) in [(0, dropper), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Chest(items)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    items[4] = Some(stack("minecraft:stick", 2));
                }
                game.dispense_from(BlockPos::new(0, 64, z), dropper)
                    .unwrap();
                let left = match game.world.block_entities.get(&(0, 64, z)) {
                    Some(BlockEntity::Chest(items)) => items[4].as_ref().map_or(0, |s| s.count),
                    _ => panic!("no dropper"),
                };
                assert_eq!(
                    (left, chest_items(game, z)),
                    if taken { (1, 1) } else { (2, 0) },
                    "{part}"
                );

                // Two planks make four sticks.
                let z = z + 2;
                for (x, block) in [(0, crafter), (2, chest)] {
                    assert_eq!(
                        game.level_set_block(level, [x, 64, z], block, flags),
                        Ok(true)
                    );
                }
                let state = game.pack.assets.registry.default_state(part).unwrap();
                assert_eq!(
                    game.level_set_block(level, [1, 64, z], state, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                if let Some(BlockEntity::Crafter(c)) =
                    game.world.block_entities.get_mut(&(0, 64, z))
                {
                    c.items[0] = Some(stack("minecraft:oak_planks", 1));
                    c.items[3] = Some(stack("minecraft:oak_planks", 1));
                }
                let items = game.dropped.len();
                game.dispense_from(BlockPos::new(0, 64, z), crafter)
                    .unwrap();
                let spawned = game.dropped.len() - items;
                assert_eq!(
                    (chest_items(game, z), spawned),
                    if taken { (4, 0) } else { (0, 1) },
                    "{part}"
                );
            }
        });
    }
}

#[cfg(test)]
mod hopper_cart_transfer_tests {
    use super::super::{
        minecarts::{CartKind, Minecart},
        native_entity_refs::tests::with_game,
    };
    use super::Game;
    use horizon_client::{containers::BlockEntity, items::ItemStack};
    use horizon_core::geometry::Vec3;

    /// Issue #8668: a hopper minecart takes one item a tick from the container
    /// above. `MinecartHopper.tick` has no transfer cooldown in 26.3: it calls
    /// `tryConsumeItems` every tick, and `consumedItemThisFrame` caps a tick at
    /// one item even though `makeStepAlongTrack` may try too. A whole stack a
    /// tick (the old behaviour) or a block hopper's eight-tick cooldown both
    /// fail this.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn hopper_cart_takes_one_item_a_tick() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.streaming.forced.insert((0, 0));
            let rail = game
                .pack
                .assets
                .registry
                .default_state("minecraft:rail")
                .unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            // A rail holds the cart still; the chest sits in the block the cart
            // pulls from (`getLevelY` is y + 0.5, so the block above is y + 1.5).
            game.world.set(8, 64, 8, rail).unwrap();
            game.world.set(8, 65, 8, chest).unwrap();
            game.world.block_entities.insert(
                (8, 65, 8),
                BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 64)); 27]),
            );
            game.next_entity += 1;
            let id = game.next_entity;
            let mut cart = Minecart::new(id, CartKind::Hopper, Vec3::new(8.5, 64.0625, 8.5));
            cart.enabled = true;
            game.carts.push(cart);
            let count = |game: &Game<'_>| {
                let in_chest: u32 = match game.world.block_entities.get(&(8, 65, 8)) {
                    Some(BlockEntity::Chest(s)) => s.iter().flatten().map(|s| s.count).sum(),
                    _ => 0,
                };
                let in_cart: u32 = game
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .unwrap()
                    .items
                    .iter()
                    .flatten()
                    .map(|s| s.count)
                    .sum();
                (in_chest, in_cart)
            };
            let (before_chest, before_cart) = count(game);
            for _ in 0..20 {
                game.tick_carts();
            }
            let (after_chest, after_cart) = count(game);
            assert_eq!(before_chest, 27 * 64);
            assert_eq!(before_cart, 0);
            assert_eq!(before_chest - after_chest, after_cart - before_cart);
            assert_eq!(
                after_cart, 20,
                "one item a tick, not a stack nor eight ticks"
            );
        });
    }
}
