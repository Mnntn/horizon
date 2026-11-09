//! Exercise the generated proxies and service against the real guest dispatcher, without wasm.
// Generated fixtures exercise every shape; TODO(#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#![allow(dead_code, unused_variables, unused_mut, unsafe_op_in_unsafe_fn)]
use mod_api::wire::{Id, Reader, WireError, Writer, guest};
use mod_api::{Hooks, LoopContext, StructurePiece};
use std::{rc::Rc, sync::Arc};
mod boundary {
    include!(concat!(env!("OUT_DIR"), "/wire_host_generated.rs"));
}
#[derive(Clone)]
struct WasmMod {
    group: u8,
    subscribed: &'static [Option<mod_api::HookId>],
    listeners: &'static [mod_api::Listener],
    corrupt: bool,
}
impl std::fmt::Debug for WasmMod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Loopback")
    }
}
#[derive(Debug)]
struct WasmHandler {
    index: Id,
    module: WasmMod,
}
#[derive(Clone, Copy)]
enum CallClass {
    Load,
    Hook,
    Worker,
    Callback,
}
trait NoAnswer {
    fn no_answer() -> Self;
}
impl<T: Default> NoAnswer for T {
    fn no_answer() -> Self {
        Self::default()
    }
}
fn call_refused<E: From<(mod_api::Identifier, String)>>() -> E {
    E::from((
        mod_api::Identifier::parse("test:refused").unwrap(),
        "refused".into(),
    ))
}
impl WasmHandler {
    fn new(module: &WasmMod, index: Id) -> Self {
        Self {
            module: module.clone(),
            index,
        }
    }
    fn belongs_to(&self, module: &WasmMod) -> bool {
        self.module.same_group(module)
    }
    fn subscriptions(&self, owner: &'static str, id: u32) -> &'static [Option<mod_api::HookId>] {
        let names = self
            .module
            .call(
                CallClass::Load,
                boundary::Frame::default(),
                id,
                Some(self.index),
                |_| {},
                |r| r.take(),
            )
            .unwrap();
        boundary::subscriptions_of(owner, names)
    }
}
impl WasmMod {
    fn same_group(&self, other: &Self) -> bool {
        self.group == other.group
    }
    fn mod_id(&self) -> &str {
        if self.group == 0 { "loopback" } else { "other" }
    }
    fn call<R>(
        &self,
        _: CallClass,
        frame: boundary::Frame,
        id: u32,
        handler: Option<Id>,
        encode: impl FnOnce(&mut Writer),
        decode: impl FnOnce(&mut Reader<'_>) -> Result<R, WireError>,
    ) -> Option<R> {
        // Each mock Cx models a fresh store; real stores own their guest string table.
        guest::reset_static_strings();
        let mut cx = Box::new(boundary::Cx::default());
        cx.frames.push(frame);
        cx.module = self as *const Self;
        let pointer = &mut *cx as *mut boundary::Cx;
        let _resources = unsafe { boundary::resource_scope(pointer, true) };
        // The pointer remains live until this synchronous call finishes; guest callbacks never retain it.
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            let _resources = unsafe { boundary::resource_scope(pointer, false) };
            unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        let mut input = Writer::new();
        if let Some(handler) = handler {
            input.put(&handler);
        }
        encode(&mut input);
        if input.error().is_some() {
            return None;
        }
        let at = guest::input(input.bytes().len());
        unsafe {
            std::ptr::copy_nonoverlapping(input.bytes().as_ptr(), at, input.bytes().len());
        }
        let (at, len) = guest::run(id);
        guest::set_transport(None);
        if len == u32::MAX {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(at, len as usize) };
        let mut forged;
        let bytes = if matches!(APPEND_ATTACK.get(), 1 | 2) {
            forged = bytes.to_vec();
            let at = forged
                .windows(4)
                .position(|b| b == 0x7fc01234u32.to_le_bytes())
                .unwrap();
            if APPEND_ATTACK.get() == 1 {
                forged[at..at + 4].copy_from_slice(&1.0f32.to_le_bytes());
            } else {
                let count = u32::from_le_bytes(forged[at - 8..at - 4].try_into().unwrap());
                forged[at - 8..at - 4].copy_from_slice(&(count - 1).to_le_bytes());
                forged.drain(at - 4..at + 4);
            }
            forged.as_slice()
        } else {
            bytes
        };
        let bytes = if self.corrupt || APPEND_ATTACK.get() == 3 {
            &bytes[..bytes.len().saturating_sub(1)]
        } else {
            bytes
        };
        decode(&mut Reader::new(bytes)).ok()
    }
}
struct Context;
impl LoopContext for Context {
    fn side(&self) -> mod_api::LoopSide {
        mod_api::LoopSide::Server
    }
    fn registry(&self, _: &str) -> Option<Arc<[String]>> {
        None
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn mods(&self) -> mod_api::LoadedMods {
        unreachable!("not used by this fixture")
    }
}
#[unsafe(no_mangle)]
fn __horizon_mod_hooks() -> Box<dyn Hooks> {
    struct Mod;
    impl Hooks for Mod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            REGISTERED_ITEM.set(Some(registrar.item_key("minecraft:stone")));
            REGISTERED_BLOCK.set(Some(registrar.block_key("minecraft:stone")));
            REGISTERED_EFFECT.set(Some(registrar.effect_key("minecraft:poison")));
            let kind = registrar.block_entity_type("counter").blocks(&[]).build();
            REGISTERED_KIND.set(Some(kind));
            registrar.block_entity_part(&kind, MachineBlock).unwrap();
        }
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[Some(mod_api::HookId::RenderGuiPre)]
        }
        fn listeners(&self) -> &'static [mod_api::Listener] {
            &[mod_api::Listener {
                hook: Some(mod_api::HookId::RenderGuiPre),
                priority: -37,
                receive_cancelled: true,
            }]
        }
        fn render_gui_pre(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::RenderGuiPreEvent,
        ) -> mod_api::Flow {
            assert!(event.cancelled);
            event.cancelled = false;
            mod_api::Flow::Continue
        }
        fn item_tooltip(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut mod_api::ItemTooltipEvent<'_>,
        ) {
            let mask = u32::from(event.display.name_visible(false))
                | (u32::from(event.display.name_visible(true)) << 1)
                | (u32::from(event.display.shows("minecraft:banner_patterns")) << 2)
                | (u32::from(event.display.shows("test:hidden")) << 3);
            event
                .lines
                .push((mod_api::Text::literal("projection"), mask));
        }
        fn apply_effect_tick(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::ApplyEffectTickEvent<'_>,
        ) {
            event.heal(2.5);
            event.hurt("minecraft:wither", 3.);
            event.eat(2, 0.25);
            event.add_exhaustion(0.5);
            event.keep = false;
        }
        fn mob_layer(&self, _: &dyn mod_api::ClientHost, event: &mut mod_api::MobLayerEvent<'_>) {
            assert_eq!(event.mob.kind, "test:mob");
            assert_eq!(event.mob.custom_name, Some("custom"));
            if let Some(who) = event.who {
                assert_eq!((who.name, who.local), ("test:player", true));
                event.draw.set_light(42);
            } else {
                event.draw.set_light(7);
            }
        }
        fn game_rule_changed(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::GameRuleChangedEvent<'_>,
        ) {
            assert_eq!(event.rule, "test:rule");
            if let Some(actor) = event.actor {
                assert_eq!(
                    (actor.uuid, actor.entity_id, actor.name),
                    (123, 45, "test:actor")
                );
                event.value = mod_api::GameRuleValue::Bool(true);
            } else {
                event.value = mod_api::GameRuleValue::Bool(false);
            }
        }
        fn loot_drops(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LootDropsEvent<'_>,
        ) {
            assert_eq!(event.context.entity, Some((12, "test:mob")));
            assert_eq!(event.context.table.len(), 300);
            event.drops.clear();
        }
        fn item_stacked_on_other(
            &self,
            _: &dyn LoopContext,
            event: &mut mod_api::ItemStackedOnOtherEvent<'_, '_>,
        ) -> mod_api::Flow {
            *event.event.carried.0 = None;
            event.event.cancellation_result = true;
            mod_api::Flow::Continue
        }
        fn living_jump_power(
            &self,
            context: &dyn LoopContext,
            event: &mut mod_api::LivingJumpPowerEvent<'_>,
        ) {
            // This is a plain import made through the generated Guest, not a stubbed reply.
            assert!(matches!(context.side(), mod_api::LoopSide::Server));
            event.entity = 99;
            event.power = 7.25;
            event.kind = "must not write back";
        }
    }
    Box::new(Mod)
}
#[derive(Debug)]
struct Piece;
impl StructurePiece for Piece {
    fn bounding_box(&self) -> mod_api::BoundingBox {
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    }
    fn post_process(
        &self,
        _: &mut dyn mod_api::PieceContext,
    ) -> Result<(), mod_api::StructureError> {
        Ok(())
    }
    fn add_additional_save_data(&self, _: &mut mod_api::Compound) {}
}
struct MachineBlock;
impl mod_api::BlockPart for MachineBlock {
    fn block_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[]
    }
}
impl mod_api::BlockEntityPart for MachineBlock {
    fn block_entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::BlockEntityTick)]
    }
    fn block_entity(
        &self,
        level: mod_api::LevelSide<'_>,
        event: &mut mod_api::BlockEntityEvent<'_>,
    ) -> Option<mod_api::Compound> {
        assert_eq!(event.block, "test:machine");
        matches!(level, mod_api::LevelSide::Server(_)).then(mod_api::Compound::new)
    }
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        event.entity.data[0] += 3;
    }
}
#[test]
fn wire_loopback_machine_tick_uses_checked_snapshot_writeback() {
    use mod_api::BlockEntityPart;
    let id = guest::add_handler(vec![guest::view::<dyn mod_api::BlockEntityPart>(Arc::new(
        MachineBlock,
    ))]);
    let mut entity = mod_api::ModBlockEntity::default();
    entity.data = vec![5];
    let identity = entity.id().to_string();
    let born = entity.revision().identity();
    let proxy = WasmHandler {
        index: id,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        },
    };
    proxy.block_entity_tick(
        &mut guest::Guest,
        &mut mod_api::BlockEntityTickEvent {
            pos: [1, 2, 3],
            entity: mod_api::ModBlockEntityView::new(&mut entity),
        },
    );
    assert_eq!(entity.data, vec![8]);
    assert_eq!(entity.id(), identity);
    assert_eq!(entity.revision().identity(), born);
    let before = entity.revision().changes();
    let bad = WasmHandler {
        index: id,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: true,
        },
    };
    bad.block_entity_tick(
        &mut guest::Guest,
        &mut mod_api::BlockEntityTickEvent {
            pos: [1, 2, 3],
            entity: mod_api::ModBlockEntityView::new(&mut entity),
        },
    );
    assert_eq!(entity.data, vec![8]);
    assert_eq!(entity.revision().changes(), before);
}
#[test]
fn borrowed_view_reply_is_byte_identical_to_owned_snapshot() {
    use horizon_core::nbt::Tag;
    install_counter();
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 17).unwrap();
    let mut entity = mod_api::ModBlockEntity::default();
    entity.items = vec![Some(stack), None];
    entity.data = vec![i32::MIN, 7, i32::MAX];
    entity.nbt.insert(
        "nested".into(),
        Tag::compound([
            ("float", Tag::Float(0x7fc01234)),
            ("double", Tag::Double(0x7ff8000000001234)),
            ("negative_zero", Tag::Float((-0.0f32).to_bits())),
            (
                "list",
                Tag::List(vec![
                    Tag::Double((-0.0f64).to_bits()),
                    Tag::String("payload".into()),
                ]),
            ),
        ]),
    );
    let mut update = mod_api::Compound::new();
    update.insert("display".into(), Tag::Int(42));
    mod_api::ModBlockEntityView::new(&mut entity).set_client_update_fields(update);
    let mut owned = Writer::new();
    owned.put(&mod_api::item::containers::ModBlockEntitySnapshot::from(
        &entity,
    ));
    let mut borrowed = Writer::new();
    borrowed.put(&mod_api::ModBlockEntityView::new(&mut entity));
    assert!(owned.error().is_none());
    assert!(borrowed.error().is_none());
    assert_eq!(borrowed.bytes(), owned.bytes());
    let mut reply = Writer::new();
    mod_api::ModBlockEntityView::new(&mut entity).__wire_put_snapshot(&mut reply);
    assert!(reply.error().is_none());
    assert_eq!(reply.bytes(), owned.bytes());
}
fn static_reply(
    tag: u8,
    id: u32,
    text: Option<&str>,
    trailing: bool,
) -> Result<&'static str, WireError> {
    let mut out = Writer::new();
    out.put(&tag);
    out.put(&id);
    if let Some(text) = text {
        out.put(text);
    }
    if trailing {
        out.put(&7u8);
    }
    let bytes = out.into_bytes();
    guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
    let result = guest::host(0, guest::begin())
        .take_static::<()>()
        .map(|(text, ())| text);
    guest::set_transport(None);
    result
}
#[test]
fn static_reply_definitions_are_checked_before_acknowledgement() {
    guest::reset_static_strings();
    assert_eq!(
        static_reply(0, 0, None, false),
        Err(WireError::Invalid("unknown static string id"))
    );
    assert_eq!(static_reply(1, 1, Some("second"), false).unwrap(), "second");
    assert_eq!(
        guest::static_ack(),
        1,
        "acknowledge only the exact decoded id, independently of holes"
    );
    let first = static_reply(1, 0, Some("first"), false).unwrap();
    assert_eq!(guest::static_ack(), 0);
    assert!(std::ptr::eq(
        first,
        static_reply(0, 0, None, false).unwrap()
    ));
    assert!(std::ptr::eq(
        first,
        static_reply(1, 0, Some("first"), false).unwrap()
    ));
    assert!(static_reply(1, 0, Some("changed"), false).is_err());
    assert_eq!(
        static_reply(1, 2, Some("not admitted"), true),
        Err(WireError::Trailing)
    );
    assert_eq!(guest::static_ack(), 0);
    assert!(static_reply(0, 2, None, false).is_err());
    assert_eq!(static_reply(9, 0, None, false), Err(WireError::Tag(9)));
    assert!(static_reply(1, u32::MAX, Some("bad id"), false).is_err());
    assert!(static_reply(1, 2, Some(&"x".repeat(257)), false).is_err());
    for tail in [None, Some(2u8)] {
        let mut out = Writer::new();
        out.put(&1u8);
        out.put(&2u32);
        out.put("unadmitted scalar tail");
        if let Some(tail) = tail {
            out.put(&tail);
        }
        let bytes = out.into_bytes();
        guest::set_transport(Some(Rc::new(move |_, _| bytes.clone())));
        assert!(
            guest::host(0, guest::begin())
                .take_static::<bool>()
                .is_err()
        );
        guest::set_transport(None);
        assert!(static_reply(0, 2, None, false).is_err());
    }
    assert_eq!(
        static_reply(2, u32::MAX, Some("uncached"), false).unwrap(),
        "uncached"
    );
    assert_eq!(
        guest::static_ack(),
        0,
        "uncached text does not claim a table id"
    );
    assert!(static_reply(2, 0, Some("bad marker"), false).is_err());
    guest::reset_static_strings();
    assert_eq!(guest::static_ack(), u32::MAX);
    assert!(
        static_reply(0, 0, None, false).is_err(),
        "a fresh instance cannot resolve an old id"
    );
}
#[test]
fn static_reply_table_is_bounded() {
    guest::reset_static_strings();
    for id in 0..4096 {
        assert_eq!(static_reply(1, id, Some("value"), false).unwrap(), "value");
    }
    assert_eq!(guest::static_ack(), 4095);
    assert!(static_reply(1, 4096, Some("overflow"), false).is_err());
    assert_eq!(guest::static_ack(), 4095);
}
#[test]
fn native_static_reply_resends_until_guest_acknowledges() {
    use std::cell::Cell;
    struct World(Cell<usize>);
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World, except(config));
    impl mod_api::prelude::ConfigHost for World {
        fn dimension(&self) -> &'static str {
            self.0.set(self.0.get() + 1);
            "minecraft:overworld"
        }
    }
    let world = World(Cell::new(0));
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let pointer = &mut cx as *mut boundary::Cx;
    let _resources = unsafe { boundary::resource_scope(pointer, true) };
    let id = boundary::import_id("Host", "dimension").unwrap();
    let mut call = |ack: u32| {
        let mut args = Writer::new();
        args.put(&ack);
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, args.bytes(), &mut out) }?;
        Ok::<_, boundary::ServeError>(out.into_bytes())
    };
    assert!(call(1).is_err());
    assert_eq!(
        world.0.get(),
        0,
        "invalid acknowledgement must refuse before native invocation"
    );
    for ack in [u32::MAX, u32::MAX, 0, u32::MAX] {
        let bytes = call(ack).unwrap();
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.take::<u8>().unwrap(), u8::from(ack == u32::MAX));
        assert_eq!(reader.take::<u32>().unwrap(), 0);
        if ack == u32::MAX {
            assert_eq!(reader.take::<String>().unwrap(), "minecraft:overworld");
        }
        assert!(reader.rest().is_empty());
    }
    assert_eq!(world.0.get(), 4);
    assert!(call(2).is_err());
    assert_eq!(world.0.get(), 4);
}
#[test]
fn native_static_reply_table_refuses_overflow_without_losing_existing_ids() {
    use std::cell::Cell;
    struct World(Cell<&'static str>);
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World, except(config));
    impl mod_api::prelude::ConfigHost for World {
        fn dimension(&self) -> &'static str {
            self.0.get()
        }
    }
    let world = World(Cell::new(""));
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let pointer = &mut cx as *mut boundary::Cx;
    let _resources = unsafe { boundary::resource_scope(pointer, true) };
    let id = boundary::import_id("Host", "dimension").unwrap();
    let call = |ack: u32| {
        let mut args = Writer::new();
        args.put(&ack);
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, args.bytes(), &mut out) }?;
        Ok::<_, boundary::ServeError>(out.into_bytes())
    };
    for i in 0..4096 {
        world
            .0
            .set(Box::leak(format!("test:dimension_{i}").into_boxed_str()));
        assert!(call(u32::MAX).is_ok());
    }
    world.0.set("test:overflow");
    let uncached = call(u32::MAX).unwrap();
    let mut reader = Reader::new(&uncached);
    assert_eq!(reader.take::<u8>().unwrap(), 2);
    assert_eq!(reader.take::<u32>().unwrap(), u32::MAX);
    assert_eq!(reader.take::<String>().unwrap(), "test:overflow");
    assert!(reader.rest().is_empty());
    world.0.set("test:dimension_0");
    let bytes = call(0).unwrap();
    assert_eq!(bytes, [0, 0, 0, 0, 0]);
    assert!(
        call(4096).is_err(),
        "overflow must not insert a new table entry"
    );
}
#[test]
fn wire_loopback_readonly_private_projection() {
    use horizon_core::nbt::Tag;
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let stack = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap();
    let hidden = Tag::compound([(
        "hidden_components",
        Tag::List(vec![Tag::String("test:hidden".into())]),
    )]);
    let suppressed = Tag::compound([("hide_tooltip", Tag::Byte(1))]);
    for (component, ominous_banner, expected) in [
        (None, false, 15),
        (None, true, 11),
        (Some(&hidden), true, 7),
        (Some(&suppressed), false, 2),
    ] {
        let display = mod_api::TooltipDisplay::__wire_from_fields(component, ominous_banner);
        let mut lines = vec![(mod_api::Text::literal("prefix"), 99)];
        module.item_tooltip(
            &Client,
            &mut mod_api::ItemTooltipEvent {
                player: 1,
                player_entity_id: 2,
                stack: &stack,
                display: &display,
                advanced: false,
                creative: false,
                lines: &mut lines,
            },
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].1, 99);
        assert_eq!(lines[1].1, expected);
    }
}
#[test]
fn wire_loopback_checked_append_is_atomic_and_bit_exact() {
    use mod_api::EffectTickChange;
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            group: 0,
            subscribed: &[Some(mod_api::HookId::ApplyEffectTick)],
            listeners: &[],
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
    let holder = mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 7));
    mod_api::testing::with_mods(Arc::from(mods), || {
        for attack in [0, 1, 2, 3] {
            APPEND_ATTACK.set(attack);
            let mut changes = vec![EffectTickChange::Heal(f32::from_bits(0x7fc01234))];
            let mut keep = true;
            mod_api::dispatch_apply_effect_tick(
                &World,
                mod_api::__private::ApplyEffectTickCall {
                    effect: REGISTERED_EFFECT.get().unwrap(),
                    dimension: "test:world",
                    entity: Some(7),
                    amplifier: 0,
                    holder,
                    health: 10.,
                    max_health: 20.,
                    food: None,
                    changes: &mut changes,
                    keep: &mut keep,
                },
            );
            assert!(matches!(changes[0], EffectTickChange::Heal(v) if v.to_bits() == 0x7fc01234));
            if attack == 0 {
                assert!(!keep);
                assert_eq!(
                    &changes[1..],
                    &[
                        EffectTickChange::Heal(2.5),
                        EffectTickChange::Hurt("minecraft:wither".into(), 3.),
                        EffectTickChange::Eat(2, 0.25),
                        EffectTickChange::AddExhaustion(0.5)
                    ]
                );
            } else {
                assert!(
                    keep,
                    "refused prefix must also refuse the public keep writeback"
                );
                assert_eq!(changes.len(), 1);
            }
        }
        APPEND_ATTACK.set(0);
    });
}
#[test]
fn wire_loopback_optional_borrowed_render_record() {
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    struct Draw(u32);
    impl mod_api::BlockEntityDrawContext for Draw {
        fn set_light(&mut self, value: u32) {
            self.0 = value;
        }
    }
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mob = mod_api::MobRender {
        id: 1,
        kind: "test:mob",
        custom_name: Some("custom"),
        baby: false,
        partial: 0.5,
        age: 1.,
        body_yaw: 0.,
        head_yaw: 0.,
        pitch: 0.,
        walk_position: 0.,
        walk_speed: 0.,
        hurt_time: 0,
        death_time: 0,
        on_ground: true,
    };
    let player = mod_api::PlayerRender {
        name: "test:player",
        local: true,
    };
    for who in [None, Some(&player)] {
        let mut draw = Draw(0);
        module.mob_layer(
            &Client,
            &mut mod_api::MobLayerEvent {
                mob: &mob,
                who,
                draw: &mut draw,
                parts: None,
            },
        );
        assert_eq!(draw.0, if who.is_some() { 42 } else { 7 });
    }
}
#[test]
fn wire_loopback_optional_borrowed_record() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let actor: mod_api::GameRuleActor = mod_api::GameRuleActor {
        uuid: 123,
        entity_id: 45,
        name: "test:actor",
    };
    for actor in [None, Some(&actor)] {
        let mut event = mod_api::GameRuleChangedEvent {
            rule: "test:rule",
            value: mod_api::GameRuleValue::Bool(!actor.is_some()),
            actor,
        };
        module.game_rule_changed(&mut World, &mut event);
        assert_eq!(event.value, mod_api::GameRuleValue::Bool(actor.is_some()));
    }
}
#[test]
fn wire_loopback_array_of_optional_borrowed_records() {
    use mod_api::BlockEntityDrawContext;
    struct Draw(bool);
    impl BlockEntityDrawContext for Draw {
        fn cube(
            &mut self,
            from: [f32; 3],
            to: [f32; 3],
            faces: [Option<mod_api::CubeFace<'_>>; 6],
            tint: u32,
        ) -> bool {
            assert_eq!((from, to, tint), ([1.; 3], [2.; 3], 0x123456));
            assert!(faces[0].is_none());
            let face = faces[1].unwrap();
            assert_eq!(face.sprite, "test:face");
            assert_eq!(face.uv, Some([1., 2., 3., 4.]));
            assert!(faces[2..].iter().all(Option::is_none));
            self.0 = true;
            true
        }
    }
    let mut draw = Draw(false);
    let mut cx = Box::new(boundary::Cx::default());
    let mut frame = boundary::Frame::default();
    frame.set_block_entity_draw_context_mut(&mut draw);
    cx.frames.push(frame);
    let pointer = &mut *cx as *mut boundary::Cx;
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let sprite = String::from("test:face");
    let faces = [
        None,
        Some(mod_api::CubeFace::new(&sprite).with_uv([1., 2., 3., 4.])),
        None,
        None,
        None,
        None,
    ];
    assert!(guest::Guest.cube([1.; 3], [2.; 3], faces, 0x123456));
    assert!(draw.0);
    guest::set_transport(None);
}
#[test]
fn wire_loopback_borrowed_tuple_inside_optional_context() {
    install_counter();
    let table = "t".repeat(300);
    let context: mod_api::LootDrops = mod_api::__private::built::LootDrops {
        table: &table,
        dimension: "minecraft:overworld",
        origin: [0.; 3],
        block: None,
        entity: Some((12, "test:mob")),
        player: None,
        tool: None,
        explosion: None,
        used: false,
    }
    .into();
    let mut drops = vec![mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap()];
    WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    }
    .loot_drops(
        &mut guest::Guest,
        &mut mod_api::LootDropsEvent {
            context: &context,
            drops: &mut drops,
        },
    );
    assert!(drops.is_empty());
}
#[test]
fn wire_loopback_nested_tuple_reference_writes_back() {
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    install_counter();
    let mut carried = Some(mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1).unwrap());
    let mut clicked = guest::Guest;
    let mut stack = mod_api::StackedOnOther {
        player: None,
        player_entity_id: None,
        slot: 0,
        container: 0,
        container_slot: 0,
        primary: true,
        carried: mod_api::item::menu::CarriedAccess(&mut carried),
        clicked: &mut clicked,
        cancellation_result: false,
    };
    module.item_stacked_on_other(
        &Context,
        &mut mod_api::ItemStackedOnOtherEvent {
            event: &mut stack,
            cancelled: false,
        },
    );
    assert!(stack.carried.0.is_none());
    assert!(stack.cancellation_result);
}
thread_local! {
    static REGISTERED_ITEM:std::cell::Cell<Option<mod_api::ItemKey>> = const {std::cell::Cell::new(None)};
    static REGISTERED_BLOCK:std::cell::Cell<Option<mod_api::BlockKey>> = const {std::cell::Cell::new(None)};
    static REGISTERED_EFFECT:std::cell::Cell<Option<mod_api::EffectKey>> = const {std::cell::Cell::new(None)};
    static APPEND_ATTACK:std::cell::Cell<u8> = const {std::cell::Cell::new(0)};
    static REGISTERED_KIND:std::cell::Cell<Option<mod_api::BlockEntityTypeKey>> = const {std::cell::Cell::new(None)};
}
fn install_counter() {
    let mods = [mod_api::ModEntry::new(
        "wire_test",
        Box::new(WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        }),
    )];
    mod_api::__private::install_parts(
        &mods,
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )
    .unwrap();
}
#[test]
fn wire_loopback_registration_attaches_subscribed_part() {
    install_counter();
    assert!(
        mod_api::__private::block_entity_types()
            .iter()
            .any(|decl| decl.id == "wire_test:counter")
    );
    assert!(mod_api::__private::has_part_hook(
        mod_api::HookId::BlockEntityTick
    ));
}
#[test]
fn wire_loopback_tagged_frame_enum_preserves_side() {
    use mod_api::BlockEntityPart;
    install_counter();
    let index = guest::add_handler(vec![guest::view::<dyn mod_api::BlockEntityPart>(Arc::new(
        MachineBlock,
    ))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            subscribed: &[],
            listeners: &[],
            group: 0,
            corrupt: false,
        },
    };
    let mut event = mod_api::BlockEntityEvent {
        block: "test:machine",
        state: mod_api::BlockStateRecord::new(REGISTERED_BLOCK.get().unwrap(), 0).into(),
    };
    assert!(
        proxy
            .block_entity(mod_api::LevelSide::Server(&guest::Guest), &mut event)
            .is_some()
    );
    assert!(
        proxy
            .block_entity(mod_api::LevelSide::Client(&guest::Guest), &mut event)
            .is_none()
    );
}
#[test]
fn wire_loopback_owned_box_handlers_inside_result_and_vec() {
    use mod_api::ReloadApply;
    let trace = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut out = Writer::new();
    {
        let _resources = guest::resource_scope();
        let trace = trace.clone();
        let value: Result<Box<dyn ReloadApply>, u32> = Ok(<dyn ReloadApply>::new(move || {
            trace.lock().unwrap().push(7)
        }));
        out.put_owned(value);
    }
    assert_eq!(out.error(), None);
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    cx.module = &module;
    {
        let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
        let value = Reader::decode::<Result<Box<dyn ReloadApply>, u32>>(out.bytes())
            .unwrap()
            .unwrap();
        value.apply();
    }
    assert_eq!(*trace.lock().unwrap(), vec![7]);
    out.clear();
    {
        let _resources = guest::resource_scope();
        let values = (0..3)
            .map(|n| {
                let trace = trace.clone();
                <dyn ReloadApply>::new(move || trace.lock().unwrap().push(n))
            })
            .collect::<Vec<_>>();
        out.put_owned(values);
    }
    assert_eq!(out.error(), None);
    {
        let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
        for value in Reader::decode::<Vec<Box<dyn ReloadApply>>>(out.bytes()).unwrap() {
            value.apply();
        }
    }
    assert_eq!(*trace.lock().unwrap(), vec![7, 0, 1, 2]);
}
#[test]
fn wire_loopback_returned_frame_resource_uses_source_permissions() {
    struct Start {
        draws: i64,
    }
    impl mod_api::StructureRandom for Start {
        fn next_long(&mut self) -> i64 {
            self.draws += 1;
            41 + self.draws
        }
        fn next_int_bound(&mut self, _: i32) -> i32 {
            unreachable!()
        }
        fn next_float(&mut self) -> f32 {
            unreachable!()
        }
        fn next_double(&mut self) -> f64 {
            unreachable!()
        }
        fn next_boolean(&mut self) -> bool {
            unreachable!()
        }
    }
    impl mod_api::GenerationContext for Start {
        fn metadata(&self) -> &dyn mod_api::BlockMetadata {
            unreachable!()
        }
        fn random(&mut self) -> &mut dyn mod_api::StructureRandom {
            self
        }
        fn chunk(&self) -> mod_api::ChunkPos {
            unreachable!()
        }
        fn seed(&self) -> i64 {
            unreachable!()
        }
        fn sea_level(&self) -> i32 {
            unreachable!()
        }
        fn min_y(&self) -> i32 {
            unreachable!()
        }
        fn max_y(&self) -> i32 {
            unreachable!()
        }
        fn base_height(&mut self, _: i32, _: i32, _: mod_api::HeightmapType) -> i32 {
            unreachable!()
        }
        fn base_columns(&mut self, _: i32, _: i32, _: &[mod_api::HeightmapType]) -> Vec<Vec<bool>> {
            unreachable!()
        }
        fn noise_column(&mut self, _: i32, _: i32) -> mod_api::NoiseColumn {
            unreachable!()
        }
        fn noise_biome(&mut self, _: [i32; 3]) -> mod_api::BiomeId {
            unreachable!()
        }
        fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
            unreachable!()
        }
        fn config(&self) -> Arc<mod_api::Tag> {
            unreachable!()
        }
    }
    let mut start = Start { draws: 0 };
    let mut cx = boundary::Cx::default();
    let mut frame = boundary::Frame::default();
    frame.set_generation_context_mut(&mut start);
    cx.frames.push(frame);
    let mut out = Writer::new();
    let id = boundary::import_id("StructureRandom", "next_long").unwrap();
    unsafe { boundary::serve(id, &mut cx, &[], &mut out) }.unwrap();
    assert_eq!(Reader::decode::<i64>(out.bytes()).unwrap(), 42);
    assert_eq!(start.draws, 1);
    cx.frames.clear();
    let mut frame = boundary::Frame::default();
    frame.set_generation_context(&start);
    cx.frames.push(frame);
    out.clear();
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, &[], &mut out) },
        Err(boundary::ServeError::Missing(_))
    ));
    assert_eq!(start.draws, 1);
    assert!(out.bytes().is_empty());
}
#[test]
fn wire_loopback_provider_queries_return_owned_options_and_atomic_cursor_writeback() {
    use mod_api::*;
    struct Table;
    impl CapabilityProviders for Table {}
    impl BlockEntitiesCapabilityProviders for Table {
        fn block_entity_provider(
            &self,
            _: &str,
            _: BlockEntityTypeKey,
            index: &mut u32,
        ) -> Option<(BlockProviderOf, &'static str, bool)> {
            *index += 1;
            Some((
                BlockProviderOf::EnergyHandler(Arc::new(|_: &mut dyn ServerHost, _: BlockSide| {
                    None::<Box<dyn EnergyHandler>>
                })),
                "test:block",
                true,
            ))
        }
    }
    impl EntitiesCapabilityProviders for Table {
        fn entity_provider(
            &self,
            _: &str,
            _: &str,
            index: &mut u32,
        ) -> Option<(EntityProviderOf, &'static str, bool)> {
            *index += 2;
            Some((
                EntityProviderOf::EnergyHandlerOptionDirection(Arc::new(
                    |_: &mut dyn ServerHost, _: EntityRef, _: Option<Direction>| {
                        None::<Box<dyn EnergyHandler>>
                    },
                )),
                "test:entity",
                false,
            ))
        }
    }
    impl ItemsCapabilityProviders for Table {
        fn item_provider(
            &self,
            _: &str,
            _: ItemKey,
            index: &mut u32,
        ) -> Option<(ItemProviderOf, &'static str, bool)> {
            *index += 3;
            Some((
                ItemProviderOf::EnergyHandler(Arc::new(|_: &mut dyn ServerHost, _: &ItemPlace| {
                    None::<Box<dyn EnergyHandler>>
                })),
                "test:item",
                true,
            ))
        }
        fn place_provider(&self, _: &str) -> Option<(Arc<dyn PlaceProvider>, &'static str)> {
            None
        }
    }
    impl CapabilitiesCapabilityProviders for Table {
        fn provided(&self, _: ProvidedBy, _: &str) -> bool {
            true
        }
    }
    struct World;
    impl Host for World {
        fn capability_providers(&self) -> &'static dyn CapabilityProviders {
            &Table
        }
    }
    mod_api::impl_host_areas!(World);
    install_counter();
    let world = World;
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut cx = Box::new(boundary::Cx::default());
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    cx.frames.push(frame);
    cx.module = &module;
    let pointer = &mut *cx as *mut boundary::Cx;
    let _root = unsafe { boundary::resource_scope(pointer, true) };
    let _guest = guest::resource_scope();
    let corrupt = Rc::new(std::cell::Cell::new(false));
    let flag = corrupt.clone();
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        if id == mod_api::wire::HOST_LOG {
            return Vec::new();
        }
        if id == mod_api::wire::HOST_RELEASE {
            let id = Reader::decode(bytes).unwrap();
            let released = unsafe { (&mut *pointer).held.remove(id) }.unwrap();
            drop(released);
            return Vec::new();
        }
        let _scope = unsafe { boundary::resource_scope(pointer, false) };
        let mut out = Writer::new();
        unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
        if flag.get() && id != mod_api::wire::HOST_RELEASE {
            out.put(&0xffu8)
        }
        out.into_bytes()
    })));
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            guest::set_transport(None);
        }
    }
    let _reset = Reset;
    let mut index = 5;
    let (block, owner, before) = guest::Guest
        .block_entity_provider("test:energy", REGISTERED_KIND.get().unwrap(), &mut index)
        .unwrap();
    assert!(matches!(block, BlockProviderOf::EnergyHandler(_)));
    assert_eq!((index, owner, before), (6, "test:block", true));
    let (entity, owner, before) = guest::Guest
        .entity_provider("test:energy", "test:mob", &mut index)
        .unwrap();
    assert!(matches!(
        entity,
        EntityProviderOf::EnergyHandlerOptionDirection(_)
    ));
    assert_eq!((index, owner, before), (8, "test:entity", false));
    let (item, owner, before) = guest::Guest
        .item_provider("test:energy", REGISTERED_ITEM.get().unwrap(), &mut index)
        .unwrap();
    assert!(matches!(item, ItemProviderOf::EnergyHandler(_)));
    assert_eq!((index, owner, before), (11, "test:item", true));
    corrupt.set(true);
    index = 42;
    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        guest::Guest.item_provider("test:energy", REGISTERED_ITEM.get().unwrap(), &mut index)
    }));
    assert!(refused.is_err());
    assert_eq!(
        index, 42,
        "trailing bytes applied the cursor before validating the whole reply"
    );
}

