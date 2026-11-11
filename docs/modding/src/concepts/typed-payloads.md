# Design: typed payloads

Accepted design for [#813](https://github.com/arismoko/horizon/issues/813), with vanilla-compatible framing. Stage 1 is in the code (see [Stage 1](#stage-1-capacities-tokens-and-send-services)); nothing registers, negotiates or delivers a typed payload yet. Registrar and send services remain object-safe.

## Decision: shared codec, registration and delivery

Register a typed value codec, phase, direction, schema version, required/optional peer policy and exactly one owning handler per receiving side at load. Freeze declarations after dependency-ordered registration. Resolve namespaced identifiers once into connection-local numeric slots; dispatch by slot directly to the owner, never broadcast to all mods. Duplicate declarations fail load, including identical duplicates. Only the declaring namespace may register its payload; dependencies use its exported typed token.

Propose ToServer<T> and ToClient<T> tokens with separate send services and ServerHost/ClientHost handlers. Bidirectional declarations produce both tokens and two explicitly sided handlers. Configuration and play are distinct capabilities; common declarations permit both. Configuration handlers get only capabilities valid before world admission. Every callback gets the appropriate Host; codecs receive bounded immutable input and a scoped registry lookup, never a mutable Host or world access.

Share the typed schema/value foundation proposed by [PR #2332 (#811)](https://github.com/arismoko/horizon/pull/2332): the mod owns schema and pure codec, the game owns orchestration, validation and publication. Separate data-format decoding from network encoding while reusing primitive bounds, structured errors, registry references and generated native/wasm adapters. Wire is an ABI representation, not permission to serialize Rust memory, pointers or Any. Erase typed callbacks internally once at registration with a checked type token. Do not invent a second codec framework for networking. Horizon signatures remain proposals; the verified counterpart contract is recorded below. Before implementation, specify API-owned registrar and send-service signatures: keep generic typed registration outside dynamic dispatch, and typed send helpers outside the object-safe Host service, delegating to a non-generic erased send operation. Tokens bind the registered declaration, phase and load-session identity; validate these plus direction and connection epoch before sending. Forged or stale wasm tokens return a structured refusal without dispatch. Erased handlers recover only their registration-checked type. Generate Wire adapters for every supported type, or refuse load explicitly with the unsupported type named on both native and wasm; no implicit adapter failure.

## Stage 1: capacities, tokens and send services

`crates/horizon-mod-vocab/src/content/payloads.rs` holds the API-owned types the later stages build on. Token values, including Wire-decoded ones, are untrusted claims. Only admission against the engine-owned frozen declaration table grants a slot; there is no send implementation yet.

| Item | Contract |
| --- | --- |
| `SERVERBOUND_PAYLOAD_CAPACITY`, `CLIENTBOUND_PAYLOAD_CAPACITY` | Discard-fallback body limits only, not registered-payload caps |
| `PayloadPhase` (`Configuration`, `Play`, `Common`), `ConnectionPhase` | `permits` says whether a declaration may travel in the connection's phase |
| `PeerPolicy` (`Required`, `Optional`) | As under [Negotiation and absent mods](#negotiation-and-absent-mods) |
| `PayloadToken` | The erased token: slot, phases, direction and the load session that issued it. Private fields, issued by the game. `admit(direction, load_session, phase, declarations)` returns the slot or `StaleToken`, `WrongDirection`, `WrongPhase`, after exact comparison against the engine-owned frozen slot table, before any codec or handler runs |
| `ToServer<T>`, `ToClient<T>` | A `PayloadToken` bound to its value type and direction. Binding a token of the other direction fails |
| `ServerboundPayloads::send_erased(token, body)`, `ClientboundPayloads::send_erased(player, token, body)` | The two object-safe send services, one erased operation each. A constant in the module stops compiling if either stops being usable behind `dyn` |
| `PayloadSendError` | `PayloadTooLarge`, `UnsupportedPeer`, `Backpressure`, `StaleToken`, `WrongDirection`, `WrongPhase`, `NotConnected`; has a Wire form |

The typed send helper (stage 2, with the codec) is generic and lives on the token: `ToServer<T>` encodes a value and calls `ServerboundPayloads::send_erased`, `ToClient<T>` calls `ClientboundPayloads::send_erased`. A `ToClient<T>` has no method taking the serverbound service, which is the compile-time refusal the acceptance matrix asks for. The connection epoch is the connection's, not the token's: the send service checks it with the token.

## Evidence and limits

The clean-room 26.3 contract confirms registrar modifiers `versioned`, `optional`
and `executesOn` return copies; registration methods return the registrar.
Play, configuration and common each support ToClient, ToServer and Bidirectional.
Handlers default to MAIN. Required missing payloads refuse admission; optional
ones may be absent. NeoForge peers compare versions exactly (trimmed strings).
Duplicate ids in the same protocol and registration after setup are refused.
These are contract evidence, not claims that Horizon has implemented negotiation.

The 32,767 serverbound and 1,048,576 clientbound constants limit only the
unknown-id discard fallback body in both phases. Registered payloads have no
per-payload cap beyond their codec and frame/compression limits (2,097,151 framed
bytes and 8,388,608 uncompressed bytes). They are not typed-envelope ceilings.

## Negotiation and absent mods

Exchange bounded manifests during configuration, before accepting application payloads. Match identifier, schema version, direction and phase; assign deterministic slots to their intersection. Required missing or incompatible declarations refuse admission with a diagnostic naming both versions. Optional absence yields UnsupportedPeer on send and no packet; never silently substitute raw delivery. Vanilla peers may join only when every declaration needed for admission is optional. Mod package dependency versions and network schema versions are separate checks. Reconfiguration invalidates slots and queued messages from the old connection epoch; renegotiate before play resumes. No version gates retaining an obsolete API or schema conversion shim.

Provisional limits: 4096 manifest entries, identifier 129 UTF-8 bytes, version 64 bytes, manifest total 1 MiB. Reject oversized manifests before creating entries. A forged unknown slot in the negotiated transport is a protocol error; unknown unrelated vanilla/plugin channels retain their existing behavior. Reserve the typed envelope channel against generic send_packet injection without banning unrelated plugin channels.

## Bounds, trust and delivery

Registered payloads obey their codec and frame limits, not discard-fallback limits. Depth 64, strings 4096 UTF-8 bytes, lists 4096 entries, decoded allocation budget at least sufficient for a full legal envelope; the earlier 256 KiB proposal cannot reject otherwise legal payloads. Structured codec bounds are reviewed per schema so they do not become a lower transport ceiling. Validate signed lengths, integer overflow, remaining bytes, collection counts and cumulative budget **before** reserve/copy/allocation. A small encoded count must not permit huge decoded objects. Require complete consumption, valid discriminants, finite domain floats where declared and valid registry references. Bound outer framing/decompression before custom decoding; a later codec check cannot repair an earlier unbounded allocation.

Both native input and wasm input are untrusted. Generated bounded primitives enforce these limits; custom codecs require runtime allocation/fuel accounting and must not escape their input scope. Native arbitrary allocation cannot be sandboxed: native code must obey the codec contract, and process-aborting panics remain unrecoverable. Catch unwind/traps at the codec/handler boundary, abort the active transaction, discard pending sends, disable/report the offending callback under runtime policy and close the affected session. Never unwind through the ABI. Invalid remote data disconnects its sender without invoking the handler; local encode failure returns a structured error and sends nothing. Revalidate authority, holder incarnation and handles at apply, not only decode.

Run mod codecs and handlers on the logical receiving main thread, with bounded byte queues from network I/O. Preserve per-connection receive order across payload types and ordinary gameplay actions at the same ingress queue; do not promise ordering across connections. Handler sends become visible after successful root transaction commit; nested commit merges and abort emits zero packets. Queue acceptance is not delivery acknowledgement. Replies run at the next receiving dispatch turn, including integrated play, preventing recursive callback delivery. Configuration completion waits for required sync acknowledgement. Propose 32 messages per peer per tick, with byte and connection queue budgets large enough to admit a complete maximum-size legal envelope plus queue bookkeeping; a byte scheduling budget must allow one such message to make progress; reject local sends with Backpressure, disconnect flooding remote senders. No silent truncation or unbounded retry.

## Stage 2: the value codec

`crates/horizon-mod-vocab/src/content/payload_codec.rs` holds the pure codec half of stage 2, exported from `mod_api`, and `#[derive(PayloadCodec)]` writes a type's codec from its fields. It is the part that touches neither registration nor delivery, so it stands apart from the raw transport of [#2882](https://github.com/arismoko/horizon/issues/2882). Nothing calls it yet: the typed handler adapter that decodes a received body, and the typed send helper that encodes one, arrive with registration.

| Item | Contract |
| --- | --- |
| `PayloadCodec` | The trait a payload's value type implements: `encode(&self, &mut PayloadWriter)` and `decode(&mut PayloadReader) -> Result<Self, PayloadDecodeError>`. Both see only the buffer, never a `Host`. Implemented for `bool`, `u8`, `i8`, `i16`, `u16`, `i32`, `i64`, `f32`, `f64`, `VarInt`, `VarLong`, `String`, `Vec<T>`, `Option<T>` and `Box<T>` |
| Encoding | Vanilla's network encoding, so a ported payload keeps its exact bytes: big-endian numbers, VarInt and VarLong, a VarInt byte length before a string or byte array, a VarInt count before a list, one boolean byte before an optional value |
| `decode_payload::<T>(body)` | The whole body as one `T`; a byte left over is `Trailing { bytes }` |
| `encode_payload(&value, limit)` | The body, or `PayloadEncodeError`; the byte limit is the caller's (the transport's frame bound), checked before the buffer grows |
| `PAYLOAD_MAX_DEPTH` | 64. Each list, each present option and each `nested` level counts one; level 65 is `TooDeep` |
| `PAYLOAD_MAX_STRING_BYTES`, `PAYLOAD_MAX_LIST_ENTRIES` | 4096 each, the defaults of `String` and `Vec<T>`. A schema with another bound calls `str_within(limit)`, `bytes(limit)` or `list(limit, read)` itself, so these are not a transport ceiling |
| `PAYLOAD_DECODE_BUDGET` | 8,388,608 bytes of allocation for one value. A list is charged `count * size_of::<T>()` before it is reserved, an owned string its length, so a small body cannot ask for a large allocation |
| `#[derive(PayloadCodec)]` | The generated codec, in `crates/horizon-mod-api-macros/src/payload.rs`. A struct is its fields in declaration order with nothing between them. An enum is a VarInt tag, the variant's position from 0 whatever its discriminant, then that variant's fields; a tag no variant has is `InvalidDiscriminant`. Each field goes through its own `PayloadCodec`, so the bounds above hold inside a derived type, and it adds no nesting level of its own. An enum with no variants, a union and a field without a codec do not compile |
| `PayloadDecodeError` | `UnexpectedEnd`, `VarIntTooLong`, `NegativeLength`, `LengthOverLimit`, `Utf8`, `TooDeep`, `InvalidDiscriminant`, `NonFinite`, `BudgetExhausted`, `Trailing`, `InvalidValue`; has a Wire form |
| `PayloadEncodeError` | `TooLarge`, `LengthOverLimit`, `TooDeep`, `NonFinite`; has a Wire form. An encoder refuses what the peer's decoder would refuse |

A length is read as a signed VarInt and checked for sign and bound before anything is reserved or copied; strings and byte arrays are borrowed from the body. A boolean other than 0 or 1 is refused (vanilla reads any non-zero byte as true). `finite_f32` and `finite_f64` are how a schema declares a finite float. No decode path indexes, and none panics on any input.

Two things this stage does not do. The bounded field kinds of [#811](https://github.com/arismoko/horizon/issues/811) (`DatapackFieldKind`) are not merged with this reader: they describe JSON entries the game decodes, are named for datapacks, and their issue is in flight; a shared name for the two belongs to whoever lands second. And the budget counts what the reader's own primitives allocate, not what a hand-written codec allocates beside them; native code has to obey the contract, as the section above says.

## Routes to replace

Owners verified again for stage 1 (main at `8b8578047a`), named by function because line numbers drift. Paths are under `crates/horizon-client/src/` unless they start with `crates/`. These are current code owners, not claims that typed delivery exists.

| Route | Owner and treatment |
| --- | --- |
| Server send to player | `game/modding/native_net.rs` `net_send_to_player` queues, `flush_native_net` builds the packet; replace byte queue with typed envelopes and negotiated recipients |
| Client send to server | Same file, `net_send_to_server`; replace the `remote.mod_net` boolean with negotiated token/phase checks |
| Server receive, LAN and dedicated | `game/network/lan_actions/packets.rs` `lan_guest_packet` to `native_net.rs` `guest_native_net`, queued dispatch in `run_native_net`; both must enter one ordered typed ingress |
| Client receive | `game/network/remote/mod.rs` `handle_packet` to `game/modding/native_book_net.rs` `remote_book_or_mod_payload` to `native_net.rs` `remote_native_net`; replace immediate raw callback with ordered delivery |
| Raw hook adapters | `game/native_hooks/base.rs` `native_net_from_client` and `native_net_from_server`; delete along with `crates/horizon-mod-vocab/src/hooks/net_from_client.rs` and `net_from_server.rs` |
| Public sends | `crates/horizon-mod-vocab/src/host/network.rs` `net_send_to_player` and `net_send_to_server`; delete raw methods and regenerate native/wasm API, docs and sample callers together |
| Configuration advertisement | `lan/conn.rs` `login_and_configure` sends the empty `horizon:mod_net` payload; `net.rs` `receive_brand` turns it into `Event::ModNet`; `game/network/remote/mod.rs` `poll_remote_events` sets and resets `remote.mod_net`. Replace the announcement with the manifest exchange |
| Server dispatch and flush | `game/network/lan/mod.rs` `tick_lan` runs `run_native_net` after the tick's packets and `flush_native_net` at its end; `native_players_leaving` (`game/native_hooks/shared.rs`), `stop_integrated` (`game/world/integrated.rs`) and `stop_dedicated` (`game/interface/dedicated/mod.rs`) flush once more. Preserve these commit boundaries |
| Integrated and split-screen client | `game/network/remote/mod.rs` `connect_local`, called for the window and by `game/runtime/split/players.rs`. Use the same negotiated transport and queued handlers for each logical client, never direct calls |
| Attachment sends and receive | `game/modding/attachment_sync.rs` `recipients` and `attachment_snapshot` select recipients and build snapshots, `flush_attachment_sync` sends through `lan/guest.rs` `send_attachment_sync`; the client's `handle_packet` calls `remote_attachment_sync`. Migrate to reserved typed declarations, retain tracker authorization |
| Codec classification | `crates/horizon-protocol/src/packets/custom_payload.rs` `uses_registered_payload_codec` is the seam the frozen declaration table replaces; `horizon:mod_net` and `horizon:attachment_sync` leave it with their modules |

Generic packet sending (`send_packet` in `game/modding/native_net.rs`) must also reject forged typed transport packets. This does not remove vanilla packet interop. The execution trace below gives the socket and scheduler owners.

### Execution trace

Read from the code for stage 1; nothing here was measured or run.

**Server, every kind (LAN host, dedicated, single player's server thread).** One code path: `Game` with a `LanHost`.

1. *Accept.* `lan/mod.rs` `accept_loop` (thread `horizon-lan-accept`) starts a `horizon-lan-conn` thread per socket; `Lan::connect_local` starts a `horizon-local-conn` thread on one end of an in-memory `Pipe` for the integrated server's own player and each split-screen player. Both run `lan/conn.rs` `serve_connection`.
2. *Login and configuration* run on that connection thread, blocking, in `login_and_configure`: it reads `cfg::Serverbound` packets in three loops (to `SelectKnownPacks`, to the `Pong`, to `FinishConfiguration`) and sends directly. No game state is reachable there; it sees only `Shared`. The mod list is compared here. A typed manifest exchange and any configuration-phase handler therefore need a hand-off to the server's main thread that does not exist today: stage 3 has to add it, and configuration completion has to wait on it.
3. *Play ingress.* `play` moves the socket to the I/O pool (`lan/io_pool.rs`, one thread per two cores, a connection pinned to one thread). `PlayConn::packet` decodes each frame with `play::Serverbound::decode` (which applies the directional custom-payload bound), counts it against `Undrained` (twice the inbox caps, else a kick) and sends `NetEvent::Packet` on one channel shared by all connections.
4. *Main thread.* `Hub::poll` (first level pass of a tick) moves `NetEvent::Packet` into the guest's `inbox` in arrival order (caps `INBOX_LIMIT`, `INBOX_BYTES`). `Hub::drain_packets` runs each guest's `process_inbox` and emits `HostEvent::Packet` in that order; `lan_process_events` calls `lan_guest_packet` for each, with the guest's state swapped in. Mod messages are not delivered there: `guest_native_net` only queues them, and `run_native_net` fires the hook after all of that pass's packets, with no player swapped in. So one connection's mod messages keep their own order but run after its gameplay packets of the same tick. The design's "same ingress queue" order needs the typed handler to run at the packet's place in `lan_process_events`, or the note to accept the current rule; stage 3 decides and tests it.
5. *Egress.* A hook's send is held in `NativeNet::to_players` until `flush_native_net` at the end of `tick_lan`, which hands bodies to each guest's `PlayerSender` (bounded `Outbox`); `lan::flush_all` then posts them to the I/O pool, which frames, compresses and writes. Frames are bounded by `frame::MAX_FRAME` (2,097,151 bytes) and `MAX_UNCOMPRESSED` (8,388,608) on sockets, not on the in-memory pipe.
6. *Reconfiguration.* This server never sends `StartConfiguration`. Only the client handles it, for other servers and proxies.

**Client.**

1. `net.rs` `Session::connect` and `connect_local` start one network thread that runs `run`. Its `configure` loop handles `cfg::Clientbound` on that thread: it answers the brand, sends the mod list, and reports `Event::ModNet` and the like to the game. `FinishConfiguration` is answered there too, so the game thread does not gate it today.
2. In play the thread forwards `Event::Packet`. `StartConfiguration` becomes `Event::Reconfiguring` and the thread configures again.
3. `poll_remote_events` on the game thread takes the events in order and calls `handle_packet`, which calls the mod hook at once (`remote_native_net`), in order with every other packet. The window, a split-screen player and a LAN guest all take this path; the integrated server's replies arrive through the pipe, never by a direct call.

**What the trace leaves to stage 3:** the configuration hand-off on both sides (server step 2, client step 1), the ordering choice in server step 4, and a reconfiguration path on the server if a payload ever needs one.

## Shared synchronization and clean break

Carry server config snapshots during configuration with schema and revision, validate atomically, acknowledge before admission, and discard on disconnect; never overwrite the client's local config file. Live changes publish a complete committed revision or trigger reconfiguration when required. No separate config transport.

Follow [PR #2328 (#796)](https://github.com/arismoko/horizon/pull/2328): attachment envelope includes world/dimension, holder incarnation, residency join, schema and committed revision. Full snapshot on tracking entry precedes deltas; send only to authorized trackers/owner. Stale joins/incarnations/revisions cannot mutate replicas. Gaps request a fresh bounded snapshot; visibility loss clears the replica. The proposed 64 KiB attachment value is subject to codec and frame limits including envelope overhead. Use bounded snapshots with atomic publication; chunk only when the complete envelope would exceed transport capacity. Assembly budgets must accommodate the declared snapshot and legal transport capacity, with timeout after 100 ticks; larger snapshots fail explicitly rather than imposing the earlier 256 KiB transport ceiling. Root rollback sends nothing. Client replicas never authorize server state changes.

Delete raw mod_net messages, net_send_to_* methods, net_from_* hooks, old transport advertising and separate attachment transport in the implementation change. Update catalog, generator, native and wasm consumers, tests, book, all old hook documentation, wasm coverage rows and horizon-machine-sample together. These are implementation prerequisites. No compatibility alias, fallback handler, migration shim or dual transport. Native and wasm have identical registration, dependencies, limits and semantics; wasm uses validated opaque tokens and copied bounded values, never guest pointers.

With no declarations, perform one empty-table check at connection setup, no mod manifest traffic, no codec calls, payload allocation or per-tick all-mod scans. Registered delivery costs one slot lookup plus work proportional to the payload and actual recipients. Attachment dirtiness must not trigger encoding when no negotiated recipient needs the value. No-listener means no callback fanout, not skipping validation of an already received transport envelope.

## Acceptance matrix

Required future tests, not runtime results of this note.

| Fixture | Concrete expected result |
| --- | --- |
| ToServer counter=7; reply ToClient counter=8 | Exactly one server handler sees 7 and one client handler sees 8, on respective main threads, native and wasm |
| Send ToClient through serverbound service | Compile-time refusal; forged direction invokes 0 handlers and disconnects sender |
| Two owners declare cells:update | Load fails with both owners; 0 installed duplicate slots |
| Required schema 2 versus peer schema 1 | Admission refused; optional mismatch returns UnsupportedPeer, 0 packets |
| Length -1, count 4097, depth 65, trailing byte | Reject before collection allocation; 0 callbacks; next valid independent session accepts counter=7 |
| Forged or stale wasm token; unsupported Wire type | Structured token refusal, 0 handlers/sends; unsupported type named at load on both targets |
| 33 sends in one tick | First 32 accepted within byte budgets; last Backpressure |
| Receive A=1, B=2, A=3 on one connection | Handler order 1,2,3 on integrated, LAN and dedicated |
| Handler sends reply then panics/traps | 0 replies published, transaction restored, callback disabled, session closed |
| Config revision 4 sets counter=7 | Client admits only after acknowledgement; local file unchanged; disconnect clears override |
| Attachment join 10 revision 11 value 8; late join 9 value 3 | Replica remains 8; non-trackers receive 0 updates |
| Root abort after attachment edit 7 to 9 | Authoritative value 7, 0 sync packets |
| 100 ticks with no declarations | 0 codec calls, typed packets and payload allocations |
| Generic send_packet for typed channel / unrelated plugin channel | First refused; second retains existing interop behavior |

## Proposed implementation stages

(1) Apply the accepted vanilla limits and object-safe service requirement; verify public 26.3 contract and complete socket/scheduler route trace. *Contract evidence recorded above.* (2) Share #811 codec primitives and generated native/wasm registration with bounded adversarial decoding fixtures. (3) Add configuration negotiation and ordered dispatch through integrated, LAN and dedicated routes. (4) Move config and #796 attachment sync to this transport with atomic snapshot tests. (5) Delete every raw caller/catalog surface and update samples in the same clean-break change; demonstrate the matrix and allocation measurements.

## Open questions

- **Shared codec ABI and custom allocation enforcement?** Recommendation: approve one generated bounded foundation with #811; require a reviewed accounting interface before accepting custom network codecs. Native sandboxing is not promised.
- **Required versus optional sync?** Recommendation: required for gameplay interpretation/config dependencies; optional cosmetic attachment replicas only when all consumers tolerate absence.
- **Budgets and snapshot size?** Only codec and frame limits apply to registered payloads. Size queue, decoding and assembly budgets to preserve that capacity; measure scheduling defaults with representative packs. Never silently truncate #796 values.
- **Network-thread handlers?** Recommendation: expose no mod network-thread handler initially; preserve main-thread Host transactions on both targets. Public counterpart differences need explicit review.
- **Order against gameplay packets?** The trace shows today's server runs a connection's mod messages after its gameplay packets of the same tick. Recommendation: deliver typed payloads at the packet's own place, as the design says, and test A=1, B=2, A=3 interleaved with a gameplay packet.
- **Configuration on the main thread?** Both sides finish configuration on a network thread today. Recommendation: stage 3 adds one bounded hand-off per side and makes completion wait for it, rather than running handlers on the connection thread.

Strings use a 4096 UTF-8 byte default, not a character count. A schema may pass
3N bytes to `str_within` for a vanilla N-character limit. Each box, list and
present option consumes one of 64 nesting levels; boxes charge their inner
allocation size before decoding.
