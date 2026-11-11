use super::*;

impl Game<'_> {
    pub(super) fn dark_outside(&self) -> bool {
        !self.dimension_type().has_fixed_time && self.sky_darken() >= 4
    }
    pub(super) fn can_have_weather(&self) -> bool {
        level_can_have_weather(
            self.world.shape.sky_light,
            self.world.shape.has_ceiling,
            self.dim == Dim::End,
        )
    }
    pub(super) fn weather_levels(&self, partial: f32) -> (f32, f32) {
        if self.can_have_weather() {
            self.world.weather.levels(partial)
        } else {
            (0.0, 0.0)
        }
    }
}
