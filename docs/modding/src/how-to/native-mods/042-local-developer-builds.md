<!-- chapter: A local unsigned developer base -->
<!-- separator: 1 -->

## A local unsigned developer base

A fresh launcher home can use a game built from a trusted checkout without installing a signed release first.
This is a developer route, not a release channel: the launcher labels the base and its derived games
**Local unsigned developer build**, including the source commit. Automatic updates never replace it.
The game sends neither telemetry nor the live-count ping when launched on this base.

Commit tracked changes before building. From that clean checkout, build the game and its companion programs,
then compile the original Horizon exporter classes against your own pinned Minecraft installation:

```sh
cargo build -p horizon-client --bin horizon-client -p horizon-client-lib --bin horizon-ctl \
  -p horizon-prepare --bin horizon-prepare -p horizon-server --bin horizon-server
bundle=$(target/debug/horizon-prepare compile-exporters --root "$PWD" | tail -n 1)
mkdir -p exporters
cp -R "$bundle/server" "$bundle/client" "$bundle/manifest.json" exporters/
horizon-launcher developer-install "$PWD" --program "$PWD/target/debug/horizon-client" --trust-unsigned
```

`--trust-unsigned` permits **executing the checkout's build code**, not just copying the game. The command
runs the existing `cargo run -p xtask -- package` release packager. Trust both the checkout and the executable
with your computer's full rights. Missing companion programs, missing or incompatible exporters, dirty tracked
inputs and executables for another platform are refused. Nothing publishes a release or changes the selected base.

The returned `developer-…` tag hashes the packaged files and a complete tracked HEAD source snapshot. Select
that exact tag, prepare its resources from your own Minecraft installation, and pin an instance to it:

```sh
horizon-launcher settings selected-version DEVELOPER_TAG
horizon-launcher prepare --version DEVELOPER_TAG --minecraft YOUR_MINECRAFT_FOLDER
horizon-launcher instance create Development
horizon-launcher instance version Development DEVELOPER_TAG
horizon-launcher native-build YOUR_NATIVE_MOD_FOLDER --instance Development --trust-native-code --profile developer
```

Native builds use the saved local source snapshot, including commits never pushed to GitHub. They do not fetch
Horizon source from GitHub or silently fall back when that local snapshot is missing. A changed checkout needs
a new commit, rebuilt executable and another developer installation. `import-build --version DEVELOPER_TAG`
can also reuse this base; an unselected developer base is never chosen as an automatic fallback instead of a
release or Nightly. Remove it with the normal `horizon-launcher remove DEVELOPER_TAG` command when no longer needed.
