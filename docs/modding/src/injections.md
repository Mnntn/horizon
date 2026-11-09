# Injections

A function-slot injection is a `[[inject]]` table in `horizon-mod.toml`. It is an escape hatch for a moment
the mod API has no hook for, and **every injection is a request for a hook**: write it so it can become one
by renaming its target, then [ask for the hook](#ask-for-the-hook).

Choose `access = "api"` for API-owned records, or `access = "internals"` for a declared
internals facade. Both use the same `[[inject]]` fields. Native internals listeners use a
compiled slot when available; an unlisted function or supported call requires a source rebuild.
The launcher generates the private horizon-mixins input; mods name a Rust `with` function,
not a source payload file or expression selector.

An API point injection names a crate-root `with` function and listens at an admitted head or return
slot ([shape rules](#which-functions-are-points)). The prebuilt game fills those slots at load with the
mods' listeners, without changing game source.

There are two access tiers, and they never mix:

- **`access = "api"`: native and `mod.wasm` alike.** A point is a hook whose signature comes from the
  engine function. The listener gets the side's host and the point's record: a copy of **every**
  parameter as an API value (scalars, text, std containers, `mod_api` types, and the engine types
  with a `mod_api` mirror, such as `Text` and `BlockStateId`), plus `cancel` or `result`. A `&mut`
  parameter is copied in and its place gets the listener's value back; by-value and `&T` parameters
  run the function with the listener's values. Both targets get the same record and the same
  semantics. A point with a parameter that has no API type yet has no API record: an `api` listener
  there is refused at registration on both targets, naming the parameter and its type.
- **`access = "internals"`: native only.** The listener gets the real receiver and a `&mut` to every
  parameter in its engine type, and the call kinds and support modules below. It is the mixin tier;
  a `mod.wasm` declaring internals or an internals injection is refused at load:
  "internals access is native-only".

## Adding a pinned declaration

Run `cargo horizon-mod inject add <at> --kind <kind> --with <listener>` in a native mod's folder.
The tool appends `[[inject]]`, preserving comments, formatting and permissions. A symlinked
manifest is preserved; edits atomically replace its referent. Checked `[native].horizon` metadata supplies `hash`.
`--with` supplies both listener and unique inject id (default `on_inject`); implement it separately.
Access defaults to `api`; edit it to `internals` when needed. The tool prints each tier's availability
at the place, such as `api: refused, argument 2 `batch` (`&mut HudBatch`) has no API type yet` and
`internals: native only`. The launcher validates route support.
Call kinds copy canonical `call`, `nth` and `ctx` from the pinned `points.json`. Use
`--call <canonical callee> --nth N` (default 0, per callee). Omitting `--call` requires one callee
at that ordinal. Ambiguous identities and unlisted source calls are refused; no discovery runs.
Unknown places return `E_INJECT_MISSING` naming the target. Invalid kinds, callbacks,
duplicate ids, or call flags on function kinds leave the manifest untouched.

## Refreshing pinned declarations

`cargo horizon-mod inject update` checks every declaration against the current game's catalogs;
it does not change `[native].horizon` (use top-level `update` to change that pin).
A unique call context that moved reports `E_INJECT_MOVED` and updates only `nth`.
Missing targets, missing contexts and ambiguous contexts are never retargeted.

Signature changes report `E_INJECT_CHANGED` with old and new hashes and signatures,
including parameters, return type and flags. Older cached catalogs supply the old signature;
when unavailable, the report explicitly says so. No hash changes without explicit consent:
run `cargo horizon-mod inject update --accept <id>` to accept that declaration's new signature.
Repeat `--accept` for multiple ids. Acceptance cannot override missing or lost targets.
Any refused entry prevents all writes. Successful edits preserve unrelated fields and comments,
use atomic replacement, and an exact no-op leaves the file and its modification time untouched.

## Publishing slots to a game checkout

After adding native declarations, request their missing slots in a game source checkout:

```sh
cargo horizon-mod inject sync --repo /path/to/horizon
```

This writes `crates/horizon-mod-api-gen/slots/<mod_id>.txt` in that checkout, not in the mod.
It emits function paths, `at | call | nth | ctx` call rows, and `type <path>` extension rows.
Places already listed by another mod need no duplicate row. Existing rows in your own list
are retained; row-only files and tool-written files are regenerated in sorted order.
A standalone or trailing comment other than the tool's own header prevents rewriting:
the error names its file and line and asks you to move the comment out first.
Running the command again makes no change.

The shared slot parser validates every row before writing. Conflicting call contexts are
refused with both file locations, leaving your file untouched. Invalid mod ids are refused,
including Windows device names (`con`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`, etc.),
and `horizon.txt` is reserved for the engine seed. Commit the generated list with the game
change that supports it; this command does not compile slots or change the mod's build pin.

## Function-slot declarations

API listeners use `access = "api"` (the default) and the host/API record shown below. Native internals
listeners choose `access = "internals"` and the generated facade's receiver/raw record:

```toml
[[inject]]
id = "before_tick"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "before_tick"
access = "internals"
hash = "<the function hash in internals.json>"
```

The function takes the slot's exact receiver and raw record types from the generated internals
facade. `head` and `return` use function slots. If no compiled slot exists, native internals
listeners use a private source point and rebuild the game; the callback and its facade record path
stay unchanged. The log says `source (private slot, reason)` and private rows are never published
in `points.json`. Targets below `mod_points` are refused because dispatch would create a Cargo cycle.
`before_call`, `after_call`, `modify_value`, `wrap_call` and `wrap_if` also use private source calls
when the canonical callee is absent from every slot list. The launcher validates their exact identity
against the staged SDK before any game build; missing, moved or ambiguous calls refuse. Listed stale
calls never fall back to source. `wrap_call` controls whether and how often to invoke the original;
`wrap_if` runs it only when its condition returns true and requires unit output. The unit check runs
once the private site's metadata exists, before game compilation. For all call kinds, use the catalog's
canonical `call`, `nth` and statement `ctx`, not its place fingerprint `hash`.
The generated `Listener`, `Receiver`, `Args` and `Output` facade names derive from that canonical callee
and are identical on listed and source routes; physical IDs and dispatch remain distinct. The same
`with` function therefore compiles on either route without an alias or compatibility export.
The build log prints `mod:inject: target -> slot (head/return/site, internals)` for each accepted declaration. A changed
hash or missing target is refused, not silently redirected. Points order by descending priority,
then registration (dependency) order; ties within a mod use inject id. Sites retain priority, mod id
and inject id order. `modify_arg` is accepted on listed head slots but is a typed refusal on the
source route. `modify_call_arg` remains a typed refusal until its operand payload is specified.
An API `replace` owns the function, as a mixin overwrite does: its function takes the head's inputs
and returns the point's return type, which the caller gets, and the body never runs. At a point
returning `()` it returns `()` and the cancel is forced. It runs after the point's other API head
listeners, whatever its priority, so they may change its inputs but not its outcome; internals head
listeners run after it and can't undo it either. A point whose return has no API form refuses it at
load, naming the point and type, and so does one returning `Result<(), E>`, whose cancel would
answer `Ok(())`; a wasm module built without `cargo horizon-mod` is held to the same check when it
loads. A second `replace` at one point is refused at registration, naming both mods. A replacing
mod that stops or is disabled hands the function back: it runs as written again.
Native and wasm share it through `mod_points::replace!`.
Native API call sites, API source injects, internals `replace`, locals and extension fields remain
refused.
The `native` table accepts only build pins and supporting modules; unknown fields are rejected.
API head/return functions also load from wasm, with the same record.

## For agents

Apply these rules in order and stop at the first that fits.

1. A hook or a Part method exists for the moment: implement it. **No injection.**
2. No hook: an API point declaration. Use `[[inject]]` with `at`, lowercase `kind = "head"` or
   `"return"`, a crate-root `with` function, and the catalog's `hash`. Add `horizon-mod-points` beside
   `horizon-mod-api`; the function takes the side's host and the point's public record. Keep its body
   API-only so the same call can become a hook.
3. You need game internals: declare them and use `access = "internals"` with the generated
   receiver/record types. Native source routing rebuilds the game when no compiled slot exists.
   File a hook request; do not splice a hand-written helper into game source.

| Field | Rule |
| --- | --- |
| `id` | unique, nonempty |
| `at`, `hash` | canonical function path and its tool-written signature hash |
| `kind`, `with` | lowercase kind and a crate-root callback function |
| `access` | `api` (default) or `internals` |
| `priority` | descending; defaults to 0 |
| `call`, `nth`, `ctx` | exact canonical callee, occurrence and statement context for a call kind |

Function-slot errors name the declaration or target. The generated constructor checks the callback's
signature. Freshness errors never redirect an injection to a different function or call.

Naming: events and methods follow NeoForge 26.3 names with no `on_` prefix; the event is `&mut`, handles come from
the event, and writes take `host`.

## The decision ladder

1. **A hook or Part method exists: use it.** Read [Hooks](concepts/trait-mods.md) and the
   [rustdoc](reference/api.md). A per-block, per-item or per-entity moment is a Part method. Nothing in this
   chapter applies.
2. **No hook: an API `[[inject]]` function.** The default. Your crate-root function gets the side's host
   and the point's record (`mod_points::<Point>Args`): every argument as an API value, and a
   `cancel` or `result` where the function returns `()` or an API value. It uses only `mod_api` types.
   Every `Game` method is a point on both routes: the prebuilt game was built with them all, and a source
   build gets the same points, so a point injection builds against the prebuilt game.
3. **You truly need private internals:** declare their paths and fingerprints, then choose
   `access = "internals"`. The callback depends on that game build, not a stable API contract;
   it remains a hook request. A missing compiled slot requires a native source rebuild.

## The forms

| Form | Where | Game rebuild | `mod.wasm` |
| --- | --- | --- | --- |
| API `[[inject]]` | admitted head/return slots | no | yes |
| Internals `[[inject]]` | head/return slots and supported call sites | only without a compiled slot | no (refused at load) |
| Hook or Part method (no injection) | its documented firing point | no | yes |

## Ordering

Head slots run before head source callbacks; return source callbacks run before return slots.
Slots run API listeners before internals listeners. Within each phase priority descends, then
stable dependency/registration order; inject id breaks ties within one mod. A call wrapper may
invoke the original zero, one or several times. A head cancellation skips the body and the return
point, not the listeners after it: every head listener runs and sees `cancel` as the ones before it
left it, and one may clear it again. Return listeners observe the result only after the body actually
ran. Trait-mod hooks run by `#[listener]`
priority descending, then dependency order; a cancel skips later mods except those receiving cancelled
events, which may undo it. Hook contracts determine their own cancellation and replacement rules.

## Make your injection convertible

An injection becomes a hook when its body is a single call a hook can take. Keep to four rules:

- **Take the host**, never `&mut Game`.
- **Never read `Game` fields.** Whatever you need comes from handles, attachments and services reached from the
  host.
- **One call.** Keep the body to a single call into your own code, so the hook's listener is that call.
- **One injection per moment.** Two injections at a moment are two hook requests.

Counting server ticks in an attachment needs no game internals. Prefer an API point until the
matching hook exists; use exactly the same API-only call in either form.

**1. A point injection** (the default). `Game::tick` is a point, so this builds on the prebuilt game. The
mod depends on `horizon-mod-points` as well as `horizon-mod-api`:

```toml
[[inject]]
id = "count"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "tick"
hash = "98019f886ec32cc7"
```

```rust,ignore
{{#include ../examples/src/lib.rs:points}}
```

**2. The hook it becomes.** Say the catalog has a hook at that moment (the `server_tick` hook, which fires every server tick, stands in here), so the injection disappears and the
mod implements the hook. The listener body is the same call:

```rust,ignore
{{#include ../examples/src/lib.rs:tick_hook}}
```

```rust,ignore
{{#include ../examples/src/lib.rs:count}}
```

The `COUNTER` attachment is the same in both:

```rust,ignore
{{#include ../examples/src/lib.rs:counter}}
```

Moving from the point to the hook keeps the same call: the function body becomes the hook's body, and its record becomes the hook's event. That is what "convertible" means, and why shape 2 is the default.

## Ask for the hook

A gap is a hook request. File one as an issue on
[arismoko/horizon](https://github.com/arismoko/horizon/issues) with the contract below.
Copy this, and fill every field:

```markdown
**Hook request: <short name>**

- **What the mod needs:** <one sentence: the moment, and what the mod does there>
- **NeoForge 26.3 counterpart:** <event or method name, or "none">
- **Vanilla class and method:** <`net.minecraft…Class.method`, the call the hook wraps>
- **The injection it replaces:** <the `[[inject]]` table, `at`, `kind`, `access` and callback signature>
- **Mod:** <name, repository or `.hmod`>
- **Side:** <server, client or both>
- **Contract wanted:** <listen, cancel, replace a value, collect; what a listener may change>
```

A hook with a NeoForge counterpart takes its name, side and contract. One without is welcome and names the
mod mixin or use it replaces. State the vanilla method: the hook fires where vanilla calls it.

## The build log

The launcher reports each unified declaration's target, access and selected slot/source route.
A source route means a game rebuild, not a fallback after failed freshness checks. Compiler
errors name the callback registration or source adapter. Inspect `native/build.log` for the
mod and inject id rather than guessing which callback filled a slot.

A panic inside a point listener reports the mod and the method
(`in native mod <id>, hook <method>`).

## Support modules

A `[[native.module]]` table contributes a whole Rust module to a game crate, beside the injections:
self-contained items for source-level support code. It is the shape
for support code a moment needs and one function cannot hold — Portizon's runtime is the case that
asked for it.

```toml
[[native.module]]
id = "runtime"
file = "crates/horizon-client/src/game.rs"
name = "portizon"
source = "src/runtime.rs"
visibility = "pub"          # optional: "pub" or "pub(crate)" (the default)
```

`source` holds the module's items without an outer wrapper; the tool emits them into `file` as
`mod name`. The destination file must carry no top-level glob import — horizon-mixins refuses one,
because it cannot prove which names the glob introduces — and the payload must be self-contained: no
`mod` items, `#[path]`, `include!` or file attributes; assemble multi-file code into one file first.
Ids are unique across a package's injections and modules together.

Modules are **native-only** and last-resort support code, outside the `mod_api`-only
line: a mod's behaviour still goes through hooks, handles and Parts. Any capability a module needs
that the API lacks is a hook request in #44's row form, not a reason to grow the module.

## Which functions are points

Every function of the game module is a point: `Game` methods, methods of its other types (`GameLevel`,
`Levels`…), associated functions and free functions. A crate below the game adds the functions
`crates/horizon-mod-api-gen/slots/<mod_id>.txt` lists, one path per line (`horizon_client_render::chunk::Chunk::get`);
each one there is measured first, since a slot on a hot function costs on every call. Left out:

- the hook functions themselves (`native_*`), which are hooks;
- a function compiled only under a `cfg`, and one in a `cfg`-gated module;
- `const`, `async` and `unsafe` functions, and methods that take `self` by value;
- `impl Trait` or `!` in the signature, `#[inline(always)]`, and a parameter named `cancel` or `result`.

A `Game` method that takes `self` has a point named after the method (`tick`). Any other is named by its full path, snake-cased
without `horizon_client::`: `game_levels_levels_parked_dims` for `Levels::parked_dims`, and for a lower
crate without `horizon_`: `client_render_chunk_chunk_get`, and `Game::new` is `game_game_new`. `Game` methods'
points get a host; free and associated functions have none. A method of another type has no API
record until `mod_api` has a handle for that type; it keeps its internals form.

A method whose return value borrows the game (`-> &T`, `-> Option<&mut T>`) has a head and no API
return. A `usize` or `isize` crosses the record as `u64` or `i64`, one width for native and wasm
listeners. A `&[T]` parameter is copied as a `Vec<T>`, and `&str` as text a listener may replace. A
value a mirror can't convert at a call skips that call's API listeners, logged; a listener's value
the engine refuses leaves the engine's own in place, logged. Mod input never panics the game.

`points.json` gives each point's `api_refusal`: absent when it has an API record, else the first
parameter with no API type, as the registration refusal states it.

**The raw form (internals).** A point whose signature has no type or const generics and no `impl Trait` also has a raw
form: a native listener that gets the real receiver and a `&mut` to every argument in its own type,
internal types included, with `cancel` or `result` to stop the body. Its return listener gets the value
returned, a borrowed one included (`&mut &mut T`), where the API form has no return. The launcher's
generated `main` registers raw listeners by type; declaring them in `horizon-mod.toml` comes with the
per-mod internals list.
`points.json`, beside the prebuilt game, lists them all; the rustdoc of `horizon-mod-points` documents
each with the function it listens at.

The source tree carries no `#[point]`: the build of the prebuilt game, and the launcher's source builds,
put one on each method in the copy they compile. A plain `cargo build` of the game in a checkout has no
points; a mod listening there builds, and its listeners never fire.

## Raw listeners and declared internals

When an API-typed point cannot carry the state a mod needs, a native mod can declare game
internals and a raw listener. Each internal is an API request: propose the missing hook, handle
or Part on the [roadmap](https://github.com/arismoko/horizon/issues/771) so the mod can move back
to the typed API.

```toml
internals = [
    { path = "horizon_client::game::Game", hash = "<written by the tool>" },
    { path = "horizon_client::game::native_ticks::TickRound::listening", kind = "method", hash = "<written by the tool>" },
]

[native]
game = "26.3"
horizon = "<exact game commit>"
mixins = "<exact mixins commit>"

[[inject]]
id = "count"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "count"
access = "internals"
hash = "<written by inject add>"
```

Run `cargo horizon-mod internals sync` in the mod's folder after editing its Rust code. It
checks the mod against a discovery-only copy of the pinned game's source, uses compiler
warnings to find actual internal uses (including glob imports and macros), and replaces
top-level `internals` with their exact paths, kinds and hashes. Unused declarations are removed;
retained declarations keep their formatting. The command prints added and removed entries.
It is explicit: a normal build never changes the declarations reviewers and players read.

Install `horizon-launcher` on `PATH`, or set `HORIZON_LAUNCHER` to its executable. The command
invokes `horizon-launcher native-build --sync-internals <mod folder> --trust-native-code`, whose
stdout is a JSON list of discovered catalog entries for the tool. This runs `cargo check`, not
the game, but Rust build scripts and procedural macros execute with your rights: run it only
on code you trust. No installed game or instance is needed. The launcher caches the isolated
source and metadata build per pin under `native/internals-sync/<commit>/`; the second check
reuses those compiler outputs. Neither that cache nor its temporary diagnostic attributes is
used by normal builds or published prebuilt games. The source must be cached or downloadable;
`E_INTERNAL_SOURCE_UNAVAILABLE` explains when it cannot be obtained, without changing the
manifest or falling back to a different pin. Any compiler or catalog error also leaves the
manifest alone. Discovery checks the mod's library on the current host with default features;
configuration-gated catalog entries are still refused. Mods with source-spliced helper files
or module injections are refused with `E_INTERNAL_SOURCE_INJECTION`: declare their internals
explicitly with `internals add` instead. This invocation stages one mod project; required mod
dependencies that are not in that project are refused by the dependency resolver, not silently
omitted.

`cargo horizon-mod internals add <path>` remains useful for those explicit declarations and
writes the path and hash from the pinned game's `internals.json`; don't invent hashes. When several member kinds share a path (such as a field
and a method named `listening`), pass `--kind <kind>` to select the catalog entry, for example
`cargo horizon-mod internals add horizon_client::game::native_ticks::TickRound::listening --kind method`.
The tool reports the available kinds if the choice is missing and writes the resolved `kind`
into each added declaration, even for unambiguous paths. A declaration may omit `kind` only
when its path has a single kind. Kind names match `internals.json`: inherent methods use `method`, free
functions use `fn`, and fields use `field`. A `--kind` applies to all paths in that invocation.
`cargo horizon-mod update` shows the signature changes and refreshes hashes while retaining
member kinds. Use `cargo horizon-mod inject add` to select the target and write a unified
injection with its function hash. `with` names the callback in the mod's crate root.

Depend on `horizon-internals` (library `internals`) at the same pin as `horizon-mod-api`, never
on `horizon-client` directly. The build generates a facade for this mod alone. Its nested
modules re-export declared types and free functions: the declaration above makes
`internals::game::Game` available. Declaring a field or inherent method exports its owner type;
a type necessarily brings all its public fields and inherent methods. Undeclared types have no
name in the facade and do not compile.

The facade is generated when the game builds the mod, and it re-exports items the prebuilt
game makes public; the game's source keeps them private. So a plain `cargo check` of the
mod resolves `horizon-internals` to an empty crate and can't see `internals::…`. Compile
a mod that declares internals with `horizon-launcher native-build <mod folder>`, which stages it
against the pinned game the same way the launcher does on install, and report its errors.

A raw injection also exposes the point's listener and raw record in `internals::raw`, for
example `__TickListener` and `__TickRaw`. Discovery keeps those same typed exports and does
not change `[[inject]]`; slot listeners need no special sync step. Implement `pub fn count` with the exact listener type
from `points.json` (including the receiver and argument-record lifetime). The facade's safe,
typed `raw::tick` constructor registers it in the generated executable; a mismatched function
fails at that registration line, naming the injection. `return` uses the point's return
listener and `raw::tick_return` constructor. Raw listeners work against the prebuilt game as
well as source builds; unlisted source points and modules require a game rebuild.

Native slot and source routes check the actual game's metadata before compiling mods:

- `E_INTERNAL_MISSING`: the declared path is absent (type shapes not exposed as internals
  cannot be declared). Run `cargo horizon-mod internals sync` to discover this pin's used
  internals. Undeclared-item compiler errors also point to this command.
- `E_INTERNAL_CHANGED`: the hash changed; the error names the new signature and asks for
  `cargo horizon-mod update`.
- `E_INTERNAL_AMBIGUOUS`: several member kinds share the path; choose one of the listed kinds
  with `internals add <path> --kind <kind>`.
- `E_INTERNAL_GATED`: the entry exists only under a `cfg`; target-specific declarations are
  not supported.
- `E_NOT_RAW`: the injection's target has no raw form, for example because it is generic or
  takes `impl Trait`.
- `E_GAME_DEPENDENCY`: the mod depends directly on `horizon-client`; use the facade instead.
- `E_INTERNAL_PATH`: the entry cannot be re-exported as an item, for example an entire module;
  declare its types or free functions instead.

The build log lists each mod's internals and raw places. The same information is recorded in
`native-mods.json` beside the built game.

### Listed call sites (native only)

An internals `[[inject]]` with `call`, `nth` and `ctx` wraps that exact call, not the enclosing function. Both native
build routes use the same `points.json` object: `points` lists function points and `sites` lists
call sites. Only reviewed call entries in `crates/horizon-mod-api-gen/slots/<mod_id>.txt` become sites:

```text
horizon_client::game::Game<'_>::next_autosave_interval | autosave_interval | 0 | <ctx from points.json>
```

Each slot list accepts a function path, `at | call | nth | ctx`, or `type <path>` (reserved for extension
fields; not consumed yet). Files support `#` comments; identical declarations merge, and conflicting
contexts name both files. The stage reads their sorted union, evaluates the receiver once, and rewrites
only the selected call expression in its copy of the game.

Call identity uses the resolved callee path, not its spelling: aliases and different receivers of the
same type share one ordinal sequence. Ordinals count runtime calls in source order; constant contexts
and nested item bodies are not selected. `points.json` publishes each site's resolved `call`, `ctx`,
and `hash`. The fingerprint includes the callee path and signatures. `ctx` hashes the containing
statement's tokens and its block/if/match-arm path, ignoring source formatting. Empty contexts are
for tool discovery only; copy the generated context when listing a slot, never invent one.

A unique matching context at another ordinal refuses with `E_INJECT_MOVED`, naming the new `nth`.
A changed or missing context refuses with `E_INJECT_LOST`. Two calls to the same callee with identical
contexts are also lost, even when `nth` still matches: the tool cannot prove which identical statement
was intended. Refresh a moved ordinal with `inject update`; reselect a lost call with `inject add`.
Function signature changes and missing functions use `E_INJECT_CHANGED` and `E_INJECT_MISSING` at the
unified routing boundary. `E_INJECT_NO_SLOT` reports an unsupported route or kind, with its reason
and requested slot declaration. `E_INJECT_INVALID` reports malformed declaration fields.
Freshness errors never silently select a different call.

```toml
[[inject]]
id = "autosave"
at = "horizon_client::game::Game<'_>::next_autosave_interval"
kind = "modify_value"
with = "autosave"
access = "internals"
hash = "<function hash written by inject add>"
call = "horizon_client::game::autosave_interval"
nth = 0
ctx = "<statement context written by inject add>"
```

The facade's `internals::raw` module exports this site's exact `Args`, `Receiver`, `Output` and
`Listener` aliases and a typed registration constructor, all named from the canonical callee (the same names on slot and source routes).
Use those aliases for the callback. The generated main adapts each kind to one wrap listener:

| Kind | Callback inputs and result |
| --- | --- |
| `before_call` | Receiver and mutable arguments; runs before the original call |
| `after_call` | Receiver and a shared reference to the result; keeps that result |
| `modify_value` | Receiver and the result by value; returns the replacement |
| `wrap_call` | Receiver, mutable arguments and mutable `original` continuation; returns the result |
| `wrap_if` | Receiver and shared arguments; returns whether to call the original; unit output only |

Listeners nest by descending priority, then mod ID, then injection ID; the first is outermost.
A wrap may call `original` zero, one or several times. `cancellable` is refused: returning from the
caller is not a call-site operation. Borrowing results, non-record arguments, ambiguous targets
and alias-dependent receiver arguments are refused with the declaration's file, line and reason.

An unlisted canonical call can use native source routing after the same freshness checks.
A listed stale call never falls back to source. Use the function hash and statement context,
not a site's physical fingerprint, as the declaration identity. `wrap_if` needs unit output.
These call kinds are not available to wasm mods.

## Reference

- A point's methods are `<point>` (its head: the arguments it was called with) and `<point>_return` (after
  its body: the value it returns), in a crate-root `with` function. Its record is `mod_points::<Point>Args`.
- Depend on `horizon-mod-points` only to listen at points. It holds one record per point;
  a mod without point injections leaves this dependency out.
- A listener's host is a `HostSide` on a function that takes `&mut self`, and `&dyn Host` on one that takes
  `&self`.
- The game sets a point's slots once, after the mods are installed; a point no mod listens at costs one
  atomic load and a branch.
- A point injection's `hook`, bindings and horizon-mixins fields are refused: a hook is its own `Hooks` method,
  and the record carries what the point passes.
- `[[inject]]` has no binding or hook-version fields. Implement a catalog hook through `Hooks`;
  use the slot's generated callback signature for an injection.
