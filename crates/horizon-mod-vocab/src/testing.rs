//! Test support, enabled by the `testing` feature: scoped trait-mod installation
//! ([`with_mods`]), and [`FakeHost`], a host built from small fakes so that a mod's tests run
//! without starting the game.
//!
//! ```
//! use mod_api::testing::FakeHost;
//! use mod_api::prelude::ItemsHost;
//! use mod_api::{HostTransaction, ITEMS, ItemResource, ItemStack, ServerHost, move_resources};
//!
//! let mut host = FakeHost::new();
//! let level = host.level.level();
//! host.level.block_entity([0, 64, 0]).with_items(1);
//! host.level.block_entity([1, 64, 0]).with_items(1);
//! let stone_id = mod_api::Identifier::parse("minecraft:stone").unwrap();
//! host.registries.resolve_items([stone_id.clone()]).unwrap();
//! let stone = ItemStack::new(host.item_key(&stone_id).unwrap(), 5).unwrap();
//! host.level.block_entity([0, 64, 0]).items.as_mut().unwrap().slots[0] = Some(stone);
//!
//! let server: &mut dyn ServerHost = &mut host;
//! let from = level.capability(server, &ITEMS, [0, 64, 0], None).unwrap();
//! let into = level.capability(server, &ITEMS, [1, 64, 0], None).unwrap();
//! let moved = server.transaction(|host, tx| {
//!     move_resources(host, &from, &into, |_: &ItemResource| true, 3, tx)
//! });
//! assert_eq!(moved.ok(), Some(3));
//! let into = host.level.block_entity_at([1, 64, 0]).unwrap().items.as_ref().unwrap();
//! assert_eq!(into.count("minecraft:stone"), 3);
//! ```
#[path = "testing/journal.rs"]
mod journal;
#[path = "testing/level.rs"]
mod level;
#[path = "testing/players.rs"]
mod players;
#[path = "testing/registries.rs"]
mod registries;
#[path = "testing/services.rs"]
mod services;

/// An item forward reference missing from a fake's load-owned catalog.
pub use crate::parts::ItemKeyError;
pub use journal::FakeJournal;
pub use level::{FakeBattery, FakeBlockEntity, FakeContainer, FakeEntity, FakeLevel, FakeTank};
pub use players::{FakePlayer, FakePlayers};
pub use registries::FakeRegistries;
pub use services::{FakeCommands, FakeMessages};

use crate::item::transaction::{Transaction, TransactionSeal};
use crate::runtime::Installed;
use crate::world::ticket_controller::{ControllerInfo, TEST_CONTROLLERS};
use crate::{
    Backend, BackendMut, BackendMutRef, BackendRef, BlockEntityRef, BlockEntityTypeKey, BlockSide,
    ClientHost, Commands, EntityRef, FluidStack, Host, ItemPlace, ItemStack, LevelRef, Messages,
    Mods, PlayerRef, Players, ServerHost, TEST_MODS, Tag, Text, TicketController, WorldEpoch,
};

/// The world epoch of every fake's handles.
const FAKE_EPOCH: u64 = 1;

/// Run a closure with these owned mods on this thread, restoring the previous list on exit.
///
/// The test allocation is leaked because dispatch exposes a static slice. Nested scopes and
/// panic unwinding restore the preceding list; other threads are unaffected.
pub fn with_mods<R>(mods: Mods, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static Installed>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_MODS.set(self.0);
        }
    }
    let (controllers, poses) = crate::parts::registered_controllers(&mods);
    let controllers: &'static [_] = Box::leak(controllers.into_boxed_slice());
    let _poses = crate::client::arm_pose::test_table(poses);
    let installed = Box::leak(Box::new(Installed::new(mods)));
    let _restore = Restore(TEST_MODS.replace(Some(installed)));
    let _controllers = RestoreControllers(TEST_CONTROLLERS.replace(Some(controllers)));
    f()
}

