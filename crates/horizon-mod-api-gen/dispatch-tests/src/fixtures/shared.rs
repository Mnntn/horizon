//! Shared exception: dispatch infrastructure and cross-area fixture types/tests; no new hooks.
use horizon_core::game_mode::GameMode;
use mod_api::mod_api::client::hud_layout::GuiDraw;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct ItemStack {
    item: Arc<str>,
    count: u32,
}
// Dispatch tests isolate the runtime key table: the fixture key retains its name.
// Production key resolution and slot stability are tested in mod-runtime.
#[derive(Clone, Copy)]
struct ItemKey<'a>(&'a str);
impl ItemStack {
    fn count(&self) -> u32 {
        self.count
    }
    fn item(&self) -> ItemKey<'_> {
        ItemKey(&self.item)
    }
}
mod content {
    pub(super) mod registry_keys {
        pub(in crate::tests) fn block_key(name: &str) -> Option<super::super::BlockKey> {
            match name {
                "test:block" => Some(super::super::BlockKey::Test),
                "test:other" => Some(super::super::BlockKey::Other),
                _ => None,
            }
        }
        pub(in crate::tests) fn item_id(key: super::super::ItemKey<'_>) -> Option<&str> {
            Some(key.0)
        }
    }
}

struct BlockEntityDraw(Vec<u32>);
use serde_json::{Map, Value};
use stubs::*;
struct Text;
struct ScreenshotImage;
#[derive(Debug, PartialEq)]
struct Zoom(u32);
enum Flow {
    Continue,
    Cancel,
}
trait Host {}
trait ServerHost: Host {}
trait ClientHost: Host {}
trait WorldDrawContext {}
trait DrawContext {
    fn mark(&mut self, value: u32);
}
impl DrawContext for HudBatch {
    fn mark(&mut self, value: u32) {
        self.0.push(value);
    }
}
/// Records which part drew, in order.
trait BlockEntityDrawContext {
    fn mark(&mut self, part: u32);
}
impl BlockEntityDrawContext for BlockEntityDraw {
    fn mark(&mut self, part: u32) {
        self.0.push(part);
    }
}
enum HostSide<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a mut dyn ClientHost),
}
impl HostSide<'_> {
    fn reborrow(&mut self) -> HostSide<'_> {
        match self {
            Self::Server(host) => HostSide::Server(&mut **host),
            Self::Client(host) => HostSide::Client(&mut **host),
        }
    }
}
enum LevelSide<'a> {
    Server(&'a dyn LevelHost),
    Client(&'a dyn LevelHost),
}
impl LevelSide<'_> {
    fn reborrow(&mut self) -> LevelSide<'_> {
        match self {
            Self::Server(host) => LevelSide::Server(*host),
            Self::Client(host) => LevelSide::Client(*host),
        }
    }
}
struct TestHost;
struct TestLevel;
impl LevelHost for TestLevel {}
impl LevelRandomHost for TestLevel {}
impl Host for TestHost {}
impl ServerHost for TestHost {}
impl ClientHost for TestHost {}
#[allow(dead_code)]
mod generated {
    pub(super) mod mod_api {
        pub struct Block;
        pub struct Item;
        pub(super) use super::super::ItemStack;
        pub(super) use super::super::{
            Command, Config, Dispensed, InteractionResult, MapDecoration, MapInfo, Minimap, Screen,
            Zoom,
        };
        pub struct FluidModel;
        include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_api.rs"));
    }
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
    pub(super) use self::mod_api::{BlockKey, BlockState};
    pub use horizon_mod_api_macros::__subscriptions;
}
use generated::*;
struct Installed {
    mods: Arc<[ModEntry]>,
    subscribers: horizon_mod_runtime::subscribers::Subscribers,
}
thread_local! {
    /// The hooks each dispatch entered, a mod at a time.
    static ENTERED: std::cell::RefCell<Vec<(usize, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) };
    static MODS: std::cell::Cell<Option<&'static Installed>> = const { std::cell::Cell::new(None) };
}
fn install(mods: Arc<[ModEntry]>) {
    let subscriptions = mods.iter().enumerate().flat_map(|(index, entry)| {
        let listeners = entry.hooks.listeners();
        entry.hooks.subscribed().iter().flatten().map(move |hook| {
            let listener = listeners.iter().find(|listener| {
                listener
                    .hook
                    .is_some_and(|id| id as usize == *hook as usize)
            });
            horizon_mod_runtime::subscribers::Subscription {
                index,
                hook: *hook as usize,
                priority: listener.map_or(0, |listener| listener.priority),
                receives_cancelled: listener.is_some_and(|listener| listener.receive_cancelled),
            }
        })
    });
    let subscribers =
        horizon_mod_runtime::subscribers::Subscribers::new(HOOK_COUNT, mods.len(), subscriptions);
    // Test dispatch needs the same static lifetime as the real installation.
    MODS.set(Some(Box::leak(Box::new(Installed { mods, subscribers }))));
}
fn listened(hook: HookId) -> bool {
    MODS.get()
        .is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}
