<!-- chapter: Rebuilds stay on the instance's Horizon -->
<!-- separator: 1 -->

## Rebuilds stay on the instance's Horizon

Game source edits below come from mixins and their support tables, not injects.
An `[[inject]]` fills a compiled slot or is refused; it never compiles game code.

A build starts from the version the instance plays (`native_mods::base_for`). When that is a native build, the
next one compiles the same commit (its `native-mods.json`'s `horizon`) from the same game files, unless a mod pins
another, so a rebuild after a mod changes or is switched off compiles only what changed. A newer Horizon is taken
only when the player asks: `native-build --update` starts from the newest installed release or Nightly (a full
rebuild); `native_mods::update_for` says which one, if any, the window can offer.

The compiler runs as the release's build does, whatever the player's own Rust setup: the launcher's toolchain
drops the variables that change compiler flags or the target (`RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`,
`RUSTDOCFLAGS`, `CARGO_INCREMENTAL`, `CARGO_PROFILE_*`, `CARGO_BUILD_*` but `CARGO_BUILD_JOBS`, `CARGO_TARGET_*`,
rustup's and the wrappers'), and starts Cargo from the disk's root with `--manifest-path`. Cargo reads `.cargo/config.toml` from
the folder it starts in and every one above it, whatever `CARGO_HOME` says, so from the launcher's data folder it
would read the player's `~/.cargo/config.toml`, whose `[profile]`, `[build]` and `[target]` settings beat the
workspace and the environment; no switch turns that search off. The source's own `.cargo/config.toml`, if it
has one, is passed with `--config`.

Every path the build's tools see — the toolchain, the Cargo home, `native/` and the tools' temp dir (TMPDIR,
TMP and TEMP, where gcc, the assembler, `ld` and `cargo install` put their scratch files) — comes from
`backend::build_dir`: the launcher's data folder, resolved (never through a link, which horizon-mixins
refuses). On Windows, when that path is not spelled in ASCII — a Cyrillic user name in `%APPDATA%`, which
MinGW-w64's tools can't open — it is a real folder under `ProgramData` named for it (`horizon-launcher-<id>`),
and only when that can't be created the data folder's short (8.3) name. A short name alone is not enough
(#6009): rustc canonicalizes its own path to find its sysroot, which spells the long name again for the
linker. A data folder that is itself a junction to a drive with an ASCII path resolves to that and needs
neither.