/// Capture this thread's test installation for work sent to another thread.
///
/// Production installations are process-wide. Tests isolate them per thread; a worker
/// using an owning thread's immutable resource graph must see that same installation.
pub fn inherit_mods<A, R, F: FnOnce(&A) -> R + Send>(f: F) -> impl FnOnce(&A) -> R + Send {
    let installed = crate::runtime::installed();
    let parts = crate::parts::installed();
    move |argument| {
        struct Restore(
            Option<&'static Installed>,
            Option<&'static crate::parts::Installed>,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                TEST_MODS.set(self.0);
                crate::parts::TEST_PARTS.set(self.1);
            }
        }
        let _restore = Restore(
            TEST_MODS.replace(installed),
            crate::parts::TEST_PARTS.replace(parts),
        );
        f(argument)
    }
}

/// The ticket controller with this id that the mods of the enclosing `with_mods` registered.
pub fn ticket_controller(id: &str) -> Option<TicketController> {
    TicketController::from_id(id)
}

struct RestoreControllers(Option<&'static [ControllerInfo]>);
impl Drop for RestoreControllers {
    fn drop(&mut self) {
        TEST_CONTROLLERS.set(self.0);
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A server and client host for tests, composed of one fake per service: the registries, the
/// players, one level, the commands and messages services, and the transaction journal, which
/// keeps the players and the level on each transaction and puts them back when it aborts.
///
/// Set a test's world up through the fakes' fields, run the code under test with
/// `&mut host` as its `&mut dyn ServerHost`, then read the fakes back. A host method no fake
/// answers panics, naming the method, as a test double's would. The host has no client
/// backend, so client handle writes refuse.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeHost {
    /// The registries.
    pub registries: FakeRegistries,
    /// The players online.
    pub players: FakePlayers,
    /// The one level.
    pub level: FakeLevel,
    /// The command service.
    pub commands: FakeCommands,
    /// The message service.
    pub messages: FakeMessages,
    /// The transaction journal over the players and the level.
    pub journal: FakeJournal<(FakePlayers, FakeLevel)>,
}

impl FakeHost {
    /// A host with an empty overworld, no players and no mods loaded.
    pub fn new() -> Self {
        Self::default()
    }
    /// Player `uuid` online as entity `entity` in the level, joined if missing.
    pub fn join(&mut self, uuid: u128, entity: u32) -> &mut FakePlayer {
        self.players.join(uuid, entity, self.level.level())
    }

    fn max_stack(&self, resource: &crate::ItemResource) -> u32 {
        match resource.value_id(self, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) if (1..=99).contains(&size) => size as u32,
            None => 1,
            _ => 0,
        }
    }
}
impl Host for FakeHost {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl crate::prelude::BlockEntitiesHost for FakeHost {
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        self.level.standing(level, pos)?;
        Some(BlockEntityRef {
            level,
            position: pos,
            generation: 0,
        })
    }
}
impl crate::prelude::EntitiesHost for FakeHost {
    fn level(&self, dimension: &str) -> Option<LevelRef> {
        let level = self.level.level();
        (crate::content::registry_keys::dimension_id(level.dimension()) == Some(dimension))
            .then_some(level)
    }
    fn entity(&self, id: u32) -> Option<EntityRef> {
        let known = self.players.by_entity(id).is_some() || self.level.entity_at(id).is_some();
        known.then(|| EntityRef::new(WorldEpoch::new(FAKE_EPOCH), id))
    }
}
impl crate::prelude::PlayersHost for FakeHost {
    fn player(&self, uuid: u128) -> Option<PlayerRef> {
        self.players.handle(uuid)
    }
}
impl crate::prelude::ItemsHost for FakeHost {
    fn item_key(&self, id: &crate::Identifier) -> Option<crate::ItemKey> {
        crate::content::registry_keys::item_key_by_identifier(id)
    }
    fn item_default_component(&self, item: crate::ItemKey, component: &str) -> Option<Tag> {
        self.registries
            .default_component(FakeRegistries::resolved_item_id(item)?, component)
    }
    fn max_stack_size(&self, item: crate::ItemKey) -> Option<u32> {
        Some(
            self.registries
                .max_stack_size(FakeRegistries::resolved_item_id(item)?),
        )
    }
    fn max_damage(&self, item: crate::ItemKey) -> Option<u32> {
        let id = FakeRegistries::resolved_item_id(item)?;
        match self
            .registries
            .default_component(id, "minecraft:max_damage")
        {
            Some(Tag::Int(damage)) => u32::try_from(damage).ok(),
            None => Some(0),
            _ => None,
        }
    }
}
impl crate::prelude::TagsHost for FakeHost {
    fn tags(&self) -> &dyn crate::Tags {
        &self.registries
    }
}
impl crate::prelude::LevelsHost for FakeHost {
    fn is_loaded(&self, id: &str) -> bool {
        self.registries.is_loaded(id)
    }
}
crate::impl_host_areas!(
    FakeHost,
    except(block_entities, entities, items, levels, players, tags)
);

impl ServerHost for FakeHost {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }
}
impl crate::prelude::ModStateServerHost for FakeHost {
    fn transaction_open(&mut self) -> TransactionSeal {
        self.journal
            .open((self.players.clone(), self.level.clone()))
    }
    fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool) {
        if let Some((players, level)) = self.journal.close(seal, commit) {
            (self.players, self.level) = (players, level);
        }
    }
}
impl crate::prelude::ServicesServerHost for FakeHost {
    fn commands(&mut self) -> &mut dyn Commands {
        &mut self.commands
    }
    fn messages(&self) -> &dyn Messages {
        &self.messages
    }
    fn players(&self) -> &dyn Players {
        &self.players
    }
}
crate::impl_server_host_areas!(FakeHost, except(mod_state, services));
impl ClientHost for FakeHost {}
crate::impl_client_host_areas!(FakeHost);

