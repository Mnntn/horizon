# Saved-world mod mismatches

`Hooks::mod_mismatch` reports changed and removed saved mods whenever the world's
saved list exists, including empty and unchanged lists; newly added mods and
`minecraft` are excluded. Both logical sides can open worlds, and singleplayer
opens once on the integrated server.

Unlike NeoForge's raw-read timing, this runs at the earliest available Host before
world-state restoration, after preparation, because it only reports and logs and
its resolution output does not depend on the earlier moment.

Versions use the loader's semantic version comparison, padding omitted minor and
patch components with zero; invalid or empty versions compare as exact text.
Mark a migration with `event.resolved.push("my_mod".to_owned().into())`.
The first listener to mark an id owns its `resolver` attribution; later listeners
cannot erase or replace that marker. Markers for other ids are allowed.
Differences are logged sorted by mod id, resolved at debug with the resolver and
self/other distinction, unresolved at warn. No dialog or cancellation is added.
The current mod list is written on the next save, not when this event fires.
