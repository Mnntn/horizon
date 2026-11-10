<!-- chapter: What the launcher does -->
<!-- separator: 1 -->

## What the launcher does

**Developing a mod?** Pass `--profile developer` to `horizon-launcher native-build`.
The choice is remembered for that instance, not shared with other instances.
Developer builds the game crate unoptimized for iteration rather than game performance. Use it while developing, not for playing. A needed build picker starts at
Balanced until a choice is saved; automatic prebuilt builds still use the release profile.

In the window: install a native mod from the Mods page, into the instance picked on the left ("Installs into
{instance}", shown even with one instance). The first one shows a trust notice once (native mods aren't sandboxed;
a build may download Rust and other build tools); `mods install` prints it too. Each
installed native mod's row says "Installed in {instance}" and has **Open folder** (`<instance>/horizon-native-mods/`).
The installed list has a **Native build of {instance}** row for the instance picked on the left, never another's:

- A badge: **Not built** (the version it plays isn't a `native-*` build), **Needs rebuild** (the `mods` in its
  `native-mods.json` differ from those turned on, by id and version) or **Up to date**, with **Build** for the first
  two;
- For a native build, "Plays native-…, built on Horizon {version}" (the `base` of its `native-mods.json`);

The instance list on the left shows such an instance as "Native build · Nightly 10-28" (its base). The launcher names
the mods, warns in red, and builds only from "I trust these mods: build"; the row shows each step and the error if
one fails, on the instance being built only; one build runs at a time. From the command line:
`horizon-launcher native-build [FOLDER] [--instance NAME] [--update] --trust-native-code`. FOLDER is a bundle, an
unpacked `.hmod`, or a folder of them; without it, the native mods turned on in the instance. It is not an MCP tool:
trusting native code is the player's decision, never an agent's.

1. **Rust.** The launcher's own toolchain ([`rust_toolchain`](../../../../../launcher/src/rust_toolchain.rs)): Rust 1.99.1's
   official `rustc`, `cargo` and `rust-std` (and `rust-mingw` on Windows), each pinned by SHA-256, downloaded once
   from static.rust-lang.org into `<launcher data>/rust/`, with its own Cargo home `<launcher data>/cargo/`. No
   rustup, nothing on the system; the player's own Rust is never used. It ships for macOS (Apple Silicon and Intel), Linux
   x86-64 and Windows x64. Windows uses the `windows-gnu` host, which links without Visual Studio. `rust-mingw` only links, so on Windows the toolchain also brings MinGW-w64
   (`rust_toolchain::MINGW`: WinLibs' GCC 16.2.0 on MinGW-w64 14.0.0, MSVCRT like `rust-mingw`, one zip pinned by
   SHA-256, its `mingw64/` without the documentation) on the build's `PATH`: `ring` and `hidapi` compile C,
   and `windows-sys`' `raw-dylib` imports need `dlltool` and the assembler it runs. The linker is the system's on
   macOS (the Command Line Tools: the launcher asks for `xcode-select --install`) and Linux (`cc`). On Linux the
   game also builds against the system's ALSA (`alsa-sys`) and libudev (`libudev-sys`, `hidapi`), found with
   `pkg-config`, whose development files desktop distributions don't install. Before downloading anything the
   launcher checks for `cc`, the C library's headers (`cc` compiles an include of `stdint.h`), `pkg-config`,
   `alsa` and `libudev` and names what is missing, with the install command for the distribution in
   `/etc/os-release` (Debian, Ubuntu and Mint:
   `sudo apt install build-essential pkg-config libasound2-dev libudev-dev`). SteamOS has GCC without the C
   library's headers, and a read-only system: native builds don't work there yet.
   On Windows, Smart App Control can block newly compiled build scripts or toolchain programs with
   `os error 4551`. The launcher adds advice when it sees that error; it does not check the setting
   before a build. To build after a block, Windows Security > App & browser control > Smart App Control
   settings offers an off switch. On some Windows versions, turning it back on requires reinstalling
   Windows. Content mods need no build and work with Smart App Control on.
   `horizon-launcher rust status|install|remove`.
2. **horizon-mixins**, at the exact commit the mods name (`[native] mixins` in `horizon-mod.toml`),
   built once with that cargo (`cargo install --locked --git … --rev …`) into `native/tools/<commit>/`.
3. **The source** of the pinned commit: GitHub's archive of it, once per commit (`native/source/<commit>/`, about
   63 MB unpacked), with the commit in `.horizon-revision`. Never the player's own checkout. Links in the
   archive are left out: the tool refuses them and nothing needs them to build.
