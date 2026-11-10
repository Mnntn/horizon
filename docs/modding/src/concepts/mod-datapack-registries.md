# Design: mod datapack registries

Provisional contract for [#811](https://github.com/arismoko/horizon/issues/811), not an implemented API. Ari's side posts the decision on the issue. This note files no issues or PRs.

## Decision: decoder and generation ownership

The declaring mod owns the typed codec and its schema. The game owns decoding orchestration, candidate storage, reference resolution, validation, generation assignment and atomic publication. A decoder never installs values into live state. The runtime retains its callable only while the declaring mod is loaded; unloading requires closing its worlds first. No raw native pointer or wasm guest address is a registry value in host storage.

Propose one load-time registrar, corresponding to DataPackRegistryEvent.NewRegistry, with the appropriate Host and a scoped registrar. Illustrative declaration: `example:spell`, type `Spell`, data codec `{power: integer 0..100}`, optional network codec and explicit client requirement. Names and signatures await API review. Require namespaced registry keys owned by the declaring mod; reject a second declaration even with an identical codec, reporting both owners. Freeze declarations after dependency-ordered loading. Other mods and packs may contribute entry names in their own namespaces. Entry path for `example:spell`, entry `addon:fire`, is `data/addon/example/spell/fire.json`; pin this proposed mapping in tests before implementation. The namespaced registry folder is NeoForge's rule for datapack registries outside `minecraft` (its Registries guide, section "Datapack Registries"); vanilla's own registries use the registry's path only, and registration refuses `minecraft` keys, so the vanilla form never applies to a mod's registry.

Codecs consume bounded immutable data plus a candidate registry lookup, returning a typed immutable value or structured error. No world mutation, I/O, retained Host, scheduling or active-generation lookup during decoding. Resolve references against the complete candidate graph, including forward references; reject unresolved references and unsupported cycles. Use one generated codec/value representation for native and wasm, with typed reads through registry handles. Erasure is internal and checked against the declaration's type token; no unchecked downcasts or guest memory retained after calls.

## Evidence and limits

Vanilla 26.3 was inspected only through `cd /workspace/horizon && python3 tools/rea_vanilla.py method net.minecraft.resources.RegistryDataLoader load --overload 0`. It accepts context lookups, registry descriptors and an executor, creates resource-manager load tasks per descriptor, and delegates to the common loader returning a future of frozen registry access. This supports descriptor-driven candidate loading; it does not prove arbitrary mod registration, reload scope or failure atomicity. Those are Horizon proposals here. Vanilla caller coverage and the distinction between world-generation registries and reloadable registries still need oracle fixtures before implementation.

The [public registry guide](https://docs.neoforged.net/docs/concepts/registries/) was reachable but labels itself **26.1**. It is background only, not proof of 26.3 behavior. The [26.3 NewRegistry Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/neoforged/neoforge/registries/DataPackRegistryEvent.NewRegistry.html) was requested and inaccessible through the reader. Exact overloads, network codec semantics, absent-mod behavior, reload guarantees and failure policy are **unverified**. The issue names the counterpart and asks for declarations and entry decoders. No NeoForge source, patches or other project's source was read. Do not advertise exact counterpart equivalence until public documentation verifies it.

## Decode, commit and retained identity

Decode after pack selection and overlay precedence have selected one winning file per entry, before any live table or network publication. Reuse existing pack conditions and overrides; never invent another pack scanner. Include prepared base data even when there are no overlay files: the early return in build_world_tables must not skip declared registries. Report mod, registry, entry, pack, file and field path for invalid data. Duplicate declarations are registration errors; multiple pack definitions are ordinary priority overrides. Invalid winning data must fail, never fall back to a lower-priority value. Initial failure prevents world admission. Reload failure retains all old tables, registry values, generations and client snapshots.

Use a world/session identity plus monotonically increasing u64 registry-set generation. Initial successful publication is generation 1, every successful reload increments once, including identical content. Refuse exhaustion instead of wrapping. Failed reload does not increment. A handle contains session, registry type, generation and entry identity. All access and use checks them against the active set: generation 1 handles and held entry wrappers return StaleGeneration after generation 2, even if names and slot numbers match. Deleted entry lookup returns MissingEntry. Closing and reopening a world invalidates session identity. A retained entry wrapper must validate on every read; do not expose an Arc or reference that bypasses validation. Explicit copied plain data is a snapshot, carries no live entry authority, and cannot be passed as a current entry. Persist names, never session slots; re-resolve explicitly after load.

Prepare codecs, references, derived tables and wire payloads before commit. Publish the registry set with WorldTables and dependent data under one coordinator at a safe server boundary. Consumers cannot observe mixed generations. Existing reload-listener apply callbacks run before world preparation today; implementation must move fallible preparation before effects or constrain callbacks to candidate-local effects, rather than claiming existing ordering provides rollback. Listener panic or guest trap aborts the candidate; no partial publication. Catch native unwind at the boundary, disable/report the failing callable under runtime policy; process-aborting panics cannot be recovered.

## Verified Horizon route owners

Paths and lines are inspected at base `8b736c3cbc`; these identify integration owners, not implementation claims.

| Route | Owner and required treatment |
| --- | --- |
| World open, including local/integrated and dedicated Game entry | `crates/horizon-client/src/game.rs:11805` selects packs; line 11808 calls build_world_tables. Prepare generation 1 here before admission. |
| Shared table builder | `crates/horizon-client/src/game/world/world_entry.rs:218` owns build_world_tables and overlays; line 511 reuses it for reload. Add candidate registries to its prepared result. |
| Explicit reload and datapack selection | `crates/horizon-client/src/game/administration/functions.rs:208` owns reload_datapacks; line 948 owns cmd_reload; lines 253 and 278 prepare/install world tables. All run_load authoritative routes share this coordinator. |
| Content inventory changes | `crates/horizon-client/src/game/modding/mods/mod.rs:1329` calls reload_datapacks(true). Declaration changes cannot sneak through this route: require a new mod-load session; data-only changes use normal reload. |
| Additional content refresh | `crates/horizon-client/src/game/administration/functions.rs:1281` prepares listeners and line 1301 prepares world reload. It must use the same candidate/commit boundary, not a second generation allocator. |
| Table publication | `crates/horizon-client/src/game/world/world_entry.rs:337` owns install_world_tables; line 505 owns prepare_world_reload, including wire preparation. Extend both, keeping failure before publication. |
| Reload callbacks | `crates/horizon-client/src/game/world/server_reload/mod.rs:52,100,117` own prepare, apply callbacks and store admission. Enforce the candidate ordering described above. |
| Remote client | reload_datapacks at `crates/horizon-client/src/game/administration/functions.rs:251` excludes remote worlds from authoritative preparation. Remote clients receive server candidates; never decode local datapacks as server authority. |
| Join negotiation and send | `crates/horizon-client/src/lan/conn.rs:1048,1084,1150` collect/decode/compare mod lists; line 1222 sends RegistryData. Extend negotiation before sending custom entries. |
| Client resource reload | `crates/horizon-client/src/game/world/packs/mod.rs:240` owns reload_packs. Resource-only refresh must not advance authoritative datapack generations. |

This is the inspected production route inventory; tests also call the shared functions directly. Exact receive-side configuration commit and all vanilla callers remain implementation trace prerequisites; packet tests alone are not proof of client publication coverage.

## Clients and absence negotiation

Propose three explicit modes: server-only (no network codec), synchronized-required, synchronized-optional. A network codec defines a bounded client projection, which may omit server secrets. Negotiate registry key, declaring mod, schema fingerprint and mode before any payload. Required missing mod or incompatible codec refuses join with a precise reason before entry decoding. Optional absent client mod receives no payload and gains no fabricated empty registry; server gameplay may not require that client's interpretation. Server-only registries do not require the client mod merely because they exist. Reconcile this with current mod-list comparison so blanket mismatch refusal does not silently defeat optionality; preserve unrelated dependency requirements.

Server is authoritative. Send a full sorted snapshot initially and after reload with session, generation, entry keys and bounded projected values. Client decodes and validates off to the side, acknowledges readiness, then activates at the agreed boundary; generation-tagged traffic is held until activation. Failed required-client decode disconnects that client without exposing a partial set; successful server reload need not roll back for a disconnected peer. Never send generation-2 references to a peer still using generation 1. Late joins receive only the current complete generation. Network numeric slots are scoped to this snapshot and never persisted. Transport and configuration re-entry details require protocol review; do not create a parallel registry packet store.

## Existing registrars, runtime, and cost

Existing register_* hooks declare built-in content or codec types at mod load; they do not become per-reload callbacks. The new registrar declares the registry schema once; loading its JSON happens each candidate build. add_server_reload_listeners remains for independent resource consumers, not a second way to declare this registry or publish entries. Data maps annotate existing entries and remain a separate feature.

`crates/horizon-mod-runtime/src/registry_keys.rs:6` owns load-time Requests and line 18 owns Names, including requested slots and resolved maps. These tables are name/slot machinery, not typed datapack value storage or a reload generation authority. Reuse their validated key resolution where appropriate; place immutable per-world generation sets under the world coordinator. Keep mod-runtime engine-independent. Generate native and wasm registration, codec calls, typed reads and refusal errors together; both use the same dependency ordering and semantics. Native exact-build checks and wasm interface checks remain their existing admission rules.

Treat files, decoder results and network bytes as untrusted. Proposed hard defaults: 1 MiB per encoded entry, depth 64, 65,536 entries per registry, 64 MiB aggregate encoded candidate data, and 256 registries per session. Check lengths before allocation, decoded collection counts as well as encoded sizes, identifiers, finite floats, schema ranges, reference count and total decoded memory budget. Bound decoded memory to 128 MiB per candidate and callback recursion to 64; wasm additionally consumes its existing fuel/memory limits. Native callbacks require trusted executable admission and cannot be forcibly time-limited safely; document this limit. Over-budget candidates fail wholly, never truncate. Refuse only custom registry keys owned by this feature; unrelated unknown-key behavior remains with its existing owner.

Resolve declarations once. With none, skip custom scanning, codec invocation, candidate allocations and synchronization extensions; no per-tick mod scans. Reads validate session/generation in constant time and perform indexed lookup. Reload work scales with selected entries and references, not ticks. Measure the empty declaration path and native/wasm bounds independently.

## Acceptance matrix for implementation

Expected values below are proposals, not tests reported passing.

| Fixture | Expected value |
| --- | --- |
| Test mod declares example:spell; base fire power=3, higher pack power=7 | Typed fire.power=7, generation=1, exactly one selected entry. |
| Reload override to power=9 | New read=9, generation=2; held generation-1 handle and entry both return StaleGeneration. |
| Winning override power=101 or malformed JSON | Reload fails with pack/file/field error; power=7 and generation=1 remain; no snapshot sent. |
| Two mods declare example:spell | Admission fails with both owner names; zero published custom registries. |
| Entry refers forward to addon:ice, power=4 | Resolves after candidate completion; absent ice refuses entire candidate. |
| Remove fire on successful reload | Fresh lookup=MissingEntry; old handle=StaleGeneration; generation=2. |
| Identical successful reload; failed reload afterwards | Generation advances 1 to 2 once; failure leaves 2. |
| Close world then reopen with same entry and slot | Old wrapper=WrongSession; new generation=1 with distinct session. |
| Required client mod missing/schema mismatch | Join refused before payload; zero custom entries installed. |
| Optional client mod absent; server-only registry | Zero bytes for omitted registries, no empty fake registry; join succeeds if other requirements pass. |
| Present native and wasm client, power projection=7 | Both read 7 in generation 1; activation after full decode only. |
| Client reload decode traps | Zero partial entries; client disconnected; server retains committed generation 2. |
| 1,048,577-byte entry, depth 65, 65,537 entries, NaN result | Each candidate refused; old generation unchanged; subsequent valid reload succeeds. |
| Native unwind or wasm decoder trap | Candidate discarded, no sends, old value=7; next valid candidate can publish. |
| No declaration; resource-only reload | Zero custom decode calls/allocations/sends; datapack generation unchanged by resource refresh. |
| Every authoritative route above with override=9 | Exactly one publication and generation increment; remote local packs cannot replace server value. |

## Proposed stages

File none. Stage 1: obtain exact public 26.3 contract and vanilla load/reload caller fixtures; settle the decisions below. Stage 2: generated registrar and bounded codec representation for native and wasm, duplicate/type validation and test-mod fixtures. Stage 3: candidate ownership, generation-checked reads and common atomic coordinator across every listed route, including listener failure. Stage 4: negotiation, snapshot transport, receive-side activation and absent-mod fixtures. Stage 5: acceptance matrix, route parity, adversarial limits and empty-path allocation measurements. No stage authorizes Rust work in this documentation task.

## Open questions

- What does the exact 26.3 counterpart guarantee? Recommendation: require reachable public Javadoc before implementation; retain explicit Horizon proposals where public documentation is silent.
- Which registry categories reload? Recommendation: custom registries here always replace on reload; do not infer that vanilla world-generation registries also reload. Verify their distinction with oracle fixtures.
- Shared codec API with #813? Recommendation: use one generated bounded value/codec foundation, with separate registry lifecycle ownership; avoid a second transport representation.
- How should callback ordering become atomic? Recommendation: complete all fallible world and registry preparation before effectful listener apply, and require candidate-local writes until publication; review existing callback semantics before changing them.
- Are optional synchronized registries safe for all consumers? Recommendation: permit omission only through explicit declaration and negotiation; required gameplay interpretation must select required mode.
- Are proposed budgets appropriate? Recommendation: retain the concrete limits above as provisional defaults, measure representative packs, and approve changes before freezing the API; never accept unbounded values.
- Exact client receive and reload activation owner? Recommendation: trace configuration collector through world admission and reconnect/reconfiguration before coding transport, then add verified file:line owners and tests to this note.
