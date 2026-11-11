# Building a mod to `mod.wasm`

## Boundary benchmark

From the root of the worktree containing the fixture and runtime to measure, build the tool, fixture,
and shared std, then run the release benchmark. This requires `rust-src` and the
`wasm32-unknown-unknown` target for the pinned wasm toolchain:

```sh
cargo build -p cargo-horizon-mod
(cd crates/horizon-mod-wasm/tests/fixture && "$OLDPWD/target/debug/cargo-horizon-mod" horizon-mod build --wasm --catalog points.json && "$OLDPWD/target/debug/cargo-horizon-mod" horizon-mod wasm-std "$OLDPWD/target/horizon-wasm-std.wasm" --catalog points.json)
HORIZON_WASM_STD="$PWD/target/horizon-wasm-std.wasm" HORIZON_WASM_FIXTURE="$PWD/crates/horizon-mod-wasm/tests/fixture/target/horizon-mod/wasm_fixture.hmod" cargo test --release -p horizon-mod-wasm --features listener-fixture --test bench empty_subscribed_listener -- --ignored --exact --nocapture
```

Building the fixture rewrites `crates/horizon-mod-wasm/tests/fixture/horizon-mod.toml` with the
generated wasm hash and toolchain identity. Inspect and discard those generated manifest changes
after measurement rather than committing toolchain-specific hashes; preserve any intended edits.
Keep the machine, build profile and conditions consistent when comparing revisions.

The empty-listener benchmark dispatches the same subscribed `server_stopped` listener natively and
in wasm. It reports 100 samples of 100,000 dispatches after 20 warm-up samples, with loading,
initialization, subscription installation and the in-memory host outside timing. It is not a
server teardown or world tick. The existing `block_entity_ticks` case measures 1,000 in-memory
block entity ticks per sample, with and without two host reads per tick; replace
`empty_subscribed_listener` with `block_entity_ticks` in the command to run it. The `wasm-mods`
workflow runs that case only on manual dispatch.

For an outer wasm call, an immediately available group lock does not start the busy timer. On
contention, non-worker calls wait up to five seconds from the first failed lock attempt; they
return busy immediately if the group is lent to another thread. Worker calls continue waiting
instead of using that timeout or lending refusal. A synchronous same-thread reentrant call uses
the lent context and its enclosing deadline; frame, resource and trap cleanup still apply.

A mod written against `mod_api` builds either way from the same source: native, compiled into
the game by the launcher ([Native mods](native-mods.md)), or to `mod.wasm`, which any game with
the same mod API runs with no rebuild. Only the build step differs: the hooks, Parts,
`[dependencies]` and the manifest mean the same for both.

```rust,ignore
{{#include ../../examples/wasm-hello/src/lib.rs:mod}}
```

## Owned values and `Wire`

Native and wasm use the same owned-value encoding. A mod chooses which of its own
values may cross that boundary by explicitly deriving `mod_api::Wire`:

```rust
#[derive(mod_api::Wire)]
struct CounterUpdate {
    counter: mod_api::Identifier,
    value: u32,
}
```

Records write fields in declaration order; enum tags are zero-based declaration
indices, not Rust discriminants. Each field must have a Wire form. Borrowed hook
arguments are call-table views, not owned Wire values; a borrowed record cannot
use this derive. The derive is never added automatically to a mod's types.

For a value with checked construction, `#[wire(via = SomeOwnedType)]` uses
`From<&Self>` to encode and `TryFrom<SomeOwnedType>` to decode. A failed conversion
refuses the value rather than constructing unchecked state. This also lets a type
retain a deliberately chosen encoding instead of exposing its Rust fields.

The API's own plain values use this same derive, including particle options,
package descriptions, handles and reload file collections. Checked registry keys
and item stacks retain codecs that validate host registry slots, nonzero stack
counts and component patches. A declaration retaining a handwritten codec states
its particular validation, projection or nonstandard byte format; merely having a
handwritten codec is not an exception to the explicit Wire policy.

The loader compares composed encoding layouts, including nested fields and
recursive shapes. Equivalent standard representations share one encoding:
`Vec<T>` and `Box<[T]>`, for example, are both lists, while `Box<T>` is transparent.
Changing a field's encoded form changes its callers' layout too. Layouts are
computed once at load; they are not per-tick work.

## Build

```sh
cargo horizon-mod build --wasm
```

It builds the crate with the pinned Rust toolchain (installed through rustup the first time, with
`rust-src`), writes `mod.wasm` beside `horizon-mod.toml`, records it in a `[wasm]` section, and
packs `target/horizon-mod/<id>.hmod`. `[wasm]` is written by the tool, never by hand:

```toml
[wasm]
sha512 = "…"                      # mod.wasm's hash, checked when the game loads it
toolchain = "1.99.1 <commit>"     # every mod linked together needs the same one
[wasm.linked]                     # the exact builds of its wasm dependencies
my_library = "…"
```

