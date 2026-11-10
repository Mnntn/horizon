// Test module extracted without changing its logical path.
use super::super::native_entity_refs::tests::with_game;
use super::super::native_item_places::tests::{guest, open_to_lan};
use horizon_client::items::ItemStack;
use mod_api::{Host, HostSide, PlayerRef};
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq)]
struct Snapshot {
    phase: &'static str,
    side: &'static str,
    uuid: u128,
    diamonds: u32,
    xp: i32,
    cooldown: Option<i32>,
}

struct Log(Arc<Mutex<Vec<Snapshot>>>);
impl Log {
    fn record(&self, side: HostSide<'_>, player: PlayerRef, phase: &'static str) {
        let (host, side, cooldown): (&dyn Host, _, _) = match side {
            HostSide::Server(host) => {
                assert_eq!(
                    host.players().acting_player().unwrap().uuid(),
                    player.uuid()
                );
                let cooldown = player
                    .item_cooldowns(host)
                    .unwrap()
                    .first()
                    .map(|c| c.ticks_left);
                (host, "server", cooldown)
            }
            HostSide::Client(host) => {
                if host.local_player() != Some(player) {
                    assert_eq!(player.uuid(), 789);
                    assert_eq!(player.entity().id(), 66);
                    assert!(player.inventory(host).is_none());
                }
                (host, "client", None)
            }
        };
        self.0.lock().unwrap().push(Snapshot {
            phase,
            side,
            uuid: player.uuid(),
            diamonds: player
                .inventory(host)
                .unwrap_or_default()
                .iter()
                .filter(|s| {
                    mod_api::__private::item_key_id(s.item()).unwrap() == "minecraft:diamond"
                })
                .map(|s| s.count())
                .sum(),
            xp: player.experience(host).map_or(0, |xp| xp.total),
            cooldown,
        });
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Log {
    fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
        self.record(host, event.player, "pre");
    }
    fn player_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPostEvent) {
        self.record(host, event.player, "post");
    }
    fn can_continue_sleeping(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::CanContinueSleepingEvent<'_>,
    ) -> Option<bool> {
        // The sleep rule runs inside the tick, never ahead of Pre.
        let heard = self.0.lock().unwrap();
        assert_eq!(heard.last().unwrap().phase, "pre");
        assert_eq!(heard.last().unwrap().uuid, event.entity);
        Some(false)
    }
}

#[test]
#[ignore = "needs prepared resources"]
fn guest_tick_brackets_sleep_cooldowns_pickups_and_death() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_api::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut g = guest(1);
            g.loaded = true;
            g.bed = Some([0, 0, 0]);
            g.sleeping = true;
            g.sleep_timer = 4;
            let mut other = guest(2);
            other.loaded = true;
            other.position = [50.0, 0.0, 50.0];
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.extend([g, other]);
            host.play
                .entry(1)
                .or_default()
                .cooldowns
                .add("minecraft:ender_pearl".into(), 3);
            let diamond = ItemStack::new("minecraft:diamond", 2);
            game.spawn_item(diamond.clone(), [0.0; 3], [0.0; 3], 0);
            let level = game.level("minecraft:overworld").unwrap();
            assert!(game.level_award_experience(level, 0.0, 0.0, 0.0, 7));
            game.tick_guests();
            let observations = heard.lock().unwrap();
            assert_eq!(
                observations
                    .iter()
                    .map(|s| (s.phase, s.uuid))
                    .collect::<Vec<_>>(),
                [("pre", 123), ("post", 123), ("pre", 124), ("post", 124)]
            );
            assert_eq!(
                observations[0],
                Snapshot {
                    phase: "pre",
                    side: "server",
                    uuid: 123,
                    diamonds: 0,
                    xp: 0,
                    cooldown: Some(3)
                }
            );
            assert_eq!(
                observations[1],
                Snapshot {
                    phase: "post",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(observations[2].diamonds, 0);
            drop(observations);
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].sleep_timer, 100);
            assert!(game.lan.as_ref().unwrap().hub.guests[0].bed.is_none());
            assert!(game.inventory.iter().all(Option::is_none));
            heard.lock().unwrap().clear();
            // A real zero-health death-screen tick still gets both endpoints,
            // without use/cooldown/pickup work re-enabled.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.dead = true;
                g.health = 0.0;
            });
            game.spawn_item(diamond, [0.0; 3], [0.0; 3], 0);
            assert!(game.level_award_experience(level, 0.0, 0.0, 0.0, 7));
            game.tick_guests();
            let observations = heard.lock().unwrap();
            assert_eq!(
                observations[0],
                Snapshot {
                    phase: "pre",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(
                observations[1],
                Snapshot {
                    phase: "post",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(observations.len(), 4);
            assert_eq!(game.dropped.len(), 1);
        })
    });
}

#[test]
#[ignore = "needs prepared resources"]
fn client_player_ticks_on_the_death_screen_too() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_api::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "tick-client").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.profile = 456;
            remote.entity_id = 42;
            game.player = horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::ZERO,
            )
            .ok();
            game.remote.as_mut().unwrap().level_players.insert(66, 789);
            let kind = game
                .remote
                .as_ref()
                .unwrap()
                .player_type
                .as_ref()
                .unwrap()
                .clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                66,
                kind,
                horizon_core::geometry::Vec3::ZERO,
                0.0,
            ));
            game.cooldowns.add("minecraft:ender_pearl".into(), 3);
            game.sleep_timer = 4;
            // LocalPlayer.tick waits for the client load and its chunk: only
            // the other player's pair runs, and the level pass still ticks.
            game.tick_remote().unwrap();
            // Player.tick owns both counters, so they wait with it.
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 3);
            assert_eq!(game.sleep_timer, 4);
            let phases = || {
                std::mem::take(&mut *heard.lock().unwrap())
                    .iter()
                    .map(|s| (s.phase, s.side, s.uuid))
                    .collect::<Vec<_>>()
            };
            assert_eq!(phases(), [("pre", "client", 789), ("post", "client", 789)]);
            game.remote.as_mut().unwrap().loaded_sent = true;
            game.streaming
                .insert_remote(&mut game.world, horizon_client::chunk::Chunk::empty(0, 0));
            // The other player's tick ends before the local player's starts.
            game.tick_remote().unwrap();
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 2);
            assert_eq!(game.sleep_timer, 5);
            game.screen = super::super::Screen::Dead;
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            game.tick_remote().unwrap();
            assert_eq!(
                phases(),
                [
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456),
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456)
                ]
            );
            server.shutdown();
        })
    });
}
