<!-- chapter: The mod's crate -->
<!-- separator: 1 -->

### The mod's crate

The root `Cargo.toml` and `src/` form the mod's one crate. It depends on `horizon-mod-api` (lib
`mod_api`) and never on the game, so the same crate builds native or to `mod.wasm`; the launcher
links it into the generated game executable, rather than making the game library depend on the
mod. The launcher stages the crate as `native-api/<mod>/`, rewrites its mod API dependency to the
staged one and resolves [mod dependencies](../../concepts/trait-mods/001-dependencies-between-trait-mods.md#dependencies-between-trait-mods).
A project has no separate workspace or lock file. Arbitrary path and git dependencies are refused;
the launcher supplies the checked paths needed for the game and declared mod dependencies.

**Dependencies come from Horizon's `Cargo.lock`.** In `Cargo.toml`, a mod names registry crates by
version requirement (`crc32fast = "1"`, `{ version, features, default-features, optional }`); each
must already be in Horizon's lock file in a matching version. The launcher checks every downloaded
package in the staged lock against Horizon's: name, version, source and checksum. Only the staged
workspace packages are added. A crate the lock lacks, or has only in a version your requirement
doesn't accept (`zip = "2"` against a locked `zip` 9.1.3), stops the build before Cargo resolves anything,
with a message naming the crate and requirement and the versions Horizon locks; nothing is downloaded
for it, because a mod can't bring dependencies of its own. Change the requirement to a locked version. A
feature that introduces an unlocked dependency stops the build too; the game still builds with
`--locked`.

Crates included only for mods are listed as optional dependencies in
`crates/horizon-mod-deps/Cargo.toml`: `mysql` 27 (blocking MariaDB/MySQL, with
`minimal-rust` and `rustls-tls-ring`, no default features), `redis` 1 (blocking,
no default features), `tungstenite` 0.30 (websockets over a stream the mod opens,
no default features), and `rusty-opus` 1 (Opus encoder and decoder). These entries
add their dependencies to the lock without building them into the game. A feature
that introduces a dependency not listed there is refused.