A mod carries native code or wasm code, not both. A wasm mod has no `[native]` table.

A mod with `[[inject]]` listeners builds against the game's staged point catalog. Pass
`--catalog path/to/points.json`, or set `HORIZON_WASM_STD` to the toolchain's std member
with `points.json` beside it. Mods without listeners need no catalog. When packaging the
wasm toolchain, `cargo horizon-mod wasm-std <out.wasm> --catalog <staged-points.json>`
copies that catalog beside the std member.

The std member also owns the group's shared state: its cache budget and the one table of handlers (capability providers and parts) that the mods of a
group give the game, so a handler one mod of a group gave is usable by another mod of the same group. A
mod built for a toolchain whose std lacks that table is refused at load; rebuild the mod and the std
together.

The state is 32 words, zeroed, exported by the std member as `horizon_group_state`: the handler table, the cache budget, and
29 slots for eviction callbacks. A member's allocator is `mod_api`'s `#[global_allocator]` on wasm32. When an allocation
fails because the group's memory (256 MiB) is full, it calls every callback registered with
`mod_api::wire::guest::register_eviction(fn())` once and retries once; if that frees nothing the allocation fails as
usual (`try_reserve` returns an error, an infallible allocation ends the call with a trap). Register a callback to
give back what a cache can rebuild. It must free memory and must not allocate. Registering the same function twice is
harmless; `false` means all 29 slots are taken.

A wasm mod must not declare its own `#[global_allocator]`: `cargo horizon-mod build --wasm` fails with rustc's error
"the `#[global_allocator]` in this crate conflicts with global allocator in: mod_api". Allocate through the standard
collections and let the group's allocator do the rest.

## Dependencies

`[dependencies]` work as for native mods. List the library mod in `horizon-mod.toml` and its
crate in `Cargo.toml`. The tool stages the resolved mod packages into one Cargo workspace:
each library's single compiler unit produces both its rlib and its position-independent member,
and dependents use that exact rlib through Cargo's ordinary dependency edges. Cargo still owns
feature resolution, build scripts and non-mod dependencies. Registration generation is scoped to
each package, not to the mod from which the command was launched.

Ordinary Rust calls such as `my_library::function()`, statics, generics and owned Rust types need
no generated interface or export annotations. Members export their public definitions without
symbol interposition: duplicate definitions are refused, rather than overriding another mod.
Each member records its compiler, target, settings,
rlib digest and dependency artifact identities in `horizon.build` metadata covered by its module
hash. Mod crate disambiguators use the mod ID, version and resolved Cargo features rather than
the temporary workspace path. A shared crate disambiguator alone is not an ABI guarantee:
the loader checks the exact dependency build recorded in `[wasm.linked]`, including when the
library and its consumers were built separately. Changing a library's source, toolchain,
features or settings requires rebuilding its consumers.

The build packages every member of the resolved group into the invoking project's
`target/horizon-mod/` directory. A consumer build does not replace another project's previously
packaged `.hmod`. To include additional local roots in the same Cargo invocation, repeat
`--with path/to/another/mod`. The game links a mod with its
dependencies at load, one copy of each library shared by every mod that uses it, and refuses a
mod whose library is missing or another build than the one it was linked against, naming it on
the Mods page. An optional dependency is built in with the Cargo feature of its id, so its crate's
entry in `Cargo.toml` is `optional = true` under that id; an absent one is left out of the group at
load, and `if_installed!` keeps the mod from calling it.

## Injection points

API-typed `[[inject]]` listeners use the same generated `mod_points` records and constructors as
native listeners. When injections are declared, the wasm build requires the pinned game's staged
`points.json`, supplied via `--catalog` or beside the configured std member.
Listeners register in the host's ordinary point list, with the same priority, dependency order,
owner enable flag and stop behavior as native listeners. Mixins remain native-only.

A point's record is a copy of every parameter as an API value, `&mut` parameters copied back
after the listener, exactly as for a native listener; a point with a parameter that has no API type
yet refuses an `api` listener at registration, naming it. `access = "internals"` (raw records,
call kinds, support modules) is native-only: a `mod.wasm` declaring `[[internals]]` or an internals
injection is refused at load with "internals access is native-only".

## ABI reference

A dependency group consists of core wasm shared modules with `dylink.0`, not components.
The loader provides one linear memory, function table and standard-library member per group,
plus the `env` base globals and the `GOT.mem` and `GOT.func` relocations used by the members.
The loader supplies linker globals such as `GOT.mem.__heap_base` and `GOT.mem.__data_end`;
member constructors run after linking. Groups do not share memory with one another.

