use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before the open screen draws itself (`ScreenEvent.Render.Pre`): draw
    /// here to put something under it, or set `canceled` and the screen is not
    /// drawn (nor its `native_screen_render_background`).
    pub(in crate::game) fn native_screen_render_pre(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
        canceled: &mut bool,
    ) {
        self.fire_screen_render_pre(draw, screen, canceled);
    }
    /// Before the open screen's contents (`ScreenEvent.Render.Background`).
    pub(in crate::game) fn native_screen_render_background(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
    ) {
        self.fire_screen_render_background(draw, screen);
    }
    /// After the open screen's contents (`ScreenEvent.Render.Foreground`).
    pub(in crate::game) fn native_screen_render_foreground(
        &self,
        draw: &mut HudBatch,
        screen: &ScreenView<'_>,
    ) {
        self.fire_screen_render_foreground(draw, screen);
    }
}
