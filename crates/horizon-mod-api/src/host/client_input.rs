//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_input` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait ClientInputClientHost {
    /// Side: client.
    /// Right-clicks with these items (at most 64 ids) are mod `mod_id`'s; a later call replaces the list.
    #[access(WorldOnly)]
    fn claim_items(&mut self, mod_id: &str, ids: &[String]);
    /// Side: client.
    /// Mod `mod_id`'s events since its last poll (at most 256 wait).
    #[access(WorldOnly)]
    fn poll(&mut self, mod_id: &str) -> Vec<ModEvent>;
    /// Side: client.
    /// Where the mouse is, in GUI pixels.
    #[access(ClientSetup)]
    fn cursor(&self) -> [f32; 2];
    /// Side: client.
    /// The saved key and modifiers for an owner-bound registration, or its declared default.
    #[access(ClientSetup)]
    fn mod_key(&self, mod_id: &str, name: &str) -> Option<(String, ClientModifiers)>;
    /// Side: client.
    /// Playing, with nothing over the world: keys and zoom apply.
    #[access(ClientSetup)]
    fn playing(&self) -> bool;
    /// Side: client.
    /// A vanilla mapping (`key.forward`, `key.jump`, `key.attack`…): whether
    /// it is held (`KeyMapping.isDown`), and its key's name as Controls shows
    /// it. `Ok(None)` for a mapping it doesn't answer for. Refuses an invalid
    /// label with [`TextFormError`], including components deeper than 512 nodes.
    #[access(ClientSetup)]
    fn mapping(&self, name: &str) -> Result<Option<KeyMapping>, TextFormError>;
    /// Side: client.
    /// Milliseconds on the game's own clock (`Util.getMillis`), counting from the game's start:
    /// what a text field's cursor blinks by and what tells a double click from two clicks.
    /// Only differences between two readings mean anything.
    #[access(ClientSetup)]
    fn millis(&self) -> u64;
    /// Side: client.
    /// The current window modifier keys in the same GLFW encoding as input hooks.
    /// Cleared when the window loses focus; available even outside a world.
    #[access(ClientSetup)]
    fn modifiers(&self) -> ClientModifiers;
    /// Side: client.
    /// The clipboard's text (`KeyboardHandler.getClipboard`), empty when it holds none,
    /// while the game handles a key, mouse or typed-text event: a mod's key or screen hook
    /// pasting into its own field. None at any other moment (a tick, a frame, a packet),
    /// so no mod reads the clipboard without the player doing something, and None on a
    /// server.
    #[access(ClientSetup)]
    fn clipboard_text(&self) -> Option<String>;
    /// Side: client.
    /// Puts `text` on the clipboard (`KeyboardHandler.setClipboard`) on the same terms as
    /// `native_clipboard_text` reads it. False, with nothing copied, at any other moment or
    /// on a server; empty text copies nothing, as vanilla's.
    #[access(ClientSetup)]
    fn set_clipboard_text(&self, text: &str) -> bool;
}
