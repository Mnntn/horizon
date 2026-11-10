#[cfg(test)]
mod tests {
    use super::super::super::{
        native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
    };
    use horizon_client::level_loading::Reason;
    use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
    use mod_api::{Hooks, HostSide, LevelRef, LevelTickPostEvent, LevelTickPreEvent, ModEntry};
    use std::sync::{Arc, Mutex};

    struct Moments(Arc<Mutex<Vec<(&'static str, LevelRef, Option<i64>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Moments {
        fn level_tick_pre(&self, host: HostSide, event: &mut LevelTickPreEvent) {
            let time = match host {
                HostSide::Client(h) => event.level.game_time(h),
                HostSide::Server(h) => event.level.game_time(h),
            };
            self.0.lock().unwrap().push(("pre", event.level, time));
        }
        fn level_tick_post(&self, host: HostSide, event: &mut LevelTickPostEvent) {
            let time = match host {
                HostSide::Client(h) => event.level.game_time(h),
                HostSide::Server(h) => event.level.game_time(h),
            };
            self.0.lock().unwrap().push(("post", event.level, time));
        }
    }

    /// One pre then one post for each client level tick, naming the level in
    /// place, and nothing before a tick or for a client with no level.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn client_level_tick_fires_pre_then_post_around_each_tick() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("moments", Box::new(Moments(heard.clone())))].into();
        mod_api::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.native_lifecycle = NativeLifecycle::default();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "LevelTickTest").unwrap();
                game.tick_remote_level().unwrap();
                assert!(heard.lock().unwrap().is_empty(), "no level means no event");
                let spawn = SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 1,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                game.remote.as_mut().unwrap().logged_in = true;
                let level = game.level("minecraft:overworld").unwrap();

                // Login alone is no tick, and neither is a level with no tick.
                assert!(heard.lock().unwrap().is_empty());

                let before = game.world.game_time;
                game.tick_remote_level().unwrap();
                let after = game.world.game_time;
                assert_eq!(after, before + 1, "the tick advanced the world's time");
                assert_eq!(
                    *heard.lock().unwrap(),
                    [("pre", level, Some(before)), ("post", level, Some(after))],
                    "the level's own work ran between them"
                );

                heard.lock().unwrap().clear();
                game.tick_remote_level().unwrap();
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("pre", level, Some(after)),
                        ("post", level, Some(after + 1))
                    ]
                );
                heard.lock().unwrap().clear();
                let failed = game.tick_remote_level_with(|_| Err("failed tick body".into()));
                assert!(failed.is_err());
                assert_eq!(
                    *heard.lock().unwrap(),
                    [("pre", level, Some(after + 1))],
                    "failed ticks have no post event"
                );
                heard.lock().unwrap().clear();
                let mut second = spawn.clone();
                second.dimension = "minecraft:the_nether".into();
                game.enter_remote_dimension(&second, false, Reason::Other);
                let other = game.level("minecraft:the_nether").unwrap();
                assert_ne!(level, other);
                game.tick_remote_level().unwrap();
                let moments = heard.lock().unwrap();
                assert_eq!(moments.len(), 2);
                assert_eq!((moments[0].1, moments[1].1), (other, other));
            });
        });
    }
}
