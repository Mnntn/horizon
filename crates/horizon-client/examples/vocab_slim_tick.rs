use mod_api::*;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static COUNTS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
struct TickWorkload;
#[mod_api::__subscriptions]
impl Hooks for TickWorkload {
    fn server_tick(&self, _: &mut dyn ServerHost, e: &mut ServerTickEvent<'_>) { COUNTS[0].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn level_tick_pre(&self, _: HostSide, e: &mut LevelTickPreEvent) { COUNTS[1].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn level_tick_post(&self, _: HostSide, e: &mut LevelTickPostEvent) { COUNTS[2].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn entity_tick_listened(&self, _: &mut dyn ServerHost, e: &mut EntityTickListenedEvent) { COUNTS[3].fetch_add(1, Relaxed); e.listened = true; }
    fn entity_tick_pre(&self, _: &mut dyn ServerHost, e: &mut EntityTickPreEvent<'_>) -> Flow { COUNTS[4].fetch_add(1, Relaxed); std::hint::black_box(e); Flow::Continue }
    fn entity_tick_post(&self, _: &mut dyn ServerHost, e: &mut EntityTickPostEvent<'_>) { COUNTS[5].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn player_tick_pre(&self, _: HostSide, e: &mut PlayerTickPreEvent) { COUNTS[6].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn player_tick_post(&self, _: HostSide, e: &mut PlayerTickPostEvent) { COUNTS[7].fetch_add(1, Relaxed); std::hint::black_box(e); }
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) { r.block_part_tag("minecraft:dirt", BlockTicker); }
    fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut ServerStoppedEvent) {
        eprintln!("VOCAB_TICK_COUNTS {:?}", COUNTS.each_ref().map(|n| n.load(Relaxed)));
    }
}
struct BlockTicker;
#[mod_api::horizon_part]
impl BlockPart for BlockTicker {
    fn random_tick(&self, _: &mut dyn LevelWriteHost, e: &mut RandomTickEvent) -> Flow { COUNTS[8].fetch_add(1, Relaxed); std::hint::black_box(e); Flow::Continue }
    fn tick_block(&self, _: &mut dyn LevelWriteHost, e: &mut TickBlockEvent) -> Flow { COUNTS[9].fetch_add(1, Relaxed); std::hint::black_box(e); Flow::Continue }
}
static GAME_IDENTITY: [u8; horizon_mod_package::game_identity::MARK.len()] = horizon_mod_package::game_identity::MARK;
const BUILT: &[horizon_mod_package::runtime::BuiltMod] = &[horizon_mod_package::runtime::BuiltMod { id: "vocab_tick", version: "0.1.0", side: horizon_mod_package::manifest::Side::Both }];
fn main() {
    let startup = horizon_client::entry::prepare();
    std::hint::black_box(&horizon_mod_package::holder::CAPABILITY);
    std::hint::black_box(&GAME_IDENTITY);
    horizon_client::entry::main_with(vec![ModEntry::new("vocab_tick", Box::new(TickWorkload))], &[], BUILT, startup);
}
