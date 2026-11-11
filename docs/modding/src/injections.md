# Injections

An `[[inject]]` attaches a typed listener to a compiled slot: a dispatch point already
present in the game. An inject never compiles game code. No slot means refused,
on native too; use a [mixin](mixins.md) or request the slot. The launcher-picked
route from #1607 is gone.

Prefer a named hook or Part method when one exists. Otherwise use `access = "api"`
for API-owned records on native and wasm, or `access = "internals"` for raw native
listeners and declared internals. Every injection is a request for a named hook.

## Declaration

```toml
[[inject]]
id = "count"
at = "horizon_client::game::Game<'_>::tick"
kind = "head"
with = "count"
access = "api"
priority = 0
# hash = "<function hash written by inject add>"
```

| Key | Meaning |
| --- | --- |
| `id` | Unique, nonempty declaration ID |
| `at` | Canonical function path |
| `kind` | One of the eight kinds below |
| `with` | Listener function in the mod crate root |
| `access` | `api` (default) or `internals` |
| `priority` | Descending priority, default 0 |
| `hash` | Optional function signature pin; required for another mod's point |
| `call`, `nth`, `ctx` | Canonical resolved callee, zero-based occurrence and statement context; required together for call kinds |

| Kind | Contract |
| --- | --- |
| `head` | Before the body; mutable argument record and cancellation/result |
| `return` | After the body; mutable returned value |
| `replace` | Owns the function; returns its result without running the body |
| `before_call` | Receiver and mutable evaluated arguments, before the call |
| `after_call` | Receiver and shared result; keeps the result |
| `modify_value` | Receiver and owned result; returns its replacement |
| `wrap_call` | Receiver, arguments and original-call continuation; returns the result |
| `wrap_if` | Receiver and shared arguments; returns whether to call; unit output only |

API listeners get every parameter copied into an API record. Mutable parameters
are copied back; by-value and shared parameters use the listener's replacement.
A `Game` method's API point also supplies the side's Host. A parameter with no
API/Wire form refuses registration, naming its type, on both targets.
Internals listeners use the exact receiver and raw argument record exported in
`internals::raw`; their listener types are in `points.json`. Declare engine types
through the top-level `internals` list and depend on `horizon-internals`, not the
game crate. Engine internals and call sites are native only; wasm uses API
function points. There are no API call-site adapters.

## Ordering

Head cancellation skips the body and return point, but every head listener still
runs and may clear cancellation. Return listeners run only after the body.
API listeners precede internals listeners; priority descends, then dependency
registration order, with inject ID breaking ties within a mod. A replace runs
after head listeners of its tier; only one replace may own a point across both
tiers. Disabling it restores the original body. It requires an outcome the record
can carry: borrowed returns and `Result<(), E>` are refused. Calling the replaced
function from its replacement re-enters the point, not the original body.

## Writing a declaration

An `[[inject]]` needs `id`, `at`, `kind` and `with` (plus `call`, `nth` and `ctx` for a call kind,
copied from the pinned `points.json`). `hash` is optional pinning. Without it, the build takes the
function's fingerprint from the catalog of the game `[native] horizon` pins, which fixes every
signature exactly, and records it in the build's `native-mods.json`. With it, the build checks it
as written and refuses a stale one with `E_INJECT_CHANGED`. The same holds for each `internals`
entry's `hash` (`E_INTERNAL_CHANGED`). Moving the pin with `cargo horizon-mod update` compares the
old and new games for every declaration without a hash: an inject whose function changed refuses the
update with `E_INJECT_CHANGED` until you pass `--accept <inject id>`, as a stale hash refuses the
build, and internals without a hash are reported and stay without one.

