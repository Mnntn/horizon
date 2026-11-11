<!-- chapter: Cache and disk -->
<!-- separator: 1 -->

## Cache and disk

Game source edits below come from mixins and their support tables, not injects.
An `[[inject]]` fills a compiled slot or is refused; it never compiles game code.

The key hashes the commit, the horizon-mixins commit, Rust's version, the profile and features, and every file of
every bundle. The same set on the same commit is never built twice. A change gives a new build, and
`native/target/` stays, so a rebuild compiles only `horizon-client` and what the mods touched.

**Incremental rebuilds.** With Balanced and Developer, Cargo recompiles only the changed parts of `horizon-client`, not the whole
crate with LTO. Patched source is cached separately by its mixin inputs and pins (the three most
recent snapshots are kept), so editing only a trait crate skips the mixins pass. This holds because
the stage (`native/stage/`) is one folder for every build and the launcher writes
only the files whose content changed (Cargo goes by file times), switching mods on the same commit reuses the same
`native/target/`, and the launcher removes `CARGO_INCREMENTAL` and `CARGO_BUILD_INCREMENTAL` from the build's
environment. Optimized has no incremental compilation (the release profile's own).

**Build status.** Before it starts, the build identifies its plan and profile: up to date, incremental rebuild,
first build, or prebuilt game. While compiling it shows the crate and crates done of the total, from Cargo's own
progress. Afterwards it identifies the completed profile and plan. Neither the window nor `native-build` promises
or displays a build duration. `native/build.log` retains Cargo's diagnostic output, without the progress bar.

Native client builds share the prepared resources of their base version instead of copying them into every build. The launcher passes that base folder to the game as `--resources` and retains the base while an installed native build references it. Remove those builds before removing their base. Dedicated server exports remain standalone and include their own resources.

Disk use varies with the platform, profile and set of mods. Run `horizon-launcher storage` to inspect the actual
cache rather than estimating its size from another computer's build.

Instance references and newest-build retention select which builds could be disposable; process scans never
prove that a game cannot start using them. Current game builds take their own kernel-held runtime leases,
including direct starts and Windows launches. Unix launcher starts also run through a holder. These claims
survive the launcher exiting or being killed and remain held until the game process exits.

Cleanup classifies each unit separately. An installed version requires its own executable's current lease
capability. Shared caches conservatively require that every potentially referencing local version and cached
native build has that capability, including incomplete builds that an older executable could still start.
A pre-holder build therefore keeps shared caches protected without preventing removal of an unrelated capable
version. Kept space appears in Storage with its reason instead of being counted as freed.

Deletion reacquires the unit exclusively and rechecks the capability decision. Executable publication
(installation, import, prebuilt extraction and compilation) shares a registry publication lock; cleanup holds it
exclusively from its capability recheck through detachment, so an older build cannot land between the scan and
the rename. The central storage LRU replaces separate native spare-build quotas. Installation completion markers
are removed first. Markerless cache and resource units are renamed away on the same volume before their detached
trees are deleted, so a late start sees either the complete unit or no unit, never a partially cleared directory.
A late game that finds cleanup's exclusive lease, missing registration, missing resources or an incomplete install
refuses startup with a clear message rather than opening half-deleted files. Start the launcher once to register
an older installation before starting its executable directly.

Rust/Cargo/temp build-tool caches retain their Cargo-lock cleanup rules. `native/target/` and its profiles remain
residual even for current builds: their in-place Cargo cleanup preserves lock inode ancestry, but they still need
a completion marker or a retire-executables-first protocol before automatic or manual eviction can be safe.
Units containing operational Cargo lock files are likewise retained.

Private launch records and raw stdout/stderr capture files are collected as one group only after every protocol
owner exits, under an exclusive namespace gate. Malformed records and unknown files remain untouched. Record
collection retains its 4,096-entry trigger. The permanent gate and published live-unit keys are not collected;
deleting a live key would break direct startup's noncreating contract. Cleanup retires a deleted unit's own key
under its exclusive lease. Units deleted outside the launcher can leave empty registry files behind.