fn receives_cancelled(hook: HookId) -> bool {
    MODS.get()
        .is_some_and(|installed| installed.subscribers.receives_cancelled(hook as usize))
}
fn entered(hook_id: HookId, hook: &'static str) -> impl Iterator<Item = &'static dyn Hooks> {
    entered_with_id(hook_id, hook).map(|(_, hooks)| hooks)
}
fn entered_with_id(
    hook_id: HookId,
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> {
    listening(hook_id).map(move |(_, entry)| {
        let ((), hooks) = enter_listener(entry, hook);
        (entry.id, hooks)
    })
}
fn listening(hook_id: HookId) -> impl Iterator<Item = (bool, &'static ModEntry)> {
    MODS.get().into_iter().flat_map(move |installed| {
        installed
            .subscribers
            .subscribers(hook_id as usize)
            .map(move |subscriber| {
                (
                    subscriber.receives_cancelled,
                    &installed.mods[subscriber.index],
                )
            })
    })
}
fn enter_listener(entry: &'static ModEntry, hook: &'static str) -> ((), &'static dyn Hooks) {
    let index = MODS
        .get()
        .unwrap()
        .mods
        .iter()
        .position(|e| std::ptr::eq(e, entry))
        .unwrap();
    ENTERED.with(|entered| entered.borrow_mut().push((index, hook)));
    ((), &*entry.hooks)
}
struct ModEntry {
    id: &'static str,
    hooks: Box<dyn Hooks>,
}
impl ModEntry {
    fn new(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        Self { id, hooks }
    }
}
trait Registrar {}
/// The goal registrar of the generated `EntityPart`, as the fixture sees
/// it: no methods, its type is all the generated trait asks of the scope.
#[expect(dead_code)]
struct GoalRegistrar<'a> {
    _lifetime: std::marker::PhantomData<&'a ()>,
}
/// The behavior registrar of the generated `EntityPart` (#824), as the
/// fixture sees it: like [`GoalRegistrar`], only its type is asked for.
#[expect(dead_code)]
struct BehaviorRegistrar<'a> {
    _lifetime: std::marker::PhantomData<&'a ()>,
}
/// Scope fixtures install their own Parts. Block attachments retain their API keys;
/// other scopes keep their existing single-target fixture tables.
#[derive(Default)]
struct Parts {
    blocks: Vec<(BlockKey, Vec<&'static dyn BlockPart>)>,
    block_entities: Vec<&'static dyn BlockEntityPart>,
    items: Vec<&'static dyn ItemPart>,
    menus: Vec<&'static dyn MenuPart>,
    fluids: Vec<&'static dyn FluidPart>,
}
static NO_PARTS: Parts = Parts {
    blocks: Vec::new(),
    block_entities: Vec::new(),
    items: Vec::new(),
    menus: Vec::new(),
    fluids: Vec::new(),
};
thread_local! {
    static PARTS: std::cell::Cell<&'static Parts> = const { std::cell::Cell::new(&NO_PARTS) };
}
fn install_parts(parts: Parts) {
    PARTS.set(Box::leak(Box::new(parts)));
}
/// `part` for the parts list of a test: parts live as long as the installation.
fn leak<T>(part: T) -> &'static T {
    Box::leak(Box::new(part))
}
fn has_part_hook(_: HookId) -> bool {
    // Manual fixture Parts override methods without emitting subscription bits.
    // Their scope tables still refuse an empty installation and unrelated block keys.
    let parts = PARTS.get();
    !parts.blocks.is_empty()
        || !parts.block_entities.is_empty()
        || !parts.items.is_empty()
        || !parts.menus.is_empty()
}
fn parts(_: HookId, _: &'static str) -> &'static Parts {
    PARTS.get()
}
impl Parts {
    fn for_hook(&self, _: HookId) -> Option<&Self> {
        Some(self)
    }
    fn block_value(&self, state: BlockState) -> &[&'static dyn BlockPart] {
        // Both fixture block definitions have two local state ordinals, 0 and 1.
        if state.ordinal() > 1 {
            return &[];
        }
        self.block_key(Some(state.block()))
    }
    fn block_key(&self, key: Option<BlockKey>) -> &[&'static dyn BlockPart] {
        key.and_then(|key| self.blocks.iter().find(|(block, _)| *block == key))
            .map_or(&[], |(_, parts)| parts.as_slice())
    }
    fn block_entity<'k>(
        &self,
        _: impl FnOnce() -> Option<&'k str>,
    ) -> &[&'static dyn BlockEntityPart] {
        &self.block_entities
    }
    fn block_entity_kind<'k>(
        &self,
        _: impl FnOnce() -> Option<&'k str>,
    ) -> &[&'static dyn BlockEntityPart] {
        &self.block_entities
    }
    fn item<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn ItemPart] {
        &self.items
    }
    fn menu<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn MenuPart] {
        &self.menus
    }
    fn fluid<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn FluidPart] {
        &self.fluids
    }
}
fn entered_parts<'a, P: ?Sized>(
    parts: &'a [&'a P],
    _: &'static str,
) -> impl Iterator<Item = &'a P> {
    parts.iter().copied()
}