impl Backend for FakeHost {}
impl crate::prelude::LevelsBackend for FakeHost {
    fn level_in_place(&self, level: LevelRef) -> bool {
        level == self.level.level()
    }
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<crate::BlockKey> {
        self.level
            .block_at(level, pos)
            .and_then(crate::__private::block_key)
    }
}
impl crate::prelude::BlockEntitiesBackend for FakeHost {
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let standing = self.level.standing(entity.level, entity.position)?;
        standing.kind().cloned()
    }
}
impl crate::prelude::ItemsBackend for FakeHost {
    fn container_size(&self, at: BlockSide) -> Option<usize> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        Some(items.slots.len())
    }
    fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        items.slots.get(slot)?.clone()
    }
    fn container_capacity(&self, at: BlockSide, slot: usize, stack: &ItemStack) -> u32 {
        let items = self
            .level
            .standing(at.level, at.pos)
            .and_then(|standing| standing.items.as_ref());
        items.map_or(0, |items| {
            items.capacity(slot, self.max_stack(&crate::ItemResource::of(stack)))
        })
    }
    fn entity_container_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_automation_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_equipment_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        self.registries.container_slots(stack)
    }
}
impl crate::prelude::FluidsEnergyBackend for FakeHost {
    fn fluid_tank_count(&self, at: BlockSide) -> Option<usize> {
        Some(self.level.standing(at.level, at.pos)?.tanks.len())
    }
    fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack> {
        let standing = self.level.standing(at.level, at.pos)?;
        standing.tanks.get(tank)?.held.clone()
    }
    fn fluid_tank_capacity(&self, at: BlockSide, tank: usize, _: &FluidStack) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.tanks.get(tank))
            .map_or(0, |tank| tank.capacity)
    }
    fn energy_amount(&self, at: BlockSide) -> Option<u32> {
        let energy = self.level.standing(at.level, at.pos)?.energy.as_ref()?;
        Some(energy.amount)
    }
    fn energy_capacity(&self, at: BlockSide) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_ref())
            .map_or(0, |energy| energy.capacity)
    }
}
impl crate::prelude::EntitiesBackend for FakeHost {
    fn entity_kind(&self, entity: EntityRef) -> Option<String> {
        if self.players.by_entity(entity.id()).is_some() {
            return Some("minecraft:player".to_owned());
        }
        Some(self.level.entity_at(entity.id())?.kind.clone())
    }
    fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        if let Some((_, player)) = self.players.by_entity(entity.id()) {
            return Some(player.level);
        }
        Some(self.level.entity_at(entity.id())?.level)
    }
}
impl crate::prelude::KeysBackend for FakeHost {
    fn item_key_id(&self, key: crate::ItemKey) -> String {
        crate::content::registry_keys::item_id(key)
            .expect("fake item key belongs to its load")
            .to_owned()
    }
}
impl crate::prelude::PlayersBackend for FakeHost {
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(self.players.get(player.uuid())?.selected_slot)
    }
    fn player_name(&self, player: PlayerRef) -> Option<String> {
        Some(self.players.get(player.uuid())?.name.clone())
    }
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        (self.players.handle(player.uuid())? == player).then_some(())?;
        Some(self.players.get(player.uuid())?.level)
    }
    fn player_dimension(&self, player: PlayerRef) -> Option<crate::Identifier> {
        let level = self.player_level(player)?;
        crate::Identifier::parse(level.dimension_id()).ok()
    }
}
crate::impl_backend_areas!(
    FakeHost,
    except(
        block_entities,
        entities,
        fluids_energy,
        items,
        keys,
        levels,
        players
    )
);

