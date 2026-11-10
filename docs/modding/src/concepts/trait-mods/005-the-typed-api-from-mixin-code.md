<!-- chapter: The typed API from mixin code -->
<!-- separator: 1 -->

### The typed API from mixin code

An `[[inject]]` listener can use the typed API: an API point's listener gets the side's `Host` and calls
attachments and handles through it. The worked examples, and which shape to choose, are in
[Injections](../../injections.md#make-your-injection-convertible); each mixin injector's form is in
[From mixins and legacy kinds](../../injections.md#from-mixins-and-legacy-kinds).

A catalog hook's trait method, event and return make up its `hook_version` contract
([Named hook points](../../how-to/native-mods/011-named-hook-points.md#named-hook-points)). These per-hook contract versions are catalog metadata, not minimum game
versions. Catalog hooks, `Host` calls and internals `[[inject]]` declarations all build against the
[game's exact source revision](../../how-to/native-mods/005-native-mod-source-revisions.md#native-mod-source-revisions). Rebuild the mod when that contract changes.

Every `impl Hooks` must have `#[mod_api::horizon_mod]`. The attribute derives
subscriptions from the methods implemented in that block, so dispatch knows them at installation.