Run `cargo horizon-mod inject add <at> --kind <kind> --with <listener>` in a native mod's folder.
The tool appends `[[inject]]`, preserving comments, formatting and permissions. A symlinked
manifest is preserved; edits atomically replace its referent. Checked `[native].horizon` metadata supplies `hash`, pinning it.
`--with` supplies both listener and unique inject id (default `on_inject`); implement it separately.
Access defaults to `api`; edit it to `internals` when needed. The tool prints each tier's availability
at the place, such as `api: refused, argument 2 `batch` (`&mut HudBatch`) has no API type yet` and
`internals: native only`. The launcher validates compiled slot support.
Call kinds copy canonical `call`, `nth` and `ctx` from the pinned `points.json`. Use
`--call <canonical callee> --nth N` (default 0, per callee). Omitting `--call` requires one callee
at that ordinal. Ambiguous identities and unlisted calls are refused; no discovery runs.
Unknown places return `E_INJECT_MISSING` naming the target. Invalid kinds, callbacks,
duplicate ids, or call flags on function kinds leave the manifest untouched.

## Refreshing pinned declarations

`cargo horizon-mod inject update` checks every declaration against the current game's catalogs (a
declaration without a hash has none to refresh);
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
points get a host; free and associated functions have none.
A point name is always one snake-cased form, for engine and mod points alike. Snake-casing is not injective
(`a::b_c` and `a_b::c` are both `a_b_c`), so uniqueness comes from refusal: the build fails, naming both
functions, when two points have one name. A method of another type has no API
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

## Mod points: patching another mod

A mod's own functions are points too ([#2339](https://github.com/arismoko/horizon/issues/2339)),
by the same rules as the game's: every eligible function of the crate, public and private, free
functions, inherent methods and functions of inline modules (trait impl methods are not points). The
build compiles a staged copy with a point on each, in a slot table of the mod's own
(`<crate>::__horizon_points`); the author's source is never edited. A point is named by its path
from the crate root, snake-cased: `tanks_tank_fill` for `chemlib::tanks::Tank::fill`. Two functions whose
paths give one name (`a::b_c` and `a_b::c`) fail the build, naming both; rename one or opt it out.
A library's crate must be named after its mod id, with `-` as `_` (`chem-lib` builds crate
`chem_lib`): an addon's manifest names the library by id and each `at` by crate, and on both
targets the points of a library whose crate is named otherwise are refused.

- **Opting out.** `#[mod_api::no_point]` on a function gives it no point. An addon declaring a
  listener there is refused: `chemlib::audit has no point (opted out with #[mod_api::no_point])`.
- **Declaring.** An addon depends on the library and pins the function with
  `cargo horizon-mod inject add chemlib::transfer --kind head --with double_rate`, exactly as for
  the game's points: `at` is the function's path in its crate.
- **`access = "api"`.** The listener gets the point's record, `<crate>::__horizon_points::<Point>Args`:
  a copy of every parameter, `&mut` ones copied back, and `cancel` or `result`. Every parameter needs
  a `Wire` form: scalars, text, std containers, `mod_api`'s types, and the library's own publicly
  reachable types that carry `#[derive(mod_api::Wire)]`. Nothing adds the derive; a parameter without one refuses the
  point's API listeners, naming it. A method of the library's own types has no API record (its
  `self` has no handle), as on the game.
- **No Host.** As on the game, only a `Game` method's point gets a `Host`; free functions and
  methods of other types get none, and a mod crate's functions have no `Game` receiver. A mod
  point's `api` listener gets its record and nothing else; an addon that acts on the game does it
  from its own hooks, which get a `Host`.
- **`access = "internals"`.** The listener gets the receiver and every parameter by reference, in
  their own types, through the raw form declared beside the function. It works from the library's
  own target: the same binary on native, the same dependency group on wasm. A wasm listener
  at a declared dependency's point needs no engine internals declaration. The group registers
  listeners through typed functions exported by the library, then seals every library registry
  before initialization; each member keeps its own `mod_api` copy.
- **The signature check.** Each mod point's hash covers its path, its signature as written and its
  record's field types. When the library's function changes, the addon is refused at load:
  `E_INJECT_CHANGED: addon:double_rate: chemlib 1.1.0 changed chemlib::transfer: pinned …, installed …
  (signature)`. `cargo horizon-mod inject update` reports the change and repins with `--accept`.
- **Across targets.** A native library with a wasm addon, or the reverse, is refused at load. A native
  addon's listener at a wasm library's point says why it can never cross: "internals access to chemlib
  needs the same target", or an `api` listener on a parameter without a `Wire` form, naming it. Any
  other mixed edge gets the general refusal, "mixed native and wasm dependencies aren't supported yet"
  ([Wire types](how-to/wire-types.md)).
