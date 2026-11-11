use horizon_client::game::Game;
use horizon_client::game::autosave_interval;

pub fn next_autosave_interval(this: &Game<'_>) -> i64 {
    let rate = if this.clock.sprinting() {
        1.0e9 / (this.clock.average_nanos() + 1) as f32
    } else {
        this.clock.rate()
    };
    autosave_interval(rate)
}
