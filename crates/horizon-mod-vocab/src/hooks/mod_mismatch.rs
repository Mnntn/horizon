//! The `mod_mismatch` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A saved mod list was read while opening a world. Fires even when the list
    /// is empty or unchanged, never when absent. Both logical sides may open worlds;
    /// singleplayer opens once on its integrated server. No cancellation or GUI warning.
    /// Versions use the loader's comparison; minecraft is excluded. Newly added mods
    /// are not differences. Push a mod id converted with `.into()` into `resolved`.
    /// Dispatch assigns the first resolving listener and preserves earlier markers.
    /// The Host boundary is before world-state restoration, but after preparation:
    /// raw-read-before-upgrade timing requires a pre-world Host which does not exist.
    #[notes = r##"## Mod mismatch

A world records the code mods it was saved with at the root of `level.dat`, beside `Data`, under
`fml.LoadingModList`, the list NeoForge keeps: one `{ModId, ModVersion}`
compound per loaded mod, in load order, rewritten on every save of the world.
The version is the mod's `horizon-mod.toml` `version`, or `""` when the game
doesn't know it. A world without the list (vanilla, or saved before Horizon
wrote one) is no mismatch: it fires nothing and gets the list on its next save.

`Hooks::mod_mismatch` runs whenever a saved list exists, even if empty or unchanged.
It runs on either logical side, once in singleplayer on the integrated server, before
world-state restoration. Raw-read-before-upgrade timing is not reachable without a
pre-world Host; preparation has already upgraded the world at this boundary.
Versions compare using the loader's semantic version parser (omitted minor and patch
are zero); empty or invalid versions compare by exact text. `minecraft` is excluded.
Push `"your_mod".to_owned().into()` into `event.resolved` to mark a migration.
Dispatch records the first resolving mod in each marker's `resolver` and preserves
prior resolutions against deletion or replacement. Entries are logged sorted by id:
resolved at debug (self or other resolver), unresolved at warn. Loading never blocks.
"##]
    #[hook(
        side = "both",
        contract = "collect",
        out = "resolved",
        collect_validator = "crate::game::mismatch_resolution::attribute",
        neoforge = "ModMismatchEvent"
    )]
    fn mod_mismatch();
}
