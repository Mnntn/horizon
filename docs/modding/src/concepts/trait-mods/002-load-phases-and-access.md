<!-- chapter: Load phases and access -->
<!-- separator: 1 -->

### Load phases and access

Native and wasm hooks share the installed dependency-ordered mod slice, including registration,
config loading and reload-listener registration. The shared resolver keeps bundle-order ties,
checks present optional dependencies too, and refuses cycles for both build targets. Content-only
packages stay after code in the loaded-mod inventory; listener-specific edges still use `ReloadOrder`.

Each owning thread progresses through `Packages → Registration → Setup → Config → World`.

| Phase | Available access |
| --- | --- |
| Packages | Package discovery; no game Host operations. |
| Registration | Declarations through `RegistrationContext`, with its mod-list, registry and settings reads; no Host. |
| Setup | Client setup reads, published asset/data queries and registry/settings/mod-list reads; no world handles or transactions. |
| Config | Settings reads and the existing mod-scoped `ConfigFiles` service; no world handles, transactions or other game operations. |
| Reloading | Published asset/data queries, registry facts, settings and mod-list reads, listener registration, and existing `DataLoadContext` callbacks; no world handles or transactions. |
| World | The owning game's side-typed `ServerHost` or `ClientHost`. |

`RegistrationContext::instance_dir()` reads the instance directory captured before
registration begins, as an `InstanceDirectory`: `Unix(Vec<u8>)` preserves raw Unix
path bytes, and `Windows(Vec<u16>)` preserves Windows UTF-16 code units, including
unpaired surrogates. Never convert Unix bytes with a lossy UTF-8 conversion when
constructing a native filesystem path. Wasm receives the identical host-encoded
value through Wire as data only; this does not grant filesystem access. A context
with no engine snapshot returns `RegistrationReadError::Unavailable`.

Registration runs once: `register`, `component_types`, `blocks`, `items` and `standalone_models`
receive only `RegistrationContext`. Its `mods()`, `registry(name)` and `setting(mod_id, key)` read
one view captured before the first of them runs, so every registration step sees the same values
whatever the dispatch order: the loaded packages, the built-in registries as `registries.json`
lists them (not what mods declare during registration) and the settings on disk at that moment. A
mod validates the keys it declares against them from `Hooks::register`; a name that is not a
namespaced identifier, an unknown registry, an empty or path-like setting key, a package that is
not loaded, or a context the engine gave no data returns a `RegistrationReadError`. Client setup
runs once after the initial asset apply. The window's Client/Common configs load once immediately after client setup and before title,
connection, input, render or ordinary tick callbacks. This includes pack-error recovery: the failed
selection is cleared and saved during initialization, but its options screen opens only after Config
completes. A title-screen callback is already in World: world-handle constructors can return `None`,
rather than being forbidden in Setup. A dedicated
server loads Common once before its first world; Server configs load and unload per world.
A second world does not repeat registration or client setup. Config loading can read settings and
use the existing scoped `ConfigFiles` service to read or save its own config files. World handles,
transactions and other game operations panic with `Config` in the diagnostic.
Config file polling is not a phase transition, and Server config unloading leaves the World phase.

Every data or asset reload enters `Reloading`, including listener registration, prepare, apply and
the reload completion notice. Initial loads can nest in Setup; runtime reloads nest in World.
Returning, including after a failed prepare, restores the entering phase. Published asset/data
queries are classified in the catalog (`resource_read`), not granted by helper-name exceptions.
They read the currently published generation, not pending prepare output: completion callbacks can
refresh sprite and recipe caches after apply, including on first world open, without gaining world
handles or transactions. Their rustdoc names their load-phase availability. Server listener
registration retains `&dyn Host`, restricted to these published facts, registry facts, settings and
mod-list queries; callbacks already taking `DataLoadContext` keep that context. Neither permits
opening a world handle or transaction. The first client registration and prepare run on the loader
thread; later registration runs on the render thread, prepare on its worker, and apply on the owning
game thread. Startup's empty client Host is not an extra permission.

`DataLoadOrigin` and `LoopSide` are API-owned enums, not engine exports. Load and loop
callbacks read frozen registry and settings metadata through `DataLoadContext` and `LoopContext`.
The engine supplies borrowed `LoadReader` and `LoopReader` implementations; the API's owning
views contain no engine types and do not copy or rescan the resource snapshot. Mod listeners
keep using the context traits, with the same origin, side and registry ordering on both targets.
The reader adapters themselves are native host implementation details, not wasm resources.

The game alone advances phases. An illegal or wrong-thread advance is refused before dispatch;
a forbidden mod call panics naming the phase, rather than returning a Result a listener could ignore.
There is no separate common-setup, load-complete or intermod event: registration and the first server
reload cover common setup, client setup is `client_setup`, dedicated setup is `server_about_to_start`,
and dependencies express intermod ordering. Load complete is the edge after initial Config and before
`server_about_to_start`, not a hook.

Settings snapshots accept only regular UTF-8 files, without following file or package-directory symlinks (including internal links). Invalid UTF-8, special files and refused entries are absent, not errors. Each file is limited to 64 KiB before trimming, each mod to 1,024 captured keys, and the whole snapshot to 16 MiB before trimming. Over-limit keys are omitted in sorted package-id and key-name order. Enumeration is limited to 4,096 names per package; packages exceeding that budget are omitted entirely rather than retaining an unstable enumeration prefix. One warning per refusal class is emitted per snapshot. These generous limits cover the small scalar setting values used by the settings API; they do not apply to mod data files.
