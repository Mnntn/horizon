use mod_api::Hooks;

pub mod area_macros;
pub mod replace;
pub mod shutdown;
pub use replace::own_average;
pub use shutdown::on_shutdown;

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    #[listener(priority = 37)]
    fn server_stopped(&self, _: &mut dyn mod_api::ServerHost, _: &mut mod_api::ServerStoppedEvent) {
    }

    #[listener(priority = -12, receive_cancelled)]
    fn render_gui_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderGuiPreEvent,
    ) -> mod_api::Flow {
        event.cancelled = false;
        mod_api::Flow::Continue
    }

    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = registrar.block_entity_type("counter").blocks(&[]).build();
        registrar.block_entity_part(&kind, Counter).unwrap();
        let reads = registrar
            .block_entity_type("counter_reads")
            .blocks(&[])
            .build();
        registrar.block_entity_part(&reads, CounterReads).unwrap();
    }
}

// Only the guest ABI exports reach this fixture function; the bench target does not call it.
#[allow(dead_code)]
pub fn on_tick(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    args.cancel = true;
}

pub struct Counter;
pub struct CounterReads;

#[mod_api::horizon_part]
impl mod_api::BlockPart for CounterReads {}

#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for CounterReads {
    fn block_entity_tick(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        let first = host.is_loaded("wasm_fixture");
        let second = host.is_loaded("missing");
        event
            .entity
            .nbt
            .insert("counter".into(), mod_api::Tag::Int(17));
        event.entity.data = vec![if first == second { 24 } else { 25 }];
        event.entity.items.clear();
    }
}

#[mod_api::horizon_part]
impl mod_api::BlockPart for Counter {}

#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for Counter {
    fn block_entity_tick(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        event
            .entity
            .nbt
            .insert("counter".into(), mod_api::Tag::Int(17));
        event.entity.data = vec![23];
        event.entity.items.clear();
    }
}
