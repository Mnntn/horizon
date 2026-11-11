use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// A snapshot of the current graphics preferences.
    pub(in crate::game) fn graphics_options(&self) -> ClientGraphicsOptions {
        let s = &self.settings;
        ClientGraphicsOptions {
            fov: s.fov,
            render_distance: s.render_distance,
            simulation_distance: s.simulation_distance,
            brightness: s.gamma,
            max_fps: s.max_fps,
            vsync: s.vsync,
            fullscreen: s.fullscreen,
            view_bobbing: s.view_bobbing,
            smooth_lighting: s.smooth_lighting,
            particles: s.particles,
            entity_distance: s.entity_distance,
            biome_blend: s.biome_blend,
        }
    }
    /// A snapshot of the current GUI and accessibility preferences.
    pub(in crate::game) fn gui_options(&self) -> ClientGuiOptions {
        gui_options_snapshot(&self.settings)
    }
    /// A snapshot of the current chat preferences.
    pub(in crate::game) fn chat_options(&self) -> ClientChatOptions {
        let s = &self.settings;
        ClientChatOptions {
            visibility: s.chat_visibility,
            colors: s.chat_colors,
            links: s.chat_links,
            opacity: s.chat_opacity,
            scale: s.chat_scale,
            width: s.chat_width,
            height_focused: s.chat_height_focused,
            height_unfocused: s.chat_height_unfocused,
            line_spacing: s.chat_line_spacing,
            delay: s.chat_delay,
        }
    }
    /// A snapshot of the current audio preferences.
    pub(in crate::game) fn audio_options(&self) -> ClientAudioOptions {
        ClientAudioOptions {
            master_volume: self.settings.master_volume,
        }
    }
}

/// Convert live client settings into the API-owned GUI snapshot.
fn gui_options_snapshot(s: &horizon_client::settings::Settings) -> ClientGuiOptions {
    ClientGuiOptions {
        gui_scale: s.gui_scale,
        attack_indicator: s.attack_indicator,
        main_hand_left: s.main_hand_left,
        reduced_debug_info: s.reduced_debug_info,
        advanced_item_tooltips: s.file.bool("advancedItemTooltips"),
        high_contrast: s.high_contrast,
        show_subtitles: s.show_subtitles,
        notification_display_time: s.notification_display_time,
        text_background_opacity: s.text_background_opacity,
        menu_blur: s.menu_blur,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_snapshot_tracks_advanced_tooltip_toggle() {
        let mut settings = horizon_client::settings::Settings::default();
        for enabled in [false, true, false] {
            settings.file.set(
                "advancedItemTooltips",
                horizon_client::options_file::V::Bool(enabled),
            );
            assert_eq!(
                gui_options_snapshot(&settings).advanced_item_tooltips,
                enabled
            );
        }
    }
}