#[test]
fn cfg_disabled_hooks_are_not_subscribed_and_cfg_attr_stays() {
    struct Gated;
    #[mod_api::__subscriptions]
    impl Hooks for Gated {
        #[cfg(any())]
        fn loot_table_load(&self, _: &dyn DataLoadContext, _: &mut LootTableLoadEvent<'_>) {
            unreachable!()
        }
        #[cfg(not(any()))]
        fn player_logged_in(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedInEvent<'_>) {}
        #[cfg_attr(test, inline)]
        fn player_logged_out(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedOutEvent<'_>) {}
    }
    install(vec![ModEntry::new("gated", Box::new(Gated))].into());
    assert!(
        !listened(HookId::LootTableLoad),
        "a cfg-disabled hook was subscribed"
    );
    assert!(
        listened(HookId::PlayerLoggedIn),
        "a cfg-enabled hook was not subscribed"
    );
    assert!(
        listened(HookId::PlayerLoggedOut),
        "cfg_attr never unsubscribes"
    );
}

struct Mod {
    id: u32,
    calls: Arc<Mutex<Vec<u32>>>,
}
impl BlockPart for Mod {
    fn block_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
}
impl BlockEntityPart for Mod {
    fn block_entity_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_button(&self, _: &mut dyn ServerHost, event: &mut MenuButtonEvent<'_>) {
        assert_eq!(event.context.player, 123);
        assert_eq!(event.context.player_entity_id, 42);
        assert_eq!(event.context.pos, [1, 2, 3]);
        assert_eq!(event.context.id, 9);
        assert_eq!(event.entity.id(), "machine");
        event.entity.data.push(9);
        *event.carried = Some(ItemStack {
            item: "bucket".into(),
            count: 1,
        });
    }
    fn block_entity_submit(&self, _: &dyn ClientHost, event: &mut BlockEntitySubmitEvent<'_>) {
        assert_eq!(event.pos, [1, 2, 3]);
        assert_eq!(event.block, "example:machine");
        event.draw.mark(self.id);
    }
}
impl MenuPart for Mod {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_slot_rules(&self, context: &dyn LoopContext, event: &mut MenuSlotRulesEvent<'_>) {
        assert_eq!(context.side(), "loop");
        event.may_place = false;
        event.max_stack = self.id;
    }
}
impl ItemPart for Mod {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn tooltip(&self, _: &dyn ClientHost, event: &mut TooltipEvent<'_>) {
        event.lines.push((Text, self.id));
    }
    fn use_on(&self, _: &mut dyn ServerHost, _: &mut UseOnEvent<'_>) -> Flow {
        self.calls.lock().unwrap().push(self.id);
        if self.id == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn living_entity_use_item_finish(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        *event.result = None;
    }
    fn mouse_button_pre(&self, _: &mut dyn ClientHost, _: &mut MouseButtonPreEvent) -> Flow {
        self.calls.lock().unwrap().push(self.id);
        if self.id == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn living_entity_use_item_tick(
        &self,
        _: HostSide,
        event: &mut LivingEntityUseItemTickEvent<'_>,
    ) -> Flow {
        event.duration -= 1;
        if self.id == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn loot_table_load(&self, _: &dyn DataLoadContext, event: &mut LootTableLoadEvent<'_>) {
        event.table.as_array_mut().unwrap().push(self.id.into());
    }
    fn modify_default_components(
        &self,
        context: &dyn DataLoadContext,
        event: &mut ModifyDefaultComponentsEvent<'_>,
    ) {
        assert_eq!(context.origin(), "defaults");
        let value = event
            .components
            .entry("order")
            .or_insert_with(|| serde_json::json!([]));
        value.as_array_mut().unwrap().push(self.id.into());
    }
    fn level_tick_pre(&self, _: &mut dyn ServerHost, _: &mut LevelTickPreEvent<'_>) {
        self.calls.lock().unwrap().push(self.id);
    }
    fn zoom(&self, _: &dyn ClientHost, _: &mut ZoomEvent) -> Option<Zoom> {
        self.calls.lock().unwrap().push(self.id);
        Some(Zoom(self.id))
    }
    fn should_apply_effect_tick_this_tick(
        &self,
        _: &dyn Host,
        event: &mut ShouldApplyEffectTickThisTickEvent,
    ) {
        assert_eq!(
            (event.effect, event.tick_count, event.amplifier),
            (EffectKey(7), 19, 2)
        );
        event.apply = !event.apply;
        self.calls.lock().unwrap().push(self.id);
    }
    fn living_incoming_damage(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        event.container.0 = event.container.0 * 2.0 + self.id as f32;
        if self.id == 2 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
fn part(id: u32, calls: &Arc<Mutex<Vec<u32>>>) -> &'static Mod {
    leak(Mod {
        id,
        calls: calls.clone(),
    })
}

mod stubs {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) struct BlockStateId;
    #[derive(Clone, Copy)]
    pub(super) struct LevelRef;
    #[derive(Clone, Copy)]
    pub(super) struct BlockPos;
    #[derive(Clone, Copy)]
    pub(super) struct Direction;
    #[derive(Clone, Copy)]
    pub(super) struct LevelDimension;
    #[derive(Clone, Copy)]
    pub(super) struct PlayerRef;
    #[derive(Clone, Copy)]
    pub(super) enum SoundSource {}
    pub(super) struct NeighbourUpdate;
    #[derive(Clone, Copy)]
    pub(super) struct ConnectionKind;
    pub(super) struct GlobalPos;
    pub(super) struct AttributeModifier;
    #[derive(Clone, Copy)]
    pub(super) enum AttributeOperation {}
    #[derive(Clone, Copy)]
    pub(super) enum ModifierLifetime {}
    #[derive(Clone, Copy)]
    pub(super) enum EquipmentSlotGroup {}
    pub(super) struct ItemAttributeModifier;
    pub(super) struct ItemAttributeModifiers;
    pub(super) struct ItemEnchantments;
    pub(super) struct Identifier;
    pub(super) trait LevelHost {
        fn state_block(&self, _: BlockStateId) -> Option<&str> {
            None
        }
    }
    pub(super) trait LevelTicksHost: LevelHost {}
    pub(super) trait LevelRandomHost: LevelHost {}
    pub(super) trait LevelWriteHost: LevelTicksHost + LevelRandomHost {}
    pub(super) struct Difficulty;
    pub(super) struct SpawnSource;
    pub(super) struct Tag;
    pub(super) struct FeatureFlags(pub ());
    pub(super) struct NoteInstrument;
    pub(super) struct ScreenView<'a>(pub &'a ());
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct EffectKey(pub(super) u32);
    #[derive(Debug, PartialEq)]
    pub(super) enum ScreenOpening {
        Cancel,
        ModScreen,
    }
    #[derive(Debug, PartialEq)]
    pub(super) struct Answered<T> {
        pub(super) answer: T,
        pub(super) mod_id: &'static str,
    }
    pub(super) struct StackedOnOther<'a> {
        pub(super) _lifetime: std::marker::PhantomData<&'a mut ()>,
    }
    pub(super) struct ClientPacket;
    #[derive(Default)]
    pub(super) struct HudBatch(pub Vec<u32>);
    #[derive(Clone, Copy)]
    pub(super) struct TooltipPlace;
    #[derive(Clone, Copy)]
    pub(super) struct Hand;
    #[derive(Clone, Copy)]
    pub(super) struct BlockHit;
    pub(super) struct BlockClick<'a>(pub &'a ());
    #[derive(Clone, Copy)]
    pub(super) enum BonemealSource {
        Interaction,
    }
    pub(super) struct TooltipDisplay<'a>(pub &'a ());
    pub(super) struct TriState;
    pub(super) struct InteractionResult;
    pub(super) struct HurtSource<'a> {
        pub(super) damage_type: &'a str,
        pub(super) position: Option<[f64; 3]>,
    }
    pub(super) struct Victim {
        pub(super) position: [f64; 3],
        pub(super) view: [f64; 3],
    }
    pub(super) use crate::tests::mod_api::mod_api::DamageContainer;
    #[derive(Default)]
    pub(super) struct ModBlockEntity {
        id: String,
        pub(super) data: Vec<i32>,
    }
    impl ModBlockEntity {
        pub(super) fn new(id: &str) -> Self {
            Self {
                id: id.into(),
                ..Default::default()
            }
        }
        pub(super) fn id(&self) -> &str {
            &self.id
        }
    }
    pub(super) struct ModBlockEntityView<'a> {
        id: &'a str,
        pub(super) data: &'a mut Vec<i32>,
    }
    impl<'a> ModBlockEntityView<'a> {
        pub(super) fn new(entity: &'a mut ModBlockEntity) -> Self {
            Self {
                id: &entity.id,
                data: &mut entity.data,
            }
        }
        pub(super) fn id(&self) -> &str {
            self.id
        }
    }
    pub(super) struct LootStack;
    pub(super) struct LootContext;
    pub(super) struct NativeReloadListener;
    pub(super) struct RecipePropertyAddition;
    pub(super) struct AdvancementProgress {
        pub done: bool,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ProgressType {
        Grant,
        Revoke,
    }
    pub(super) struct RegistrationContext(pub &'static str);
    pub(super) trait DataLoadContext {
        fn origin(&self) -> &'static str;
    }
    impl DataLoadContext for Load {
        fn origin(&self) -> &'static str {
            self.0
        }
    }
    pub(super) trait LoopContext {
        fn side(&self) -> &'static str;
    }
    pub(super) struct Loop(pub &'static str);
    impl LoopContext for Loop {
        fn side(&self) -> &'static str {
            self.0
        }
    }
    pub(super) struct SoundPlay;
    pub(super) struct ChannelRef;
    #[derive(Clone, Copy)]
    pub(super) struct PlayerFood;
    pub(super) use super::generated::mod_api::LivingRef;
    #[derive(Clone, Copy)]
    pub(super) struct EntityRef;
    #[derive(Clone, Copy)]
    pub(super) struct MinecartRef;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum MinecartKind {
        Rideable,
    }
    pub(super) struct EffectTickChange;
    #[derive(Clone, Copy)]
    pub(super) struct EquipmentSlot;
}