impl BackendMut for FakeHost {}
impl crate::prelude::ItemsBackendMut for FakeHost {
    fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<Option<ItemStack>, crate::StackWriteError> {
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        let put = match standing.and_then(|standing| standing.items.as_mut()) {
            Some(items) => items.insert(slot, resource, amount, max_stack),
            None => 0,
        };
        Ok(resource.to_stack(amount - put))
    }
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        _: &Transaction,
    ) -> Option<ItemStack> {
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.items.as_mut()?.extract(slot, max)
    }
    fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
        let (player, slot) = self.players.place(place)?;
        player.slots.get(&slot).cloned()
    }
    fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<u32, crate::StackWriteError> {
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        Ok(match self.players.place(place) {
            Some((player, slot)) => player.insert(slot, resource, amount, max_stack),
            _ => 0,
        })
    }
    fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<u32, crate::StackWriteError> {
        Ok(self
            .players
            .place(place)
            .map_or(0, |(player, slot)| player.extract(slot, resource, amount)))
    }
}
impl crate::prelude::FluidsEnergyBackendMut for FakeHost {
    fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        _: &Transaction,
    ) -> Option<FluidStack> {
        let standing = self.level.standing_mut(at.level, at.pos);
        match standing.and_then(|standing| standing.tanks.get_mut(tank)) {
            Some(tank) => tank.insert(fluid),
            None => Some(fluid),
        }
    }
    fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        _: &Transaction,
    ) -> Option<FluidStack> {
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.tanks.get_mut(tank)?.extract(max)
    }
    fn energy_insert(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.insert(amount))
    }
    fn energy_extract(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.extract(amount))
    }
}
impl crate::prelude::EntitiesBackendMut for FakeHost {
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        if let Some((uuid, _)) = self.players.by_entity(entity.id()) {
            return Some(uuid);
        }
        Some(self.level.entity_at(entity.id())?.uuid)
    }
}
impl crate::prelude::PlayersBackendMut for FakeHost {
    fn player_action_bar(
        &mut self,
        player: PlayerRef,
        line: Text,
    ) -> Result<bool, crate::text::TextFormError> {
        line.validate_depth()?;
        let Some(player) = self.players.get_mut(player.uuid()) else {
            return Ok(false);
        };
        player.action_bar.push(line);
        Ok(true)
    }
}
crate::impl_backend_mut_areas!(FakeHost, except(entities, fluids_energy, items, players));

