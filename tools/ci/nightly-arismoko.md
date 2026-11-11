## Nightly from arismoko/horizon

This is the modding fork at [arismoko/horizon](https://github.com/arismoko/horizon), not an official upstream Horizon release. Use the [fork launcher for Linux x86_64 or Windows x64](https://github.com/arismoko/horizon/releases?q=launcher%40&expanded=true) to install these signed game builds. The official Horizon launcher does not install them.

The fork launcher has separate settings, game data, accounts and updates. Microsoft sign-in is configured using upstream's application with permission; signed Nightly Play requires an account that owns Minecraft. Windows launcher packages are not Authenticode-signed: expect SmartScreen or unknown-publisher warnings.

The game archives below are not launcher installers. They include CI-tested game builds, a dedicated Linux server and the exact source pin for native mods. Compiled SDKs are separate `prebuilt-<full source SHA>` releases and must match the installed game's source revision; the source archive and `mod-catalog.json` are not compiled SDKs.

For client development without sign-in, [build an unsigned developer base from source](https://github.com/arismoko/horizon/blob/main/docs/modding/src/how-to/native-mods/042-local-developer-builds.md) and opt into single-player-only Play with `HORIZON_LAUNCHER_DEVELOPER_OFFLINE=1`. This does not enable signed Nightly Play or account features.

Report bugs to this fork with the exact Nightly date and commit. You need your own copy of Minecraft: Java Edition.
