<!-- chapter: What the launcher does -->
<!-- separator: 1 -->

## What the launcher does

Native mods run with your rights. Read their source and dependencies before allowing native code.
Install them into the selected instance from the Mods page, then choose **Build**, or run:

```sh
horizon-launcher native-build path/to/mod --instance INSTANCE --trust-native-code
```

Omit the folder to build the instance's enabled native mods. The launcher keeps the instance on
its pinned Horizon commit; `--update` selects a newer installed base and rebuilds the mods against
that commit's SDK.

The build has one route:

1. Copy the mod projects into an immutable incoming directory and validate their pins and dependencies.
2. Download the pinned prebuilt game SDK and verify its checksums and compiler identity. Install the
   launcher's pinned Rust toolchain and required linker tools if needed; the player's Rust is not used.
3. Compile the mods against the SDK's game libraries. Registry dependencies use the mod's supplied
   lockfile and the same immutable SDK. The launcher asks for permission before executing dependency
   build scripts or procedural macros.
4. For `[[native.mixin]]` rows, download the pinned game source, copy the targeted functions into one patch
   library, apply the rows with horizon-mixins, and compile that library against the SDK. The game
   crates are not compiled. Each checked replacement is installed through its typed installer.
5. Link the native executable, install it as `native-<commit>-<key>`, and select it for the instance.
   `native-mods.json` records the base, pins, mods and build tools. Client builds share their base's
   prepared resources; server exports include their resources.

A matching installed build is reused. A failed build stops with its diagnostic; it never compiles
the game as a fallback. The build log is `native/build.log`. The Mods page shows the running step
and lets you cancel it.

### Build refusals

| Error | What to do |
|---|---|
| `E_NATIVE_BUILD_SCRIPT` | Remove the mod's build script and ship its generated Rust source. `build = false` disables an incidental `build.rs`. Registry dependency build scripts are handled by the separate dependency permission prompt. |
| `E_NATIVE_FEATURES` | Use the pinned SDK's feature set; remove mod feature declarations and custom SDK feature requests. |
| `E_NATIVE_HOST` | Build on a host supported by the prebuilt SDK. |
| `E_NATIVE_NO_RELEASE` | Pin a commit with a published prebuilt SDK. |
| `E_MIXIN_STATE_LAYOUT` | Store per-object mod data in attachments instead of declaring `[[native.state]]`. |

Unavailable downloads can be retried. Missing SDKs, incompatible compiler identities and unsupported
transformations are refusals, not reasons to build the game locally.