#[test]
fn wire_loopback_panic_hook_without_transport_does_not_panic_again() {
    install_counter();
    guest::set_transport(None);
    let panic = std::panic::catch_unwind(|| panic!("native panic after the guest call ended"));
    assert!(panic.is_err());
}

#[test]
fn wire_loopback_erased_resource_restores_identity_and_refuses_other_groups() {
    struct Capture(std::sync::Mutex<Vec<String>>);
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, r: &log::Record<'_>) {
            self.0.lock().unwrap().push(r.args().to_string());
        }
        fn flush(&self) {}
    }
    static LOG: Capture = Capture(std::sync::Mutex::new(Vec::new()));
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    let original: Arc<dyn std::any::Any + Send + Sync> = Arc::new(97u32);
    let handler: Arc<str> = format!("test:long-handler-{}", "a".repeat(300)).into();
    let mut outbound = Writer::new();
    {
        let _scope = guest::resource_scope();
        outbound.put(&handler);
        outbound.put(&original);
    }
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &module;
    let native: mod_api::OwnProvider = {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        Reader::decode(outbound.bytes()).unwrap()
    };
    let mut back = Writer::new();
    {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        back.put(&native);
    }
    assert_eq!(back.bytes(), outbound.bytes());
    {
        let _scope = guest::resource_scope();
        let mut input = Reader::new(back.bytes());
        assert_eq!(input.take::<Arc<str>>().unwrap(), handler);
        let restored: Arc<dyn std::any::Any + Send + Sync> = input.take().unwrap();
        assert!(input.rest().is_empty());
        assert!(Arc::ptr_eq(&original, &restored));
        assert_eq!(restored.downcast_ref::<u32>(), Some(&97));
    }
    let other_module = WasmMod {
        group: 1,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &other_module;
    for _ in 0..2 {
        back.clear();
        {
            let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
            back.put(&native);
        }
        let _scope = guest::resource_scope();
        let mut input = Reader::new(back.bytes());
        assert_eq!(input.take::<Arc<str>>().unwrap(), handler);
        let refused: Arc<dyn std::any::Any + Send + Sync> = input.take().unwrap();
        assert!(input.rest().is_empty());
        assert_eq!(
            refused
                .downcast_ref::<mod_api::wire::Unsupported>()
                .unwrap()
                .reason,
            "cross-group capability"
        );
        assert!(refused.downcast_ref::<u32>().is_none());
    }
    assert_eq!(
        LOG.0
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.contains(&*handler) && s.contains("cross-group capability"))
            .count(),
        1
    );
}

