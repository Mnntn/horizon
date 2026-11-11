//! The explicit run deadline and the default deadline for frame-limited runs.
use super::Options;
use std::time::Duration;

impl Options {
    /// Explicit timeouts apply to every run; only frame-limited runs get a default.
    pub(super) fn time_limit(&self) -> Option<Duration> {
        self.timeout
            .or(self.frames.map(|_| Duration::from_secs(120)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(args: &[&str]) -> Option<Duration> {
        Options::parse(args.iter().map(|arg| arg.to_string()))
            .unwrap()
            .unwrap()
            .time_limit()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn explicit_deadline_exits_the_live_idle_path_without_frame_limit() {
        use crate::game::event_loop::EventLoopCtl;
        use std::cell::Cell;
        use winit::event_loop::ControlFlow;

        struct TestLoop(Cell<bool>);
        impl EventLoopCtl for TestLoop {
            fn exit(&self) {
                self.0.set(true);
            }
            fn set_control_flow(&self, _: ControlFlow) {}
        }

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.frames = None;
            game.options.world_frames = None;
            game.options.timeout = Some(Duration::from_secs(1));
            game.started = std::time::Instant::now() - Duration::from_secs(2);
            let event_loop = TestLoop(Cell::new(false));
            game.idle(&event_loop);
            assert!(event_loop.0.get(), "the live idle path must request exit");
            assert_eq!(
                game.error.as_ref().map(ToString::to_string).as_deref(),
                Some("run timed out after 1 s")
            );
        });
    }

    /// Upstream's regression: a control-socket run must not outlive its explicit deadline.
    #[test]
    fn timeout_seconds_limits_every_run() {
        let secs = |n| Some(Duration::from_secs(n));
        assert_eq!(limit(&[]), None);
        assert_eq!(limit(&["--control-socket", "c.sock"]), None);
        assert_eq!(
            limit(&["--control-socket", "c.sock", "--timeout-seconds", "5"]),
            secs(5)
        );
        assert_eq!(limit(&["--timeout-seconds", "600"]), secs(600));
        assert_eq!(limit(&["--frames", "10"]), secs(120));
        assert_eq!(limit(&["--world-frames", "10"]), secs(120));
        assert_eq!(
            limit(&["--frames", "10", "--timeout-seconds", "30"]),
            secs(30)
        );
    }
}
