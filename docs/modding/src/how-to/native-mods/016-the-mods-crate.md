<!-- chapter: The mod's crate -->
<!-- separator: 1 -->

### The mod's crate

The root `Cargo.toml` and `src/` form the mod's crate. It depends on `horizon-mod-api` (lib
`mod_api`), never directly on the game. The launcher supplies the pinned SDK and declared
[mod dependencies](../../concepts/trait-mods/001-dependencies-between-trait-mods.md#dependencies-between-trait-mods).

**Registry dependencies belong to the mod.** A mod may use crates.io packages that the game does
not use, or different versions of packages the game uses. Each package must have an exact version
and SHA-256 checksum in the mod's registry dependency lock, stored as `Cargo.lock` beside its
`Cargo.toml`. Keep this file in version control and include it in the mod package. Changing it
changes the native build's cache identity.

Generate it explicitly after choosing your registry dependencies:

```sh
horizon-launcher native-lock path/to/mod
# To deliberately resolve fresh pins and replace an existing lock:
horizon-launcher native-lock path/to/mod --update-lock
```

This is the **registry dependency lock consumed by launcher staging**, not a promise that ordinary
Cargo can resolve the original pinned SDK declaration offline. The command creates an inert copy
of the manifests, removes only SDK dependencies naming the exact pinned Horizon repository and
revision, and resolves registry metadata. It does not copy or execute mod sources, build scripts
or proc macros. It does not modify the original manifests or Cargo configuration. Existing pins
are not replaced without `--update-lock`; a refused input leaves the existing lock untouched.
A starter that uses only the launcher-owned SDK does not need registry pins until it adds a
registry dependency.

Builds consume an existing lock; they never silently generate it or repin a missing dependency.
The launcher may resolve registry **index metadata** online using an inert manifest whose
requirements are exact pins. Before fetching crate archives, every resulting identity and
checksum must match a supplied lock. Cargo checks downloaded archives against those checksums.
Only after this fetch does the launcher resolve the actual staged workspace **offline**.

The game's locked packages, checksums and dependency graph stay immutable. A different mod-owned
version may coexist where Cargo permits it. If a requested version would replace a game version,
the build refuses it rather than updating the game. A native library with conflicting providers
cannot be silently substituted. Feature selections must not introduce unpinned packages or
change the game's graph.

Git dependencies other than the proven launcher-owned SDK declaration, alternate registries,
paths escaping the mod, linked dependency paths, `[patch]`, `[replace]`, and mod-owned Cargo source
configuration are refused. Local crates inside the mod are traversed without copying their code
into the registry resolver. Registry dependencies remain native code: their build scripts and
proc macros execute during compilation, and the resulting mod runs with your rights. Trust their
authors as well as the mod's author.