#[test]
fn wire_loopback_builtin_provider_invokes_nested_box_handler() {
    use mod_api::{EnergyHandler, ItemCapabilityProvider, ItemProviderOf, ServerHost, Transaction};
    struct Energy(u32);
    impl EnergyHandler for Energy {
        fn amount(&self, _: &mut dyn ServerHost) -> u32 {
            self.0
        }
        fn capacity(&self, _: &mut dyn ServerHost) -> u32 {
            self.0
        }
        fn insert(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
            0
        }
        fn extract(&self, _: &mut dyn ServerHost, _: &Transaction, _: u32) -> u32 {
            0
        }
    }
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    let place = mod_api::ItemPlace::PlayerSlot {
        player: 19,
        slot: 3,
    };
    let provider: Arc<dyn ItemCapabilityProvider<dyn EnergyHandler>> = Arc::new(
        |_: &mut dyn ServerHost, place: &mod_api::ItemPlace| match place {
            mod_api::ItemPlace::PlayerSlot { player: 19, slot } => {
                Some(Box::new(Energy(*slot as u32)) as Box<dyn EnergyHandler>)
            }
            _ => None,
        },
    );
    let mut out = Writer::new();
    {
        let _scope = guest::resource_scope();
        out.put(&ItemProviderOf::EnergyHandler(provider));
    }
    let mut cx = boundary::Cx::default();
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    cx.module = &module;
    let decoded: ItemProviderOf = {
        let _scope = unsafe { boundary::resource_scope(&mut cx, true) };
        Reader::decode(out.bytes()).unwrap()
    };
    let ItemProviderOf::EnergyHandler(provider) = decoded else {
        panic!("wrong typed provider")
    };
    let energy = provider.provide(&mut World, &place).unwrap();
    assert_eq!(energy.amount(&mut World), 3);
    assert_eq!(energy.capacity(&mut World), 3);
}