- **Cost when nothing listens.** One relaxed load of the place's slot and a branch at the head, one
  more for the return; the listened path stays out of line. On the chemicals fixture's per-tick
  transfer (a ~2 ns function) that is about 1.2 ns a call natively and 1.6–2 ns on wasm.

The build writes the mod's points to `mod-points.json` in a wasm package and pins its SHA-512 in
the manifest's `[wasm] points`; a package whose `mod-points.json` is missing, unpinned or changed is
refused.
The launcher lists a native mod's points from its source.

## Listed call sites (native only)

An internals `[[inject]]` with `call`, `nth` and `ctx` wraps that exact call, not the enclosing function. The compiled game publishes the `points.json` object: `points` lists function points and `sites` lists
call sites. Only reviewed call entries in `crates/horizon-mod-api-gen/slots/<mod_id>.txt` become sites:

```text
horizon_client::game::Game<'_>::next_autosave_interval | autosave_interval | 0 | <ctx from points.json>
```

Each slot list accepts a function path, `at | call | nth | ctx`, or `type <path>` (a type listing). Files support `#` comments; identical declarations merge, and conflicting
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
slot validation boundary. `E_INJECT_NO_SLOT` reports an unsupported slot or kind, with its reason
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
`Listener` aliases and a typed registration constructor, all named from the canonical callee.
Use those aliases for the callback. The generated main adapts each kind to one wrap listener:

| Kind | Callback inputs and result |
| --- | --- |
| `before_call` | Receiver and mutable arguments; runs before the original call |
| `after_call` | Receiver and a shared reference to the result; keeps that result |
| `modify_value` | Receiver and the result by value; returns the replacement |
| `wrap_call` | Receiver, mutable arguments and mutable `original` continuation; returns the result |
| `wrap_if` | Receiver and shared arguments; returns whether to call the original; unit output only |

