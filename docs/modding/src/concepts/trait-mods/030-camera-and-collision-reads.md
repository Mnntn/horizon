<!-- chapter: Camera and collision reads -->
<!-- separator: 0 -->

## Camera and collision reads

`ClientHost::is_paused()` reads the applied client pause state, not whether a pause
screen is open. A multiplayer game continues running while its pause screen is open.
The pause-change pre hook sees the old state; the post hook sees the new state.

`ClientHost::playing()` is true while the player is in the world with nothing over it: no screen, title, sign
editor or book, so keys and zoom apply. `ClientHost::mapping(name)` reads a vanilla key mapping (`key.forward`,
`key.left`, `key.back`, `key.right`, `key.jump`, `key.sneak`, `key.sprint`, `key.attack`, `key.use`) as a
`KeyMapping`: whether it is held (`KeyMapping.isDown`) and the `label` of the key it is bound to, as Controls
shows it. It is `None` for any other name. Both read without `&mut`, so the HUD and screen hooks can ask them.

`ClientHost::clipboard_text()` reads the system clipboard (`KeyboardHandler.getClipboard`, empty text when it
holds none) and `ClientHost::set_clipboard_text(text)` writes it (`setClipboard`; empty text copies nothing). Both
work only while the game handles a key, mouse or typed-text event, from the window or from the control socket
(`horizon-ctl key`, `mouse`, `type` and its clicks): a key, mouse or screen hook that fires inside one, such as
Ctrl+V into a mod's own field. At any other moment (a tick, a frame, a packet) the read is `None` and the write
is `false` with nothing copied, so no mod reaches the clipboard without the player doing something. Both are
`None` and `false` on a server, and take `&self`, so a screen hook can call them. Vanilla's `KeyboardHandler`
has no such rule: Horizon adds it.

The `camera()` helper returns the most recently extracted world frame's camera, or none before one exists. Its position, interpolated eye height, partial tick, angles, final FOV, near/far planes and dimension accompany column-major view and projection matrices. The view is camera-relative and includes bob and spin; projection uses reversed Z. Earlier callbacks in a frame can see the previous frame. `block_collision(pos)` returns block-local `[min_x, min_y, min_z, max_x, max_y, max_z]` boxes with an empty collision context; unloaded blocks and collision errors give no boxes.

`ClientHost::set_camera_type(type)` is a queued client-only effect and takes `&self`, so
shared callbacks such as `camera_pose` can request a perspective. `camera_type()` reports
the latest request immediately; several requests coalesce to the last one. Requests before
or during `camera_pose` apply before that frame's final camera, FOV and spectator post effects.
The pose hook runs only once: its explicit position or angle overrides survive a refreshed
vanilla camera. Requests from later render callbacks apply at the next frame's extraction.
Setting a type never emits `camera_type_cycle`; F5 alone cycles, and its final event choice
wins over requests made inside the cycle callback.

`ClientHost::crosshair_target()` returns the ordinary interaction-range target as
`CrosshairTarget::Block { pos, state, hit }` or
`CrosshairTarget::Entity { entity, point }`. The block hit contains its world-space
location, face and inside flag; the entity point is also in world coordinates.
It uses the game's current targeting ray and block/entity reach, respects blocks
occluding entities, and returns `None` on a miss or without a client world.
It can be read with a screen open. `entity` is an `EntityRef` carrying the current
client level's epoch and network entity ID, including dragon parts. It becomes stale
when the client level is replaced or the client disconnects; a same-level respawn
keeps its epoch. This supplies identity, not client entity reads: the server-only
entity data methods still reject client handles. Unlike `view_hit` and `raycast`, this does not find blocks
in distant terrain or return a block behind a targeted entity.
