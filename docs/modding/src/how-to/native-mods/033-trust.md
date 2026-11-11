<!-- chapter: Trust -->
<!-- separator: 1 -->

## Trust

- **A notice, then a warning.** The first native install shows a trust notice once. The command needs
  `--trust-native-code`; the window
  shows a warning in red before every build: the mods run with full access to the computer, and their ids and
  versions. The Mods page says who made each one, and "Not made or endorsed by the Horizon project" for any mod not
  Horizon's own.
- **The registry** (horizon-rust/horizon-mods) lists them as their own kind, `native-mod`, only after a maintainer
  has read the source and the repository they come from. Its entry names the GitHub repository (`repo`); each
  release of it with a `.hmod` asset is a version once the bundle passes the registry's checks, with no further
  review (nothing is built or run by the registry; the player's launcher builds it behind its warning).
  The `.hmod` contains the flat Rust project and may contain `assets/`, `data/` and an optional `icon.png`.
  The launcher checks the bundle without running anything. Native code is trusted. Mixins are code compiled into the game, with the same trust as internals.
  The file is pinned by SHA-256 like every registry file; its identity and exact build pins come from
  `horizon-mod.toml`. `revoked.toml` takes a bad release down.
  A project's kind never changes.
- **Isolation of data.** Native mods run only in the instance they are installed in, with its worlds, options and
  logs (the default Horizon instance's, when they are installed there); other instances play their own versions.
- What this does not do: sandbox the compiled game. horizon-mixins' checks stop mistakes, not a malicious helper
  ([security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md)).
