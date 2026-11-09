# Design: custom recipe-book settings

Proposal for [#1023](https://github.com/arismoko/horizon/issues/1023), not an
implemented API. This is a Horizon design; vanilla 26.3 is the reference for the
four existing books. No other project's implementation is a reference.

## Registration and ownership

Add an API-owned `RecipeBookTypeKey`, a validated namespaced resource id, and
register declarations in the existing mod registration phase. Registration
returns a typed key; it does not extend `BookType` or its ordinals. Require an
explicit namespace owned by the registering mod, lowercase resource-id syntax,
a nonempty namespace and path, and at most 256 UTF-8 bytes. Reject `minecraft:`.
Duplicate keys fail registration with the key and both declaring mod identities;
invalid ids report the mod, supplied id and violated rule. Never silently select
a winner. Freeze the registry when registration finishes, before loading player
state or negotiating connections. Late registration returns an explicit frozen
registry error. Resource reload does not reopen it.

Declarations are shared content on server and client. Native and wasm use the
same declaration, validation, limits and generated boundary contract. A client
must register a key to negotiate it; a server-only declaration can retain state
but cannot make an unmodified client display a book. Reads belong on a player's
recipe-book settings handle with `&dyn Host`; authoritative writes take
`&mut dyn ServerHost`. A client UI requests a change through its client settings
handle; it cannot directly mutate server state. Do not add player-id helpers to
`Host`. Registration uses its existing registration receiver.

Book identities are separate from `RecipeBookCategory` in
`crates/horizon-mod-api/src/item/recipe_book.rs`. Categories group recipes and provide
icons; a book owns two flags. A screen can associate categories with a registered
book explicitly. Neither category registration nor recipe-type registration
implicitly registers a book. Display layout, unlocking and custom recipe displays
remain separate work; this proposal supplies settings only.

## Storage, persistence, copy and replace

Keep `Settings.open` and `Settings.filtering` as `[bool; 4]`. Add a custom map
from validated key to a value `{ open: bool, filtering: bool }` alongside them in
`crates/horizon-client/src/recipe_book.rs`. Missing keys read as `(false,false)`.
There is no fifth vanilla index. Use one shared map implementation for local,
host and guest players, persistence and payload application.

Persist the map inside `recipeBook` under `horizon:book_settings`, a compound
whose keys are book ids and whose values contain byte booleans `open` and
`filtering`. Write both flags, including zero, and sort keys for stable output.
Missing extension means an empty map. Preserve well-formed entries even when
the owning mod is absent; registration is required for live access and sync,
not for retention. Parse the extension independently of vanilla recipe-list
validation so an invalid recipe list does not erase retained custom flags.

Bound the extension to 1024 entries, 256 bytes per key and 512 KiB encoded data.
Check lengths before allocating or iterating nested values. Accept only a
compound of compounds with both flags byte 0 or 1; skip malformed individual
entries with a diagnostic. Reject a duplicate key or an over-limit extension
as a whole, leaving the custom map empty and vanilla settings intact. The NBT
reader must enforce the byte/depth budget before materializing this subtree.
Registration also caps custom types at 1024, reporting overflow at registration.
Do not evict absent-mod entries to make room: a write exceeding the cap fails
without changing state. This bounds retention as well as network input.

Copy is an independent value copy of all four vanilla pairs and every custom
entry, including absent-mod entries. Mutating either player afterward cannot
change the other. Replace assigns the complete source settings value, clearing
destination-only entries; it is never map merge. Existing recipe and highlight
sets retain their existing whole-Settings copy semantics. Import without the
`recipe_book` section preserves the complete destination value. Import with the
section replaces it, including clearing custom entries when the extension is
missing. Host/guest swaps must move the complete value.

## First sync and subsequent changes

The server owns persisted player state. Negotiate a custom-settings capability
and the intersection of registered keys during connection setup. Send no
extension payload without successful negotiation. Namespaced keys, not process
local registration indices, identify entries on the wire. Vanilla settings and
seen/add/remove packets retain their current fields, ordinals and meaning;
a vanilla client receives exactly the existing four-book traffic.

After player load and ordinary initial recipe-book packets, send one bounded
custom snapshot on the negotiated mod payload channel, before enabling custom
book UI. Snapshot includes every negotiated key, explicitly `(false,false)` when
absent from storage. Applying it replaces the client's custom view. Retained
unregistered or unnegotiated keys never leave the server. Snapshot delivery is
also required on imported-state replacement and every reconnect. Clear the old
client view and pending requests at disconnect; never upload that cache as a
replacement for server state.

Use reliable ordered delivery on the connection. A snapshot starts a fresh
connection epoch at revision 0; each accepted server change increments the
revision. Updates contain epoch, revision, key and both flags. Clients apply
only the current epoch in revision order; a gap disables the custom UI until
a fresh snapshot arrives. Client requests carry the current epoch and a
monotonic request number; the server validates ownership of the connection's
player, capability, negotiated key, booleans and bounds before applying once.
Reject stale/duplicate requests without mutation; answer with authoritative
state so the UI can settle. Later valid requests are processed in arrival order.
Server writes use the same apply path, then send the resulting update. Notify
gameplay once after an accepted change, never for rejection or an unchanged
pair, through the existing settings-change moment extended to typed identity.

Cap payloads at 512 KiB and snapshots at 1024 distinct entries; updates contain
exactly one entry. Reject malformed/oversized payloads before allocation or
mutation, including duplicate snapshot keys. Rate-limit requests to 20 per
second per player; excess requests are rejected and cannot alter flags. Client
UI may show pending intent but only authoritative replies enter saved state.
Single-player uses the same server apply operation without network serialization.

## Worked book and acceptance cases

A cooking mod registers `demo:cooking_pot` during registration and binds its
cooking screen to that key. The screen reads `(false,false)` initially. Opening
it requests `(true,false)`; the server accepts, stores that pair and sends it
back. Selecting the craftable-only filter requests `(true,true)`. A category
such as `demo:soups` can appear in that screen without owning either flag.
These are proposed operations, not compilable API examples.

Use these independent expectations in implementation tests:

| Case | Expected result |
| --- | --- |
| Fresh pot and `demo:oven` | Both `(false,false)`; opening pot leaves oven `(false,false)` |
| Save pot `(true,false)`, load, reconnect | Pot exactly `(true,false)` before UI opens; all four vanilla pairs unchanged |
| Disconnect with pending `(false,true)` not accepted | Reconnect snapshot restores server's `(true,false)` |
| Save with mod absent, then reinstall | Retained pot `(true,false)` becomes accessible again; no absent key was synced |
| Copy A with pot `(true,false)` into B; set A to `(false,true)` | B remains `(true,false)` |
| Replace B containing oven `(true,true)` with A containing only pot `(true,false)` | B map contains exactly pot `(true,false)`; oven reads default |
| Load without extension | Empty custom map; registered pot reads `(false,false)` |
| Unknown, wrong-side, stale or oversized update | No flags change and no gameplay notification |
| Add category `demo:soups` | No new settings entry or vanilla ordinal |
| Vanilla peer | Existing four-book packet bytes and behavior match vanilla 26.3 |

## Current routes to update

References below are verified against this worktree at `84e57e6a79`; paths are
relative to `crates/horizon-client/src/`. Implementation must keep each route
on the shared settings operations rather than duplicate custom-map handling.

| Route | Current file:line | Required handling |
| --- | --- | --- |
| UI toggles and send | `game/items/recipe_book.rs:644`, `:827` | Dispatch vanilla identity to existing packet; custom identity to request |
| Remote settings application | `game/items/recipe_book.rs:1339` | Keep vanilla assignment separate from custom snapshot/update |
| Server validation | `lan/hub.rs:4491` | Preserve four-type ordinal validation; validate custom payload separately |
| Gameplay notification | `game/network/lan/mod.rs:2853` | Notify after shared authoritative apply |
| Initial sync and reconnect | `lan/hub.rs:4001`, callers `:1551`, `:1704` | Send negotiated snapshot after ordinary packets |
| Initial packet construction and resend | `lan/hub.rs:63`, `:4077` | Preserve vanilla packet construction; include custom snapshot on replacement |
| Persistence | `save.rs:4426`, `:4601`; `recipe_book.rs:926`, `:963` | Load/write extension with shared bounded codec |
| Host to guest and guest to host | `lan/playerdata.rs:291`, `:633` | Clone the complete value independently |
| Import preservation and replacement | `lan/import.rs:408`, `:460` | Preserve or replace whole map according to section presence |
| Acting-player swaps | `game/network/lan_actions/turns.rs:354`, `:808` | Swap complete Settings, including custom map |

The vanilla reference queries are `rea_vanilla.py method
net.minecraft.stats.RecipeBookSettings replaceFrom` and `rea_vanilla.py method
net.minecraft.stats.ServerRecipeBook sendInitialRecipeBook`. Custom registration,
retention and negotiation are Horizon decisions, not claims about vanilla.

## Open questions

- Exact public handle and registration declaration names: recommend following
  the registration catalog's naming when implemented, with `RecipeBookTypeKey`
  as the owned identity and no new broad Host service.
- Payload channel allocation: recommend one dedicated recipe-book-settings
  channel within the existing negotiated mod transport. The wire schema and
  bounds above should be shared by native and wasm adapters.