4. **Stage.** The projects are copied first, so the build and its key use that copy whatever happens to the
   folder meanwhile. Compiled slot listeners register without a source splice. For source-routed internals
   declarations the launcher generates private slots and source adapters before compilation.
   It stages the mod crates and generated executable, copying only changed files into `native/stage/`,
   so Cargo, which goes by file times, recompiles only the crates the mods touch.
   Each file written gets the current time, never the archive's (the commit's) or the copy's original: on
   Windows and macOS a copy keeps it, and a file older than the last build reads as already compiled, so
   switching Nightlies linked the previous one's crates (horizon-rust/horizon#10421). A stage from before
   that fix is written again once (`native/stage.stamped` marks it). Registry dependencies
   are checked against Horizon's `Cargo.lock` before Cargo resolves the staged workspace ([The mod's crate](016-the-mods-crate.md#the-mods-crate)).
5. **Compile**: `cargo build --locked --profile <fast|release> [--config ...] [--features horizon-client/mods] -p
   horizon-native-game`, into `native/target/`, with one of three build modes:

   | Mode | Cargo profile | Use |
   |---|---|---|
   | `optimized` | `release`, the release's own (game-build.yml: thin LTO, one codegen unit) | The mode for playing: mods that need nothing but the mod API compile against the prebuilt game, built with this profile; any other set builds the whole game from source with it |
   | `balanced` (initial picker choice) | `fast`, the workspace's own: `codegen-units = 16`, incremental, and `lto = false`, Cargo's default, which is thin-local LTO across one crate's codegen units, none across crates | Always builds the game on this computer: a rebuild recompiles and links only what changed, and the game plays a little slower than Optimized |
   | `developer` | `fast` with `horizon-client` at opt-level 0 (`--config profile.fast.package.horizon-client.opt-level=0`) | Developing a mod: the quickest rebuild after an edit, but the game runs about twice as slow, so iterate with it rather than play |

   Only Optimized uses the prebuilt game. The Mods page shows how each native mod reaches the game: a bolt for
   the prebuilt game, a hammer for a full game build (its menu names the reason), a check when the game already has
   the mod. A build under way shows its stage and can be cancelled.

   `dev` lowers only `horizon-client`, so an edit in another workspace
   crate (`horizon-client-lib`, `-render`, `-world`) still compiles at opt-level 3, as with Balanced. The
   launcher sets it with `--config` (`native_mods::Profile`), so the workspace's own `[profile.fast]` builds stay
   as they are. The Mods page's "Build mode" card or `native-build --profile ...` picks and remembers the
   instance's choice. Without a source-build reason or explicit CLI choice, installed mods use the automatic
   prebuilt route; a saved preference is applied only when a picker is needed. `native-mods.json` records the
   effective profile.

   Each profile is in the build's key, so their builds never mix and may all stay. `balanced` and `developer` share
   `native/target/fast/`: every crate but `horizon-client` is built with the same settings, so switching between
   them recompiles only `horizon-client`, and Cargo keeps both builds of it, so switching back finds the other
   still there. `optimized` has its own folder (`native/target/release/`). The first build in a folder builds every
   crate once; later builds reuse them.
   Cargo downloads the locked crates from crates.io (Horizon has no git
   dependencies). The compiler and the horizon-mixins build run at the lowest priority (`rust_toolchain::low_priority`:
   nice 19 and the idle I/O class on Linux, nice 19 on macOS, below-normal priority on Windows), so the desktop and a
   game in play stay smooth; on an otherwise idle computer they still use every core. The log is `native/build.log`;
   a failure shows its last lines (a horizon-mixins diagnostic in one line).
6. **Install**: a new version, `native-<commit>-<key>`, a copy of the starting version's folder (resources
   included, cloned on APFS, Btrfs and ReFS) with the built game in place of its own, and a `native-mods.json`
   saying which release or Nightly it started from (`base`) and which commit, mods, horizon-mixins and Rust made
   it. Canonical mod projects are copied into `horizon-native-mods/<id>/` beside the executable.
   The game lists its embedded inventory in a crash report's details (`Native Mods: id version, …`;
   they stay in the local report). The instance the mods are installed in plays it, the default **Horizon** instance included: the build
   pins its version, as picking a version does. Updates of the launcher's version leave a pinned instance where
   it is, and their clean-up never removes a native build. The **Native mods**
   instance keeps its worlds and its build.