The build step moves a core start function to the explicit `__wasm_start` export. The loader
instantiates members, fills self-referencing GOT entries from their actual exports, then runs
`__wasm_start`, data relocations and constructors in that order. A raw core start is refused:
it would execute before its own addresses could be relocated. `horizon.private` metadata lists
compiler implementation exports that remain local to a member (for example an upstream inline
method emitted in several members). Their original symbols can resolve that member's own GOT,
but are not published as library definitions. This is symbol visibility metadata, not function
glue or another Rust interface.

Pointers and lengths below are wasm32 `u32` values. The game provides these imports in the
`horizon` module:

- `host(id, in_ptr, in_len, out_ptr, out_cap) -> u32` invokes a generated import ID. Arguments
  are Wire values in order. It writes the reply if it fits and returns its length; if the length
  exceeds `out_cap`, call `host_reply(out_ptr)` after allocating that many bytes to copy the
  retained reply. Host errors trap the call.
- `host_reply(out_ptr)` copies that retained reply.

Each guest exports:

- `horizon_layout() -> u64`: the generated layout hash. A mismatch is refused.
- `horizon_layout_ext() -> u64`, when `mod_points` is linked: the full point catalog's layout
  hash. The loader checks it before accepting any listener.
- `horizon_in(len) -> ptr`: the buffer into which the game writes the next call's input.
- `horizon_hook(id) -> u64`: runs the generated export ID after the input is written. The high
  32 bits are the reply pointer and the low 32 bits its length; `u32::MAX` as the length denotes
  an error, with a Wire string at the pointer. The reply remains valid until the next call at
  the same depth. A handler call starts its input with the handler's `u64` ID.

The reserved IDs are `INIT = 0xFFFF_FF00` (guest package-list initialization),
`RELEASE = 0xFFFF_FF01` (guest handler IDs to release), and `HOST_RELEASE = 0xFFFF_FF02`
(host object ID to release), and `HOST_LOG = 0xFFFF_FF03` (a guest panic string, with no reply).
Other IDs index the generated import or export table. A hook reply
contains its return value followed by writable arguments in field order.

Points use that same hook export, call arena and execution budget. Place IDs are
`0x8000_0000 + place`. The subscription query `0xBFFF_FFFF` returns `(place, injection_id)`
pairs. Point call input starts with the `u32` subscription index. A `HostSide` point then carries
a server/client boolean, followed by its listener record. Calls reply with unit and the record,
which the host decodes completely before writing any field back.

Wire values use little-endian numbers; lengths and collection counts are `u32`, strings are a
byte length followed by UTF-8 bytes, options have a byte tag (`0` or `1`), enums have a `u32`
case followed by its fields, and records contain their fields in declaration order. Readers reject
invalid tags, invalid UTF-8, trailing data, lengths that cannot fit in the remaining bytes, and
nesting beyond the shared Wire budget of 1040 protocol levels (which accommodates vanilla's
512 nested NBT lists or compounds). Interned static strings are limited to 256 bytes and 4096 distinct
strings per process. Execution, memory and table limits are enforced separately by the loader.

## In the game

Each dependency group uses shared memory and one copy of the standard library; its members are
core wasm shared modules, not components. A layout mismatch or an unresolved dependency is refused
and named on the Mods page.

The game limits execution time, memory and table usage. A trap or limit hit disables the affected
group. APIs marked `Not on wasm` in their Rust documentation cannot be used by wasm mods.

## Compiled module cache

The game caches compiled wasm modules in `<game_dir>/.horizon-cache/wasm/`, keyed by the
wasm bytes, Wasmtime compatibility and engine settings. Hits map the compiled file into memory;
files shipped beside a mod are never loaded as compiled code.

On Unix, a cache miss compiles in a short-lived child of the game executable, returning
compiler memory to the operating system when the child exits. The internal entry runs before
game initialization, with closed standard streams and an empty environment. It receives a
private input file and the expected cache key, refuses changed input, and writes through the
same atomic cache writer. The parent accepts only its expected entry through the verified
opened-file loader. Startup failure, a crash, a non-zero exit, an invalid entry or a timeout
falls back to in-process compilation with a log message. A timed-out child is killed and
reaped before fallback; its input and unfinished writer files are removed. This child path
is Unix-only; Windows continues to compile in process with persistent caching disabled.

On Unix, the game creates the cache folder with mode 0700 and entries with mode 0600. It checks
folder ownership and permissions, then checks each opened entry is a regular file owned by the
current user, not writable by group or others, and has only one hard link. Symlink entries are
refused. On Windows, persistent caching is disabled until owner, DACL and inheritance trust
can be verified for the cache folder and opened entries. Modules compile from wasm instead,
with one warning explaining the fallback. The player's account is the trust boundary.

A failed check logs the reason once per launch. Unavailable, corrupt or incompatible entries
fall back to compiling the wasm. Writes replace entries atomically without changing files that
running modules have mapped. Deleting the cache is always safe: the next launch rebuilds it.
