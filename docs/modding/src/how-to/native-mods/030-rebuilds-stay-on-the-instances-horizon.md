<!-- chapter: Rebuilds stay on the instance's Horizon -->
<!-- separator: 1 -->

## Rebuilds stay on the instance's Horizon

A native build uses the instance's pinned Horizon commit. Editing or disabling a mod rebuilds
the mods against that same prebuilt SDK. A manifest pin for another commit is refused; building
does not silently repin a mod.

`native-build --update` selects a newer installed release or Nightly. Update the mod's pins for
that commit and ensure its prebuilt SDK is published. `E_NATIVE_NO_RELEASE` means the launcher
cannot build native mods against that commit; use a commit with a published SDK.

The launcher uses its pinned Rust toolchain, not the player's Rust installation. It isolates
compiler flags, Cargo configuration and target settings from the player's environment. Build
inputs and outputs stay in managed launcher storage. On Windows the build tools require paths
the linker can open; the launcher chooses a managed ASCII path when necessary.
