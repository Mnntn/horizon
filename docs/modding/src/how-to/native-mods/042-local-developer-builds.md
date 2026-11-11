<!-- chapter: A local unsigned developer base -->
<!-- separator: 1 -->

## A local unsigned developer base

`developer-install` installs a game executable built from a trusted checkout. This is a separate
launcher installation command, not a native mod build route. It runs the checkout's packaging
code with your rights and requires `--trust-unsigned`.

Commit tracked changes, build the game and companion programs, and prepare compatible exporters
from your own Minecraft installation before installing:

```sh
horizon-launcher developer-install CHECKOUT --program CHECKOUT/target/debug/horizon-client --trust-unsigned
```

The command returns a `developer-…` tag. Select that exact tag, prepare its resources, and pin an
instance to it. Automatic updates do not replace a local unsigned base. Remove it with
`horizon-launcher remove DEVELOPER_TAG` when no instance needs it.

A native mod build needs the prebuilt SDK of the base's exact commit. A local commit has no
published one, so build it from the same checkout and name its folder; one folder holds one side:

```sh
python3 tools/prebuilt_game.py build SDK_FOLDER            # add --server for a dedicated server
HORIZON_LAUNCHER_DEVELOPER_SDK=SDK_FOLDER horizon-launcher native-build MODS --version DEVELOPER_TAG --trust-native-code
```

The variable is read for developer bases only, and the folder is used where it is. It must be
that commit's build for this computer, side and the launcher's Rust, as a downloaded SDK must;
anything else is refused. Without it, `E_NATIVE_NO_RELEASE` means the commit has no available
SDK. The native mod builder never compiles the checkout itself. For native mixins, the pinned
source must also be available for copying the target functions.
