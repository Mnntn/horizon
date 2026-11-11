# Native mods

Native mod docs live in the modding book: [native mods](modding/src/how-to/native-mods.md), [trait mods, Parts and attachments](modding/src/concepts/trait-mods.md), [injections](modding/src/injections.md) and the [hook notes](https://arismoko.github.io/horizon/reference/hook-notes.html).

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
