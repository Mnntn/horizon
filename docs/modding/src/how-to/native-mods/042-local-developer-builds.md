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

A native mod build still requires a published prebuilt SDK for the base's exact commit.
`E_NATIVE_NO_RELEASE` means that commit has no available SDK: pin a published commit instead.
The native mod builder does not compile the local checkout or use its source snapshot as an SDK.
For native mixins, the pinned source must also be available for copying the target functions.