impl crate::prelude::PlayersClientBackend for FakeHost {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::ItemsHost;
    use crate::{FLUIDS, FluidResource, Hand, HostTransaction, ItemResource, move_resources};

    fn water(amount: u32) -> FluidStack {
        FluidStack::new(
            crate::content::registry_keys::fluid_key("minecraft:water").unwrap(),
            amount,
        )
    }

    #[test]
    fn a_fluid_moves_between_tanks_of_the_same_fluid_and_an_abort_puts_both_back() {
        crate::parts::tests::game_thread();
        let mut host = FakeHost::new();
        let level = host.level.level();
        host.level.block_entity([0, 64, 0]).tanks = vec![FakeTank {
            held: Some(water(800)),
            capacity: 1000,
        }];
        host.level.block_entity([1, 64, 0]).tanks = vec![FakeTank {
            held: Some(water(700)),
            capacity: 1000,
        }];
        host.level.block_entity([2, 64, 0]).tanks = vec![FakeTank {
            held: Some(FluidStack::new(
                crate::content::registry_keys::fluid_key("minecraft:lava").unwrap(),
                100,
            )),
            capacity: 1000,
        }];
        let server: &mut dyn ServerHost = &mut host;
        let tank = |host: &mut dyn ServerHost, x| {
            level.capability(host, &FLUIDS, [x, 64, 0], None).unwrap()
        };
        let (from, into, lava) = (tank(server, 0), tank(server, 1), tank(server, 2));
        let any = |_: &FluidResource| true;
        let aborted = server.transaction(|host, tx| {
            let moved = move_resources(host, &from, &into, any, 1000, tx);
            Err::<(), _>(moved)
        });
        assert_eq!(aborted, Err(Ok(300)));
        // Lava takes no water; water fills only up to the target's room.
        let refused =
            server.transaction(|host, tx| move_resources(host, &from, &lava, any, 1000, tx));
        assert_eq!(refused, Ok(0));
        let moved =
            server.transaction(|host, tx| move_resources(host, &from, &into, any, 1000, tx));
        assert_eq!(moved, Ok(300));
        let amount = |x| host.level.block_entity_at([x, 64, 0]).unwrap().tanks[0].amount();
        assert_eq!([amount(0), amount(1), amount(2)], [500, 1000, 100]);
    }

    #[test]
    fn what_a_players_hand_cannot_hold_is_given_to_them_and_an_abort_takes_it_back() {
        let mut host = FakeHost::new();
        host.join(7, 41).selected_slot = 2;
        host.registries
            .item_component("minecraft:egg", "minecraft:max_stack_size", Tag::Int(16));
        let hand = ItemPlace::PlayerHand {
            player: 7,
            hand: Hand::Main,
        };
        let egg = crate::Identifier::parse("minecraft:egg").unwrap();
        host.registries.resolve_items([egg.clone()]).unwrap();
        let eggs = ItemResource::of(&ItemStack::of(host.item_key(&egg).unwrap()));
        let server: &mut dyn ServerHost = &mut host;
        let aborted = server.transaction(|host, tx| Err::<(), _>(hand.insert(host, tx, &eggs, 20)));
        assert_eq!(aborted, Err(20));
        assert!(host.players.get(7).unwrap().slots.is_empty());
        let server: &mut dyn ServerHost = &mut host;
        let went = server.transaction(|host, tx| Ok::<_, ()>(hand.insert(host, tx, &eggs, 20)));
        assert_eq!(went, Ok(20));
        let player = host.players.get(7).unwrap();
        assert_eq!(player.main_hand().map(ItemStack::count), Some(16));
        assert_eq!(player.slots.keys().collect::<Vec<_>>(), [&2]);
        let given: Vec<u32> = player.given.iter().map(ItemStack::count).collect();
        assert_eq!(given, [4]);
    }
}