Listed native `internals` calls also admit immutable borrowed arguments, such as `&str`.
Their generated `Args<'a>` record borrows the original values for the call; no implicit
owned-string conversion occurs. The listener is higher-ranked over that lifetime, so safe
callbacks cannot retain the record or its references beyond the call, or mutate the referent
through an immutable reference (ordinary interior mutability still follows Rust's rules).
A callback may replace an argument with another reference valid for the same lifetime.
Engine types are borrowed too: `&MobWorld` becomes a shared reference to the native
`horizon_client_lib::mobs::MobWorld<'g>` in `Args<'g, 'a>`, keeping the world's own
lifetime separate from the record's borrow. The facade re-exports the typed record;
it does not convert the world into a mod API value or expose private fields. A callback
can read the world's public fields and call its shared-reference methods using Rust's
normal field projection, including public nested values, even without naming every type.
Naming `MobWorld` itself requires its internals facade export. It cannot obtain exclusive
access or extend either lifetime. Copy arrays of scalar values, such as `[i32; 3]`, can
accompany these references in listed records.
`modify_value` still receives only the receiver and owned result, not the argument record.
Mutable borrowed arguments remain refused, as do borrowed arguments on API routes;
this does not enable API source adapters or wasm call sites.

### Owned arguments at listed calls

A listed native `internals` call can also move owned arguments, such as the `ItemStack`
in `Game::dispense_item`, into its record. Argument expressions run once in their original
order. No stack is implicitly cloned. A record with any owned non-Copy field selects the
**single-use** listener contract; named value types are conservatively treated as owned,
even when that particular type implements Copy. Non-generic private aliases are expanded
from the catalog, so the record names the actual field types rather than an inaccessible alias
in another implementation module. The existing scalar/shared-reference-only
contract remains repeatable.

`before_call` still receives `&mut Args`, allowing argument modification; an observing
callback can inspect its fields through shared references. `after_call` and `modify_value`
keep their result-only signatures. A single-use `wrap_call` instead receives `Args` **by
value** and the site's generated `Original<'g, 'a, 'next>` capability (exported beside
`Args`, `Receiver`, `Output` and `Listener` under the same canonical prefix):

```rust,ignore
pub fn wrap(receiver: Receiver<'_, '_>, mut args: Args, original: Original<'_, '_, '_>) -> Output {
    inspect(&args.stack);
    modify(&mut args.stack);
    original.call(receiver, args)
}
```

`original.call` consumes both the forwarding capability and the record. A second call,
using the moved record again, or retaining the capability past this call fails to compile.
A wrapper that does not forward must supply its own correctly typed output; its unconsumed
arguments drop normally. `wrap_if` remains unit-output-only and drops the arguments when
its condition is false. Panics unwind through ordinary Rust ownership: a value is not leaked
or dropped twice by the engine, whether the listener panics before or after forwarding.
No per-call heap allocation is needed for the capability.

Borrowed and owned fields share one lifetime-bearing record. Forwarding moves owned fields
and copies shared references; dropping the record does not drop their referents. Native
`internals` callbacks are trusted code, not a sandbox: a callback may explicitly take an
owned value through replacement or by owning Args, and may call `mem::forget`. The engine
promises no hidden copies or leaks; it cannot prohibit these explicit mod-side transfers.
Mutable-reference arguments and API/wasm call-site records remain refused.
Owned shapes containing further borrows remain refused; direct immutable-reference fields
can accompany owned fields.

A reviewed call row compiles the typed record into the prebuilt game. Attaching a listener
to that listed site does not rebuild the game. A stale listed declaration is refused by name.

The reviewed Portizon rows use the catalog's physical caller paths:
- `horizon_client_lib::mobs::villager_brain::secondary::sense_secondary`: `farmland`, ordinal 0;
- `horizon_client::game::sign_edit::Game<'_>::draw_sign_editor`: `editor_sprite`, ordinals 0 and 1;
- `horizon_client::game::machines::Game<'_>::dispense_from`: `Game::dispense_item`, ordinal 0.

Use these caller paths in the manifest. The nominal `horizon_client::game::Game::...`
caller spelling is currently refused as Missing by the loader; listing a call does not
add a nominal alias or change identity matching.

With no listener, these sites cost what every listed site costs: one relaxed atomic load
and a branch, without constructing Args or allocating. This is not literally zero overhead
relative to an uninstrumented call. An optimized author benchmark on an Intel Core i9-13900H
measured 1.44 ns per plain call versus 1.93 ns per unsubscribed listed call, a difference of
+0.49 ns per call (six alternating samples of 50 million calls, with the subscription flag
opaque to the optimizer). Run the generator's ignored `unsubscribed_listed_call_cost` test
to reproduce the measurement on another machine.

Listeners nest by descending priority, then mod ID, then injection ID; the first is outermost.
A repeatable wrap may call `original` zero, one or several times; a single-use wrap forwards
at most once. `cancellable` is refused: returning from the caller is not a call-site operation. Borrowing results, non-record arguments, ambiguous targets
and alias-dependent receiver arguments are refused with the declaration's file, line and reason.

An unlisted call is refused, never retargeted. Use the function hash and statement
context, not the site's physical fingerprint. Call kinds are not available to wasm.

## Refusals

| Code | Meaning |
| --- | --- |
| `E_INJECT_INVALID` | Malformed fields, unsupported kind or callback declaration |
| `E_INJECT_MISSING` | Target function is absent |
| `E_INJECT_CHANGED` | Function signature hash changed |
| `E_INJECT_MOVED` | Unique context moved to another `nth`; run `inject update` |
| `E_INJECT_LOST` | Context missing, changed or ambiguous; reselect with `inject add` |
| `E_INJECT_NO_SLOT` | Not listed, kind has no slot, native-only place, wasm call sites unsupported, API call sites require internals, no API record, or replace lacks a carried return value |

Refusals name the declaration and target. A missing slot includes the requested
slot row; freshness errors never silently select another function or call.

## Coming from Mixin

Use the [mixins chapter](mixins.md) for upstream horizon-mixins' source-level
vocabulary, selectors, bindings and state. It is a separate mechanism, not a
translation to a narrower inject kind.

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
