<!-- chapter: The typed API from mixin code -->
<!-- separator: 1 -->

### The typed API from mixin code

An `[[inject]]` listener can use the typed API: an API point's listener gets the side's `Host` and calls
attachments and handles through it. The listener contracts are in
[Injections](../../injections.md#declaration). Source-level helpers use the separate
[Mixins](../../mixins.md) mechanism and upstream's vocabulary.

A catalog hook's trait method, event and return make up its `hook_version` contract
([Named hook points](../../how-to/native-mods/011-named-hook-points.md#named-hook-points)). These per-hook contract versions are catalog metadata, not minimum game
versions. Catalog hooks, `Host` calls and internals `[[inject]]` declarations all build against the
[game's exact source revision](../../how-to/native-mods/005-native-mod-source-revisions.md#native-mod-source-revisions). Rebuild the mod when that contract changes.

Every `impl Hooks` must have `#[mod_api::horizon_mod]`. The attribute derives
subscriptions from the methods implemented in that block, so dispatch knows them at installation.