mod api {
    // API values are deliberately distinct from the engine BlockStateId stub.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BlockKey {
        Test,
        Other,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BlockState {
        block: BlockKey,
        ordinal: u16,
    }
    impl BlockState {
        pub fn new(block: BlockKey, ordinal: u16) -> Self {
            Self { block, ordinal }
        }
        pub fn block(self) -> BlockKey {
            self.block
        }
        pub fn ordinal(self) -> u16 {
            self.ordinal
        }
    }
    pub struct DamageContainer(pub f32);
    #[derive(Clone, Copy)]
    pub struct LivingRef;

    #[derive(Clone, Copy)]
    pub struct FluidKey;

    pub mod mod_chunk_tickets {
        pub trait TicketValidation {}
    }
    pub use mod_chunk_tickets::TicketValidation;

    pub mod client {
        pub use super::fog;
        pub mod camera {
            pub struct CameraEntity;
            pub struct FovPlayer;
        }
        pub mod first_person {
            #[derive(Clone, Copy)]
            pub struct Hand;
        }
        pub mod widgets {
            pub struct TextEdit;
        }
        pub mod hud_layout {
            pub struct HealthInputs;
            pub struct GuiDraw;
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum HeartKind {}
        }
    }
    pub mod item {
        pub mod menu {
            pub struct MenuButtonContext {
                pub player: u128,
                pub player_entity_id: i32,
                pub pos: [i32; 3],
                pub id: i32,
            }
            pub struct AnvilCraft;
            pub struct AnvilUpdate;
            pub struct GrindstonePlace;
            pub struct GrindstoneTake;
        }
        pub mod stations {
            pub struct EnchantmentRow<'a>(std::marker::PhantomData<&'a ()>);
        }
    }
}