#[test]
fn wire_loopback_bad_import_bytes_are_typed_errors() {
    let mut cx = boundary::Cx::default();
    let mut out = Writer::new();
    let id = boundary::import_id("StructureRandom", "next_int_bound").unwrap();
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, &[0xff], &mut out) },
        Err(boundary::ServeError::Decode(_))
    ));
    assert!(out.bytes().is_empty());
}
#[test]
fn wire_loopback_owner_thread_guards_access_and_drop() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Local {
        _local: Rc<()>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Local {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let local = boundary::OwnerThread::new(Local {
        _local: Rc::new(()),
        drops: drops.clone(),
    });
    assert!(local.get().is_ok());
    std::thread::spawn(move || {
        assert!(matches!(
            local.get(),
            Err(boundary::ServeError::WrongThread)
        ));
        drop(local);
    })
    .join()
    .unwrap();
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "foreign-thread destruction must leak the thread-affine value"
    );
    drop(boundary::OwnerThread::new(Local {
        _local: Rc::new(()),
        drops: drops.clone(),
    }));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "creator-thread destruction must release the value"
    );
}
#[test]
fn wire_loopback_roundtrip_and_atomic_writeback() {
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    let mut event = mod_api::LivingJumpPowerEvent {
        entity: 42,
        kind: "original",
        power: 1.5,
    };
    module.living_jump_power(&Context, &mut event);
    assert_eq!(event.power, 7.25);
    assert_eq!(event.kind, "original");
    let bad = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: true,
    };
    event.entity = 42;
    event.power = 2.0;
    bad.living_jump_power(&Context, &mut event);
    assert_eq!(
        event.power, 2.0,
        "a truncated last field must leave every host field untouched"
    );
    assert_eq!(event.entity, 42);
    let id = guest::add_handler(vec![guest::view::<dyn StructurePiece>(Arc::new(Piece))]);
    let proxy = WasmHandler { index: id, module };
    assert_eq!(
        proxy.bounding_box(),
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    );
    let mut release = Writer::new();
    release.put(&vec![id]);
    assert_eq!(
        proxy.module.call(
            CallClass::Load,
            boundary::Frame::default(),
            mod_api::wire::RELEASE,
            None,
            |w| w.put_bytes(release.bytes()),
            |r| r.take::<()>()
        ),
        Some(())
    );
    assert!(
        proxy
            .module
            .call::<mod_api::BoundingBox>(
                CallClass::Worker,
                boundary::Frame::default(),
                boundary::export_id("StructurePiece", "bounding_box").unwrap(),
                Some(id),
                |_| {},
                |r| r.take()
            )
            .is_none(),
        "released handler must be refused"
    );
}

