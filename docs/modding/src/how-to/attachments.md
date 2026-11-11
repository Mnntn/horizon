# Query attachment holders

Use an attachment query when a system needs the loaded holders of one
`AttachmentType<T>`, rather than checking every entity, chunk or block entity.
Declare the key once, as described in [server attachments](../concepts/trait-mods.md#server-attachments).
Import `AttachmentQueries` for reads and `AttachmentQueriesMut` for server writes.
Both traits use the same API for either build target.

## Read a snapshot

Call `host.query(&TYPE)` on `&dyn Host`, then borrow its rows with `batch.iter()`.
Reads require `T: Send + Sync + 'static`, not `Clone`. Each successful row has a
Copy `holder` and a borrowed `value: &T`. Match `AttachmentHolder` to work with the
existing `EntityRef`, `PlayerRef`, `BlockEntityRef`, `ChunkRef` or `LevelRef`.
Copying that identity does not keep its holder loaded. Attachment ownership follows
an entity object, not its saved UUID. A rejected passenger can own attachments
without being in the level; it is absent from queries until admitted. Equal-UUID
objects never share attachment contents.

This example defines a serialized counter and sums its captured values:

```rust,ignore
use mod_api::*;
static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "query_fixture",
    name: "value",
    default: || panic!("no defaults"),
    codec: Some(AttachmentCodec {
        encode: |value| Ok([("n".into(), Tag::Int(*value as i32))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n as u32),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

fn total(host: &dyn Host) -> u32 {
    host.query(&VALUE)
        .unwrap()
        .iter()
        .map(|row| *row.unwrap().value)
        .sum()
}
```

The example deliberately panics if a default is inserted: queries never call the
key's default, dirty the holder, or queue synchronization. It uses `unwrap` to
check test assumptions; real mods should handle key and row errors according to
their own policy. A saved entry decodes lazily when visited. A malformed saved
value or incompatible Rust type is a row error, not a silently skipped row or a
default replacing the saved data.

**A batch is a snapshot of both membership and stored value bindings at query
creation.** Later sets, removals, loads and unloads do not change that batch.
Create a fresh query to see them. Rows borrow from the batch, not from the host,
so the host can be used while a batch is retained. Rows cannot outlive their
batch. Interior mutability inside `T` is not deep-frozen by a snapshot.

## Write explicitly on the server

Use `host.query_mut(&TYPE)` on `&mut dyn ServerHost` (`T: Clone + Send + Sync +
'static`). This does not return `&mut T`. A mutable query row exposes `holder()`,
`value()`, `set(host, replacement)` and `remove(host)`.

The same example starts with 41, replaces it with 42, checks that a fresh
query sees 42 while the original row still reads 41, then removes the live value:

```rust,ignore
                let batch = host.query_mut(&VALUE).unwrap();
                let row = batch.iter().next().unwrap().unwrap();
                assert_eq!(*row.value(), 41);
                row.set(host, 42).unwrap();
                assert_eq!(total(host), 42);
                assert_eq!(*row.value(), 41);
                assert_eq!(row.remove(host).unwrap(), Some(42));
                assert_eq!(total(host), 0);
```

Every set/remove validates the captured residency against the supplied host's
current dimension before using the ordinary attachment write path. Unload or
replacement refuses the write with `AttachmentRowError::StaleHolder`; it never
revives the holder. A missing live holder is also reported as stale. Removing the
attachment value alone does not invalidate the still-loaded holder: a retained
row can explicitly set a replacement value there.

Each write joins the caller's existing transaction. For an all-or-nothing system,
import `HostTransaction` and perform the loop inside `host.transaction`, as in
the [HostTransaction reference](https://arismoko.github.io/horizon/api/mod_api/trait.HostTransaction.html).
There is **no implicit
transaction and no commit on drop**. Dropping a batch does not apply deferred
changes or undo successful writes. `remove` follows the ordinary attachment
operation, including its returned value and codec errors.

## Scope, liveness and order

A query covers only the host's **current dimension**. It does not load chunks or
entities, scan other dimensions, or offer cross-dimension joins. Saved values on
already-bound holders appear even if no earlier typed read touched them. A
well-formed absent key yields an empty batch, without initializing defaults.

A query is a **residency view**, not an immediate liveness filter. An entity whose
removal is queued but has not yet been swept may still appear, even in a fresh
query. Its handle's normal stale rules still apply, and stale writes fail. Do not
translate a captured scoped handle into an unscoped storage ID to bypass those
checks. Passing a host for another dimension does not retarget a retained row.

Traversal follows the dense index's order: insertion appends; removal fills its
slot with the final row. The same ordered mutation history is deterministic, but
order is neither sorted nor stable across structural changes.

## Costs and type identity

Native query acquisition performs one indexed lookup and clones the bucket Arc;
it neither clones `T` nor decodes values eagerly. Warm borrowed iteration makes
no allocations. A first saved decode may allocate according to the codec and
read cache. Empty native acquisition also allocates nothing.

Retaining a snapshot pins its old bindings. The next live mutation can copy the
row metadata and shared pointers once for that retained version; it does not
clone each typed value. Repeated snapshots interleaved with writes can therefore
cost more, and keeping many batches retains their old values. There is no idle
index sweep when no attachment or residency change occurs.

An erased
decoded value can only be read where its Rust type identity matches. A different
native or wasm type produces `TypeMismatch`; a failed
decoded downcast does not fall back to saved decoding. Share attachment type
identity through the normal dependency model rather than redeclaring a different
type with the same key.

## First-store errors

First stores on resident mobs, paintings and dragon parts bind their lifetime
registration. Each actual dragon part owns its own registration: stores never
alias its parent or siblings. Removing or replacing the dragon, or unloading its
physical storage, retires all eight parts even while attachment maps or query
snapshots remain retained. Parking a level preserves its resident parts.
Constructor-listener stores are indexed even when the triggering access was only
an inspection. Physical removal also releases the part's canonical attachment
map and construction state during entity cleanup; immediate network-ID reuse
clears the old part state before admitting its replacement.
Inspection and removal do not create a registration. First-store indexing for
other families is unchanged. A store can return
`AttachmentError::HolderBusy` while the
engine has lifted or extracted its physical storage: retry after the current
pass. The refused store writes no value and creates no registration.

`AttachmentError::UnsupportedHolder(family)` means the physical family cannot
own attachments; retrying cannot help. The name is bounded static text, not an
engine type. Wire decoding accepts at most 256 UTF-8 bytes for that text and
rejects longer names on both native and wasm guest paths. Family names are stable lowercase identifiers: `mob`, `item`,
`orb`, `projectile`, `armorstand`, `itemframe`, `painting`, `boat`,
`minecart`, `other`, `fangs`, `crystal`, `dragon`, `tnt`, `falling`,
`rocket`, `trident`, `spawner`, `cloud`, `cushion`, `knot`, `player`, `hook`,
and `dragonpart`.
Retained row writes report the matching `AttachmentRowError::HolderBusy` or
`AttachmentRowError::UnsupportedHolder`; a missing holder is instead
`StaleHolder`.

For an immutable keyed read (including render hooks), capture a snapshot with
`host.query(&TYPE)?`, then call `query.get(holder)?` with an `AttachmentHolder`.
The result is an owned value: `Some(value)` on a hit and `None` before sync or
when absent. `None` does not mean the holder is alive: absent and not-live
(stale or displaced) holders alike return `Ok(None)` when absent from the
snapshot. This read performs no host liveness check; only scoped residency
probes report `StaleHolder`. The expected O(1) holder probe runs before decoding, never scans
rows and never initializes defaults. Values require `Clone`; decode/type errors
use `AttachmentRowError::Value`. Fresh queries see replacements and removals;
retained queries keep their captured values. Scoped residency tokens still
reject rebound holders with `StaleHolder`.
