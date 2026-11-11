# Native mods

Native mod docs live in the modding book: [native mods](modding/src/how-to/native-mods.md), [trait mods, Parts and attachments](modding/src/concepts/trait-mods.md), [injections](modding/src/injections.md) and the [hook notes](https://arismoko.github.io/horizon/reference/hook-notes.html).

`horizon-launcher native-build <folder>` uses the same automatic build-profile
picker as the Mods page. A folder alone does not force the instance's saved
profile: API-only mods use Optimized and a matching prebuilt SDK when available.
Structural rebuild requirements still use the instance's saved picker choice.
Pass `--profile optimized`, `--profile balanced` or `--profile developer` to force
and remember that profile for the instance.

### CLI profile regression test

The ignored `native_build_profile` integration test runs the actual launcher and
SDK planner, with fresh scratch game/launcher homes and cold caches for every case.
It needs a real installed signed release directory (named for its version, with
its `.installed` marker) and an API-only native mod folder compatible with that
release. The release must have a matching published SDK for the host; the mod must
not require jars, registry consent, mixins or other source-only features. Neither
fixture is modified or committed. Network access is required to fetch and validate the SDK. Each case stops after
the real planner emits its route, before compilation begins.

```sh
HORIZON_NATIVE_BUILD_TEST_RELEASE=/path/to/launcher/versions/v2026.11.10 \
HORIZON_NATIVE_BUILD_TEST_MOD=/path/to/api-only-mod \
cargo test --manifest-path launcher/Cargo.toml --test native_build_profile -- --ignored --nocapture
```

The test checks route selection and remembered profile preferences. In particular, the folder-only case starts
with saved Balanced and must select the prebuilt Optimized route; treating the
folder as an explicit profile makes that assertion fail.

## Prebuilt SDK layout

The v4 prebuilt archive separates target artifacts (`deps/`) from host proc-macro
libraries (`host/`). Cargo builds with an explicit `--target <rustc host triple>`,
so host build dependencies cannot introduce a second metadata variant into the
target SDK. Only host proc-macro dynamic libraries are shipped, not their host
dependency metadata. Consumers pass both folders as `-L dependency` paths; named
extern and facade lookups use only `deps/`. `native/` still holds native linker
inputs. The identity manifest hashes files in all three folders.

Target compilation retains `-C prefer-dynamic` (and `-C rpath` on macOS), while
Cargo's explicit target keeps those flags off host proc-macros. Multiple versions
of a target dependency are legitimate: uniqueness is required only when selecting
a named extern or publishing an internals facade, never across the whole SDK.

To rebuild and prove the archive with the launcher's pinned Rust toolchain active:

```sh
python3 tools/prebuilt_game.py build prebuilt
python3 tools/prebuilt_game.py smoke prebuilt
python3 tools/prebuilt_game.py exports prebuilt
python3 tools/native_dylib_probe.py --sdk prebuilt --output target/native-dylib-probe
```

For a server archive, add `--server` to the build command. Use a fresh
`CARGO_TARGET_DIR` when collecting a clean-build proof; all commands must use the
same compiler as `launcher/src/rust_toolchain.rs`.