#[test]
fn wire_loopback_resource_codec_restores_context_between_groups() {
    fn encode() -> Id {
        let apply: Arc<dyn mod_api::ReloadApply> = <dyn mod_api::ReloadApply>::new(|| {}).into();
        let mut writer = Writer::new();
        writer.put_owned(apply);
        assert_eq!(writer.error(), None);
        Reader::decode(writer.bytes()).unwrap()
    }
    let mut first = Box::new(boundary::Cx::default());
    let mut second = Box::new(boundary::Cx::default());
    // Both contexts stay boxed and live through their private lexical guards.
    let outer = unsafe { boundary::resource_scope(&mut *first, true) };
    let first_id = encode();
    {
        let _inner = unsafe { boundary::resource_scope(&mut *second, true) };
        let second_id = encode();
        assert_eq!(
            second_id, first_id,
            "independent tables start with the same slot"
        );
        assert!(second.held.get(second_id).is_ok());
        assert!(mod_api::wire::borrow_token().unwrap().check().is_ok());
    }
    let next_first_id = encode();
    assert_ne!(next_first_id, first_id);
    assert!(first.held.get(first_id).is_ok());
    assert!(first.held.get(next_first_id).is_ok());
    assert!(second.held.get(next_first_id).is_err());
    drop(outer);
    let apply: Arc<dyn mod_api::ReloadApply> = <dyn mod_api::ReloadApply>::new(|| {}).into();
    let mut writer = Writer::new();
    writer.put_owned(apply);
    assert!(
        writer.error().is_some(),
        "no context survives the outer guard"
    );
}

