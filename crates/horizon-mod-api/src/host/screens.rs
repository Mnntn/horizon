//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `screens` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ScreensClientHost {
    /// Side: client.
    /// The block of the mod screen that is open (`native_screen`), as the
    /// server named it when the block opened its menu type; None when no mod
    /// screen is open. 401.
    #[access(ClientSetup)]
    fn screen_pos(&self) -> Option<[i32; 3]>;
    /// Side: client.
    /// What the block's mod wrote for the mod screen that is open
    /// (`native_menu_extra_data` on the server), empty when it wrote nothing;
    /// None when no mod screen is open. @API:menu_extra_data@.
    /// Not on wasm: it returns a borrowed view of the screen's bytes, which crosses only as a resource.
    #[access(ClientSetup)]
    fn screen_extra_data(&self) -> Option<&[u8]>;
    /// Side: client.
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `native_screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    #[access(ClientSetup)]
    fn screen_exclusion(&self) -> Vec<[f32; 4]>;
    /// Side: client.
    /// The id of the item in the slot under the pointer on the open container screen
    /// (`AbstractContainerScreen.getHoveredSlot().getItem()`), None over no slot or an empty one.
    #[access(ClientSetup)]
    fn screen_hovered_item(&self) -> Option<ItemKey>;
    /// Side: client.
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    #[access(ClientSetup)]
    fn apply_screen_blur(&self) -> bool;
    /// Side: client.
    /// The user's configured menu background blur radius (`menuBackgroundBlurriness` in options.txt,
    /// usually 0.0 to 10.0; default 5.0). 0.0 means blur is turned off. API @API:screen_transparent_background@.
    #[access(ClientSetup)]
    fn menu_blur_radius(&self) -> f32;
    /// Side: client.
    /// Opens a screen of mod `mod_id`'s own over the world: the screen hooks
    /// fire over it as over any other, so the mod draws it in
    /// `native_screen_render_post` and takes its clicks, scroll, keys and
    /// typing there; the mouse is freed, and Escape closes it. Every mod's own
    /// screen has the `ScreenView.id` `mod`, so a mod's screen hooks
    /// check `native_mod_screen()` is its own before drawing. It doesn't pause
    /// a single-player game (as the inventory doesn't). True when it is open:
    /// false while another screen is (or another mod's), over the title
    /// screen, a book, a sign editor, a dialog or a prompt, while the world
    /// loads, and anywhere but the window's own client (the server thread, a
    /// dedicated server). A gamepad can close it (B, Start) but not click on
    /// it: its clicks don't reach the mouse hooks.
    #[access(WorldOnly)]
    fn mod_screen_open(&mut self, mod_id: &str) -> bool;
    /// Side: client.
    /// Closes mod `mod_id`'s own screen, if it is open.
    #[access(WorldOnly)]
    fn mod_screen_close(&mut self, mod_id: &str);
    /// Side: client.
    /// The mod whose own screen is open, if one is (Escape, or another screen
    /// replacing it, closes it).
    #[access(ClientSetup)]
    fn mod_screen(&self) -> Option<&str>;
    /// Side: client.
    /// The open menu's title (the block's name, or the one it was given),
    /// as plain text.
    #[access(ClientSetup)]
    fn screen_title(&self) -> Option<String>;
    /// Side: client.
    /// The last window focus state reported by the window system.
    #[access(ClientSetup)]
    fn window_focused(&self) -> bool;
    /// Side: client.
    /// The same screen identifier provided to screen hooks.
    #[access(ClientSetup)]
    fn observed_screen_id(&self) -> Option<ScreenId>;
    /// Side: client.
    /// Whether a specific key is currently held down.
    /// `key` is the vanilla physical key name (`key.keyboard.left.shift`, for example).
    /// Unknown names are not down. Modifier combinations use `ClientHost::modifiers`.
    #[access(ClientSetup)]
    fn is_key_down(&self, key: &str) -> bool;
}
