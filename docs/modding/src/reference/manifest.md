# Mod files

A `.hmod` is one mod: content, native code, or both. `horizon_mod_package::Package` reads the shared manifest and
validates the package; `horizon_mod_package::Content` reads its resources, and `horizon_mod_package::index::native::Bundle`
reads its code. Content is preview, native code experimental ([data-formats.md](../../../api/data-formats.md)).

## The zip

- A zip archive. Entries are stored or deflate-compressed, the only methods the readers support.
- A path is relative and `/`-separated: no `\`, no `:`, no empty, `.` or `..` part, and it doesn't start with `/`.
  Files must be regular files, not symlinks, devices or executables. Executable payloads are refused even under
  resource paths.
- Each path appears once. Directory entries carry no payload.

## Project and archive layout

```text
my_mod.hmod
├── horizon-mod.toml
├── icon.png                # optional
├── assets/my_mod/
├── data/my_mod/
├── Cargo.toml              # code mods only
├── src/lib.rs              # code mods only
└── mod.wasm                # wasm mods only
```

- **Content only:** no `Cargo.toml` or `[native]`. Nothing compiles.
- **Native code:** root `Cargo.toml` and `src/`, with `[native]` in the same manifest. The launcher builds it
  into the game. Its resources load through the same content reader as a content mod's.
- **Wasm code:** `mod.wasm` at the top, declared by `[wasm]`, with the `Cargo.toml` and `src/` it was built from
  (a converted mod records `built_with` instead and carries no source). The game runs it with no rebuild.
- Inconsistent code declarations and files refuse the package. `mod.wasm` anywhere else, undeclared, or
  declared but missing, refuses it; so does any other executable file.

There is no mod-id directory level inside the archive. `horizon-mod.toml` is the only manifest and the only
source of the mod's id and version; Cargo's package name and version are not mod identity. IDs are plain
`[a-z0-9_]{1,64}` names, also used as resource namespaces. The launcher writes `horizon-mixin.toml` in its
staging area when needed. Hand-written tool manifests, `horizon-native.json`, and input `native/` or `rust/`
folders are unsupported.

## `horizon-mod.toml`

The shared [manifest fields](../how-to/content-mods.md#the-manifest) include `id`, `name`, `version`, `api = "0.2"`
and `capabilities = ["registries"]`. The content API is checked for exact equality. Unknown fields
refuse the manifest. Native mods may declare top-level `internals`, an array of tables with
`path`, `hash`, and optional `kind` when a path names several member kinds. Native code adds `[native]`:

| Field | Type | Rule |
| --- | --- | --- |
| `game` | string | not empty, the Minecraft version (`"26.3"`) |
| `horizon` | string | the exact Horizon commit, 40 lowercase hex characters |
| `mixins` | string | the exact horizon-mixins commit, 40 lowercase hex characters |
| `module` | array of tables | optional support modules, written as `[[native.module]]` |

Both access modes use the same top-level declarations.
API function listeners use top-level `[[inject]]`: `id`, `at`, lowercase `kind`, `with`, and optional `hash`,
with optional `access` (default `api`) and `priority` (default 0). `head`, `return` and `replace` are supported
for API slots; internals slots also accept the listed call kinds with `call`, `nth` and `ctx`.
Unknown fields are refused by the schema. API function declarations use only `[[inject]]`.
See [Injections](../injections.md) for the fields and point injections, and
[Native mods](../how-to/native-mods.md#named-hook-points) for the named hook contracts.

A module has a unique, nonempty `id` (one id space with the injections), a `file` naming the game source
file that receives it, a `name` that is a Rust module identifier, and a `source` payload file under
`src/`, holding the module's items without an outer wrapper. An optional `visibility` is exactly
`"pub"` or `"pub(crate)"`, the default. It is forwarded to horizon-mixins as `[[module]]`; see
[Mixins](../mixins.md).

`[[inject]]` only fills compiled slots and never compiles game code. Native source edits
use `[[native.mixin]]` with `id`, `target`, uppercase `kind`, `helper` and upstream
options. `[[native.bridge]]` and `[[native.state]]` forward upstream bridge and state
tables unchanged. See [Mixins](../mixins.md) for the vocabulary and trust boundary.

A wasm mod adds `[wasm]` instead, written by `cargo horizon-mod build --wasm`
([Building to `mod.wasm`](../how-to/wasm-mods.md)):

| Field | Type | Rule |
| --- | --- | --- |
| `sha512` | string | `mod.wasm`'s SHA-512, 128 lowercase hex characters |
| `toolchain` | string | not empty, the Rust toolchain it was built with |
| `linked` | table | optional, dependency id → the SHA-512 of the `mod.wasm` it was linked against |
| `built_with` | string | optional, not empty: the converter that made it, for a package with no source |

At load `mod.wasm` must start with `\0asm`, be at most 64 MiB and match `sha512`; each failure refuses the
package with its reason.

Top-level `[dependencies]` declares other mods by id: a semver requirement string, or
`{ version = "^1", optional = true }`. These versions refer to `horizon-mod.toml`, not Cargo metadata.
Content layers load in dependency order. A missing required dependency, incompatible installed version or dependency cycle refuses startup; an absent optional dependency is ignored. Native code dependencies additionally require API packages
([Dependencies between trait mods](../concepts/trait-mods.md#dependencies-between-trait-mods)).

## Original Java jar layers

Content and code mods can list ordered `[[jar]]` references. Use
[`cargo horizon-mod use-jar`](../how-to/content-mods.md#java-jars-as-external-assets) to record the
original `id`, `version` and exact file's `sha512`, without copying it into the mod.

**Pack order.** From bottom to top, assets use vanilla, then each mod's jars in
`[[jar]]` order, then that mod's own files, with later mods above earlier ones and
player resource packs above all mods. The mod's own files win over its jars for
the same path (the log lists each duplicate asset path once at load). Files every
resource pack stacks (atlases, lang, sounds, blockstates) merge in that same order
through the normal pack merger.

Data uses vanilla, then each mod's jars in `[[jar]]` order and that mod's own
`data/`, then the world's enabled datapacks. Later mods override earlier ones,
tags merge with their normal replacement rules, and a later recipe replaces an
earlier recipe at the same id. A jar may use any namespace. Only its passive
`assets/` and `data/` participate: Java classes and the jar's own pack metadata are
not loaded.

Original jars stay in `<game home>/jars/<sha512>.jar`; no bytes are copied into a
mod. The launcher checks and supplies exact files, and the existing ZIP readers
write only the decoded resource cache. The complete ordered reference list,
including version, contributes to the composed root's stamp, so changing it
rebuilds on the next resource load. Server `/reload` uses the accepted inputs of
that composed root, not another package scan.

**Unavailable originals.** Launcher preparation refuses before a native build or
game launch and names all requiring mods, the original jar and its expected
cache path. Direct game resource or world loading with a missing native
requirement fails as a whole; it never publishes code without the matching data.
A missing input during `/reload` reports the typed failure to the command source
and keeps the previous data and composed root. Content-only mods have no code to
strand: each whole mod can instead be refused, with the reason in its Mods screen panel, including
every owner of a shared missing file. The game never downloads jars.

## Source paths

A code package contains root `Cargo.toml`, optional `build.rs`, `.rs` and `.toml` files under `src/`,
and root text files named `README*`, `LICENSE*`, `LICENCE*`, `COPYING*` or `NOTICE*`.
The marked root `impl Hooks` uses `#[mod_api::horizon_mod]`; the attribute generates the factory
([Trait mods](../concepts/trait-mods.md#trait-mods-draft)).

Folder packaging selects source and resource files, excluding `.git/` and `target/`. It validates selected
files before copying them. Whole archives are checked, not treated as source folders with ignored extra entries.

## Content paths and icon

`assets/` or `data/`, then at least two parts of `[a-z0-9_.-]` (vanilla's resource-location characters).
Code and content use the same top-level resource trees; there is no separate native data folder.

`icon.png` is optional for both kinds. It has no manifest key. The package reader fully validates it once:
a PNG, nonzero square dimensions, at most 512 × 512 pixels and 256 KiB encoded. The launcher shows it in
the mod list. The scaffold does not create a default icon.

## Limits

| What | Limit |
| --- | --- |
| A registry file, as downloaded | 64 MiB |
| Selected native source files, excluding the manifest, icon and resources | 511 files, 64 MiB unpacked in total |
| Content files | 65,536 files, 16 MiB each, 512 MiB in total |
| `horizon-mod.toml` | 32 MiB |
| `icon.png` | 256 KiB encoded, 512 × 512 pixels |