#[test]
fn wire_loopback_resource_load_restores_module_after_cross_group_unwind() {
    fn load(id: Id) -> Arc<dyn StructurePiece> {
        let mut bytes = Writer::new();
        bytes.put(&id);
        Reader::decode(bytes.bytes()).unwrap()
    }
    let id = guest::add_handler(vec![guest::view::<dyn StructurePiece>(Arc::new(Piece))]);
    let mut first = Box::new(boundary::Cx::default());
    let first_module = WasmMod {
        group: 1,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    first.module = &first_module;
    let mut second = Box::new(boundary::Cx::default());
    let second_module = WasmMod {
        group: 2,
        subscribed: &[],
        listeners: &[],
        corrupt: true,
    };
    second.module = &second_module;
    let _outer = unsafe { boundary::resource_scope(&mut *first, true) };
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _inner = unsafe { boundary::resource_scope(&mut *second, true) };
        assert_eq!(load(id).bounding_box(), mod_api::BoundingBox::default());
        // Resume directly to avoid depending on the process-wide guest panic logger.
        std::panic::resume_unwind(Box::new(()));
    }));
    assert!(unwind.unwrap_err().is::<()>());
    assert_eq!(
        load(id).bounding_box(),
        mod_api::BoundingBox::new([1, 2, 3], [4, 5, 6])
    );
}

#[test]
fn listener_metadata_and_cancellation_cross_the_real_guest() {
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let metadata: Vec<(String, i32, bool)> = module
        .call(
            CallClass::Load,
            boundary::Frame::default(),
            boundary::SUBSCRIBED,
            None,
            |_| {},
            |r| r.take(),
        )
        .unwrap();
    assert_eq!(metadata, vec![("render_gui_pre".into(), -37, true)]);
    let event: mod_api::RenderGuiPreEvent = Reader::new(&[1]).take().unwrap();
    let id = boundary::export_id("Hooks", "render_gui_pre").unwrap();
    let (flow, event) = module
        .call(
            CallClass::Hook,
            boundary::Frame::default(),
            id,
            None,
            |w| w.put(&event),
            |r| {
                Ok((
                    r.take::<mod_api::Flow>()?,
                    r.take::<mod_api::RenderGuiPreEvent>()?,
                ))
            },
        )
        .unwrap();
    assert_eq!(flow, mod_api::Flow::Continue);
    assert!(!event.cancelled);
}
