<!-- chapter: Native entry point and diagnostics -->
<!-- separator: 1 -->

The generated executable uses the installed game's `crates/horizon-client/src/native_main.rs.template`.
The launcher supplies only ordered literal mod IDs and hook factory paths. The template owns entry
construction, the allocator and the canonical game identity. Source without this template is refused
before staging trait mods: update the game first.

A mod whose bootstrap cannot complete should call `cx.refuse("reason")` inside
`Hooks::register`, then return. This refuses only the calling mod's registration:
there is no mod ID argument. Its declarations from that registration (before or
after the call) are discarded. Later declarations do not add secondary validation
errors, and another mod's declarations are unchanged. Native and wasm report the
same diagnostic through the normal registration error path. This does not panic
or exit the process. The first reason wins, including an empty reason. Reasons
are preserved verbatim up to 4096 UTF-8 bytes; longer strings are truncated at the
last character boundary within that limit.

A panic in a trait mod's hook adds `in native mod <id>, hook <hook>` to the panic's message, in the crash report
and in the server thread's log line, with the `horizon-mod.toml` id. Each entry passed to
`entry::main_with` must bind its id and hooks together: `ModEntry::new("example", example::hooks())`.
The ordered list is `Mods = Arc<[ModEntry]>`; there is no separate naming step or unnamed entry.
Before installation, `mod_api::mods()` still returns an empty list. Parts retain their registering mod's
id for startup errors and hook panics. The game stops. A point listener's panic names its mod and
method; render hooks are not named yet.

A native runtime that hosts converted packages (Portizon's wasm packages, a Java interpreter's jars)
declares the packages it loaded with `mod_api::content::crash::ConvertedPackages::new(runtime_id, ids)` and
marks a guest call with `mod_api::content::crash::ConvertedPackage::enter(export, site)`. The scope nests,
restores the previous one on return and while a panic unwinds, and adds `in converted package <id>,
export <export>, call site <site>` below the runtime's `in native mod` line in the report. A package
id the runtime did not declare is refused, as is a scope entered while another mod's call is the
current one, so guest code cannot impersonate a mod the runtime did not load.

The guard is thread-affine (not Send or Sync), and attribution ends when the native frame
that entered it exits, even if the converted guard is retained. A nested native frame hides the
outer frame's package until it returns. Worker threads have no scope unless the runtime enters
a native call and package scope there. Registry ids are stored, not interned: entering a package
clones an owned id on every call (borrowed static ids do not allocate).

A code mod may listen at the game's injection points as a rare escape hatch, through a crate-root function
named by `[[inject]].with`, taking a `mod_points` record; the form
and its limits are in [Injections](../../injections.md).
